use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRootSet, Gc, Rootable, Static};
use smallvec::SmallVec;
use std::collections::BTreeMap;
use std::mem::size_of;

use crate::bytecode::{stable_bytes_hash, SharedProgramString};
use crate::number::table_key_number_bits;
use crate::value::{ShortString, Value, ValueKind};

const MAX_LUAU_ARRAY_INDEX: usize = 2_147_483_647;
const TABLE_HASH_INDEX_MIN_LEN: usize = 16;
const STRING_INDEX_MIN_CAPACITY: usize = 16;
const SLOT_INDEX_BITS: u32 = 24;
const SLOT_INDEX_MASK: u32 = (1 << SLOT_INDEX_BITS) - 1;
const SLOT_GENERATION_SHIFT: u32 = SLOT_INDEX_BITS;
const SLOT_MAX_GENERATION: u8 = u8::MAX;

/// A compact generational slot store.
///
/// Handles use 24 index bits and 8 generation bits. A slot is permanently
/// retired after generation 255 instead of wrapping, so no stale handle can
/// ever alias a later occupant. Sixteen million simultaneous slots is already
/// well above the memory limits a production VM can reasonably admit.
#[derive(Collect)]
#[collect(no_drop)]
struct ReusableSlots<T> {
    slots: Vec<ReusableSlot<T>>,
    free: Vec<u32>,
    occupied: usize,
}

#[derive(Collect)]
#[collect(no_drop)]
struct ReusableSlot<T> {
    generation: u8,
    value: Option<T>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HeapMemoryUsage {
    pub(crate) live_bytes: usize,
    pub(crate) reserved_bytes: usize,
}

/// Path census for [`VmCell::try_write_numeric_array_slot`].
///
/// This exists purely to prove which benchmark workloads reach the VM Core 2
/// C2.5a direct existing-slot lane. It is compiled out of ordinary builds:
/// the counters exist only under `cfg(test)` or the default-off
/// `table-write-diagnostics` feature, so the production hot path carries no
/// instrumentation.
#[cfg(any(test, feature = "table-write-diagnostics"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NumericArraySlotWriteCounts {
    /// Every call, including index and handle rejections.
    pub calls: u64,
    /// Calls served by the direct existing non-nil slot mutation.
    pub direct: u64,
    /// Calls stored by the generic `mutate_table_data` path (append, growth,
    /// or an existing nil slot on a metatable-free table).
    pub generic_stored: u64,
}

/// Allocation boundary captured after a hardened guest environment is sealed.
///
/// Hardened globals and their reachable configuration are immutable after this
/// point, so later decision-local allocations are appended beyond these slot
/// lengths and can be discarded without rebuilding the standard environment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HeapCheckpoint {
    tables: usize,
    functions: usize,
    upvalues: usize,
    strings: usize,
    userdatas: usize,
    buffers: usize,
    vectors: usize,
}

impl HeapMemoryUsage {
    fn add(&mut self, live_bytes: usize, reserved_bytes: usize) {
        self.live_bytes = self.live_bytes.saturating_add(live_bytes);
        self.reserved_bytes = self.reserved_bytes.saturating_add(reserved_bytes);
    }

    fn add_vec<T>(&mut self, values: &Vec<T>) {
        self.add(
            values.len().saturating_mul(size_of::<T>()),
            values.capacity().saturating_mul(size_of::<T>()),
        );
    }

    fn adjust(&mut self, before: Self, after: Self) {
        self.live_bytes = self
            .live_bytes
            .saturating_sub(before.live_bytes)
            .saturating_add(after.live_bytes);
        self.reserved_bytes = self
            .reserved_bytes
            .saturating_sub(before.reserved_bytes)
            .saturating_add(after.reserved_bytes);
    }

    fn add_slots<T>(&mut self, slots: &ReusableSlots<T>) {
        self.add(
            slots
                .occupied_len()
                .saturating_mul(size_of::<ReusableSlot<T>>()),
            slots
                .slots
                .capacity()
                .saturating_mul(size_of::<ReusableSlot<T>>()),
        );
        self.add_vec(&slots.free);
    }

    #[cfg(test)]
    fn add_slots_census<T>(&mut self, slots: &ReusableSlots<T>) {
        self.add(
            slots
                .slots
                .iter()
                .filter(|slot| slot.value.is_some())
                .count()
                .saturating_mul(size_of::<ReusableSlot<T>>()),
            slots
                .slots
                .capacity()
                .saturating_mul(size_of::<ReusableSlot<T>>()),
        );
        self.add_vec(&slots.free);
    }
}

impl<T> ReusableSlots<T> {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            occupied: 0,
        }
    }

    fn insert(&mut self, value: T) -> Option<u32> {
        if let Some(index) = self.free.pop() {
            let slot = self.slots.get_mut(index as usize)?;
            debug_assert!(slot.value.is_none());
            debug_assert!(slot.generation > 0);
            slot.value = Some(value);
            self.occupied = self.occupied.saturating_add(1);
            return Some(slot_handle(index, slot.generation));
        }

        let index = u32::try_from(self.slots.len()).ok()?;
        // Keep u32::MAX unavailable so stores that use handle + 1 as a
        // compact zero-sentinel encoding remain valid at every generation.
        if index >= SLOT_INDEX_MASK {
            return None;
        }
        self.slots.push(ReusableSlot {
            generation: 0,
            value: Some(value),
        });
        self.occupied = self.occupied.saturating_add(1);
        Some(slot_handle(index, 0))
    }

    fn planned_insert(&self) -> Option<(u32, usize)> {
        if let Some(&index) = self.free.last() {
            let slot = self.slots.get(index as usize)?;
            debug_assert!(slot.value.is_none());
            debug_assert!(slot.generation > 0);
            return Some((
                slot_handle(index, slot.generation),
                size_of::<ReusableSlot<T>>().saturating_sub(size_of::<u32>()),
            ));
        }

        let index = u32::try_from(self.slots.len()).ok()?;
        (index < SLOT_INDEX_MASK).then_some((slot_handle(index, 0), size_of::<ReusableSlot<T>>()))
    }

    #[inline]
    fn get(&self, handle: u32) -> Option<&T> {
        let (index, generation) = slot_handle_parts(handle);
        let slot = self.slots.get(index)?;
        (slot.generation == generation)
            .then_some(slot.value.as_ref())
            .flatten()
    }

    #[inline]
    fn get_mut(&mut self, handle: u32) -> Option<&mut T> {
        let (index, generation) = slot_handle_parts(handle);
        let slot = self.slots.get_mut(index)?;
        (slot.generation == generation)
            .then_some(slot.value.as_mut())
            .flatten()
    }

    fn handles(&self) -> impl Iterator<Item = u32> + '_ {
        self.slots.iter().enumerate().filter_map(|(index, slot)| {
            slot.value
                .as_ref()
                .map(|_| slot_handle(index as u32, slot.generation))
        })
    }

    fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().filter_map(|slot| slot.value.as_ref())
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|slot| slot.value.as_mut())
    }

    fn retain_handles(&mut self, mut keep: impl FnMut(u32) -> bool) -> bool {
        let mut removed = false;
        for (index, slot) in self.slots.iter_mut().enumerate() {
            if slot.value.is_none() {
                continue;
            }
            let handle = slot_handle(index as u32, slot.generation);
            if keep(handle) {
                continue;
            }
            slot.value = None;
            if slot.generation < SLOT_MAX_GENERATION {
                slot.generation += 1;
                self.free.push(index as u32);
            }
            self.occupied = self.occupied.saturating_sub(1);
            removed = true;
        }
        removed
    }

    fn reset_epoch(&mut self) {
        self.free.clear();
        for (index, slot) in self.slots.iter_mut().enumerate().rev() {
            slot.value = None;
            // Generation zero identifies first-epoch allocations. Reset
            // epochs are protected by the VM epoch and reuse generation one
            // so the ordinary free-list invariant remains explicit.
            slot.generation = 1;
            self.free.push(index as u32);
        }
        self.occupied = 0;
    }

    fn truncate_to_checkpoint(&mut self, len: usize) {
        debug_assert!(len <= self.slots.len());
        self.slots.truncate(len);
        self.free.clear();
        self.occupied = self
            .slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count();
    }

    #[inline]
    fn occupied_len(&self) -> usize {
        self.occupied
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.occupied_len()
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.occupied_len() == 0
    }

    #[cfg(test)]
    fn storage_len(&self) -> usize {
        self.slots.len()
    }

    fn release_spare_capacity(&mut self) {
        // Slot indices remain stable for the VM lifetime, including vacant and
        // permanently retired generations. Shrinking only the backing
        // allocations preserves stale-handle safety while returning allocator
        // over-allocation at an explicit quiescent boundary.
        self.slots.shrink_to_fit();
        self.free.shrink_to_fit();
    }
}

#[inline]
const fn slot_handle(index: u32, generation: u8) -> u32 {
    debug_assert!(index < SLOT_INDEX_MASK);
    index | ((generation as u32) << SLOT_GENERATION_SHIFT)
}

#[inline]
const fn slot_handle_parts(handle: u32) -> (usize, u8) {
    (
        (handle & SLOT_INDEX_MASK) as usize,
        (handle >> SLOT_GENERATION_SHIFT) as u8,
    )
}

#[derive(Collect)]
#[collect(no_drop)]
struct RootData<'gc> {
    _roots: DynamicRootSet<'gc>,
    strings: ReusableSlots<InternedString<'gc>>,
    string_index: Vec<u32>,
    userdatas: ReusableSlots<UserdataData>,
    buffers: ReusableSlots<BufferData>,
    vectors: ReusableSlots<VectorData>,
}

#[derive(Collect)]
#[collect(no_drop)]
enum InternedString<'gc> {
    /// Compact atom storage used for short strings that must acquire table-key identity.
    Inline(ShortString),
    /// Immutable compile-time bytes shared by every VM that loads the same program.
    Shared(Static<SharedProgramString>),
    /// Per-VM runtime strings too large for the inline form.
    Owned(Gc<'gc, Static<Vec<u8>>>),
}

impl InternedString<'_> {
    #[inline]
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Inline(value) => value.as_bytes(),
            Self::Shared(value) => value.0.as_bytes(),
            Self::Owned(value) => value.0.as_slice(),
        }
    }

    #[inline]
    fn stable_hash(&self) -> u64 {
        match self {
            Self::Shared(value) => value.0.stable_hash(),
            Self::Inline(value) => stable_bytes_hash(value.as_bytes()),
            Self::Owned(value) => stable_bytes_hash(value.0.as_slice()),
        }
    }
}

/// Per-VM table storage layout, selected from the VM profile at construction.
///
/// `Log` is the historical rustia backend (insertion-ordered entries with a
/// side hash index). `Puc51` ports the PUC Lua 5.1.5 `ltable.c` layout
/// (array part + chained node part with Brent's variation) so that the
/// `Puc51Compat` profile reproduces PUC sizing, rehash, and iteration-order
/// behavior exactly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableLayout {
    Log,
    Puc51,
}

#[derive(Clone)]
struct TableData {
    array: Vec<Value>,
    hash: HashPart,
    readonly: bool,
    metatable: Option<u32>,
    shape_version: u64,
    /// Increments on every guest-visible table mutation, including writes to
    /// existing slots that leave the storage shape unchanged.
    mutation_version: u64,
}

/// Hash-part storage backing a [`TableData`]. The variant is fixed per VM by
/// [`TableLayout`]; every table in one VM uses the same variant.
#[derive(Clone)]
enum HashPart {
    /// Insertion-ordered entries plus a compact side index (default rustia
    /// backend).
    Log {
        entries: Vec<TableEntry>,
        index: Vec<u32>,
    },
    /// PUC Lua 5.1.5 `ltable.c` node part.
    Puc(PucHashPart),
}

/// PUC node part: a chained scatter table with Brent's variation. `nodes` is
/// persistent (never shrunk outside `table_clear` semantics) and its size is
/// either zero (the PUC `dummynode` case) or a power of two.
#[derive(Clone)]
struct PucHashPart {
    nodes: Vec<PucNode>,
    /// PUC `lastfree`: a scan-down cursor one past the next free-slot
    /// candidate. It only moves downward and never resets except on rehash or
    /// `table_clear`.
    lastfree: usize,
}

#[derive(Clone, Copy)]
struct PucNode {
    key: PucNodeKey,
    /// Live `Value` form of the key (same role as `TableEntry.key_value`).
    key_value: Value,
    value: Value,
    /// Next slot in the collision chain; -1 is the PUC `gnext == NULL` end.
    next: i32,
}

#[derive(Clone, Copy)]
enum PucNodeKey {
    /// PUC nil key: a never-used slot, i.e. a `getfreepos` candidate.
    Free,
    Live(TableKey),
    /// GC-tombstoned collectable key; never GC-marked and never matches
    /// reads, but still answers `next()` lookups like a PUC dead key.
    Dead(TableKey),
}

const PUC_FREE_NODE: PucNode = PucNode {
    key: PucNodeKey::Free,
    key_value: Value::nil(),
    value: Value::nil(),
    next: -1,
};

/// PUC `MAXBITS`/`MAXASIZE` (ltable.c:41-47).
const PUC_MAXBITS: usize = 26;
const PUC_MAXASIZE: usize = 1 << PUC_MAXBITS;

fn empty_hash_part(layout: TableLayout) -> HashPart {
    match layout {
        TableLayout::Log => HashPart::Log {
            entries: Vec::new(),
            index: Vec::new(),
        },
        TableLayout::Puc51 => HashPart::Puc(PucHashPart {
            nodes: Vec::new(),
            lastfree: 0,
        }),
    }
}

impl TableData {
    #[inline]
    fn puc_hash(&self) -> Option<&PucHashPart> {
        match &self.hash {
            HashPart::Puc(puc) => Some(puc),
            HashPart::Log { .. } => None,
        }
    }

    #[inline]
    fn puc_hash_mut(&mut self) -> Option<&mut PucHashPart> {
        match &mut self.hash {
            HashPart::Puc(puc) => Some(puc),
            HashPart::Log { .. } => None,
        }
    }
}

#[derive(Clone)]
struct FunctionData {
    kind: FunctionKind,
    captures: SmallVec<[FunctionCapture; 2]>,
    environment: u32,
}

pub(crate) type FunctionDispatchData =
    (FunctionKind, Option<(SmallVec<[FunctionCapture; 2]>, u32)>);

/// A closure capture is either a shared mutable cell or an immutable value
/// copied into the closure. The compiler only emits `Direct` after proving
/// that every lexical alias of the source binding is never assigned.
#[repr(transparent)]
#[derive(Clone, Copy)]
pub(crate) struct FunctionCapture(Value);

impl FunctionCapture {
    #[inline]
    pub(crate) const fn from_cell(cell: u32) -> Self {
        Self(Value::capture_cell(cell))
    }

    #[inline]
    pub(crate) const fn direct(value: Value) -> Self {
        Self(value)
    }

    #[inline]
    pub(crate) const fn cell(self) -> Option<u32> {
        self.0.as_capture_cell()
    }

    #[inline]
    const fn direct_value(self) -> Option<Value> {
        if self.cell().is_some() {
            None
        } else {
            Some(self.0)
        }
    }
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
struct UpvalueData {
    value: Value,
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
struct UserdataData {
    metatable: Option<u32>,
    payload: UserdataPayload,
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub(crate) enum UserdataPayload {
    None,
    ConformanceInt64(i64),
    ConformanceVec2 {
        x: f64,
        y: f64,
    },
    ConformanceVertex {
        pos: Value,
        normal: Value,
        uv: Value,
    },
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
struct BufferData {
    bytes: Vec<u8>,
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub(crate) struct VectorData {
    x: f32,
    y: f32,
    z: f32,
}

impl VectorData {
    pub(crate) fn new(x: f32, y: f32, z: f32) -> Self {
        Self {
            x: canonicalize_vector_component(x),
            y: canonicalize_vector_component(y),
            z: canonicalize_vector_component(z),
        }
    }

    pub(crate) const fn x(self) -> f32 {
        self.x
    }

    pub(crate) const fn y(self) -> f32 {
        self.y
    }

    pub(crate) const fn z(self) -> f32 {
        self.z
    }

    fn table_key_bits(self) -> Option<[u32; 3]> {
        if self.x.is_nan() || self.y.is_nan() || self.z.is_nan() {
            return None;
        }
        Some([
            vector_table_key_bits(self.x),
            vector_table_key_bits(self.y),
            vector_table_key_bits(self.z),
        ])
    }
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
pub enum FunctionKind {
    Bytecode {
        program: Option<u32>,
        prototype: u32,
    },
    DynamicMain {
        program: u32,
    },
    Host(u32),
    Native(NativeFunction),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Collect)]
#[collect(no_drop)]
pub enum NativeFunction {
    Assert,
    Error,
    PCall,
    XPCall,
    Print,
    Select,
    IPairs,
    INext,
    Pairs,
    LoadString,
    Require,
    CoroutineCreate,
    CoroutineResume,
    CoroutineWrap,
    CoroutineYield,
    CoroutineStatus,
    CoroutineRunning,
    CoroutineIsYieldable,
    CoroutineClose,
    CoroutineWrappedCall,
    ToNumber,
    ToString,
    TypeOf,
    GetFEnv,
    SetFEnv,
    SetMetatable,
    GetMetatable,
    NewProxy,
    CollectGarbage,
    GcInfo,
    DebugTraceback,
    DebugInfo,
    OsClock,
    OsDate,
    OsTime,
    OsDiffTime,
    RawEqual,
    RawGet,
    RawSet,
    RawLen,
    Next,
    GetCoverage,
    ConformanceCxxThrow,
    ConformanceResumeError,
    ConformanceIsNative,
    ConformanceInt64New,
    ConformanceInt64Eq,
    ConformanceInt64Lt,
    ConformanceInt64Le,
    ConformanceInt64Neg,
    ConformanceInt64Add,
    ConformanceInt64Sub,
    ConformanceInt64Mul,
    ConformanceInt64Div,
    ConformanceInt64FloorDiv,
    ConformanceInt64Mod,
    ConformanceInt64Pow,
    ConformanceInt64ToString,
    ConformanceInt64Index,
    ConformanceInt64NewIndex,
    ConformanceVec2New,
    ConformanceVec2Neg,
    ConformanceVec2Add,
    ConformanceVec2Mul,
    ConformanceVec2Index,
    ConformanceVec2Dot,
    ConformanceVec2Min,
    ConformanceVertexNew,
    ConformanceVertexIndex,
    MathAbs,
    MathFloor,
    MathCeil,
    MathRound,
    MathSign,
    MathClamp,
    MathMap,
    MathLerp,
    MathMin,
    MathMax,
    MathFmod,
    MathPow,
    MathSqrt,
    MathDeg,
    MathRad,
    MathIsNan,
    MathIsInf,
    MathIsFinite,
    MathSin,
    MathCos,
    MathTan,
    MathAsin,
    MathAcos,
    MathAtan,
    MathAtan2,
    MathSinh,
    MathCosh,
    MathTanh,
    MathExp,
    MathLog,
    MathLog10,
    MathLdexp,
    MathFrexp,
    MathModf,
    MathRandom,
    MathRandomSeed,
    MathNoise,
    StringSub,
    StringLen,
    StringByte,
    StringChar,
    StringUpper,
    StringLower,
    StringReverse,
    StringRep,
    StringFind,
    StringMatch,
    StringGsub,
    StringGMatch,
    StringGMatchIterator,
    StringSplit,
    StringFormat,
    StringPack,
    StringUnpack,
    StringPackSize,
    Utf8Char,
    Utf8Codepoint,
    Utf8Codes,
    Utf8CodesIterator,
    Utf8Len,
    Utf8Offset,
    Bit32ArShift,
    Bit32Band,
    Bit32BNot,
    Bit32Bor,
    Bit32BTest,
    Bit32BXor,
    Bit32Byteswap,
    Bit32CountLz,
    Bit32CountRz,
    Bit32Extract,
    Bit32LRotate,
    Bit32LShift,
    Bit32Replace,
    Bit32RRotate,
    Bit32RShift,
    TableConcat,
    TableClear,
    TablePack,
    TableUnpack,
    TableGetN,
    TableMaxN,
    TableCreate,
    TableFind,
    TableForeach,
    TableForeachi,
    TableClone,
    TableFreeze,
    TableInsert,
    TableIsFrozen,
    TableMove,
    TableRemove,
    TableSort,
    BufferCreate,
    BufferLen,
    BufferFromString,
    BufferToString,
    BufferReadI8,
    BufferReadU8,
    BufferReadI16,
    BufferReadU16,
    BufferReadI32,
    BufferReadU32,
    BufferReadF32,
    BufferReadF64,
    BufferReadString,
    BufferWriteI8,
    BufferWriteU8,
    BufferWriteI16,
    BufferWriteU16,
    BufferWriteI32,
    BufferWriteU32,
    BufferWriteF32,
    BufferWriteF64,
    BufferWriteString,
    BufferCopy,
    BufferFill,
    BufferReadBits,
    BufferWriteBits,
    VectorCreate,
    VectorCall,
    VectorDot,
    VectorMethodDot,
    VectorCross,
    VectorMethodCross,
    VectorNormalize,
    VectorMagnitude,
    VectorAbs,
    VectorFloor,
    VectorCeil,
    VectorSign,
    VectorAngle,
    VectorMin,
    VectorMax,
    VectorClamp,
    VectorLerp,
}

#[derive(Clone, Copy)]
struct TableEntry {
    key: TableKey,
    key_value: Value,
    value: Value,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TableKey {
    Boolean(bool),
    Number(u64),
    String(u32),
    Table(u32),
    Function(u32),
    Userdata(u32),
    Buffer(u32),
    Vector([u32; 3]),
}

#[derive(Clone, Copy, Debug)]
pub enum TableNext {
    Entry(Value, Value),
    End,
    InvalidKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TableStringHashSlot {
    shape_version: u64,
    index: u32,
}

impl TableStringHashSlot {
    pub(crate) const fn from_parts(shape_version: u64, index: u32) -> Self {
        Self {
            shape_version,
            index,
        }
    }

    pub(crate) const fn shape_version(self) -> u64 {
        self.shape_version
    }

    pub(crate) const fn index(self) -> u32 {
        self.index
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableStringHashWrite {
    Stored,
    Deferred,
    Readonly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuotaTableWrite {
    Stored,
    WouldExceed,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableNumericArrayWrite {
    Stored,
    Deferred,
    Readonly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableIterationOrder {
    Insertion,
    LuauHash,
    Puc51,
}

type ArenaRoot = Rootable![Gc<'_, RefLock<RootData<'_>>>];
type VmArena = Arena<ArenaRoot>;

/// The GC cell for one VM lane.
///
/// `gc-arena` values and retained roots are not independently thread-safe, so the VM stays
/// thread-affine while it owns an arena. Parallel hosts should create one VM per worker or move
/// compiled programs/configuration between workers instead of moving a live VM.
pub struct VmCell {
    arena: VmArena,
    allocation_debt_pending: bool,
    // Tables contain only compact `Value` and metadata handles, never arena GC
    // pointers. Keeping them beside the arena removes the mutation boundary from
    // every hot table read/write while the explicit weak-table traversal still
    // walks the same graph.
    tables: ReusableSlots<TableData>,
    // Function metadata contains only stable compact handles, not GC pointers.
    // Keeping it beside the arena avoids entering `Arena::mutate` for every
    // bytecode/native dispatch while weak-table traversal can still inspect it.
    functions: ReusableSlots<FunctionData>,
    // Upvalues are compact `Value` handles as well. Their hot read/write path
    // does not need a GC mutation boundary; weak traversal receives this slice.
    upvalues: ReusableSlots<UpvalueData>,
    side_usage: HeapMemoryUsage,
    // Selected once at VM construction from the profile; every table in this
    // cell uses the same hash-part variant.
    table_layout: TableLayout,
    #[cfg(any(test, feature = "table-write-diagnostics"))]
    numeric_array_slot_writes: NumericArraySlotWriteCounts,
}

impl VmCell {
    pub(crate) const fn filled_array_table_live_bytes(size: usize) -> usize {
        size_of::<ReusableSlot<TableData>>().saturating_add(size.saturating_mul(size_of::<Value>()))
    }

    pub(crate) const fn buffer_live_bytes(size: usize) -> usize {
        size_of::<ReusableSlot<BufferData>>().saturating_add(size)
    }

    pub(crate) const fn owned_string_live_bytes(size: usize) -> usize {
        // The arena allocation is counted in addition to the reusable atom
        // slot and owned byte vector. Keep a conservative fixed allowance for
        // the arena header and Vec descriptor.
        size_of::<ReusableSlot<InternedString<'static>>>()
            .saturating_add(size)
            .saturating_add(size_of::<Vec<u8>>())
            .saturating_add(64)
    }

    /// Live bytes of a PUC-layout table with the given array-part size and
    /// hash-part element count (before `setnodevector` power-of-two rounding).
    pub(crate) const fn puc_table_live_bytes(array_size: usize, hash_size: usize) -> usize {
        size_of::<ReusableSlot<TableData>>()
            .saturating_add(array_size.saturating_mul(size_of::<Value>()))
            .saturating_add(puc_node_vector_len(hash_size).saturating_mul(size_of::<PucNode>()))
    }

    /// Default Log-layout constructor. Production VMs go through
    /// [`VmCell::with_table_layout`]; this stays for the existing test
    /// callers.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_table_layout(TableLayout::Log)
    }

    pub(crate) fn with_table_layout(table_layout: TableLayout) -> Self {
        let arena = VmArena::new(|mc| {
            Gc::new(
                mc,
                RefLock::new(RootData {
                    _roots: DynamicRootSet::new(mc),
                    strings: ReusableSlots::new(),
                    string_index: Vec::new(),
                    userdatas: ReusableSlots::new(),
                    buffers: ReusableSlots::new(),
                    vectors: ReusableSlots::new(),
                }),
            )
        });
        let allocation_debt_pending = arena.metrics().allocation_debt() > 0.0;
        Self {
            arena,
            allocation_debt_pending,
            tables: ReusableSlots::new(),
            functions: ReusableSlots::new(),
            upvalues: ReusableSlots::new(),
            side_usage: HeapMemoryUsage::default(),
            table_layout,
            #[cfg(any(test, feature = "table-write-diagnostics"))]
            numeric_array_slot_writes: NumericArraySlotWriteCounts::default(),
        }
    }

    /// Diagnostic-only path census for numeric array-slot writes.
    #[cfg(any(test, feature = "table-write-diagnostics"))]
    pub fn numeric_array_slot_write_counts(&self) -> NumericArraySlotWriteCounts {
        self.numeric_array_slot_writes
    }

    fn insert_table_data(&mut self, table: TableData) -> Option<u32> {
        let usage = table_data_memory_usage(&table);
        let handle = self.tables.insert(table)?;
        self.side_usage.add(usage.live_bytes, usage.reserved_bytes);
        Some(handle)
    }

    fn insert_function_data(&mut self, function: FunctionData) -> Option<u32> {
        let usage = function_data_memory_usage(&function);
        let handle = self.functions.insert(function)?;
        self.side_usage.add(usage.live_bytes, usage.reserved_bytes);
        Some(handle)
    }

    fn mutate_table_data<R>(
        &mut self,
        handle: u32,
        mutate: impl FnOnce(&mut TableData) -> R,
    ) -> Option<R> {
        let table = self.tables.get_mut(handle)?;
        let before = table_data_memory_usage(table);
        let result = mutate(table);
        table.mutation_version = table.mutation_version.wrapping_add(1);
        let after = table_data_memory_usage(table);
        self.side_usage.adjust(before, after);
        Some(result)
    }

    /// Same accounting wrapper as [`VmCell::mutate_table_data`], but the
    /// closure also receives the string-atom root. PUC-layout hashing needs
    /// string bytes to compute node main positions.
    fn mutate_table_data_with_root<R>(
        &mut self,
        handle: u32,
        mutate: impl FnOnce(&RootData<'_>, &mut TableData) -> R,
    ) -> Option<R> {
        let tables = &mut self.tables;
        let side_usage = &mut self.side_usage;
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = tables.get_mut(handle)?;
            let before = table_data_memory_usage(table);
            let result = mutate(&root, table);
            table.mutation_version = table.mutation_version.wrapping_add(1);
            side_usage.adjust(before, table_data_memory_usage(table));
            Some(result)
        })
    }

    pub fn finish_collection_cycle(&mut self) {
        self.arena.finish_cycle();
        self.allocation_debt_pending = self.arena.metrics().allocation_debt() > 0.0;
    }

    /// Discards every guest allocation while retaining reusable slot-vector
    /// capacity for the next decision epoch.
    ///
    /// Compact heap handles are epoch-local after this operation. Public
    /// retained handles are invalidated by `Vm` before this method runs; raw
    /// `Value`s must never cross an explicit decision reset.
    pub(crate) fn reset_decision_epoch(&mut self) {
        self.tables.reset_epoch();
        self.functions.reset_epoch();
        self.upvalues.reset_epoch();
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.strings.reset_epoch();
            root.string_index.clear();
            root.userdatas.reset_epoch();
            root.buffers.reset_epoch();
            root.vectors.reset_epoch();
        });
        self.arena.finish_cycle();
        self.arena.finish_cycle();
        self.allocation_debt_pending = self.arena.metrics().allocation_debt() > 0.0;
        self.recompute_side_usage();
    }

    /// Captures the immutable allocation prefix used by a sealed hardened
    /// environment.
    pub(crate) fn hardened_checkpoint(&mut self) -> Option<HeapCheckpoint> {
        if !self.tables.free.is_empty()
            || !self.functions.free.is_empty()
            || !self.upvalues.free.is_empty()
        {
            return None;
        }
        let mut checkpoint = HeapCheckpoint {
            tables: self.tables.slots.len(),
            functions: self.functions.slots.len(),
            upvalues: self.upvalues.slots.len(),
            strings: 0,
            userdatas: 0,
            buffers: 0,
            vectors: 0,
        };
        let arena_is_append_only = self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            if !root.strings.free.is_empty()
                || !root.userdatas.free.is_empty()
                || !root.buffers.free.is_empty()
                || !root.vectors.free.is_empty()
            {
                return false;
            }
            checkpoint.strings = root.strings.slots.len();
            checkpoint.userdatas = root.userdatas.slots.len();
            checkpoint.buffers = root.buffers.slots.len();
            checkpoint.vectors = root.vectors.slots.len();
            true
        });
        arena_is_append_only.then_some(checkpoint)
    }

    /// Drops allocations made after a hardened environment was sealed while
    /// preserving its immutable globals, standard libraries, and host
    /// namespaces in place.
    pub(crate) fn reset_hardened_decision_epoch(&mut self, checkpoint: HeapCheckpoint) {
        self.tables.truncate_to_checkpoint(checkpoint.tables);
        self.functions.truncate_to_checkpoint(checkpoint.functions);
        self.upvalues.truncate_to_checkpoint(checkpoint.upvalues);
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.strings.truncate_to_checkpoint(checkpoint.strings);
            root.userdatas.truncate_to_checkpoint(checkpoint.userdatas);
            root.buffers.truncate_to_checkpoint(checkpoint.buffers);
            root.vectors.truncate_to_checkpoint(checkpoint.vectors);
            rebuild_string_index(&mut root, STRING_INDEX_MIN_CAPACITY);
        });
        self.arena.finish_cycle();
        self.arena.finish_cycle();
        self.allocation_debt_pending = self.arena.metrics().allocation_debt() > 0.0;
        self.recompute_side_usage();
    }

    pub(crate) fn memory_usage(&mut self) -> HeapMemoryUsage {
        let mut usage = HeapMemoryUsage::default();
        usage.add_slots(&self.tables);
        usage.add_slots(&self.functions);
        usage.add_slots(&self.upvalues);
        usage.add(self.side_usage.live_bytes, self.side_usage.reserved_bytes);

        let arena_bytes = self.arena.metrics().total_gc_allocation();
        usage.add(arena_bytes, arena_bytes);
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            usage.add_slots(&root.strings);
            usage.add_slots(&root.userdatas);
            usage.add_slots(&root.buffers);
            usage.add_slots(&root.vectors);
        });
        #[cfg(test)]
        assert_eq!(
            usage,
            self.memory_usage_census(),
            "incremental heap memory accounting diverged from the exact census"
        );
        usage
    }

    #[cfg(test)]
    fn memory_usage_census(&mut self) -> HeapMemoryUsage {
        let mut usage = HeapMemoryUsage::default();
        usage.add_slots_census(&self.tables);
        usage.add_slots_census(&self.functions);
        usage.add_slots_census(&self.upvalues);
        for table in self.tables.values() {
            usage.add(
                table_data_live_bytes(table),
                table_data_reserved_bytes(table),
            );
        }
        for function in self.functions.values() {
            let function = function_data_memory_usage(function);
            usage.add(function.live_bytes, function.reserved_bytes);
        }

        let arena_bytes = self.arena.metrics().total_gc_allocation();
        usage.add(arena_bytes, arena_bytes);
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            usage.add_slots_census(&root.strings);
            usage.add_slots_census(&root.userdatas);
            usage.add_slots_census(&root.buffers);
            usage.add_slots_census(&root.vectors);
            usage.add_vec(&root.string_index);
            for string in root.strings.values() {
                if let InternedString::Owned(value) = string {
                    usage.add_vec(&value.0);
                }
            }
            for buffer in root.buffers.values() {
                usage.add_vec(&buffer.bytes);
            }
        });
        usage
    }

    fn recompute_side_usage(&mut self) {
        let mut usage = HeapMemoryUsage::default();
        for table in self.tables.values() {
            usage.add(
                table_data_live_bytes(table),
                table_data_reserved_bytes(table),
            );
        }
        for function in self.functions.values() {
            usage.add(
                function
                    .captures
                    .len()
                    .saturating_mul(size_of::<FunctionCapture>()),
                function
                    .captures
                    .capacity()
                    .saturating_mul(size_of::<FunctionCapture>()),
            );
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            usage.add_vec(&root.string_index);
            for string in root.strings.values() {
                if let InternedString::Owned(value) = string {
                    usage.add_vec(&value.0);
                }
            }
            for buffer in root.buffers.values() {
                usage.add_vec(&buffer.bytes);
            }
        });
        self.side_usage = usage;
    }

    pub fn collect_weak_tables_with_roots(&mut self, roots: &[Value]) {
        let tables = &mut self.tables;
        let functions = &mut self.functions;
        let upvalues = &mut self.upvalues;
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let mode_key = b"__mode";
            let mode_atom = string_index_lookup(&root, mode_key, stable_bytes_hash(mode_key));
            let modes = mode_atom.map_or_else(BTreeMap::new, |mode_atom| {
                tables
                    .handles()
                    .filter_map(|handle| {
                        table_weak_mode(&root, tables, handle, mode_atom).map(|mode| (handle, mode))
                    })
                    .collect()
            });
            let live = mark_heap_reachable(&root, tables, functions, upvalues, &modes, roots);
            debug_assert!(live.upvalues.live_len() <= upvalues.occupied_len());
            for (handle, mode) in modes {
                if let Some(table) = tables.get_mut(handle) {
                    sweep_weak_table(table, mode, &live);
                }
            }
            reclaim_unreachable(&mut root, tables, functions, upvalues, &live);
        });
        self.recompute_side_usage();
    }

    pub(crate) fn reachable_userdata_handles(&mut self, roots: &[Value]) -> Vec<u32> {
        let tables = &self.tables;
        let functions = &self.functions;
        let upvalues = &self.upvalues;
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let mode_key = b"__mode";
            let mode_atom = string_index_lookup(&root, mode_key, stable_bytes_hash(mode_key));
            let modes = mode_atom.map_or_else(BTreeMap::new, |mode_atom| {
                tables
                    .handles()
                    .filter_map(|handle| {
                        table_weak_mode(&root, tables, handle, mode_atom).map(|mode| (handle, mode))
                    })
                    .collect()
            });
            let live = mark_heap_reachable(&root, tables, functions, upvalues, &modes, roots);
            root.userdatas
                .handles()
                .filter(|handle| live.userdatas.contains(*handle))
                .collect()
        })
    }

    pub fn release_spare_capacity(&mut self) {
        for table in self.tables.values_mut() {
            if !matches!(table.hash, HashPart::Log { .. }) {
                // PUC tables keep their array and node sizes outside rehash:
                // popping trailing nils or shrinking vectors would change
                // border semantics and main-position layout.
                continue;
            }
            while table.array.last().is_some_and(|value| value.is_nil()) {
                table.array.pop();
                // Popping trailing nils moves the observable length border,
                // so version consumers (the table.sort mutation guard) must
                // see a bump.
                table.mutation_version = table.mutation_version.wrapping_add(1);
            }
            table.array.shrink_to_fit();
            let HashPart::Log { entries, index } = &mut table.hash else {
                unreachable!("Log layout checked above")
            };
            entries.shrink_to_fit();
            rebuild_hash_index(entries, index);
            index.shrink_to_fit();
        }
        for function in self.functions.values_mut() {
            function.captures.shrink_to_fit();
        }
        self.tables.release_spare_capacity();
        self.functions.release_spare_capacity();
        self.upvalues.release_spare_capacity();

        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            for buffer in root.buffers.values_mut() {
                buffer.bytes.shrink_to_fit();
            }
            root.strings.release_spare_capacity();
            root.string_index.shrink_to_fit();
            root.userdatas.release_spare_capacity();
            root.buffers.release_spare_capacity();
            root.vectors.release_spare_capacity();
        });
        self.recompute_side_usage();
    }

    #[inline(always)]
    pub fn collect_debt(&mut self) {
        if !self.allocation_debt_pending {
            return;
        }
        self.arena.collect_debt();
        self.allocation_debt_pending = self.arena.metrics().allocation_debt() > 0.0;
    }

    #[inline(always)]
    pub(crate) fn has_allocation_debt(&self) -> bool {
        self.allocation_debt_pending
    }

    pub fn intern_string(&mut self, value: &str) -> u32 {
        self.intern_string_bytes(value.as_bytes())
    }

    pub fn intern_string_bytes(&mut self, value: &[u8]) -> u32 {
        let hash = stable_bytes_hash(value);
        let mut allocated_gc_string = false;
        let (atom, index_before, index_after, owned_added) = self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let index_before = vec_memory_usage(&root.string_index);
            if let Some(atom) = string_index_lookup(&root, value, hash) {
                return (atom, index_before, index_before, HeapMemoryUsage::default());
            }
            let required = root.strings.occupied_len().saturating_add(1);
            if root.string_index.is_empty() || required.saturating_mul(2) > root.string_index.len()
            {
                let capacity = required
                    .saturating_mul(2)
                    .next_power_of_two()
                    .max(STRING_INDEX_MIN_CAPACITY);
                rebuild_string_index(&mut root, capacity);
            }
            let mut owned_added = HeapMemoryUsage::default();
            let stored = ShortString::new(value).map_or_else(
                || {
                    allocated_gc_string = true;
                    let bytes = value.to_vec();
                    owned_added = vec_memory_usage(&bytes);
                    InternedString::Owned(Gc::new(mc, Static(bytes)))
                },
                InternedString::Inline,
            );
            let atom = root
                .strings
                .insert(stored)
                .expect("string atom table exceeds compact u32 index address space");
            insert_string_index(&mut root, atom);
            (
                atom,
                index_before,
                vec_memory_usage(&root.string_index),
                owned_added,
            )
        });
        self.side_usage.adjust(index_before, index_after);
        self.side_usage
            .add(owned_added.live_bytes, owned_added.reserved_bytes);
        self.allocation_debt_pending |= allocated_gc_string;
        atom
    }

    pub(crate) fn intern_shared_string(&mut self, value: SharedProgramString) -> u32 {
        let stable_hash = value.stable_hash();
        if let Some(short) = ShortString::new(value.as_bytes()) {
            return self.intern_inline_string(short, stable_hash);
        }
        let (atom, index_before, index_after, owned_removed) = self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let index_before = vec_memory_usage(&root.string_index);
            if let Some(atom) = string_index_lookup(&root, value.as_bytes(), stable_hash) {
                let mut owned_removed = HeapMemoryUsage::default();
                if matches!(root.strings.get(atom), Some(InternedString::Owned(_))) {
                    if let Some(InternedString::Owned(stored)) = root.strings.get(atom) {
                        owned_removed = vec_memory_usage(&stored.0);
                    }
                    *root
                        .strings
                        .get_mut(atom)
                        .expect("looked-up string atom remains live") =
                        InternedString::Shared(Static(value));
                }
                return (atom, index_before, index_before, owned_removed);
            }
            ensure_string_index_capacity(&mut root);
            let atom = root
                .strings
                .insert(InternedString::Shared(Static(value)))
                .expect("string atom table exceeds compact u32 index address space");
            insert_string_index(&mut root, atom);
            (
                atom,
                index_before,
                vec_memory_usage(&root.string_index),
                HeapMemoryUsage::default(),
            )
        });
        self.side_usage.adjust(index_before, index_after);
        self.side_usage
            .adjust(owned_removed, HeapMemoryUsage::default());
        atom
    }

    fn intern_inline_string(&mut self, value: ShortString, stable_hash: u64) -> u32 {
        let (atom, index_before, index_after) = self.arena.mutate(|_mc, root| {
            let mut root = root.unlock(_mc).borrow_mut();
            let index_before = vec_memory_usage(&root.string_index);
            if let Some(atom) = string_index_lookup(&root, value.as_bytes(), stable_hash) {
                return (atom, index_before, index_before);
            }
            ensure_string_index_capacity(&mut root);
            let atom = root
                .strings
                .insert(InternedString::Inline(value))
                .expect("string atom table exceeds compact u32 index address space");
            insert_string_index(&mut root, atom);
            (atom, index_before, vec_memory_usage(&root.string_index))
        });
        self.side_usage.adjust(index_before, index_after);
        atom
    }

    fn planned_inline_string_intern(&mut self) -> Option<(u32, usize)> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let (atom, slot_growth) = root.strings.planned_insert()?;
            let required = root.strings.occupied_len().saturating_add(1);
            let index_len = if root.string_index.is_empty()
                || required.saturating_mul(2) > root.string_index.len()
            {
                required
                    .saturating_mul(2)
                    .next_power_of_two()
                    .max(STRING_INDEX_MIN_CAPACITY)
            } else {
                root.string_index.len()
            };
            let index_growth = index_len
                .saturating_sub(root.string_index.len())
                .saturating_mul(size_of::<u32>());
            Some((atom, slot_growth.saturating_add(index_growth)))
        })
    }

    fn planned_vector_allocation(&mut self) -> Option<(u32, usize)> {
        self.arena
            .mutate(|mc, root| root.unlock(mc).borrow().vectors.planned_insert())
    }

    #[inline]
    pub(crate) fn intern_string_value_bytes(&mut self, value: &[u8]) -> Value {
        Value::short_string(value)
            .unwrap_or_else(|| Value::string_atom(self.intern_string_bytes(value)))
    }

    #[inline]
    pub(crate) fn find_string_atom_bytes(&mut self, value: &[u8]) -> Option<u32> {
        let stable_hash = stable_bytes_hash(value);
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            string_index_lookup(&root, value, stable_hash)
        })
    }

    #[inline]
    pub(crate) fn string_atom_equals_bytes(&mut self, atom: u32, value: &[u8]) -> Option<bool> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .strings
                .get(atom)
                .map(|candidate| candidate.as_bytes() == value)
        })
    }

    #[cfg(test)]
    pub(crate) fn string_atom_count(&mut self) -> usize {
        self.arena
            .mutate(|mc, root| root.unlock(mc).borrow().strings.occupied_len())
    }

    pub fn string_bytes(&mut self, atom: u32) -> Option<Vec<u8>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .strings
                .get(atom)
                .map(|value| value.as_bytes().to_vec())
        })
    }

    pub fn alloc_array_table(&mut self, values: &[Value]) -> u32 {
        self.insert_table_data(TableData {
            array: values.to_vec(),
            hash: empty_hash_part(self.table_layout),
            readonly: false,
            metatable: None,
            shape_version: 0,
            mutation_version: 0,
        })
        .expect("table slot limit exceeded")
    }

    /// Allocates a PUC-layout table with pre-sized array and node parts,
    /// matching `luaH_new` + `setarrayvector`/`setnodevector` (ltable.c:358).
    /// The array part is filled with `fill` (`table.create`-style fills write
    /// a real value; constructor pre-sizing passes nil).
    pub(crate) fn try_alloc_puc_table(
        &mut self,
        array_size: usize,
        hash_size: usize,
        fill: Value,
    ) -> Option<u32> {
        if self.table_layout != TableLayout::Puc51 {
            return None;
        }
        let mut array = Vec::new();
        array.try_reserve_exact(array_size).ok()?;
        array.resize(array_size, fill);
        self.try_alloc_puc_table_from_parts(array, hash_size)
    }

    /// PUC-layout table taking an already-materialized array part (the
    /// `NewArray` register-window path); node part is sized like
    /// `setnodevector`.
    pub(crate) fn try_alloc_puc_table_from_parts(
        &mut self,
        array: Vec<Value>,
        hash_size: usize,
    ) -> Option<u32> {
        let node_len = puc_node_vector_len(hash_size);
        let mut nodes = Vec::new();
        nodes.try_reserve_exact(node_len).ok()?;
        nodes.resize(node_len, PUC_FREE_NODE);
        self.insert_table_data(TableData {
            array,
            hash: HashPart::Puc(PucHashPart {
                nodes,
                lastfree: node_len,
            }),
            readonly: false,
            metatable: None,
            shape_version: 0,
            mutation_version: 0,
        })
    }

    /// Current array-part size (`sizearray`), not the `#t` border.
    pub(crate) fn table_array_part_len(&mut self, table: u32) -> Option<usize> {
        self.tables.get(table).map(|table| table.array.len())
    }

    /// `luaH_resizearray` (ltable.c:327): grows the array part to exactly
    /// `nasize`, re-laying the node part into a fresh same-sized vector. No-op
    /// when the array is already large enough.
    pub(crate) fn puc_resize_array_table(&mut self, table: u32, nasize: usize) -> Option<()> {
        let existing = self.tables.get(table)?;
        if existing.readonly || existing.puc_hash().is_none() {
            return None;
        }
        if nasize <= existing.array.len() {
            return Some(());
        }
        self.mutate_table_data_with_root(table, |root, table| {
            let node_len = table.puc_hash().map_or(0, |puc| puc.nodes.len());
            puc_resize(root, table, nasize, node_len);
        })
    }

    /// Quota variant of [`VmCell::puc_resize_array_table`]: stages the resize
    /// on a candidate clone so a rejection cannot leave a partial write.
    pub(crate) fn puc_resize_array_with_live_growth_limit(
        &mut self,
        table: u32,
        nasize: usize,
        allowed_growth: usize,
    ) -> QuotaTableWrite {
        let Some(existing) = self.tables.get(table) else {
            return QuotaTableWrite::Invalid;
        };
        if existing.readonly || existing.puc_hash().is_none() {
            return QuotaTableWrite::Invalid;
        }
        if nasize <= existing.array.len() {
            return QuotaTableWrite::Stored;
        }
        let old_table_usage = table_data_memory_usage(existing);
        let mut candidate = existing.clone();
        let node_len = candidate.puc_hash().map_or(0, |puc| puc.nodes.len());
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            puc_resize(&root, &mut candidate, nasize, node_len);
        });
        candidate.mutation_version = candidate.mutation_version.wrapping_add(1);
        let candidate_usage = table_data_memory_usage(&candidate);
        let growth = candidate_usage
            .live_bytes
            .saturating_sub(old_table_usage.live_bytes);
        if growth > allowed_growth {
            return QuotaTableWrite::WouldExceed;
        }
        *self
            .tables
            .get_mut(table)
            .expect("validated table remains available") = candidate;
        self.side_usage.adjust(old_table_usage, candidate_usage);
        QuotaTableWrite::Stored
    }

    #[cfg(test)]
    pub fn alloc_filled_array_table(&mut self, size: usize, value: Value) -> u32 {
        self.insert_table_data(TableData {
            array: vec![value; size],
            hash: empty_hash_part(self.table_layout),
            readonly: false,
            metatable: None,
            shape_version: 0,
            mutation_version: 0,
        })
        .expect("table slot limit exceeded")
    }

    pub(crate) fn try_alloc_filled_array_table(
        &mut self,
        size: usize,
        value: Value,
    ) -> Option<u32> {
        let mut array = Vec::new();
        array.try_reserve_exact(size).ok()?;
        array.resize(size, value);
        self.insert_table_data(TableData {
            array,
            hash: empty_hash_part(self.table_layout),
            readonly: false,
            metatable: None,
            shape_version: 0,
            mutation_version: 0,
        })
    }

    pub fn alloc_function(
        &mut self,
        program: Option<u32>,
        prototype: u32,
        captures: Vec<FunctionCapture>,
        environment: u32,
    ) -> Option<u32> {
        self.insert_function_data(FunctionData {
            kind: FunctionKind::Bytecode { program, prototype },
            captures: captures.into_iter().collect(),
            environment,
        })
    }

    pub fn alloc_dynamic_main_function(&mut self, program: u32, environment: u32) -> Option<u32> {
        self.insert_function_data(FunctionData {
            kind: FunctionKind::DynamicMain { program },
            captures: SmallVec::new(),
            environment,
        })
    }

    pub fn alloc_native_function(
        &mut self,
        native: NativeFunction,
        environment: u32,
    ) -> Option<u32> {
        self.insert_function_data(FunctionData {
            kind: FunctionKind::Native(native),
            captures: SmallVec::new(),
            environment,
        })
    }

    pub fn alloc_host_function(&mut self, host: u32, environment: u32) -> Option<u32> {
        self.tables.get(environment)?;
        self.insert_function_data(FunctionData {
            kind: FunctionKind::Host(host),
            captures: SmallVec::new(),
            environment,
        })
    }

    pub fn alloc_native_function_with_upvalues(
        &mut self,
        native: NativeFunction,
        upvalues: Vec<u32>,
        environment: u32,
    ) -> Option<u32> {
        for upvalue in &upvalues {
            self.upvalues.get(*upvalue)?;
        }
        self.insert_function_data(FunctionData {
            kind: FunctionKind::Native(native),
            captures: upvalues
                .into_iter()
                .map(FunctionCapture::from_cell)
                .collect(),
            environment,
        })
    }

    #[inline]
    pub fn function_kind(&self, handle: u32) -> Option<FunctionKind> {
        self.functions.get(handle).map(|function| function.kind)
    }

    pub(crate) fn referenced_programs(&self) -> Vec<u32> {
        self.functions
            .values()
            .filter_map(|function| match function.kind {
                FunctionKind::Bytecode {
                    program: Some(program),
                    ..
                }
                | FunctionKind::DynamicMain { program } => Some(program),
                FunctionKind::Bytecode { program: None, .. }
                | FunctionKind::Host(_)
                | FunctionKind::Native(_) => None,
            })
            .collect()
    }

    /// Returns the data needed to dispatch a function call in one arena access.
    ///
    /// Bytecode calls need their captured upvalues and environment immediately
    /// after inspecting the function kind. Native and host calls deliberately do
    /// not clone that metadata on their dispatch path.
    #[inline]
    pub fn function_dispatch_data(&self, handle: u32) -> Option<FunctionDispatchData> {
        let function = self.functions.get(handle)?;
        let bytecode_data = match function.kind {
            FunctionKind::Bytecode { .. } | FunctionKind::DynamicMain { .. } => {
                Some((function.captures.clone(), function.environment))
            }
            FunctionKind::Host(_) | FunctionKind::Native(_) => None,
        };
        Some((function.kind, bytecode_data))
    }

    #[inline]
    pub fn function_upvalues(&self, handle: u32) -> Option<SmallVec<[u32; 4]>> {
        let function = self.functions.get(handle)?;
        function
            .captures
            .iter()
            .copied()
            .map(FunctionCapture::cell)
            .collect()
    }

    #[inline]
    pub(crate) fn function_capture(&self, handle: u32, index: usize) -> Option<FunctionCapture> {
        self.functions.get(handle)?.captures.get(index).copied()
    }

    #[inline]
    pub(crate) fn capture_get(&self, capture: FunctionCapture) -> Option<Value> {
        if let Some(cell) = capture.cell() {
            self.upvalue_get(cell)
        } else {
            capture.direct_value()
        }
    }

    #[inline]
    pub(crate) fn capture_set(&mut self, capture: FunctionCapture, value: Value) -> Option<()> {
        self.upvalue_set(capture.cell()?, value)
    }

    #[inline]
    pub fn function_environment(&self, handle: u32) -> Option<u32> {
        self.functions
            .get(handle)
            .map(|function| function.environment)
    }

    pub fn set_function_environment(&mut self, handle: u32, environment: u32) -> Option<()> {
        let function = self.functions.get_mut(handle)?;
        function.environment = environment;
        Some(())
    }

    pub fn alloc_upvalue(&mut self, value: Value) -> Option<u32> {
        self.upvalues.insert(UpvalueData { value })
    }

    #[inline]
    pub fn upvalue_get(&self, handle: u32) -> Option<Value> {
        self.upvalues.get(handle).map(|upvalue| upvalue.value)
    }

    #[inline]
    pub fn upvalue_set(&mut self, handle: u32, value: Value) -> Option<()> {
        let upvalue = self.upvalues.get_mut(handle)?;
        upvalue.value = value;
        Some(())
    }

    pub fn alloc_userdata(&mut self, metatable: Option<u32>) -> Option<u32> {
        if let Some(table) = metatable {
            self.tables.get(table)?;
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.userdatas.insert(UserdataData {
                metatable,
                payload: UserdataPayload::None,
            })
        })
    }

    pub(crate) fn alloc_userdata_with_payload(
        &mut self,
        metatable: Option<u32>,
        payload: UserdataPayload,
    ) -> Option<u32> {
        if let Some(table) = metatable {
            self.tables.get(table)?;
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.userdatas.insert(UserdataData { metatable, payload })
        })
    }

    pub fn alloc_buffer(&mut self, bytes: Vec<u8>) -> Option<u32> {
        let usage = vec_memory_usage(&bytes);
        let handle = self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.buffers.insert(BufferData { bytes })
        })?;
        self.side_usage.add(usage.live_bytes, usage.reserved_bytes);
        Some(handle)
    }

    pub(crate) fn alloc_vector(&mut self, vector: VectorData) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.vectors.insert(vector)
        })
    }

    pub(crate) fn vector(&mut self, vector: u32) -> Option<VectorData> {
        self.arena
            .mutate(|mc, root| root.unlock(mc).borrow().vectors.get(vector).copied())
    }

    pub fn has_buffer(&mut self, buffer: u32) -> bool {
        self.arena
            .mutate(|mc, root| root.unlock(mc).borrow().buffers.get(buffer).is_some())
    }

    pub fn buffer_len(&mut self, buffer: u32) -> Option<usize> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .buffers
                .get(buffer)
                .map(|buffer| buffer.bytes.len())
        })
    }

    pub fn buffer_read(&mut self, buffer: u32, offset: usize, count: usize) -> Option<Vec<u8>> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let buffer = root.buffers.get(buffer)?;
            let end = offset.checked_add(count)?;
            (end <= buffer.bytes.len()).then(|| buffer.bytes[offset..end].to_vec())
        })
    }

    pub fn buffer_write(&mut self, buffer: u32, offset: usize, bytes: &[u8]) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let buffer = root.buffers.get_mut(buffer)?;
            let end = offset.checked_add(bytes.len())?;
            if end > buffer.bytes.len() {
                return None;
            }
            buffer.bytes[offset..end].copy_from_slice(bytes);
            Some(())
        })
    }

    pub fn buffer_fill(
        &mut self,
        buffer: u32,
        offset: usize,
        count: usize,
        byte: u8,
    ) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let buffer = root.buffers.get_mut(buffer)?;
            let end = offset.checked_add(count)?;
            if end > buffer.bytes.len() {
                return None;
            }
            buffer.bytes[offset..end].fill(byte);
            Some(())
        })
    }

    pub fn userdata_metatable(&mut self, userdata: u32) -> Option<Option<u32>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .userdatas
                .get(userdata)
                .map(|userdata| userdata.metatable)
        })
    }

    pub fn has_userdata(&mut self, userdata: u32) -> bool {
        self.arena
            .mutate(|mc, root| root.unlock(mc).borrow().userdatas.get(userdata).is_some())
    }

    pub(crate) fn userdata_payload(&mut self, userdata: u32) -> Option<UserdataPayload> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .userdatas
                .get(userdata)
                .map(|userdata| userdata.payload)
        })
    }

    pub(crate) fn set_userdata_payload(
        &mut self,
        userdata: u32,
        payload: UserdataPayload,
    ) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let userdata = root.userdatas.get_mut(userdata)?;
            userdata.payload = payload;
            Some(())
        })
    }

    #[inline]
    fn table_key_from_value(&mut self, value: Value) -> Option<TableKey> {
        if value.kind() != ValueKind::Vector {
            return table_key_from_nonvector_value(value);
        }
        let vector = self.vector(value.as_vector()?)?;
        vector.table_key_bits().map(TableKey::Vector)
    }

    #[inline]
    fn table_key_visible_value(&mut self, key: TableKey, original: Value) -> Option<Value> {
        match key {
            TableKey::Number(bits) => Some(Value::number(f64::from_bits(bits))),
            TableKey::Vector(bits) => self
                .alloc_vector(VectorData::new(
                    f32::from_bits(bits[0]),
                    f32::from_bits(bits[1]),
                    f32::from_bits(bits[2]),
                ))
                .map(Value::vector),
            _ => Some(original),
        }
    }

    pub fn table_get(&mut self, table: u32, key: Value) -> Option<Value> {
        let table_key = if let Some(number) = key.as_number() {
            number_table_key(number)?
        } else if let Some(short) = key.as_short_string() {
            self.tables.get(table)?;
            TableKey::String(self.find_string_atom_bytes(short.as_bytes())?)
        } else if key.kind() == ValueKind::Vector {
            self.tables.get(table)?;
            self.table_key_from_value(key)?
        } else {
            self.table_key_from_value(key)?
        };
        let table_data = self.tables.get(table)?;
        if table_data.puc_hash().is_some() {
            // PUC hashing needs string bytes for main positions.
            let tables = &self.tables;
            return self.arena.mutate(|mc, root| {
                let root = root.unlock(mc).borrow();
                let table = tables.get(table)?;
                puc_lookup(&root, table, table_key)
            });
        }
        if let Some(index) = table_key_array_index(table_key) {
            if let Some(value) = table_data.array.get(index - 1) {
                return Some(*value);
            }
        }
        let HashPart::Log { entries, index } = &table_data.hash else {
            debug_assert!(false, "table layout changed between checks");
            return None;
        };
        table_hash_entry(entries, index, table_key).map(|entry| entry.value)
    }

    /// Reads an already-materialized numeric array slot without constructing or hashing a key.
    ///
    /// A missing table and an index outside the current array both return `None`; callers that
    /// need to distinguish those cases must use the general table path.
    pub(crate) fn read_numeric_array_slot(
        &mut self,
        table: u32,
        one_based_index: usize,
    ) -> Option<Value> {
        if one_based_index == 0 {
            return None;
        }
        self.tables
            .get(table)?
            .array
            .get(one_based_index - 1)
            .copied()
    }

    pub(crate) fn read_array_prefix(&mut self, table: u32, len: usize) -> Option<Vec<Value>> {
        let table = self.tables.get(table)?;
        Some(table.array.get(..len)?.to_vec())
    }

    pub(crate) fn try_write_array_prefix(
        &mut self,
        table: u32,
        values: &[Value],
    ) -> Option<TableNumericArrayWrite> {
        let table_data = self.tables.get(table)?;
        if table_data.readonly {
            return Some(TableNumericArrayWrite::Readonly);
        }
        if values.len() > table_data.array.len() {
            return Some(TableNumericArrayWrite::Deferred);
        }
        if table_data.puc_hash().is_some() {
            // PUC: array-range keys are never in the node part, so a prefix
            // write that fits the existing array is a pure slot write.
            return self.mutate_table_data(table, |table| {
                for (offset, value) in values.iter().copied().enumerate() {
                    table.array[offset] = value;
                }
                TableNumericArrayWrite::Stored
            });
        }
        self.mutate_table_data(table, |table| {
            let HashPart::Log { entries, index } = &mut table.hash else {
                unreachable!("Log layout checked above")
            };
            for (offset, value) in values.iter().copied().enumerate() {
                table.array[offset] = value;
                if entries.is_empty() {
                    continue;
                }
                let key = number_table_key((offset + 1) as f64)
                    .expect("materialized array offsets are valid numeric table keys");
                if value.is_nil() {
                    if let Some(entry) = table_hash_entry_mut(entries, index, key) {
                        entry.value = Value::nil();
                    }
                } else {
                    remove_hash_key(entries, index, &mut table.shape_version, key);
                }
            }
            TableNumericArrayWrite::Stored
        })
    }

    /// VM Core 2 C2.5a: commits a write to an array slot the caller has
    /// already proven present and non-nil.
    ///
    /// A present key bypasses `__newindex` in both layouts, so this lane needs
    /// no metatable test, and a present array slot shadows any hash/node-part
    /// entry carrying the same numeric key (`table_numeric_get` and
    /// `puc_getnum` both read the array first, and every array-growth site
    /// removes the numeric hash key as it fills the slot). Overwriting the
    /// slot therefore cannot allocate, grow, shrink, rehash, or change any
    /// array/hash length or capacity - exactly the `table_data_memory_usage`
    /// input set - so the write skips `mutate_table_data`'s before/after
    /// census and its no-op `side_usage` adjustment and performs only the one
    /// guest-visible `mutation_version` bump. Storing nil is included: it
    /// clears a present slot in place without moving the array boundary.
    /// Readonly is the only remaining rejection.
    #[allow(dead_code)]
    #[inline]
    fn write_present_array_slot(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Option<TableNumericArrayWrite> {
        let table_data = self.tables.get_mut(table)?;
        debug_assert!(
            table_data
                .array
                .get(one_based_index - 1)
                .is_some_and(|existing| !existing.is_nil()),
            "the caller must prove the array slot is present and non-nil"
        );
        debug_assert!(
            numeric_array_slot_shadows_hash_part(table_data, one_based_index),
            "a non-nil array slot must shadow every hash entry for the same numeric key"
        );
        if table_data.readonly {
            return Some(TableNumericArrayWrite::Readonly);
        }
        table_data.array[one_based_index - 1] = value;
        table_data.mutation_version = table_data.mutation_version.wrapping_add(1);
        #[cfg(any(test, feature = "table-write-diagnostics"))]
        {
            self.numeric_array_slot_writes.direct =
                self.numeric_array_slot_writes.direct.wrapping_add(1);
        }
        Some(TableNumericArrayWrite::Stored)
    }

    /// Stores a numeric array slot when doing so cannot observe `__newindex`.
    ///
    /// Existing non-nil slots always bypass `__newindex`. Missing slots may be stored directly
    /// only on tables without a metatable; sparse/hash cases stay on the general path.
    pub(crate) fn try_write_numeric_array_slot(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Option<TableNumericArrayWrite> {
        #[cfg(any(test, feature = "table-write-diagnostics"))]
        {
            self.numeric_array_slot_writes.calls =
                self.numeric_array_slot_writes.calls.wrapping_add(1);
        }
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(TableNumericArrayWrite::Deferred);
        }
        // VM Core 2 C2.5a admits the direct existing-slot lane from inside
        // each layout branch, on the `existing` value that branch already
        // loads, so a lane miss costs nothing beyond one test the branch was
        // going to make anyway.
        let table_data = self.tables.get(table)?;
        if table_data.puc_hash().is_some() {
            // PUC: the array never grows outside rehash, so out-of-range
            // indices defer to the general path (node part); in-range writes
            // keep the metatable/readonly checks.
            if one_based_index > table_data.array.len() {
                return Some(TableNumericArrayWrite::Deferred);
            }
            let existing = table_data.array[one_based_index - 1];
            if existing.is_nil() && table_data.metatable.is_some() {
                return Some(TableNumericArrayWrite::Deferred);
            }
            if table_data.readonly {
                return Some(TableNumericArrayWrite::Readonly);
            }
            let stored = self.mutate_table_data(table, |table| {
                table.array[one_based_index - 1] = value;
                TableNumericArrayWrite::Stored
            });
            #[cfg(any(test, feature = "table-write-diagnostics"))]
            {
                self.numeric_array_slot_writes.generic_stored = self
                    .numeric_array_slot_writes
                    .generic_stored
                    .wrapping_add(u64::from(stored.is_some()));
            }
            return stored;
        }
        let existing = table_data
            .array
            .get(one_based_index - 1)
            .copied()
            .unwrap_or_else(Value::nil);
        if existing.is_nil() && table_data.metatable.is_some() {
            return Some(TableNumericArrayWrite::Deferred);
        }
        if table_data.readonly {
            return Some(TableNumericArrayWrite::Readonly);
        }
        if value.is_nil() && one_based_index > table_data.array.len() {
            // A numeric hash entry can exist beyond the array boundary.
            return Some(TableNumericArrayWrite::Deferred);
        }
        if !value.is_nil() && one_based_index > table_data.array.len() + 1 {
            return Some(TableNumericArrayWrite::Deferred);
        }
        let stored = self.mutate_table_data(table, |table| {
            if value.is_nil() {
                let slot = table
                    .array
                    .get_mut(one_based_index - 1)
                    .expect("validated array slot remains available");
                *slot = Value::nil();
                return TableNumericArrayWrite::Stored;
            }
            if one_based_index > table.array.len() {
                table.array.resize(one_based_index, Value::nil());
            }
            table.array[one_based_index - 1] = value;
            let HashPart::Log { entries, index } = &mut table.hash else {
                unreachable!("Log layout checked above")
            };
            if !entries.is_empty() {
                let key = number_table_key(one_based_index as f64)
                    .expect("validated Luau array indices are valid numeric table keys");
                remove_hash_key(entries, index, &mut table.shape_version, key);
            }
            TableNumericArrayWrite::Stored
        });
        #[cfg(any(test, feature = "table-write-diagnostics"))]
        {
            self.numeric_array_slot_writes.generic_stored = self
                .numeric_array_slot_writes
                .generic_stored
                .wrapping_add(u64::from(stored.is_some()));
        }
        stored
    }

    pub(crate) fn table_string_hash_probe(
        &mut self,
        table: u32,
        atom: u32,
    ) -> Option<Option<(Value, TableStringHashSlot)>> {
        let table = self.tables.get(table)?;
        let HashPart::Log { entries, index } = &table.hash else {
            // PUC node slots are not stable across rehash: the inline cache
            // is bypassed and the general path handles the lookup.
            return Some(None);
        };
        let Some(index) = table_hash_entry_index(entries, index, TableKey::String(atom)) else {
            return Some(None);
        };
        let Some(entry) = entries.get(index) else {
            return Some(None);
        };
        Some(Some((
            entry.value,
            TableStringHashSlot {
                shape_version: table.shape_version,
                index: u32::try_from(index).ok()?,
            },
        )))
    }

    pub(crate) fn read_string_hash_slot(
        &mut self,
        table: u32,
        atom: u32,
        slot: TableStringHashSlot,
    ) -> Option<Option<Value>> {
        let table = self.tables.get(table)?;
        let HashPart::Log { entries, .. } = &table.hash else {
            return Some(None);
        };
        if table.shape_version != slot.shape_version {
            return Some(None);
        }
        let Some(entry) = entries.get(slot.index as usize) else {
            return Some(None);
        };
        if entry.key != TableKey::String(atom) {
            return Some(None);
        }
        Some(Some(entry.value))
    }

    pub(crate) fn write_string_hash_slot(
        &mut self,
        table: u32,
        atom: u32,
        slot: TableStringHashSlot,
        value: Value,
    ) -> Option<TableStringHashWrite> {
        let table = self.tables.get_mut(table)?;
        let HashPart::Log { entries, .. } = &mut table.hash else {
            return Some(TableStringHashWrite::Deferred);
        };
        if table.shape_version != slot.shape_version {
            return Some(TableStringHashWrite::Deferred);
        }
        let Some(entry) = entries.get_mut(slot.index as usize) else {
            return Some(TableStringHashWrite::Deferred);
        };
        if entry.key != TableKey::String(atom) || entry.value.is_nil() {
            return Some(TableStringHashWrite::Deferred);
        }
        if table.readonly {
            return Some(TableStringHashWrite::Readonly);
        }
        entry.value = value;
        table.mutation_version = table.mutation_version.wrapping_add(1);
        Some(TableStringHashWrite::Stored)
    }

    pub fn table_set(&mut self, table: u32, key: Value, value: Value) -> Option<()> {
        let key_value = key;
        let table_key = if let Some(number) = key.as_number() {
            number_table_key(number)?
        } else {
            let key = if let Some(short) = key.as_short_string() {
                let stable_hash = stable_bytes_hash(short.as_bytes());
                let atom = if let Some(atom) = self.find_string_atom_bytes(short.as_bytes()) {
                    atom
                } else {
                    let table = self.tables.get(table)?;
                    if table.readonly {
                        return None;
                    }
                    // PUC creates a tombstone node when an absent key is set
                    // to nil, so the atom must be interned as the key; only
                    // Log tables may skip the write entirely.
                    if value.is_nil() && table.puc_hash().is_none() {
                        return Some(());
                    }
                    self.intern_inline_string(short, stable_hash)
                };
                Value::string_atom(atom)
            } else {
                key
            };
            self.table_key_from_value(key)?
        };
        let is_puc = {
            let table = self.tables.get(table)?;
            if table.readonly {
                return None;
            }
            table.puc_hash().is_some()
        };
        if is_puc {
            let key_value = self.table_key_visible_value(table_key, key_value)?;
            return self.mutate_table_data_with_root(table, |root, table| {
                puc_store(root, table, table_key, key_value, value, false);
            });
        }
        let creates_hash_entry = !value.is_nil() && {
            let table = self.tables.get(table)?;
            let HashPart::Log { entries, index } = &table.hash else {
                debug_assert!(false, "table layout changed between checks");
                return None;
            };
            matches!(
                table_hash_lookup(entries, index, table_key),
                TableHashLookup::Vacant(_)
            )
        };
        let key_value = if matches!(table_key, TableKey::Vector(_)) && !creates_hash_entry {
            key_value
        } else {
            self.table_key_visible_value(table_key, key_value)?
        };
        self.mutate_table_data(table, |table| {
            set_table_data(table, table_key, key_value, value);
        })
    }

    /// Stages the complete table update before committing it, so a quota
    /// rejection cannot leave a guest-visible partial write.
    pub(crate) fn table_set_with_live_growth_limit(
        &mut self,
        table: u32,
        key: Value,
        value: Value,
        allowed_growth: usize,
    ) -> QuotaTableWrite {
        let key_value = key;
        let mut pending_inline_atom = None;
        let mut pending_vector = None;
        let mut allocation_growth = 0_usize;
        let table_key = if let Some(number) = key.as_number() {
            let Some(key) = number_table_key(number) else {
                return QuotaTableWrite::Invalid;
            };
            key
        } else {
            let key = if let Some(short) = key.as_short_string() {
                let stable_hash = stable_bytes_hash(short.as_bytes());
                let atom = if let Some(atom) = self.find_string_atom_bytes(short.as_bytes()) {
                    atom
                } else {
                    let Some((atom, growth)) = self.planned_inline_string_intern() else {
                        return QuotaTableWrite::Invalid;
                    };
                    pending_inline_atom = Some((short, stable_hash, atom));
                    allocation_growth = allocation_growth.saturating_add(growth);
                    atom
                };
                Value::string_atom(atom)
            } else {
                key
            };
            let Some(key) = self.table_key_from_value(key) else {
                return QuotaTableWrite::Invalid;
            };
            key
        };
        let (old_table_usage, mut candidate) = {
            let Some(existing) = self.tables.get(table) else {
                return QuotaTableWrite::Invalid;
            };
            if existing.readonly {
                return QuotaTableWrite::Invalid;
            }
            (table_data_memory_usage(existing), existing.clone())
        };
        let is_puc = candidate.puc_hash().is_some();
        let creates_hash_entry = if is_puc {
            // PUC nil-stores to absent keys create tombstones, so value
            // nilness does not clear `creates_hash_entry` here.
            self.arena.mutate(|mc, root| {
                let root = root.unlock(mc).borrow();
                puc_creates_hash_entry(&root, &candidate, table_key)
            })
        } else {
            let HashPart::Log { entries, index } = &candidate.hash else {
                unreachable!("Log layout confirmed above")
            };
            !value.is_nil()
                && matches!(
                    table_hash_lookup(entries, index, table_key),
                    TableHashLookup::Vacant(_)
                )
        };
        if !creates_hash_entry {
            pending_inline_atom = None;
            allocation_growth = 0;
        } else if let TableKey::Vector(bits) = table_key {
            let Some((vector, growth)) = self.planned_vector_allocation() else {
                return QuotaTableWrite::Invalid;
            };
            pending_vector = Some((bits, vector));
            allocation_growth = allocation_growth.saturating_add(growth);
        }
        let key_value = match table_key {
            TableKey::Number(bits) => Value::number(f64::from_bits(bits)),
            TableKey::Vector(_) => {
                pending_vector.map_or(key_value, |(_, vector)| Value::vector(vector))
            }
            _ => key_value,
        };
        if is_puc {
            self.arena.mutate(|mc, root| {
                let root = root.unlock(mc).borrow();
                puc_store(&root, &mut candidate, table_key, key_value, value, false);
            });
        } else {
            set_table_data(&mut candidate, table_key, key_value, value);
        }
        candidate.mutation_version = candidate.mutation_version.wrapping_add(1);
        let candidate_usage = table_data_memory_usage(&candidate);
        let table_growth = candidate_usage
            .live_bytes
            .saturating_sub(old_table_usage.live_bytes);
        if allocation_growth.saturating_add(table_growth) > allowed_growth {
            return QuotaTableWrite::WouldExceed;
        }
        if let Some((short, stable_hash, planned_atom)) = pending_inline_atom {
            let atom = self.intern_inline_string(short, stable_hash);
            debug_assert_eq!(atom, planned_atom);
        }
        if let Some((bits, planned_vector)) = pending_vector {
            let vector = self
                .alloc_vector(VectorData::new(
                    f32::from_bits(bits[0]),
                    f32::from_bits(bits[1]),
                    f32::from_bits(bits[2]),
                ))
                .expect("validated vector allocation plan remains available");
            debug_assert_eq!(vector, planned_vector);
        }
        *self
            .tables
            .get_mut(table)
            .expect("validated table remains available") = candidate;
        self.side_usage.adjust(old_table_usage, candidate_usage);
        QuotaTableWrite::Stored
    }

    pub(crate) fn table_set_array_slot_with_live_growth_limit(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
        allowed_growth: usize,
    ) -> QuotaTableWrite {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return QuotaTableWrite::Invalid;
        }
        let Some(existing) = self.tables.get(table) else {
            return QuotaTableWrite::Invalid;
        };
        if existing.readonly {
            return QuotaTableWrite::Invalid;
        }
        let old_table_usage = table_data_memory_usage(existing);
        let mut candidate = existing.clone();
        let Some(key) = number_table_key(one_based_index as f64) else {
            return QuotaTableWrite::Invalid;
        };
        if candidate.puc_hash().is_some() {
            // PUC: an in-range index is a pure slot write; anything beyond
            // the array goes to the node part (never Invalid for in-range
            // i32 indices).
            let key_value = Value::number(one_based_index as f64);
            self.arena.mutate(|mc, root| {
                let root = root.unlock(mc).borrow();
                puc_store(&root, &mut candidate, key, key_value, value, false);
            });
        } else {
            if one_based_index > candidate.array.len() {
                candidate.array.resize(one_based_index, Value::nil());
            }
            candidate.array[one_based_index - 1] = value;
            let HashPart::Log { entries, index } = &mut candidate.hash else {
                unreachable!("Log layout confirmed above")
            };
            if !entries.is_empty() {
                remove_hash_key(entries, index, &mut candidate.shape_version, key);
            }
        }
        candidate.mutation_version = candidate.mutation_version.wrapping_add(1);
        let candidate_usage = table_data_memory_usage(&candidate);
        let growth = candidate_usage
            .live_bytes
            .saturating_sub(old_table_usage.live_bytes);
        if growth > allowed_growth {
            return QuotaTableWrite::WouldExceed;
        }
        *self
            .tables
            .get_mut(table)
            .expect("validated table remains available") = candidate;
        self.side_usage.adjust(old_table_usage, candidate_usage);
        QuotaTableWrite::Stored
    }

    pub fn table_set_array_slot(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Option<()> {
        if one_based_index == 0 {
            return None;
        }
        if self.tables.get(table)?.readonly || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return None;
        }
        let key = number_table_key(one_based_index as f64)?;
        if self.tables.get(table)?.puc_hash().is_some() {
            let key_value = Value::number(one_based_index as f64);
            return self.mutate_table_data_with_root(table, |root, table| {
                puc_store(root, table, key, key_value, value, false);
            });
        }
        self.mutate_table_data(table, |table| {
            if one_based_index > table.array.len() {
                table.array.resize(one_based_index, Value::nil());
            }
            table.array[one_based_index - 1] = value;
            let HashPart::Log { entries, index } = &mut table.hash else {
                unreachable!("Log layout checked above")
            };
            if !entries.is_empty() {
                remove_hash_key(entries, index, &mut table.shape_version, key);
            }
        })
    }

    pub fn table_metatable(&mut self, table: u32) -> Option<Option<u32>> {
        self.tables.get(table).map(|table| table.metatable)
    }

    pub fn table_set_metatable(&mut self, table: u32, metatable: Option<u32>) -> Option<()> {
        if let Some(metatable) = metatable {
            self.tables.get(metatable)?;
        }
        let table = self.tables.get_mut(table)?;
        if table.metatable != metatable {
            table.metatable = metatable;
            table.mutation_version = table.mutation_version.wrapping_add(1);
        }
        Some(())
    }

    pub fn table_clone(&mut self, table: u32) -> Option<u32> {
        let mut clone = self.tables.get(table)?.clone();
        clone.readonly = false;
        self.insert_table_data(clone)
    }

    pub fn array_get(&mut self, table: u32, one_based_index: usize) -> Option<Value> {
        self.table_get(table, Value::number(one_based_index as f64))
    }

    pub fn array_len(&mut self, table: u32) -> Option<usize> {
        self.tables.get(table).map(table_array_boundary_len)
    }

    pub fn table_storage_len(&mut self, table: u32) -> Option<usize> {
        let table = self.tables.get(table)?;
        let hash_len = match &table.hash {
            HashPart::Log { entries, .. } => entries.len(),
            HashPart::Puc(puc) => puc.nodes.len(),
        };
        Some(table.array.len().saturating_add(hash_len))
    }

    pub(crate) fn table_mutation_version(&mut self, table: u32) -> Option<u64> {
        self.tables.get(table).map(|table| table.mutation_version)
    }

    pub fn table_clear(&mut self, table: u32) -> Option<()> {
        if self.tables.get(table)?.readonly {
            return None;
        }
        self.mutate_table_data(table, |table| {
            bump_table_shape_version(table);
            table.array.fill(Value::nil());
            match &mut table.hash {
                HashPart::Log { entries, index } => {
                    entries.clear();
                    *index = Vec::new();
                }
                HashPart::Puc(puc) => {
                    puc.nodes.fill(PUC_FREE_NODE);
                    puc.lastfree = puc.nodes.len();
                }
            }
        })
    }

    pub fn table_insert(&mut self, table: u32, position: i32, value: Value) -> Option<()> {
        if self.tables.get(table)?.readonly {
            return None;
        }
        if self.tables.get(table)?.puc_hash().is_some() {
            // luaB_tinsert shape: shifting goes through get/set, so
            // out-of-array indices land in the node part.
            return self.mutate_table_data_with_root(table, |root, table| {
                let len = table_array_boundary_len(table);

                if position >= 1 {
                    let position = position as usize;
                    if position <= len {
                        for index in (position..=len).rev() {
                            let value = table_numeric_get(table, index);
                            puc_store_num(root, table, (index + 1) as i64, value, false);
                        }
                    }
                }
                puc_store_num(root, table, i64::from(position), value, false);
            });
        }
        self.mutate_table_data(table, |table| {
            let len = table_array_boundary_len(table);

            if position >= 1 {
                let position = position as usize;
                if position <= len {
                    for index in (position..=len).rev() {
                        let value = table_numeric_get(table, index);
                        table_numeric_set_array_index(table, index + 1, value);
                    }
                }
            }
            table_numeric_set(table, position, value);
        })
    }

    pub fn table_remove(&mut self, table: u32, position: i32) -> Option<Option<Value>> {
        if self.tables.get(table)?.readonly {
            return None;
        }
        let len = table_array_boundary_len(self.tables.get(table)?);
        if !(1..=i32::try_from(len).ok()?).contains(&position) {
            return Some(None);
        }
        if self.tables.get(table)?.puc_hash().is_some() {
            // luaB_tremove shape: the final nil-store on a key outside the
            // array part creates a tombstone node, as luaH_setnum would.
            return self.mutate_table_data_with_root(table, |root, table| {
                let position = position as usize;
                let removed = table_numeric_get(table, position);
                for index in position..len {
                    let value = table_numeric_get(table, index + 1);
                    puc_store_num(root, table, index as i64, value, false);
                }
                puc_store_num(root, table, len as i64, Value::nil(), false);
                Some(removed)
            });
        }
        self.mutate_table_data(table, |table| {
            let position = position as usize;
            let removed = table_numeric_get(table, position);
            for index in position..len {
                let value = table_numeric_get(table, index + 1);
                table_numeric_set_array_index(table, index, value);
            }
            table_numeric_set_array_index(table, len, Value::nil());
            Some(removed)
        })
    }

    pub fn max_numeric_key(&mut self, table: u32) -> Option<f64> {
        let table = self.tables.get(table)?;
        let array_max = table
            .array
            .iter()
            .enumerate()
            .rev()
            .find(|(_, value)| value.kind() != ValueKind::Nil)
            .map(|(index, _)| (index + 1) as f64)
            .unwrap_or(0.0);
        match &table.hash {
            HashPart::Log { entries, .. } => Some(entries.iter().fold(array_max, |max, entry| {
                if entry.value.kind() == ValueKind::Nil {
                    return max;
                }
                let TableKey::Number(bits) = entry.key else {
                    return max;
                };
                let key = f64::from_bits(bits);
                if key > max {
                    key
                } else {
                    max
                }
            })),
            HashPart::Puc(puc) => Some(puc.nodes.iter().fold(array_max, |max, node| {
                if node.value.kind() == ValueKind::Nil {
                    return max;
                }
                let PucNodeKey::Live(TableKey::Number(bits)) = node.key else {
                    return max;
                };
                let key = f64::from_bits(bits);
                if key > max {
                    key
                } else {
                    max
                }
            })),
        }
    }

    pub fn table_next_after(
        &mut self,
        table: u32,
        after: Value,
        order: TableIterationOrder,
    ) -> Option<TableNext> {
        let after = if let Some(short) = after.as_short_string() {
            let Some(atom) = self.find_string_atom_bytes(short.as_bytes()) else {
                return self.has_table(table).then_some(TableNext::InvalidKey);
            };
            Value::string_atom(atom)
        } else {
            after
        };
        self.tables.get(table)?;
        let table_key = match after.kind() {
            ValueKind::Nil => None,
            _ => Some(self.table_key_from_value(after)),
        };
        let tables = &self.tables;
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = tables.get(table)?;
            match after.kind() {
                ValueKind::Nil => Some(
                    first_non_nil_table_entry(&root, table, order)
                        .map_or(TableNext::End, |(key, value)| TableNext::Entry(key, value)),
                ),
                ValueKind::Number => {
                    let Some(key) = table_key.flatten() else {
                        return Some(TableNext::InvalidKey);
                    };
                    if let Some(index) = table_key_array_index(key) {
                        if index <= table.array.len() {
                            return Some(
                                next_after_array_index(&root, table, index, order)
                                    .map_or(TableNext::End, |(key, value)| {
                                        TableNext::Entry(key, value)
                                    }),
                            );
                        }
                    }
                    Some(next_after_hash_key(&root, table, key, order).map_or(
                        TableNext::InvalidKey,
                        |entry| {
                            entry
                                .map_or(TableNext::End, |(key, value)| TableNext::Entry(key, value))
                        },
                    ))
                }
                _ => {
                    let Some(key) = table_key.flatten() else {
                        return Some(TableNext::InvalidKey);
                    };
                    Some(next_after_hash_key(&root, table, key, order).map_or(
                        TableNext::InvalidKey,
                        |entry| {
                            entry
                                .map_or(TableNext::End, |(key, value)| TableNext::Entry(key, value))
                        },
                    ))
                }
            }
        })
    }

    pub fn table_is_readonly(&mut self, table: u32) -> Option<bool> {
        self.tables.get(table).map(|table| table.readonly)
    }

    pub fn table_freeze(&mut self, table: u32) -> Option<bool> {
        let table = self.tables.get_mut(table)?;
        let was_readonly = table.readonly;
        if !was_readonly {
            table.readonly = true;
            table.mutation_version = table.mutation_version.wrapping_add(1);
        }
        Some(was_readonly)
    }

    pub fn has_table(&mut self, table: u32) -> bool {
        self.tables.get(table).is_some()
    }
}

#[inline]
fn string_index_lookup(root: &RootData<'_>, value: &[u8], stable_hash: u64) -> Option<u32> {
    if root.string_index.is_empty() {
        return None;
    }

    debug_assert!(root.string_index.len().is_power_of_two());
    let mut position = string_index_position(stable_hash, root.string_index.len());
    for _ in 0..root.string_index.len() {
        let slot = root.string_index[position];
        if slot == 0 {
            return None;
        }
        let atom = slot - 1;
        let indexed_value = root.strings.get(atom);
        debug_assert!(
            indexed_value.is_some(),
            "string index contains an invalid atom"
        );
        if indexed_value.is_some_and(|candidate| candidate.as_bytes() == value) {
            return Some(atom);
        }
        position = (position + 1) & (root.string_index.len() - 1);
    }
    None
}

fn ensure_string_index_capacity(root: &mut RootData<'_>) {
    let required = root.strings.occupied_len().saturating_add(1);
    if root.string_index.is_empty() || required.saturating_mul(2) > root.string_index.len() {
        let capacity = required
            .saturating_mul(2)
            .next_power_of_two()
            .max(STRING_INDEX_MIN_CAPACITY);
        rebuild_string_index(root, capacity);
    }
}

fn rebuild_string_index(root: &mut RootData<'_>, requested_capacity: usize) {
    let required_capacity = root
        .strings
        .occupied_len()
        .saturating_mul(2)
        .next_power_of_two()
        .max(STRING_INDEX_MIN_CAPACITY);
    let capacity = requested_capacity
        .max(required_capacity)
        .next_power_of_two();
    root.string_index.resize(capacity, 0);
    root.string_index.shrink_to_fit();
    root.string_index.fill(0);
    for (index, slot) in root.strings.slots.iter().enumerate() {
        let Some(value) = &slot.value else {
            continue;
        };
        insert_string_index_hash(
            &mut root.string_index,
            slot_handle(index as u32, slot.generation),
            value.stable_hash(),
        );
    }
}

fn insert_string_index(root: &mut RootData<'_>, atom: u32) {
    let value = root
        .strings
        .get(atom)
        .expect("string index insertion references an invalid atom");
    insert_string_index_hash(&mut root.string_index, atom, value.stable_hash());
}

fn insert_string_index_hash(string_index: &mut [u32], atom: u32, stable_hash: u64) {
    debug_assert!(!string_index.is_empty());
    debug_assert!(string_index.len().is_power_of_two());
    let encoded = atom
        .checked_add(1)
        .expect("string atom table exceeds compact u32 index address space");
    let mut position = string_index_position(stable_hash, string_index.len());
    for _ in 0..string_index.len() {
        if string_index[position] == 0 {
            string_index[position] = encoded;
            return;
        }
        position = (position + 1) & (string_index.len() - 1);
    }
    panic!("string index has no vacant slot");
}

#[inline]
fn string_index_position(stable_hash: u64, len: usize) -> usize {
    debug_assert!(len.is_power_of_two());
    (stable_hash as usize) & (len - 1)
}

fn table_key_from_nonvector_value(value: Value) -> Option<TableKey> {
    match value.kind() {
        ValueKind::Nil => None,
        ValueKind::Boolean => value.as_bool().map(TableKey::Boolean),
        ValueKind::Number => {
            let number = value.as_number()?;
            number_table_key(number)
        }
        ValueKind::String => value.as_string_atom().map(TableKey::String),
        ValueKind::Table => value.as_table().map(TableKey::Table),
        ValueKind::Function => value.as_function().map(TableKey::Function),
        ValueKind::Userdata => value.as_userdata().map(TableKey::Userdata),
        ValueKind::Buffer => value.as_buffer().map(TableKey::Buffer),
        ValueKind::Vector => None,
    }
}

fn table_key_array_index(key: TableKey) -> Option<usize> {
    let TableKey::Number(bits) = key else {
        return None;
    };
    let value = f64::from_bits(bits);
    let index = value as i32;
    if index >= 1 && f64::from(index) == value {
        Some(index as usize)
    } else {
        None
    }
}

fn table_numeric_get(table: &TableData, one_based_index: usize) -> Value {
    if one_based_index == 0 {
        return Value::nil();
    }

    if let HashPart::Puc(puc) = &table.hash {
        return puc_getnum(table, puc, one_based_index);
    }

    if let Some(value) = table.array.get(one_based_index - 1) {
        return *value;
    }

    let Some(key) = number_table_key(one_based_index as f64) else {
        return Value::nil();
    };
    let HashPart::Log { entries, index } = &table.hash else {
        debug_assert!(false, "table layout changed between checks");
        return Value::nil();
    };
    table_hash_entry(entries, index, key).map_or_else(Value::nil, |entry| entry.value)
}

fn bump_table_shape_version(table: &mut TableData) {
    table.shape_version = table.shape_version.saturating_add(1);
}

fn table_numeric_set(table: &mut TableData, index: i32, value: Value) {
    if let Ok(index) = usize::try_from(index) {
        if index >= 1 {
            table_numeric_set_array_index(table, index, value);
            return;
        }
    }
    set_hash_number_key(table, f64::from(index), value);
}

fn table_numeric_set_array_index(table: &mut TableData, index: usize, value: Value) {
    let key_value = Value::number(index as f64);
    let Some(key) = number_table_key(index as f64) else {
        return;
    };
    let HashPart::Log {
        entries,
        index: hash_index,
    } = &mut table.hash
    else {
        // Only called from the Log table_insert/table_remove shift loops.
        debug_assert!(false, "Log numeric set on a PUC-layout table");
        return;
    };

    if value.kind() == ValueKind::Nil {
        if let Some(slot) = table.array.get_mut(index - 1) {
            *slot = Value::nil();
        }
        if !entries.is_empty() {
            remove_hash_key(entries, hash_index, &mut table.shape_version, key);
        }
    } else if index <= table.array.len() + 1 {
        if index > table.array.len() {
            table.array.resize(index, Value::nil());
        }
        table.array[index - 1] = value;
        if !entries.is_empty() {
            remove_hash_key(entries, hash_index, &mut table.shape_version, key);
        }
    } else {
        set_hash_key(
            entries,
            hash_index,
            &mut table.shape_version,
            key,
            key_value,
            value,
        );
    }
}

fn set_hash_number_key(table: &mut TableData, number: f64, value: Value) {
    let Some(key) = number_table_key(number) else {
        return;
    };
    let key_value = match key {
        TableKey::Number(bits) => Value::number(f64::from_bits(bits)),
        _ => Value::number(number),
    };
    let HashPart::Log { entries, index } = &mut table.hash else {
        // Only called from the Log table_insert/table_remove shift loops.
        debug_assert!(false, "Log numeric set on a PUC-layout table");
        return;
    };
    set_hash_key(
        entries,
        index,
        &mut table.shape_version,
        key,
        key_value,
        value,
    );
}

fn table_data_live_bytes(table: &TableData) -> usize {
    let hash_bytes = match &table.hash {
        HashPart::Log { entries, index } => entries
            .len()
            .saturating_mul(size_of::<TableEntry>())
            .saturating_add(index.len().saturating_mul(size_of::<u32>())),
        HashPart::Puc(puc) => puc.nodes.len().saturating_mul(size_of::<PucNode>()),
    };
    table
        .array
        .len()
        .saturating_mul(size_of::<Value>())
        .saturating_add(hash_bytes)
}

fn table_data_reserved_bytes(table: &TableData) -> usize {
    let hash_bytes = match &table.hash {
        HashPart::Log { entries, index } => entries
            .capacity()
            .saturating_mul(size_of::<TableEntry>())
            .saturating_add(index.capacity().saturating_mul(size_of::<u32>())),
        HashPart::Puc(puc) => puc.nodes.capacity().saturating_mul(size_of::<PucNode>()),
    };
    table
        .array
        .capacity()
        .saturating_mul(size_of::<Value>())
        .saturating_add(hash_bytes)
}

/// Debug-only proof for the C2.5a guard: a materialized non-nil array slot
/// shadows any hash/node-part entry that carries the same numeric key, so
/// overwriting the slot in place cannot leave a stale duplicate behind.
///
/// Every array-growth site (`set_table_data`, `table_numeric_set_array_index`,
/// `try_write_array_prefix`, `table_set_array_slot`, and this function's own
/// generic tail) removes the numeric hash key as it fills the slot, and PUC
/// keeps array-range keys out of the node part entirely, so the invariant is
/// maintained by construction. This assertion pins it mechanically across the
/// unit, conformance, and fuzz suites.
///
/// `debug_assert!` keeps this compiled in release builds but never calls it,
/// so it must not be `cfg(debug_assertions)`-gated.
fn numeric_array_slot_shadows_hash_part(table: &TableData, one_based_index: usize) -> bool {
    let Some(key) = number_table_key(one_based_index as f64) else {
        return true;
    };
    match &table.hash {
        HashPart::Log { entries, index } => {
            entries.is_empty() || table_hash_entry(entries, index, key).is_none()
        }
        // PUC array-range keys never reach the node part: `luaH_setnum`
        // routes them to the array slot and `puc_getnum` reads the array
        // first.
        HashPart::Puc(puc) => !puc_numeric_node_is_live(puc, one_based_index),
    }
}

/// Root-free numeric node probe used only by the debug assertion above; it
/// mirrors `puc_getnum`'s chain walk without the array shortcut.
fn puc_numeric_node_is_live(puc: &PucHashPart, one_based_index: usize) -> bool {
    if one_based_index > i32::MAX as usize || puc.nodes.is_empty() {
        return false;
    }
    let bits = table_key_number_bits(one_based_index as f64);
    let mut slot = puc_number_main_position(puc.nodes.len(), bits);
    loop {
        let node = &puc.nodes[slot];
        if matches!(node.key, PucNodeKey::Live(TableKey::Number(live)) if live == bits) {
            return true;
        }
        if node.next < 0 {
            return false;
        }
        slot = node.next as usize;
    }
}

fn table_data_memory_usage(table: &TableData) -> HeapMemoryUsage {
    HeapMemoryUsage {
        live_bytes: table_data_live_bytes(table),
        reserved_bytes: table_data_reserved_bytes(table),
    }
}

fn vec_memory_usage<T>(values: &Vec<T>) -> HeapMemoryUsage {
    HeapMemoryUsage {
        live_bytes: values.len().saturating_mul(size_of::<T>()),
        reserved_bytes: values.capacity().saturating_mul(size_of::<T>()),
    }
}

fn function_data_memory_usage(function: &FunctionData) -> HeapMemoryUsage {
    HeapMemoryUsage {
        live_bytes: function
            .captures
            .len()
            .saturating_mul(size_of::<FunctionCapture>()),
        reserved_bytes: function
            .captures
            .capacity()
            .saturating_mul(size_of::<FunctionCapture>()),
    }
}

fn set_table_data(table: &mut TableData, table_key: TableKey, key_value: Value, value: Value) {
    let HashPart::Log {
        entries,
        index: hash_index,
    } = &mut table.hash
    else {
        // Only called from the Log branches of table_set and the quota path.
        debug_assert!(false, "set_table_data requires a Log-layout table");
        return;
    };
    if value.is_nil() {
        if let Some(index) = table_key_array_index(table_key) {
            if let Some(slot) = table.array.get_mut(index - 1) {
                *slot = Value::nil();
            }
        }
        if let Some(entry) = table_hash_entry_mut(entries, hash_index, table_key) {
            entry.value = Value::nil();
        }
        return;
    }
    if let Some(index) = table_key_array_index(table_key) {
        if index <= table.array.len() + 1 {
            if index > table.array.len() {
                table.array.resize(index, Value::nil());
            }
            table.array[index - 1] = value;
            if !entries.is_empty() {
                remove_hash_key(entries, hash_index, &mut table.shape_version, table_key);
            }
            return;
        }
    }
    set_hash_key(
        entries,
        hash_index,
        &mut table.shape_version,
        table_key,
        key_value,
        value,
    );
}

fn set_hash_key(
    entries: &mut Vec<TableEntry>,
    index: &mut Vec<u32>,
    shape_version: &mut u64,
    key: TableKey,
    key_value: Value,
    value: Value,
) {
    match table_hash_lookup(entries, index, key) {
        TableHashLookup::Found(index) => {
            if let Some(entry) = entries.get_mut(index) {
                entry.value = value;
            }
        }
        TableHashLookup::Vacant(vacant_hash_index) => {
            if !value.is_nil() {
                let entry_index = entries.len();
                entries.push(TableEntry {
                    key,
                    key_value,
                    value,
                });
                *shape_version = shape_version.saturating_add(1);
                record_hash_insert(entries, index, entry_index, vacant_hash_index);
            }
        }
    }
}

#[inline]
fn remove_hash_key(
    entries: &mut Vec<TableEntry>,
    index: &mut Vec<u32>,
    shape_version: &mut u64,
    key: TableKey,
) {
    if let Some(hash_index) = table_hash_entry_index(entries, index, key) {
        entries.remove(hash_index);
        *shape_version = shape_version.saturating_add(1);
        record_hash_remove(entries, index);
    }
}

#[derive(Clone, Copy)]
enum TableHashLookup {
    Found(usize),
    Vacant(Option<usize>),
}

#[inline]
fn table_hash_entry<'a>(
    entries: &'a [TableEntry],
    index: &[u32],
    key: TableKey,
) -> Option<&'a TableEntry> {
    let index = table_hash_entry_index(entries, index, key)?;
    entries.get(index)
}

#[inline]
fn table_hash_entry_mut<'a>(
    entries: &'a mut [TableEntry],
    index: &[u32],
    key: TableKey,
) -> Option<&'a mut TableEntry> {
    let index = table_hash_entry_index(entries, index, key)?;
    entries.get_mut(index)
}

#[inline]
fn table_hash_entry_index(entries: &[TableEntry], index: &[u32], key: TableKey) -> Option<usize> {
    match table_hash_lookup(entries, index, key) {
        TableHashLookup::Found(index) => Some(index),
        TableHashLookup::Vacant(_) => None,
    }
}

#[inline]
fn table_hash_lookup(entries: &[TableEntry], index: &[u32], key: TableKey) -> TableHashLookup {
    if entries.is_empty() {
        return TableHashLookup::Vacant(None);
    }
    if index.is_empty() {
        return match entries.iter().position(|entry| entry.key == key) {
            Some(index) => TableHashLookup::Found(index),
            None => TableHashLookup::Vacant(None),
        };
    }

    let mut position = table_hash_index_position(key, index.len());
    for _ in 0..index.len() {
        let slot = index[position];
        if slot == 0 {
            return TableHashLookup::Vacant(Some(position));
        }
        let entry_index = (slot - 1) as usize;
        let indexed_entry = entries.get(entry_index);
        debug_assert!(
            indexed_entry.is_some(),
            "table hash side index contains an invalid entry index"
        );
        if indexed_entry.is_some_and(|candidate| candidate.key == key) {
            return TableHashLookup::Found(entry_index);
        }
        position = (position + 1) & (index.len() - 1);
    }
    TableHashLookup::Vacant(None)
}

fn record_hash_insert(
    entries: &mut [TableEntry],
    index: &mut Vec<u32>,
    entry_index: usize,
    vacant_hash_index: Option<usize>,
) {
    if index.is_empty() {
        if entries.len() >= TABLE_HASH_INDEX_MIN_LEN {
            rebuild_hash_index(entries, index);
        }
        return;
    }
    if let Some(position) = vacant_hash_index {
        if entries.len().saturating_mul(2) > index.len() {
            rebuild_hash_index(entries, index);
        } else if let Some(slot) = index.get_mut(position) {
            *slot = compact_hash_index_slot(entry_index);
        }
        return;
    }
    insert_hash_index(entries, index, entry_index);
}

fn insert_hash_index(entries: &[TableEntry], index: &mut Vec<u32>, entry_index: usize) {
    if entries.len().saturating_mul(2) > index.len() {
        rebuild_hash_index(entries, index);
        return;
    }
    let Some(key) = entries.get(entry_index).map(|entry| entry.key) else {
        debug_assert!(
            false,
            "table hash side index insertion references an invalid entry"
        );
        return;
    };
    let mut position = table_hash_index_position(key, index.len());
    for _ in 0..index.len() {
        let slot = index[position];
        if slot == 0 {
            index[position] = compact_hash_index_slot(entry_index);
            return;
        }
        let existing_index = (slot - 1) as usize;
        match entries.get(existing_index) {
            Some(entry) if entry.key == key => {
                index[position] = compact_hash_index_slot(entry_index);
                return;
            }
            _ => position = (position + 1) & (index.len() - 1),
        }
    }
    rebuild_hash_index(entries, index);
}

fn record_hash_remove(entries: &[TableEntry], index: &mut Vec<u32>) {
    if entries.len() < TABLE_HASH_INDEX_MIN_LEN {
        *index = Vec::new();
    } else {
        rebuild_hash_index(entries, index);
    }
}

fn rebuild_hash_index(entries: &[TableEntry], index: &mut Vec<u32>) {
    if entries.len() < TABLE_HASH_INDEX_MIN_LEN {
        *index = Vec::new();
        return;
    }
    let capacity = entries
        .len()
        .saturating_mul(2)
        .next_power_of_two()
        .max(TABLE_HASH_INDEX_MIN_LEN * 2);
    *index = vec![0; capacity];
    for entry_index in 0..entries.len() {
        insert_hash_index(entries, index, entry_index);
    }
}

#[inline]
fn compact_hash_index_slot(index: usize) -> u32 {
    u32::try_from(index)
        .ok()
        .and_then(|index| index.checked_add(1))
        .expect("table hash exceeds compact u32 side-index address space")
}

#[inline]
fn table_hash_index_position(key: TableKey, len: usize) -> usize {
    debug_assert!(len.is_power_of_two());
    (table_hash_index_key(key) as usize) & (len - 1)
}

#[inline]
fn table_hash_index_key(key: TableKey) -> u64 {
    let tagged = match key {
        TableKey::Boolean(value) => u64::from(value) ^ 0x243f_6a88_85a3_08d3,
        TableKey::Number(bits) => table_numeric_hash_index_key(bits) ^ 0x1319_8a2e_0370_7344,
        TableKey::String(handle) => u64::from(handle) ^ 0xa409_3822_299f_31d0,
        TableKey::Table(handle) => u64::from(handle) ^ 0x082e_fa98_ec4e_6c89,
        TableKey::Function(handle) => u64::from(handle) ^ 0x4528_21e6_38d0_1377,
        TableKey::Userdata(handle) => u64::from(handle) ^ 0xbe54_66cf_34e9_0c6c,
        TableKey::Buffer(handle) => u64::from(handle) ^ 0xc0ac_29b7_c97c_50dd,
        TableKey::Vector(bits) => bits
            .into_iter()
            .fold(0x3f84_d5b5_b547_0917_u64, |hash, bits| {
                hash ^ u64::from(bits)
                    .wrapping_add(0x9e37_79b9)
                    .wrapping_add(hash << 6)
                    .wrapping_add(hash >> 2)
            }),
    };
    mix_index_hash(tagged)
}

#[inline]
fn table_numeric_hash_index_key(mut bits: u64) -> u64 {
    bits ^= bits >> 32;
    bits = bits.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    bits ^ (bits >> 29)
}

#[inline]
fn mix_index_hash(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

// ===========================================================================
// PUC Lua 5.1.5 table layout (port of third_party/lua-src/lua-5.1.5/ltable.c)
//
// A PUC table keeps non-negative integral keys in `TableData.array` and
// everything else in a chained scatter node vector (`PucHashPart`) with
// Brent's variation: if a node is not in its main position, the node
// colliding with it is in its own main position. Deletes are tombstones
// (value set to nil, key stays live); the node vector only ever changes size
// inside `rehash`. PUC tables never grow the array part outside `rehash`.
// ===========================================================================

/// PUC `ceillog2`: 1→0, 2→1, 3→2, 4→2, ...
const fn puc_ceil_log2(x: usize) -> usize {
    debug_assert!(x >= 1);
    (usize::BITS - (x - 1).leading_zeros()) as usize
}

/// `setnodevector` sizing (ltable.c:272): zero stays empty (the PUC
/// `dummynode` case); anything larger rounds up to a power of two and
/// saturates at 2^MAXBITS instead of raising PUC's "table overflow".
const fn puc_node_vector_len(size: usize) -> usize {
    if size == 0 {
        0
    } else {
        let log2 = puc_ceil_log2(size);
        1usize
            << if log2 > PUC_MAXBITS {
                PUC_MAXBITS
            } else {
                log2
            }
    }
}

/// `hashnum` main position (ltable.c:84), root-free. ±0.0 is canonicalized
/// to +0.0 bits before keys are built, and PUC hashes zero to slot 0.
fn puc_number_main_position(nodes_len: usize, bits: u64) -> usize {
    debug_assert!(nodes_len.is_power_of_two());
    if bits == 0 {
        return 0;
    }
    (luau_number_hash(bits) as usize) % ((nodes_len - 1) | 1)
}

/// `mainposition` (ltable.c:100). Strings and booleans use power-of-two
/// masking; numbers and raw handles use modulus by `(sizenode-1)|1`. Vectors
/// do not exist in PUC; they use the deterministic Luau vector hash so a
/// stray vector key still has well-defined placement.
fn puc_main_position(root: &RootData<'_>, nodes_len: usize, key: TableKey) -> usize {
    if nodes_len == 0 {
        // The dummynode case: every main position is slot 0, and callers
        // guard against indexing the empty node vector.
        return 0;
    }
    debug_assert!(nodes_len.is_power_of_two());
    match key {
        TableKey::Boolean(value) => usize::from(value) & (nodes_len - 1),
        TableKey::Number(bits) => puc_number_main_position(nodes_len, bits),
        TableKey::String(atom) => {
            let hash = root
                .strings
                .get(atom)
                .map_or(atom, |value| luau_string_hash(value.as_bytes()));
            (hash as usize) & (nodes_len - 1)
        }
        TableKey::Table(handle)
        | TableKey::Function(handle)
        | TableKey::Userdata(handle)
        | TableKey::Buffer(handle) => (handle as usize) % ((nodes_len - 1) | 1),
        TableKey::Vector(bits) => (luau_vector_hash(bits) as usize) % ((nodes_len - 1) | 1),
    }
}

/// Walks the collision chain from the key's main position looking for a live
/// node with exactly this key. Dead and free nodes never match.
fn puc_find_node(root: &RootData<'_>, puc: &PucHashPart, key: TableKey) -> Option<usize> {
    if puc.nodes.is_empty() {
        return None;
    }
    let mut slot = puc_main_position(root, puc.nodes.len(), key);
    loop {
        let node = &puc.nodes[slot];
        if matches!(node.key, PucNodeKey::Live(live) if live == key) {
            return Some(slot);
        }
        if node.next < 0 {
            return None;
        }
        slot = node.next as usize;
    }
}

/// `findindex`'s chain walk (ltable.c:137): like [`puc_find_node`], but a
/// dead node with the same key also matches ("key may be dead already, but
/// it is ok to use it in `next`").
fn puc_find_node_for_next(root: &RootData<'_>, puc: &PucHashPart, key: TableKey) -> Option<usize> {
    if puc.nodes.is_empty() {
        return None;
    }
    let mut slot = puc_main_position(root, puc.nodes.len(), key);
    loop {
        let node = &puc.nodes[slot];
        let matches = match node.key {
            PucNodeKey::Live(live) | PucNodeKey::Dead(live) => live == key,
            PucNodeKey::Free => false,
        };
        if matches {
            return Some(slot);
        }
        if node.next < 0 {
            return None;
        }
        slot = node.next as usize;
    }
}

/// `luaH_get` (ltable.c:469). `Some` means the key was found — including
/// tombstone nodes and in-range nil array slots, which PUC reports as found
/// nils; `None` means absent.
fn puc_lookup(root: &RootData<'_>, table: &TableData, key: TableKey) -> Option<Value> {
    if let Some(index) = table_key_array_index(key) {
        if let Some(value) = table.array.get(index - 1) {
            return Some(*value);
        }
    }
    let puc = table.puc_hash()?;
    let slot = puc_find_node(root, puc, key)?;
    Some(puc.nodes[slot].value)
}

/// `luaH_getnum` (ltable.c:435), root-free because only number keys are
/// compared. Absent reads report nil.
fn puc_getnum(table: &TableData, puc: &PucHashPart, key: usize) -> Value {
    if (1..=table.array.len()).contains(&key) {
        return table.array[key - 1];
    }
    if key > i32::MAX as usize || puc.nodes.is_empty() {
        return Value::nil();
    }
    let bits = table_key_number_bits(key as f64);
    let mut slot = puc_number_main_position(puc.nodes.len(), bits);
    loop {
        let node = &puc.nodes[slot];
        if matches!(node.key, PucNodeKey::Live(TableKey::Number(live)) if live == bits) {
            return node.value;
        }
        if node.next < 0 {
            return Value::nil();
        }
        slot = node.next as usize;
    }
}

/// `getfreepos` (ltable.c:382): a monotone scan-down cursor. Only key-free
/// slots count; tombstones and dead nodes are never reused as free space.
fn puc_get_free_pos(puc: &mut PucHashPart) -> Option<usize> {
    while puc.lastfree > 0 {
        puc.lastfree -= 1;
        if matches!(puc.nodes[puc.lastfree].key, PucNodeKey::Free) {
            return Some(puc.lastfree);
        }
    }
    None
}

/// Where a PUC `luaH_set` lookup lands, before the value is stored.
enum PucSlot {
    /// One-based array-part index.
    Array(usize),
    /// Node-part slot.
    Node(usize),
}

/// `luaH_set` (ltable.c:494) without the store: an integral key in array
/// range always lands on the array slot; an existing chain node (tombstones
/// included) is reused; anything absent goes through `newkey`.
fn puc_set_slot(
    root: &RootData<'_>,
    table: &mut TableData,
    key: TableKey,
    key_value: Value,
    during_rehash: bool,
) -> PucSlot {
    if let Some(index) = table_key_array_index(key) {
        if index <= table.array.len() {
            return PucSlot::Array(index);
        }
    }
    let found = table
        .puc_hash()
        .and_then(|puc| puc_find_node(root, puc, key));
    if let Some(slot) = found {
        return PucSlot::Node(slot);
    }
    puc_newkey(root, table, key, key_value, during_rehash)
}

/// `newkey` (ltable.c:399). `during_rehash` is set when reinserting existing
/// entries into an exactly-sized node vector, where `getfreepos` can never
/// run dry; the recursion through `rehash` therefore bottoms out after one
/// level.
fn puc_newkey(
    root: &RootData<'_>,
    table: &mut TableData,
    key: TableKey,
    key_value: Value,
    during_rehash: bool,
) -> PucSlot {
    bump_table_shape_version(table);
    let (mp, main_occupied) = {
        let Some(puc) = table.puc_hash() else {
            debug_assert!(false, "puc_newkey requires a PUC-layout table");
            return PucSlot::Node(0);
        };
        let mp = puc_main_position(root, puc.nodes.len(), key);
        // PUC tests the main position's *value*: a nil-valued node (free,
        // tombstone, or dead) is reused in place with no chain surgery. The
        // dummynode (empty node vector) counts as occupied: it always goes
        // through getfreepos and its rehash fallback.
        let occupied = puc.nodes.is_empty() || !puc.nodes[mp].value.is_nil();
        (mp, occupied)
    };
    if !main_occupied {
        let Some(puc) = table.puc_hash_mut() else {
            debug_assert!(false, "puc_newkey requires a PUC-layout table");
            return PucSlot::Node(0);
        };
        puc.nodes[mp].key = PucNodeKey::Live(key);
        puc.nodes[mp].key_value = key_value;
        debug_assert!(puc.nodes[mp].value.is_nil());
        return PucSlot::Node(mp);
    }
    let free = table.puc_hash_mut().and_then(puc_get_free_pos);
    let Some(n) = free else {
        // Node part is full: grow via rehash, then re-run luaH_set for the
        // triggering key (ltable.c:405-406). The exactly-sized new node part
        // always has room, so this recurses at most once.
        debug_assert!(
            !during_rehash,
            "PUC rehash failed to make room for its own entries"
        );
        puc_rehash(root, table, key);
        return puc_set_slot(root, table, key, key_value, true);
    };
    let Some(puc) = table.puc_hash_mut() else {
        debug_assert!(false, "puc_newkey requires a PUC-layout table");
        return PucSlot::Node(0);
    };
    let occupant_key = match puc.nodes[mp].key {
        PucNodeKey::Live(key) => key,
        PucNodeKey::Free | PucNodeKey::Dead(_) => {
            unreachable!("an occupied PUC node always has a live key")
        }
    };
    let othern = puc_main_position(root, puc.nodes.len(), occupant_key);
    if othern != mp {
        // The colliding node sits outside its own main position: relocate it
        // to the free slot and take the main position for the new key
        // (Brent's variation).
        let mut prev = othern;
        while puc.nodes[prev].next != mp as i32 {
            let next = puc.nodes[prev].next;
            debug_assert!(next >= 0, "PUC chain must reach the colliding node");
            prev = next as usize;
        }
        puc.nodes[prev].next = n as i32;
        puc.nodes[n] = puc.nodes[mp];
        puc.nodes[mp].next = -1;
        puc.nodes[mp].value = Value::nil();
        puc.nodes[mp].key = PucNodeKey::Live(key);
        puc.nodes[mp].key_value = key_value;
        debug_assert!(puc.nodes[mp].value.is_nil());
        PucSlot::Node(mp)
    } else {
        // The colliding node is in its own main position: chain the new key
        // behind it in the free slot.
        puc.nodes[n].next = puc.nodes[mp].next;
        puc.nodes[mp].next = n as i32;
        puc.nodes[n].key = PucNodeKey::Live(key);
        puc.nodes[n].key_value = key_value;
        debug_assert!(puc.nodes[n].value.is_nil());
        PucSlot::Node(n)
    }
}

/// `luaH_set` plus the `setobj2t` store (luaV_settable): finds or creates
/// the key's slot, then writes the value — including nil, which is how PUC
/// creates tombstones for absent keys.
fn puc_store(
    root: &RootData<'_>,
    table: &mut TableData,
    key: TableKey,
    key_value: Value,
    value: Value,
    during_rehash: bool,
) {
    match puc_set_slot(root, table, key, key_value, during_rehash) {
        PucSlot::Array(index) => {
            if let Some(slot) = table.array.get_mut(index - 1) {
                *slot = value;
            }
        }
        PucSlot::Node(slot) => {
            if let Some(puc) = table.puc_hash_mut() {
                puc.nodes[slot].value = value;
            }
        }
    }
}

/// Numeric store in the shape of `luaH_setnum` (ltable.c:508): indices
/// inside the array part write the slot, everything else goes to the node
/// part (negative and zero indices included).
fn puc_store_num(
    root: &RootData<'_>,
    table: &mut TableData,
    index: i64,
    value: Value,
    during_rehash: bool,
) {
    let number = index as f64;
    let Some(key) = number_table_key(number) else {
        return;
    };
    puc_store(
        root,
        table,
        key,
        Value::number(number),
        value,
        during_rehash,
    );
}

/// `countint` (ltable.c:211).
fn puc_count_int(key: TableKey, nums: &mut [usize; PUC_MAXBITS + 1]) -> usize {
    if let Some(k) = table_key_array_index(key) {
        if k <= PUC_MAXASIZE {
            nums[puc_ceil_log2(k)] += 1;
            return 1;
        }
    }
    0
}

/// `numusearray` (ltable.c:222): counts non-nil array values per
/// `(2^(lg-1), 2^lg]` slice.
fn puc_num_use_array(table: &TableData, nums: &mut [usize; PUC_MAXBITS + 1]) -> usize {
    let mut ause = 0usize;
    let mut i = 1usize;
    let mut lg = 0usize;
    let mut ttlg = 1usize;
    while lg <= PUC_MAXBITS {
        let mut lc = 0usize;
        let mut lim = ttlg;
        if lim > table.array.len() {
            lim = table.array.len();
            if i > lim {
                break;
            }
        }
        while i <= lim {
            if !table.array[i - 1].is_nil() {
                lc += 1;
            }
            i += 1;
        }
        nums[lg] += lc;
        ause += lc;
        lg += 1;
        ttlg = ttlg.saturating_mul(2);
    }
    ause
}

/// `numusehash` (ltable.c:247): counts nodes with non-nil values, buckets
/// array-candidate keys, and folds the array-candidate total into `nasize`.
fn puc_num_use_hash(
    table: &TableData,
    nums: &mut [usize; PUC_MAXBITS + 1],
    nasize: &mut usize,
) -> usize {
    let Some(puc) = table.puc_hash() else {
        debug_assert!(false, "puc_num_use_hash requires a PUC-layout table");
        return 0;
    };
    let mut totaluse = 0usize;
    let mut ause = 0usize;
    for node in &puc.nodes {
        if !node.value.is_nil() {
            if let PucNodeKey::Live(key) = node.key {
                ause += puc_count_int(key, nums);
            }
            totaluse += 1;
        }
    }
    *nasize += ause;
    totaluse
}

/// `computesizes` (ltable.c:189): the largest array size whose lower half is
/// more than half full.
fn puc_compute_sizes(nums: &[usize; PUC_MAXBITS + 1], narray: &mut usize) -> usize {
    let mut a = 0usize;
    let mut na = 0usize;
    let mut n = 0usize;
    let mut i = 0usize;
    let mut twotoi = 1usize;
    while twotoi / 2 < *narray {
        if i > PUC_MAXBITS {
            // Defensive: counted keys are capped at MAXASIZE, so `narray`
            // can never drive the slice index out of `nums` in practice.
            break;
        }
        if nums[i] > 0 {
            a += nums[i];
            if a > twotoi / 2 {
                n = twotoi;
                na = a;
            }
        }
        if a == *narray {
            break;
        }
        i += 1;
        twotoi = twotoi.saturating_mul(2);
    }
    *narray = n;
    na
}

/// `resize` (ltable.c:297). Array growth happens first; the old node vector
/// is replaced with a fresh all-free one; the vanishing array slice is
/// reinserted through the numeric store path (those indices all exceed the
/// new array size, so they land in nodes); old nodes are reinserted in
/// descending slot order through the general store path.
fn puc_resize(root: &RootData<'_>, table: &mut TableData, nasize: usize, nhsize: usize) {
    let old_array_len = table.array.len();
    if nasize > old_array_len {
        table.array.resize(nasize, Value::nil());
    }
    let node_len = puc_node_vector_len(nhsize);
    let old_nodes = {
        let Some(puc) = table.puc_hash_mut() else {
            debug_assert!(false, "puc_resize requires a PUC-layout table");
            return;
        };
        puc.lastfree = node_len;
        std::mem::replace(&mut puc.nodes, vec![PUC_FREE_NODE; node_len])
    };
    if nasize < old_array_len {
        let vanishing: Vec<Value> = table.array[nasize..].to_vec();
        table.array.truncate(nasize);
        for (offset, value) in vanishing.into_iter().enumerate() {
            if !value.is_nil() {
                puc_store_num(root, table, (nasize + offset + 1) as i64, value, true);
            }
        }
    }
    for node in old_nodes.iter().rev() {
        if node.value.is_nil() {
            // Nil-valued nodes (tombstones, dead keys, free slots) are
            // dropped by not being reinserted.
            continue;
        }
        let PucNodeKey::Live(key) = node.key else {
            continue;
        };
        puc_store(root, table, key, node.key_value, node.value, true);
    }
    bump_table_shape_version(table);
}

/// `rehash` (ltable.c:333).
fn puc_rehash(root: &RootData<'_>, table: &mut TableData, triggering_key: TableKey) {
    let mut nums = [0usize; PUC_MAXBITS + 1];
    let mut nasize = puc_num_use_array(table, &mut nums);
    let mut totaluse = nasize;
    totaluse += puc_num_use_hash(table, &mut nums, &mut nasize);
    nasize += puc_count_int(triggering_key, &mut nums);
    totaluse += 1;
    let na = puc_compute_sizes(&nums, &mut nasize);
    puc_resize(root, table, nasize, totaluse - na);
}

/// `luaH_getn` (ltable.c:560): a boundary in the array part if there is one,
/// the array size when the node part is empty, else an unbound search.
fn puc_table_border(table: &TableData, puc: &PucHashPart) -> usize {
    let mut j = table.array.len();
    if j > 0 && table.array[j - 1].is_nil() {
        let mut i = 0usize;
        while j - i > 1 {
            let m = (i + j) / 2;
            if table.array[m - 1].is_nil() {
                j = m;
            } else {
                i = m;
            }
        }
        return i;
    }
    if puc.nodes.is_empty() {
        return j;
    }
    puc_unbound_search(table, puc, j)
}

/// `unbound_search` (ltable.c:532).
fn puc_unbound_search(table: &TableData, puc: &PucHashPart, j: usize) -> usize {
    let mut i = j;
    let mut j = j.saturating_add(1);
    while !puc_getnum(table, puc, j).is_nil() {
        i = j;
        j = j.saturating_mul(2);
        if j > i32::MAX as usize {
            // The table was built with bad purposes: resort to linear
            // search (ltable.c:539-543).
            let mut i = 1usize;
            while !puc_getnum(table, puc, i).is_nil() {
                i += 1;
            }
            return i - 1;
        }
    }
    while j - i > 1 {
        let m = (i + j) / 2;
        if puc_getnum(table, puc, m).is_nil() {
            j = m;
        } else {
            i = m;
        }
    }
    i
}

/// First node with a non-nil value in ascending slot order (the hash-part
/// half of `luaH_next`, ltable.c:171-178). Dead-key nodes always carry nil
/// values, so every returned `key_value` is valid.
fn puc_first_node_entry(puc: &PucHashPart, from: usize) -> Option<(Value, Value)> {
    puc.nodes
        .get(from..)?
        .iter()
        .find_map(|node| (!node.value.is_nil()).then_some((node.key_value, node.value)))
}

/// Whether storing `key` (nil or not) would create a new node entry: the key
/// is not array-routed and not already present in its chain. PUC nil-stores
/// to absent keys create tombstones, so value nilness is irrelevant here.
fn puc_creates_hash_entry(root: &RootData<'_>, table: &TableData, key: TableKey) -> bool {
    if let Some(index) = table_key_array_index(key) {
        if index <= table.array.len() {
            return false;
        }
    }
    match table.puc_hash() {
        Some(puc) => puc_find_node(root, puc, key).is_none(),
        None => false,
    }
}

#[derive(Clone, Copy)]
struct WeakMode {
    keys: bool,
    values: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum HeapRef {
    String(u32),
    Table(u32),
    Function(u32),
    Userdata(u32),
    Buffer(u32),
    Vector(u32),
}

struct LiveSlotGenerations {
    encoded: Vec<u16>,
}

impl LiveSlotGenerations {
    fn new(slot_count: usize) -> Self {
        Self {
            encoded: vec![0; slot_count],
        }
    }

    fn insert(&mut self, handle: u32) -> bool {
        let (index, generation) = slot_handle_parts(handle);
        if let Some(slot) = self.encoded.get_mut(index) {
            let encoded = u16::from(generation) + 1;
            if *slot == encoded {
                return false;
            }
            *slot = encoded;
            return true;
        }
        false
    }

    fn contains(&self, handle: u32) -> bool {
        let (index, generation) = slot_handle_parts(handle);
        self.encoded
            .get(index)
            .is_some_and(|encoded| *encoded == u16::from(generation) + 1)
    }

    fn live_len(&self) -> usize {
        self.encoded.iter().filter(|encoded| **encoded != 0).count()
    }
}

/// The mark result shared by weak-table sweeping and ordinary heap reclamation.
///
/// Every side store uses generation-safe compact handles outside `gc-arena`.
/// Per-store generation arrays retain exact liveness without the allocation
/// and pointer overhead of general-purpose tree sets, and the same result
/// drives weak-table sweeping and ordinary reclamation.
struct HeapLiveSet {
    strings: LiveSlotGenerations,
    tables: LiveSlotGenerations,
    functions: LiveSlotGenerations,
    upvalues: LiveSlotGenerations,
    userdatas: LiveSlotGenerations,
    buffers: LiveSlotGenerations,
    vectors: LiveSlotGenerations,
}

impl HeapLiveSet {
    fn new(
        root: &RootData<'_>,
        tables: &ReusableSlots<TableData>,
        functions: &ReusableSlots<FunctionData>,
        upvalues: &ReusableSlots<UpvalueData>,
    ) -> Self {
        Self {
            strings: LiveSlotGenerations::new(root.strings.slots.len()),
            tables: LiveSlotGenerations::new(tables.slots.len()),
            functions: LiveSlotGenerations::new(functions.slots.len()),
            upvalues: LiveSlotGenerations::new(upvalues.slots.len()),
            userdatas: LiveSlotGenerations::new(root.userdatas.slots.len()),
            buffers: LiveSlotGenerations::new(root.buffers.slots.len()),
            vectors: LiveSlotGenerations::new(root.vectors.slots.len()),
        }
    }

    fn insert_ref(&mut self, reference: HeapRef) -> bool {
        match reference {
            HeapRef::String(handle) => self.strings.insert(handle),
            HeapRef::Table(handle) => self.tables.insert(handle),
            HeapRef::Function(handle) => self.functions.insert(handle),
            HeapRef::Userdata(handle) => self.userdatas.insert(handle),
            HeapRef::Buffer(handle) => self.buffers.insert(handle),
            HeapRef::Vector(handle) => self.vectors.insert(handle),
        }
    }

    fn contains_ref(&self, reference: HeapRef) -> bool {
        match reference {
            HeapRef::String(handle) => self.strings.contains(handle),
            HeapRef::Table(handle) => self.tables.contains(handle),
            HeapRef::Function(handle) => self.functions.contains(handle),
            HeapRef::Userdata(handle) => self.userdatas.contains(handle),
            HeapRef::Buffer(handle) => self.buffers.contains(handle),
            HeapRef::Vector(handle) => self.vectors.contains(handle),
        }
    }

    fn contains_value(&self, value: Value) -> bool {
        heap_ref_from_value(value).is_some_and(|reference| self.contains_ref(reference))
    }

    fn contains_key(&self, key: TableKey) -> bool {
        heap_ref_from_key(key).is_some_and(|reference| self.contains_ref(reference))
    }
}

fn table_weak_mode(
    root: &RootData<'_>,
    tables: &ReusableSlots<TableData>,
    table: u32,
    mode_atom: u32,
) -> Option<WeakMode> {
    let table = tables.get(table)?;
    let metatable = table.metatable?;
    let metatable = tables.get(metatable)?;
    let mode = match &metatable.hash {
        HashPart::Log { entries, index } => {
            table_hash_entry(entries, index, TableKey::String(mode_atom))?.value
        }
        HashPart::Puc(puc) => {
            let slot = puc_find_node(root, puc, TableKey::String(mode_atom))?;
            puc.nodes[slot].value
        }
    };
    let weak = WeakMode {
        keys: string_value_contains(root, mode, b'k')?,
        values: string_value_contains(root, mode, b'v')?,
    };
    (weak.keys || weak.values).then_some(weak)
}

fn string_value_contains(root: &RootData<'_>, value: Value, needle: u8) -> Option<bool> {
    if let Some(short) = value.as_short_string() {
        return Some(short.as_bytes().contains(&needle));
    }
    let atom = value.as_string_atom()?;
    root.strings
        .get(atom)
        .map(|value| value.as_bytes().contains(&needle))
}

fn mark_heap_reachable(
    root: &RootData<'_>,
    tables: &ReusableSlots<TableData>,
    functions: &ReusableSlots<FunctionData>,
    upvalues: &ReusableSlots<UpvalueData>,
    modes: &BTreeMap<u32, WeakMode>,
    roots: &[Value],
) -> HeapLiveSet {
    let mut live = HeapLiveSet::new(root, tables, functions, upvalues);
    let mut queue = Vec::new();

    for value in roots {
        if let Some(cell) = value.as_capture_cell() {
            mark_heap_upvalue(cell, upvalues, &mut live, &mut queue);
        } else {
            mark_heap_value(*value, &mut live, &mut queue);
        }
    }

    while let Some(reference) = queue.pop() {
        match reference {
            HeapRef::String(_) => {}
            HeapRef::Buffer(_) => {}
            HeapRef::Vector(_) => {}
            HeapRef::Userdata(handle) => {
                let Some(userdata) = root.userdatas.get(handle) else {
                    continue;
                };
                if let Some(metatable) = userdata.metatable {
                    mark_heap_value(Value::table(metatable), &mut live, &mut queue);
                }
                if let UserdataPayload::ConformanceVertex { pos, normal, uv } = userdata.payload {
                    mark_heap_value(pos, &mut live, &mut queue);
                    mark_heap_value(normal, &mut live, &mut queue);
                    mark_heap_value(uv, &mut live, &mut queue);
                }
            }
            HeapRef::Function(handle) => {
                let Some(function) = functions.get(handle) else {
                    continue;
                };
                mark_heap_value(Value::table(function.environment), &mut live, &mut queue);
                for capture in &function.captures {
                    let value = if let Some(cell) = capture.cell() {
                        mark_heap_upvalue(cell, upvalues, &mut live, &mut queue);
                        continue;
                    } else {
                        let Some(value) = capture.direct_value() else {
                            continue;
                        };
                        value
                    };
                    mark_heap_value(value, &mut live, &mut queue);
                }
            }
            HeapRef::Table(handle) => {
                let Some(table) = tables.get(handle) else {
                    continue;
                };
                if let Some(metatable) = table.metatable {
                    mark_heap_value(Value::table(metatable), &mut live, &mut queue);
                }
                let mode = modes.get(&handle).copied();
                let weak_keys = mode.is_some_and(|mode| mode.keys);
                let weak_values = mode.is_some_and(|mode| mode.values);
                if !weak_values {
                    for value in &table.array {
                        mark_heap_value(*value, &mut live, &mut queue);
                    }
                }
                match &table.hash {
                    HashPart::Log { entries, .. } => {
                        for entry in entries {
                            if !weak_keys {
                                mark_heap_key(entry.key, &mut live, &mut queue);
                            }
                            // Strings and vectors are value-like in Luau weak-table
                            // semantics. Rustia stores their payloads in reusable
                            // backing slots, so keep those slots live for every key,
                            // including keys in a weak-key table.
                            if matches!(entry.key, TableKey::String(_)) {
                                mark_heap_key(entry.key, &mut live, &mut queue);
                            } else if matches!(entry.key, TableKey::Vector(_)) {
                                mark_heap_value(entry.key_value, &mut live, &mut queue);
                            }
                            if !weak_values {
                                mark_heap_value(entry.value, &mut live, &mut queue);
                            }
                        }
                    }
                    HashPart::Puc(puc) => {
                        for node in &puc.nodes {
                            // Dead keys reference collected objects: mark
                            // nothing for them. Live keys follow the same
                            // weak/string-payload/vector-payload rules as the
                            // Log backend.
                            if let PucNodeKey::Live(key) = node.key {
                                if !weak_keys {
                                    mark_heap_key(key, &mut live, &mut queue);
                                }
                                if matches!(key, TableKey::String(_)) {
                                    mark_heap_key(key, &mut live, &mut queue);
                                } else if matches!(key, TableKey::Vector(_)) {
                                    mark_heap_value(node.key_value, &mut live, &mut queue);
                                }
                            }
                            if !weak_values {
                                mark_heap_value(node.value, &mut live, &mut queue);
                            }
                        }
                    }
                }
            }
        }
    }

    live
}

fn mark_heap_upvalue(
    cell: u32,
    upvalues: &ReusableSlots<UpvalueData>,
    live: &mut HeapLiveSet,
    queue: &mut Vec<HeapRef>,
) {
    let Some(upvalue) = upvalues.get(cell) else {
        return;
    };
    if live.upvalues.insert(cell) {
        mark_heap_value(upvalue.value, live, queue);
    }
}

fn mark_heap_value(value: Value, live: &mut HeapLiveSet, queue: &mut Vec<HeapRef>) {
    if let Some(reference) = heap_ref_from_value(value) {
        mark_heap_ref(reference, live, queue);
    }
}

fn mark_heap_key(key: TableKey, live: &mut HeapLiveSet, queue: &mut Vec<HeapRef>) {
    if let Some(reference) = heap_ref_from_key(key) {
        mark_heap_ref(reference, live, queue);
    }
}

fn mark_heap_ref(reference: HeapRef, live: &mut HeapLiveSet, queue: &mut Vec<HeapRef>) {
    if !live.insert_ref(reference) {
        return;
    }
    if matches!(
        reference,
        HeapRef::Table(_) | HeapRef::Function(_) | HeapRef::Userdata(_)
    ) {
        queue.push(reference);
    }
}

fn sweep_weak_table(table: &mut TableData, mode: WeakMode, live: &HeapLiveSet) {
    let mut changed = false;
    if mode.values {
        for value in &mut table.array {
            if weak_value_is_collectable(*value) && !live.contains_value(*value) {
                *value = Value::nil();
                changed = true;
            }
        }
    }
    match &mut table.hash {
        HashPart::Log { entries, index } => {
            let hash_len = entries.len();
            entries.retain(|entry| {
                let same_identity = table_key_value_same_identity(entry.key, entry.value);
                let remove_key = mode.keys
                    && table_key_is_collectable(entry.key)
                    && !live.contains_key(entry.key)
                    && (mode.values || !same_identity);
                let remove_value = mode.values
                    && weak_value_is_collectable(entry.value)
                    && !live.contains_value(entry.value)
                    && (mode.keys || !same_identity);
                !(remove_key || remove_value)
            });
            if entries.len() != hash_len {
                table.shape_version = table.shape_version.saturating_add(1);
                rebuild_hash_index(entries, index);
                changed = true;
            }
        }
        HashPart::Puc(puc) => {
            // PUC never removes nodes outside rehash: swept entries become
            // tombstones, and dead collectable keys become dead-key nodes
            // (LUA_TDEADKEY), which still answer next() lookups.
            for node in &mut puc.nodes {
                let PucNodeKey::Live(key) = node.key else {
                    continue;
                };
                let same_identity = table_key_value_same_identity(key, node.value);
                let remove_key = mode.keys
                    && table_key_is_collectable(key)
                    && !live.contains_key(key)
                    && (mode.values || !same_identity);
                let remove_value = mode.values
                    && weak_value_is_collectable(node.value)
                    && !live.contains_value(node.value)
                    && (mode.keys || !same_identity);
                if !(remove_key || remove_value) {
                    continue;
                }
                node.value = Value::nil();
                if remove_key {
                    node.key = PucNodeKey::Dead(key);
                    node.key_value = Value::nil();
                }
                changed = true;
            }
            if changed {
                table.shape_version = table.shape_version.saturating_add(1);
            }
        }
    }
    if changed {
        table.mutation_version = table.mutation_version.wrapping_add(1);
    }
}

fn reclaim_unreachable(
    root: &mut RootData<'_>,
    tables: &mut ReusableSlots<TableData>,
    functions: &mut ReusableSlots<FunctionData>,
    upvalues: &mut ReusableSlots<UpvalueData>,
    live: &HeapLiveSet,
) {
    let reclaimed_strings = root
        .strings
        .retain_handles(|handle| live.strings.contains(handle));
    if reclaimed_strings {
        rebuild_string_index(root, STRING_INDEX_MIN_CAPACITY);
    }

    tables.retain_handles(|handle| live.tables.contains(handle));
    functions.retain_handles(|handle| live.functions.contains(handle));
    upvalues.retain_handles(|handle| live.upvalues.contains(handle));
    root.userdatas
        .retain_handles(|handle| live.userdatas.contains(handle));
    root.buffers
        .retain_handles(|handle| live.buffers.contains(handle));
    root.vectors
        .retain_handles(|handle| live.vectors.contains(handle));
}

fn table_key_is_collectable(key: TableKey) -> bool {
    matches!(
        key,
        TableKey::Table(_) | TableKey::Function(_) | TableKey::Userdata(_) | TableKey::Buffer(_)
    )
}

fn weak_value_is_collectable(value: Value) -> bool {
    matches!(
        value.kind(),
        ValueKind::Table | ValueKind::Function | ValueKind::Userdata | ValueKind::Buffer
    )
}

fn heap_ref_from_key(key: TableKey) -> Option<HeapRef> {
    match key {
        TableKey::String(handle) => Some(HeapRef::String(handle)),
        TableKey::Table(handle) => Some(HeapRef::Table(handle)),
        TableKey::Function(handle) => Some(HeapRef::Function(handle)),
        TableKey::Userdata(handle) => Some(HeapRef::Userdata(handle)),
        TableKey::Buffer(handle) => Some(HeapRef::Buffer(handle)),
        TableKey::Boolean(_) | TableKey::Number(_) | TableKey::Vector(_) => None,
    }
}

fn heap_ref_from_value(value: Value) -> Option<HeapRef> {
    match value.kind() {
        ValueKind::String => value.as_string_atom().map(HeapRef::String),
        ValueKind::Table => value.as_table().map(HeapRef::Table),
        ValueKind::Function => value.as_function().map(HeapRef::Function),
        ValueKind::Userdata => value.as_userdata().map(HeapRef::Userdata),
        ValueKind::Buffer => value.as_buffer().map(HeapRef::Buffer),
        ValueKind::Vector => value.as_vector().map(HeapRef::Vector),
        ValueKind::Nil | ValueKind::Boolean | ValueKind::Number => None,
    }
}

fn table_key_value_same_identity(key: TableKey, value: Value) -> bool {
    match key {
        TableKey::Table(handle) => value.as_table() == Some(handle),
        TableKey::Function(handle) => value.as_function() == Some(handle),
        TableKey::Userdata(handle) => value.as_userdata() == Some(handle),
        TableKey::Buffer(handle) => value.as_buffer() == Some(handle),
        _ => false,
    }
}

fn table_array_boundary_len(table: &TableData) -> usize {
    if let HashPart::Puc(puc) = &table.hash {
        return puc_table_border(table, puc);
    }
    let len = table.array.len();
    if len == 0 {
        return table_unbound_search(table, 0);
    }
    if table.array[len - 1].kind() != ValueKind::Nil {
        return table_unbound_search(table, len);
    }

    let mut base = 0usize;
    let mut rest = len;
    while rest >> 1 != 0 {
        let half = rest >> 1;
        if table.array[base + half].kind() != ValueKind::Nil {
            base += half;
        }
        rest -= half;
    }
    usize::from(table.array[base].kind() != ValueKind::Nil) + base
}

fn table_unbound_search(table: &TableData, lower: usize) -> usize {
    let Some(mut upper) = lower.checked_add(1) else {
        return lower;
    };
    if table_numeric_get(table, upper).kind() == ValueKind::Nil {
        return lower;
    }

    let mut lower = upper;
    loop {
        let Some(next_upper) = upper.checked_mul(2) else {
            return table_boundary_binary_search(table, lower, MAX_LUAU_ARRAY_INDEX);
        };
        upper = next_upper.min(MAX_LUAU_ARRAY_INDEX);
        if table_numeric_get(table, upper).kind() == ValueKind::Nil {
            return table_boundary_binary_search(table, lower, upper);
        }
        if upper == MAX_LUAU_ARRAY_INDEX {
            return upper;
        }
        lower = upper;
    }
}

fn table_boundary_binary_search(table: &TableData, mut lower: usize, mut upper: usize) -> usize {
    while upper - lower > 1 {
        let middle = lower + (upper - lower) / 2;
        if table_numeric_get(table, middle).kind() == ValueKind::Nil {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    lower
}

fn first_non_nil_table_entry(
    root: &RootData<'_>,
    table: &TableData,
    order: TableIterationOrder,
) -> Option<(Value, Value)> {
    table
        .array
        .iter()
        .copied()
        .enumerate()
        .find(|(_, value)| value.kind() != ValueKind::Nil)
        .map(|(index, value)| (Value::number((index + 1) as f64), value))
        .or_else(|| first_hash_entry(root, table, order))
}

fn next_after_array_index(
    root: &RootData<'_>,
    table: &TableData,
    one_based_index: usize,
    order: TableIterationOrder,
) -> Option<(Value, Value)> {
    table
        .array
        .iter()
        .copied()
        .enumerate()
        .skip(one_based_index)
        .find(|(_, value)| value.kind() != ValueKind::Nil)
        .map(|(index, value)| (Value::number((index + 1) as f64), value))
        .or_else(|| first_hash_entry(root, table, order))
}

fn first_hash_entry(
    root: &RootData<'_>,
    table: &TableData,
    order: TableIterationOrder,
) -> Option<(Value, Value)> {
    match &table.hash {
        // A PUC table always enumerates node-slot ascending, regardless of
        // the requested order.
        HashPart::Puc(puc) => puc_first_node_entry(puc, 0),
        HashPart::Log { entries, .. } => {
            if order == TableIterationOrder::Insertion {
                return entries.iter().find_map(|entry| {
                    (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
                });
            }
            ordered_hash_entries(root, entries, order)
                .into_iter()
                .find_map(|entry| {
                    (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
                })
        }
    }
}

fn next_after_hash_key(
    root: &RootData<'_>,
    table: &TableData,
    key: TableKey,
    order: TableIterationOrder,
) -> Option<Option<(Value, Value)>> {
    match &table.hash {
        HashPart::Puc(puc) => {
            // findindex (ltable.c:137): live or dead keys both answer next().
            let slot = puc_find_node_for_next(root, puc, key)?;
            Some(puc_first_node_entry(puc, slot + 1))
        }
        HashPart::Log { entries, index } => {
            if order == TableIterationOrder::Insertion {
                let position = table_hash_entry_index(entries, index, key)?;
                return Some(entries.iter().skip(position + 1).find_map(|entry| {
                    (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
                }));
            }
            let ordered = ordered_hash_entries(root, entries, order);
            let position = ordered.iter().position(|entry| entry.key == key)?;
            Some(ordered.into_iter().skip(position + 1).find_map(|entry| {
                (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
            }))
        }
    }
}

fn ordered_hash_entries<'a>(
    root: &RootData<'_>,
    entries: &'a [TableEntry],
    order: TableIterationOrder,
) -> Vec<&'a TableEntry> {
    match order {
        TableIterationOrder::Insertion => entries.iter().collect(),
        // Puc51 never reaches order-based replay: PUC-layout tables dispatch
        // on the hash variant first. Map it to the Luau replay to keep the
        // match total.
        TableIterationOrder::LuauHash | TableIterationOrder::Puc51 => {
            luau_hash_nodes(root, entries)
        }
    }
}

fn luau_hash_nodes<'a>(root: &RootData<'_>, entries: &'a [TableEntry]) -> Vec<&'a TableEntry> {
    let Some(hash_size) = table_hash_size(entries) else {
        return Vec::new();
    };
    let mut nodes = vec![LuauHashNode::default(); hash_size];
    let mut last_free = hash_size;
    for entry in entries {
        luau_hash_insert(root, &mut nodes, &mut last_free, entry);
    }
    nodes.into_iter().filter_map(|node| node.entry).collect()
}

#[derive(Clone, Copy, Default)]
struct LuauHashNode<'a> {
    entry: Option<&'a TableEntry>,
    next: Option<usize>,
}

fn luau_hash_insert<'a>(
    root: &RootData<'_>,
    nodes: &mut [LuauHashNode<'a>],
    last_free: &mut usize,
    entry: &'a TableEntry,
) {
    let mut position = luau_hash_position(root, entry.key, nodes.len());
    if let Some(colliding) = nodes[position].entry {
        let Some(free) = luau_hash_free_position(nodes, last_free) else {
            return;
        };
        let colliding_position = luau_hash_position(root, colliding.key, nodes.len());
        if colliding_position != position {
            if let Some(previous) = luau_hash_chain_previous(nodes, colliding_position, position) {
                nodes[previous].next = Some(free);
            }
            nodes[free] = nodes[position];
            nodes[position] = LuauHashNode::default();
        } else {
            nodes[free].next = nodes[position].next;
            nodes[position].next = Some(free);
            position = free;
        }
    }
    nodes[position].entry = Some(entry);
}

fn luau_hash_free_position<'a>(nodes: &[LuauHashNode<'a>], last_free: &mut usize) -> Option<usize> {
    while *last_free > 0 {
        *last_free -= 1;
        if nodes[*last_free].entry.is_none() {
            return Some(*last_free);
        }
    }
    None
}

fn luau_hash_chain_previous<'a>(
    nodes: &[LuauHashNode<'a>],
    start: usize,
    target: usize,
) -> Option<usize> {
    let mut previous = start;
    while let Some(next) = nodes[previous].next {
        if next == target {
            return Some(previous);
        }
        previous = next;
    }
    None
}

fn table_hash_size(entries: &[TableEntry]) -> Option<usize> {
    if entries.is_empty() {
        None
    } else {
        Some(entries.len().next_power_of_two())
    }
}

fn luau_hash_position(root: &RootData<'_>, key: TableKey, hash_size: usize) -> usize {
    debug_assert!(hash_size.is_power_of_two());
    (luau_hash_key(root, key) as usize) & (hash_size - 1)
}

fn luau_hash_key(root: &RootData<'_>, key: TableKey) -> u32 {
    match key {
        TableKey::Boolean(value) => u32::from(value),
        TableKey::Number(bits) => luau_number_hash(bits),
        TableKey::String(atom) => root
            .strings
            .get(atom)
            .map_or(atom, |value| luau_string_hash(value.as_bytes())),
        TableKey::Table(handle)
        | TableKey::Function(handle)
        | TableKey::Userdata(handle)
        | TableKey::Buffer(handle) => luau_pointer_hash(handle),
        TableKey::Vector(bits) => luau_vector_hash(bits),
    }
}

fn luau_number_hash(bits: u64) -> u32 {
    (bits as u32).wrapping_add((bits >> 32) as u32)
}

fn luau_pointer_hash(handle: u32) -> u32 {
    handle.wrapping_mul(2654435761)
}

fn luau_vector_hash(bits: [u32; 3]) -> u32 {
    bits.into_iter().fold(0x9e37_79b9, |hash, bits| {
        hash ^ bits
            .wrapping_add(0x9e37_79b9)
            .wrapping_add(hash << 6)
            .wrapping_add(hash >> 2)
    })
}

fn luau_string_hash(bytes: &[u8]) -> u32 {
    let mut hash = bytes.len() as u32;
    let step = (bytes.len() >> 5) + 1;
    let mut len = bytes.len();
    while len >= step {
        hash ^= (hash << 5)
            .wrapping_add(hash >> 2)
            .wrapping_add(bytes[len - 1] as u32);
        len -= step;
    }
    hash
}

fn number_table_key(value: f64) -> Option<TableKey> {
    if value.is_nan() {
        None
    } else {
        Some(TableKey::Number(table_key_number_bits(value)))
    }
}

fn canonicalize_vector_component(value: f32) -> f32 {
    if value.is_nan() {
        f32::from_bits(0x7fc0_0000)
    } else {
        value
    }
}

fn vector_table_key_bits(value: f32) -> u32 {
    let normalized = if value == 0.0 { 0.0 } else { value };
    canonicalize_vector_component(normalized).to_bits()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{Constant, Function, Instruction, Opcode, Program};
    use std::mem::size_of;

    fn empty_table() -> TableData {
        TableData {
            array: Vec::new(),
            hash: empty_hash_part(TableLayout::Log),
            readonly: false,
            metatable: None,
            shape_version: 0,
            mutation_version: 0,
        }
    }

    /// Destructures the Log hash part of a test table (test tables are
    /// Log-layout unless built through a PUC `VmCell`). The shape version
    /// comes along so callers can pass it to the Log hash machinery without
    /// borrowing the table twice.
    fn log_hash_parts(table: &mut TableData) -> (&mut Vec<TableEntry>, &mut Vec<u32>, &mut u64) {
        let HashPart::Log { entries, index } = &mut table.hash else {
            panic!("test table should be Log-layout");
        };
        (entries, index, &mut table.shape_version)
    }

    #[test]
    fn compact_index_slots_have_locked_layout() {
        assert_eq!(size_of::<u32>(), 4);
        if cfg!(target_pointer_width = "64") {
            assert_eq!(size_of::<TableEntry>(), 48);
            assert_eq!(size_of::<SharedProgramString>(), 16);
            assert_eq!(size_of::<InternedString<'static>>(), 24);
        }
    }

    #[test]
    fn string_index_preserves_binary_atoms_across_growth() {
        let mut heap = VmCell::new();
        let values = (0_u32..1_024)
            .map(|index| {
                let mut value = index.to_le_bytes().to_vec();
                value.extend_from_slice(b"\0atom");
                value
            })
            .collect::<Vec<_>>();
        let atoms = values
            .iter()
            .map(|value| heap.intern_string_bytes(value))
            .collect::<Vec<_>>();

        for (value, atom) in values.iter().zip(&atoms) {
            assert_eq!(heap.intern_string_bytes(value), *atom);
            assert_eq!(heap.string_bytes(*atom).as_deref(), Some(value.as_slice()));
        }

        heap.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            assert_eq!(root.strings.len(), values.len());
            assert!(root.string_index.len() >= root.strings.len() * 2);
            assert_eq!(
                root.string_index.iter().filter(|slot| **slot != 0).count(),
                root.strings.len()
            );
        });
    }

    #[test]
    fn short_atoms_are_inline_and_shared_long_atoms_reuse_program_bytes() {
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
            vec![
                Constant::String(b"thirteen-byte".to_vec()),
                Constant::String(b"a compiled string longer than thirteen bytes".to_vec()),
            ],
        );
        let Constant::String(long_bytes) = &program.constants()[1] else {
            panic!("long constant should be a string");
        };
        let long_pointer = long_bytes.as_ptr();
        let mut first = VmCell::new();
        let mut second = VmCell::new();
        let short = first.intern_shared_string(program.shared_string(0).expect("short descriptor"));
        let first_long =
            first.intern_shared_string(program.shared_string(1).expect("long descriptor"));
        let second_long =
            second.intern_shared_string(program.shared_string(1).expect("long descriptor"));

        first.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            assert!(matches!(
                root.strings.get(short),
                Some(InternedString::Inline(_))
            ));
            let Some(InternedString::Shared(stored)) = root.strings.get(first_long) else {
                panic!("compiled long string should use shared storage");
            };
            assert_eq!(stored.0.as_bytes().as_ptr(), long_pointer);
        });
        second.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let Some(InternedString::Shared(stored)) = root.strings.get(second_long) else {
                panic!("second VM should retain shared storage");
            };
            assert_eq!(stored.0.as_bytes().as_ptr(), long_pointer);
        });
    }

    #[test]
    fn compiled_and_runtime_long_strings_share_atom_identity_without_copying_bytes() {
        let bytes = b"same long binary string\0\xff".to_vec();
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
            vec![Constant::String(bytes.clone())],
        );

        let mut runtime_first = VmCell::new();
        let runtime_atom = runtime_first.intern_string_bytes(&bytes);
        let compiled_atom = runtime_first
            .intern_shared_string(program.shared_string(0).expect("compiled descriptor"));
        assert_eq!(runtime_atom, compiled_atom);
        let Constant::String(compiled_bytes) = &program.constants()[0] else {
            panic!("compiled constant should remain a string");
        };
        runtime_first.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let Some(InternedString::Shared(stored)) = root.strings.get(runtime_atom) else {
                panic!("compiled arrival should upgrade runtime-owned storage");
            };
            assert_eq!(stored.0.as_bytes().as_ptr(), compiled_bytes.as_ptr());
        });
        assert_eq!(
            runtime_first.string_bytes(runtime_atom),
            Some(bytes.clone())
        );

        let mut compiled_first = VmCell::new();
        let compiled_atom = compiled_first
            .intern_shared_string(program.shared_string(0).expect("compiled descriptor"));
        let runtime_atom = compiled_first.intern_string_bytes(&bytes);
        assert_eq!(compiled_atom, runtime_atom);
        assert_eq!(compiled_first.string_bytes(compiled_atom), Some(bytes));
    }

    #[test]
    fn equal_compiled_strings_from_distinct_programs_canonicalize_to_one_atom() {
        let bytes = b"equal long strings from separate compiled programs";
        let make_program = || {
            Program::new(
                Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
                vec![Constant::String(bytes.to_vec())],
            )
        };
        let first = make_program();
        let second = make_program();
        let mut heap = VmCell::new();

        let first_atom =
            heap.intern_shared_string(first.shared_string(0).expect("first descriptor"));
        let second_atom =
            heap.intern_shared_string(second.shared_string(0).expect("second descriptor"));

        assert_eq!(first_atom, second_atom);
        assert_eq!(
            heap.string_bytes(first_atom).as_deref(),
            Some(bytes.as_slice())
        );
    }

    #[test]
    fn inline_string_table_keys_canonicalize_to_existing_atoms() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        let inline = Value::short_string(b"binary\0key").expect("short string");
        let atom = Value::string_atom(heap.intern_string_bytes(b"binary\0key"));

        assert_eq!(heap.table_set(table, inline, Value::number(42.0)), Some(()));
        assert_eq!(
            heap.table_get(table, atom).and_then(Value::as_number),
            Some(42.0)
        );
        assert_eq!(
            heap.table_get(table, inline).and_then(Value::as_number),
            Some(42.0)
        );
        assert!(matches!(
            heap.table_next_after(table, Value::nil(), TableIterationOrder::Insertion),
            Some(TableNext::Entry(key, value))
                if key.as_short_string().is_some_and(|key| key.as_bytes() == b"binary\0key")
                    && value.as_number() == Some(42.0)
        ));
    }

    #[test]
    fn inline_string_lookup_misses_and_failed_writes_do_not_create_atoms() {
        let mut heap = VmCell::new();
        let writable = heap.alloc_array_table(&[]);
        let readonly = heap.alloc_array_table(&[]);
        heap.table_freeze(readonly).expect("table should freeze");
        let missing = Value::short_string(b"unseen-key").expect("short string");

        let string_count = |heap: &mut VmCell| {
            heap.arena
                .mutate(|mc, root| root.unlock(mc).borrow().strings.len())
        };
        assert_eq!(string_count(&mut heap), 0);

        assert!(heap.table_get(writable, missing).is_none());
        assert!(matches!(
            heap.table_next_after(writable, missing, TableIterationOrder::Insertion),
            Some(TableNext::InvalidKey)
        ));
        assert_eq!(heap.table_set(writable, missing, Value::nil()), Some(()));
        assert_eq!(heap.table_set(readonly, missing, Value::number(1.0)), None);
        assert_eq!(heap.table_set(u32::MAX, missing, Value::number(1.0)), None);
        assert_eq!(string_count(&mut heap), 0);

        assert_eq!(
            heap.table_set(writable, missing, Value::number(42.0)),
            Some(())
        );
        assert_eq!(string_count(&mut heap), 1);
        assert_eq!(
            heap.table_get(writable, missing).and_then(Value::as_number),
            Some(42.0)
        );
    }

    #[test]
    fn numeric_table_paths_preserve_key_rules_without_creating_string_atoms() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        let string_count = |heap: &mut VmCell| {
            heap.arena
                .mutate(|mc, root| root.unlock(mc).borrow().strings.len())
        };
        let cases = [
            (1.0, 11.0),
            (1.5, 15.0),
            (MAX_LUAU_ARRAY_INDEX as f64 + 1.0, 21.0),
            (f64::INFINITY, 99.0),
        ];

        for (key, value) in cases {
            assert_eq!(
                heap.table_set(table, Value::number(key), Value::number(value)),
                Some(())
            );
            assert_eq!(
                heap.table_get(table, Value::number(key))
                    .and_then(Value::as_number),
                Some(value)
            );
        }

        assert_eq!(
            heap.table_set(table, Value::number(-0.0), Value::number(7.0)),
            Some(())
        );
        assert_eq!(
            heap.table_get(table, Value::number(0.0))
                .and_then(Value::as_number),
            Some(7.0)
        );

        assert_eq!(
            heap.table_set(table, Value::number(1.5), Value::nil()),
            Some(())
        );
        assert!(heap
            .table_get(table, Value::number(1.5))
            .is_some_and(Value::is_nil));
        assert_eq!(
            heap.table_set(table, Value::number(1.0), Value::nil()),
            Some(())
        );
        assert!(heap
            .table_get(table, Value::number(1.0))
            .is_some_and(Value::is_nil));
        assert_eq!(
            heap.table_set(table, Value::number(f64::NAN), Value::number(1.0)),
            None
        );
        assert!(heap.table_get(table, Value::number(f64::NAN)).is_none());

        let readonly = heap.alloc_array_table(&[]);
        heap.table_freeze(readonly).expect("table should freeze");
        assert_eq!(
            heap.table_set(readonly, Value::number(1.0), Value::number(1.0)),
            None
        );
        assert_eq!(
            heap.table_set(u32::MAX, Value::number(1.0), Value::number(1.0)),
            None
        );
        assert!(heap.table_get(u32::MAX, Value::number(1.0)).is_none());
        assert_eq!(string_count(&mut heap), 0);
    }

    #[test]
    fn numeric_array_shortcuts_only_store_when_metamethods_cannot_intercept() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[Value::number(10.0)]);

        assert_eq!(
            heap.read_numeric_array_slot(table, 1)
                .and_then(Value::as_number),
            Some(10.0)
        );
        assert!(heap.read_numeric_array_slot(table, 2).is_none());
        assert_eq!(
            heap.try_write_numeric_array_slot(table, 1, Value::number(11.0)),
            Some(TableNumericArrayWrite::Stored)
        );
        assert_eq!(
            heap.try_write_numeric_array_slot(table, 2, Value::number(12.0)),
            Some(TableNumericArrayWrite::Stored)
        );
        assert_eq!(
            heap.read_array_prefix(table, 2)
                .expect("materialized prefix")
                .into_iter()
                .map(Value::as_number)
                .collect::<Vec<_>>(),
            vec![Some(11.0), Some(12.0)]
        );
        assert_eq!(
            heap.try_write_array_prefix(table, &[Value::number(12.0), Value::number(11.0)]),
            Some(TableNumericArrayWrite::Stored)
        );
        assert_eq!(
            heap.try_write_array_prefix(
                table,
                &[Value::number(1.0), Value::number(2.0), Value::number(3.0)]
            ),
            Some(TableNumericArrayWrite::Deferred)
        );

        assert_eq!(
            heap.try_write_numeric_array_slot(table, 10, Value::number(99.0)),
            Some(TableNumericArrayWrite::Deferred)
        );
        assert_eq!(
            heap.table_set(table, Value::number(10.0), Value::number(99.0)),
            Some(())
        );
        assert_eq!(
            heap.try_write_numeric_array_slot(table, 10, Value::nil()),
            Some(TableNumericArrayWrite::Deferred)
        );
        assert_eq!(
            heap.table_get(table, Value::number(10.0))
                .and_then(Value::as_number),
            Some(99.0)
        );

        let metatable = heap.alloc_array_table(&[]);
        heap.table_set_metatable(table, Some(metatable))
            .expect("table should accept a metatable");
        assert_eq!(
            heap.try_write_numeric_array_slot(table, 3, Value::number(13.0)),
            Some(TableNumericArrayWrite::Deferred)
        );

        heap.table_freeze(table).expect("table should freeze");
        assert_eq!(
            heap.try_write_numeric_array_slot(table, 1, Value::number(20.0)),
            Some(TableNumericArrayWrite::Readonly)
        );
        assert_eq!(
            heap.try_write_numeric_array_slot(u32::MAX, 1, Value::number(1.0)),
            None
        );
    }

    // =========================================================================
    // VM Core 2 C2.5a: direct existing dense-array slot mutation
    // =========================================================================

    /// Everything `table_data_memory_usage` reads, plus the guest-visible
    /// shape counter. The direct lane must leave all of it untouched.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TableStorageShape {
        array_len: usize,
        array_capacity: usize,
        hash_kind: &'static str,
        hash_len: usize,
        hash_capacity: usize,
        index_len: usize,
        index_capacity: usize,
        shape_version: u64,
    }

    fn table_storage_shape(heap: &VmCell, table: u32) -> TableStorageShape {
        let table = heap.tables.get(table).expect("table exists");
        let (hash_kind, hash_len, hash_capacity, index_len, index_capacity) = match &table.hash {
            HashPart::Log { entries, index } => (
                "log",
                entries.len(),
                entries.capacity(),
                index.len(),
                index.capacity(),
            ),
            HashPart::Puc(puc) => (
                "puc",
                puc.nodes.len(),
                puc.nodes.capacity(),
                puc.lastfree,
                0,
            ),
        };
        TableStorageShape {
            array_len: table.array.len(),
            array_capacity: table.array.capacity(),
            hash_kind,
            hash_len,
            hash_capacity,
            index_len,
            index_capacity,
            shape_version: table.shape_version,
        }
    }

    fn table_version(heap: &VmCell, table: u32) -> u64 {
        heap.tables
            .get(table)
            .expect("table exists")
            .mutation_version
    }

    fn layout_heaps() -> Vec<(&'static str, VmCell)> {
        vec![("log", VmCell::new()), ("puc", puc_heap())]
    }

    #[test]
    fn direct_array_slot_mutation_serves_existing_non_nil_slots_in_both_layouts() {
        for (layout, mut heap) in layout_heaps() {
            let table = heap.alloc_array_table(&[
                Value::number(10.0),
                Value::number(20.0),
                Value::number(30.0),
            ]);

            let shape_before = table_storage_shape(&heap, table);
            let usage_before = heap.memory_usage();
            let version_before = table_version(&heap, table);
            let counts_before = heap.numeric_array_slot_write_counts();

            // 1. existing number -> a different number.
            assert_eq!(
                heap.try_write_numeric_array_slot(table, 2, Value::number(99.0)),
                Some(TableNumericArrayWrite::Stored),
                "{layout}: an existing non-nil slot stores directly"
            );
            let counts_after = heap.numeric_array_slot_write_counts();
            assert_eq!(
                counts_after.direct - counts_before.direct,
                1,
                "{layout}: the write used the direct lane"
            );
            assert_eq!(
                counts_after.generic_stored, counts_before.generic_stored,
                "{layout}: the generic mutate_table_data path was not used"
            );
            assert_eq!(
                table_version(&heap, table) - version_before,
                1,
                "{layout}: mutation_version increments exactly once"
            );
            assert_eq!(
                table_storage_shape(&heap, table),
                shape_before,
                "{layout}: array length/capacity and hash representation are unchanged"
            );
            assert_eq!(
                heap.memory_usage(),
                usage_before,
                "{layout}: logical live/reserved accounting is unchanged"
            );
            assert_eq!(
                heap.table_get(table, Value::number(2.0))
                    .and_then(Value::as_number),
                Some(99.0),
                "{layout}: the new value is visible"
            );

            // 2. existing number -> nil, in place.
            let version_before = table_version(&heap, table);
            let counts_before = heap.numeric_array_slot_write_counts();
            assert_eq!(
                heap.try_write_numeric_array_slot(table, 2, Value::nil()),
                Some(TableNumericArrayWrite::Stored),
                "{layout}: an existing slot may be replaced with nil"
            );
            let counts_after = heap.numeric_array_slot_write_counts();
            assert_eq!(counts_after.direct - counts_before.direct, 1);
            assert_eq!(counts_after.generic_stored, counts_before.generic_stored);
            assert_eq!(
                table_version(&heap, table) - version_before,
                1,
                "{layout}: the nil write also bumps mutation_version exactly once"
            );
            assert_eq!(
                table_storage_shape(&heap, table),
                shape_before,
                "{layout}: clearing a slot does not move the array boundary"
            );
            assert_eq!(heap.memory_usage(), usage_before);
            assert!(heap
                .table_get(table, Value::number(2.0))
                .expect("live table")
                .is_nil());
            assert_eq!(
                heap.table_get(table, Value::number(3.0))
                    .and_then(Value::as_number),
                Some(30.0),
                "{layout}: neighbouring slots are untouched"
            );

            // 3. the now-nil slot leaves the direct lane.
            let counts_before = heap.numeric_array_slot_write_counts();
            assert_eq!(
                heap.try_write_numeric_array_slot(table, 2, Value::number(21.0)),
                Some(TableNumericArrayWrite::Stored),
                "{layout}: refilling a nil slot still stores"
            );
            let counts_after = heap.numeric_array_slot_write_counts();
            assert_eq!(
                counts_after.direct, counts_before.direct,
                "{layout}: an existing nil slot is not a direct-lane hit"
            );
            assert_eq!(
                counts_after.generic_stored - counts_before.generic_stored,
                1,
                "{layout}: it falls back to the generic path"
            );
        }
    }

    #[test]
    fn direct_array_slot_mutation_keeps_readonly_and_metatable_gates() {
        for (layout, mut heap) in layout_heaps() {
            let metatable = heap.alloc_array_table(&[]);

            // An existing non-nil slot bypasses __newindex even with a
            // metatable installed, so it stays on the direct lane.
            let with_metatable =
                heap.alloc_array_table(&[Value::number(1.0), Value::nil(), Value::number(3.0)]);
            heap.table_set_metatable(with_metatable, Some(metatable))
                .expect("metatable installs");
            let counts_before = heap.numeric_array_slot_write_counts();
            assert_eq!(
                heap.try_write_numeric_array_slot(with_metatable, 1, Value::number(7.0)),
                Some(TableNumericArrayWrite::Stored),
                "{layout}: an existing slot writes directly through a metatable"
            );
            assert_eq!(
                heap.numeric_array_slot_write_counts().direct - counts_before.direct,
                1
            );

            // A present-but-nil slot and an absent slot both defer, so the VM
            // can still run __newindex.
            assert_eq!(
                heap.try_write_numeric_array_slot(with_metatable, 2, Value::number(7.0)),
                Some(TableNumericArrayWrite::Deferred),
                "{layout}: an existing nil slot defers to the metamethod path"
            );
            assert_eq!(
                heap.try_write_numeric_array_slot(with_metatable, 9, Value::number(7.0)),
                Some(TableNumericArrayWrite::Deferred),
                "{layout}: a missing slot defers to the metamethod path"
            );

            // Readonly wins over the direct lane and mutates nothing.
            let frozen = heap.alloc_array_table(&[Value::number(1.0), Value::nil()]);
            heap.table_freeze(frozen).expect("table freezes");
            let version_before = table_version(&heap, frozen);
            let shape_before = table_storage_shape(&heap, frozen);
            let counts_before = heap.numeric_array_slot_write_counts();
            assert_eq!(
                heap.try_write_numeric_array_slot(frozen, 1, Value::number(5.0)),
                Some(TableNumericArrayWrite::Readonly),
                "{layout}: a readonly table rejects an existing-slot write"
            );
            assert_eq!(
                heap.numeric_array_slot_write_counts().direct,
                counts_before.direct,
                "{layout}: a readonly rejection is not a direct-lane store"
            );
            assert_eq!(
                table_version(&heap, frozen),
                version_before,
                "{layout}: a rejected write does not bump mutation_version"
            );
            assert_eq!(table_storage_shape(&heap, frozen), shape_before);
            assert_eq!(
                heap.table_get(frozen, Value::number(1.0))
                    .and_then(Value::as_number),
                Some(1.0)
            );
            assert_eq!(
                heap.try_write_numeric_array_slot(frozen, 2, Value::number(5.0)),
                Some(TableNumericArrayWrite::Readonly),
                "{layout}: an existing nil slot on a readonly table still reports readonly"
            );
        }
    }

    #[test]
    fn direct_array_slot_mutation_rejects_invalid_handles_and_indices() {
        for (layout, mut heap) in layout_heaps() {
            let table = heap.alloc_array_table(&[Value::number(1.0)]);

            assert_eq!(
                heap.try_write_numeric_array_slot(u32::MAX, 1, Value::number(2.0)),
                None,
                "{layout}: an invalid handle reports a missing table"
            );
            assert_eq!(
                heap.try_write_numeric_array_slot(table, 0, Value::number(2.0)),
                Some(TableNumericArrayWrite::Deferred),
                "{layout}: index 0 is not an array index"
            );
            assert_eq!(
                heap.try_write_numeric_array_slot(
                    table,
                    MAX_LUAU_ARRAY_INDEX + 1,
                    Value::number(2.0)
                ),
                Some(TableNumericArrayWrite::Deferred),
                "{layout}: indices past the Luau array limit defer"
            );
            let counts = heap.numeric_array_slot_write_counts();
            assert_eq!(
                counts.direct, 0,
                "{layout}: no rejection reached the direct lane"
            );
            assert_eq!(counts.calls, 3, "{layout}: every call is censused");

            // The live table is untouched by any of the three rejections.
            assert_eq!(
                heap.table_get(table, Value::number(1.0))
                    .and_then(Value::as_number),
                Some(1.0),
                "{layout}: rejections do not mutate the table"
            );
            assert_eq!(table_version(&heap, table), 0);
        }
    }

    /// Applies one numeric write the way the VM does: try the shortcut first,
    /// then fall back to the general raw-set path when it declines.
    fn shortcut_then_general_write(
        heap: &mut VmCell,
        table: u32,
        index: usize,
        value: Value,
    ) -> Option<TableNumericArrayWrite> {
        let result = heap.try_write_numeric_array_slot(table, index, value);
        if matches!(result, Some(TableNumericArrayWrite::Deferred) | None) {
            let _ = heap.table_set(table, Value::number(index as f64), value);
        }
        result
    }

    /// Reads every observable slot around the array boundary, so the two
    /// paths are compared on guest-visible state rather than on internals
    /// alone.
    fn numeric_readback(heap: &mut VmCell, table: u32, probe_limit: usize) -> Vec<Option<Value>> {
        (0..=probe_limit)
            .map(|index| heap.table_get(table, Value::number(index as f64)))
            .collect()
    }

    fn same_test_values(left: &[Option<Value>], right: &[Option<Value>]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right)
                .all(|(left, right)| match (left, right) {
                    (Some(left), Some(right)) => same_test_value(*left, *right),
                    (None, None) => true,
                    _ => false,
                })
    }

    fn collect_iteration(
        heap: &mut VmCell,
        table: u32,
        order: TableIterationOrder,
    ) -> Vec<(Value, Value)> {
        let mut entries = Vec::new();
        let mut after = Value::nil();
        loop {
            match heap.table_next_after(table, after, order) {
                Some(TableNext::Entry(key, value)) => {
                    entries.push((key, value));
                    after = key;
                }
                Some(TableNext::End) => return entries,
                Some(TableNext::InvalidKey) => panic!("iteration key became invalid"),
                None => panic!("table handle is missing"),
            }
        }
    }

    #[test]
    fn direct_array_slot_mutation_matches_the_general_write_path() {
        // (array prefix, extra numeric hash keys, metatable, readonly)
        let shapes: [(&[f64], &[f64], bool, bool); 6] = [
            (&[10.0, 20.0, 30.0], &[], false, false),
            (&[10.0, 20.0, 30.0], &[], true, false),
            (&[10.0, 20.0, 30.0], &[], false, true),
            (&[10.0, 20.0, 30.0], &[7.0, 9.0], false, false),
            (&[], &[4.0], false, false),
            (&[1.0], &[2.0, 3.0], false, false),
        ];
        let values = [
            Value::number(-0.5),
            Value::nil(),
            Value::boolean(true),
            Value::number(0.0),
        ];

        let mut direct_lane_hits = 0_u64;
        for layout in ["log", "puc"] {
            for (shape_index, (prefix, extra, metatable, readonly)) in
                shapes.into_iter().enumerate()
            {
                for index in 0..=6usize {
                    for (value_index, value) in values.into_iter().enumerate() {
                        let build = |heap: &mut VmCell| {
                            let table = heap.alloc_array_table(
                                &prefix
                                    .iter()
                                    .copied()
                                    .map(Value::number)
                                    .collect::<Vec<_>>(),
                            );
                            for key in extra {
                                heap.table_set(table, Value::number(*key), Value::number(*key))
                                    .expect("hash key stores");
                            }
                            if metatable {
                                let meta = heap.alloc_array_table(&[]);
                                heap.table_set_metatable(table, Some(meta))
                                    .expect("metatable installs");
                            }
                            if readonly {
                                heap.table_freeze(table).expect("table freezes");
                            }
                            table
                        };

                        let mut fast = if layout == "log" {
                            VmCell::new()
                        } else {
                            puc_heap()
                        };
                        let mut general = if layout == "log" {
                            VmCell::new()
                        } else {
                            puc_heap()
                        };
                        let fast_table = build(&mut fast);
                        let general_table = build(&mut general);

                        let case =
                            format!("{layout}/shape{shape_index}/index{index}/value{value_index}");
                        assert_eq!(
                            table_storage_shape(&fast, fast_table),
                            table_storage_shape(&general, general_table),
                            "{case}: identical construction"
                        );

                        let fast_version = table_version(&fast, fast_table);
                        let general_version = table_version(&general, general_table);
                        let result =
                            shortcut_then_general_write(&mut fast, fast_table, index, value);
                        let _ =
                            general.table_set(general_table, Value::number(index as f64), value);

                        if readonly {
                            assert!(
                                !matches!(result, Some(TableNumericArrayWrite::Stored)),
                                "{case}: readonly tables never store, got {result:?}"
                            );
                        }
                        if readonly && index >= 1 && index <= prefix.len() {
                            assert!(
                                matches!(result, Some(TableNumericArrayWrite::Readonly)),
                                "{case}: an in-array index on a readonly table reports readonly, got {result:?}"
                            );
                        }
                        assert!(
                            same_test_values(
                                &numeric_readback(&mut fast, fast_table, 8),
                                &numeric_readback(&mut general, general_table, 8)
                            ),
                            "{case}: identical guest-visible values"
                        );
                        let order = if layout == "log" {
                            TableIterationOrder::Insertion
                        } else {
                            TableIterationOrder::Puc51
                        };
                        let fast_entries = collect_iteration(&mut fast, fast_table, order);
                        let general_entries = collect_iteration(&mut general, general_table, order);
                        assert_eq!(
                            fast_entries.len(),
                            general_entries.len(),
                            "{case}: identical iteration length"
                        );
                        assert!(
                            fast_entries.iter().zip(&general_entries).all(
                                |((fast_key, fast_value), (general_key, general_value))| {
                                    same_test_value(*fast_key, *general_key)
                                        && same_test_value(*fast_value, *general_value)
                                }
                            ),
                            "{case}: identical iteration order and values"
                        );
                        assert_eq!(
                            fast.array_len(fast_table),
                            general.array_len(general_table),
                            "{case}: identical border length"
                        );
                        assert_eq!(
                            table_storage_shape(&fast, fast_table),
                            table_storage_shape(&general, general_table),
                            "{case}: identical storage shape"
                        );
                        assert_eq!(
                            table_version(&fast, fast_table) - fast_version,
                            table_version(&general, general_table) - general_version,
                            "{case}: identical mutation_version movement"
                        );
                        assert_eq!(
                            fast.memory_usage(),
                            general.memory_usage(),
                            "{case}: identical live/reserved accounting"
                        );
                        direct_lane_hits += fast.numeric_array_slot_write_counts().direct;
                    }
                }
            }
        }
        assert!(
            direct_lane_hits > 0,
            "the differential matrix must actually exercise the direct lane"
        );
    }

    #[test]
    fn ordinary_short_string_values_do_not_allocate_vm_atoms() {
        let mut heap = VmCell::new();
        let values = (0_u32..4_096)
            .map(|index| {
                let mut bytes = [0_u8; crate::value::SHORT_STRING_CAPACITY];
                bytes[..4].copy_from_slice(&index.to_le_bytes());
                heap.intern_string_value_bytes(&bytes)
            })
            .collect::<Vec<_>>();

        assert!(values.iter().all(|value| value.as_short_string().is_some()));
        heap.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            assert!(root.strings.is_empty());
            assert!(root.string_index.is_empty());
        });
    }

    #[test]
    fn compact_hash_index_handles_collisions_removal_and_clone() {
        let mut table = empty_table();
        let mut atoms = Vec::new();
        for atom in 0..100_000_u32 {
            if table_hash_index_position(TableKey::String(atom), 64) == 0 {
                atoms.push(atom);
                if atoms.len() == 20 {
                    break;
                }
            }
        }
        assert_eq!(atoms.len(), 20);

        for atom in &atoms {
            let (entries, index, shape_version) = log_hash_parts(&mut table);
            set_hash_key(
                entries,
                index,
                shape_version,
                TableKey::String(*atom),
                Value::string_atom(*atom),
                Value::number(f64::from(*atom)),
            );
        }
        let HashPart::Log { entries, index } = &table.hash else {
            panic!("test table should be Log-layout");
        };
        assert_eq!(index.len(), 64);
        for atom in &atoms {
            assert_eq!(
                table_hash_entry(entries, index, TableKey::String(*atom))
                    .and_then(|entry| entry.value.as_number()),
                Some(f64::from(*atom))
            );
        }

        let removed = atoms.remove(7);
        let (entries, index, shape_version) = log_hash_parts(&mut table);
        remove_hash_key(entries, index, shape_version, TableKey::String(removed));
        assert_eq!(entries.len(), atoms.len());
        assert!(!index.is_empty());
        assert!(table_hash_entry(entries, index, TableKey::String(removed)).is_none());
        for atom in &atoms {
            assert!(table_hash_entry(entries, index, TableKey::String(*atom)).is_some());
        }

        let clone = table.clone();
        let HashPart::Log {
            entries: clone_entries,
            index: clone_index,
        } = &clone.hash
        else {
            panic!("cloned table should be Log-layout");
        };
        for atom in &atoms {
            assert!(
                table_hash_entry(clone_entries, clone_index, TableKey::String(*atom)).is_some()
            );
        }

        loop {
            let (entries, index, shape_version) = log_hash_parts(&mut table);
            if entries.len() < TABLE_HASH_INDEX_MIN_LEN {
                break;
            }
            let key = entries[0].key;
            remove_hash_key(entries, index, shape_version, key);
        }
        let HashPart::Log { index, .. } = &table.hash else {
            panic!("test table should be Log-layout");
        };
        assert!(index.is_empty());
        assert_eq!(index.capacity(), 0);
    }

    #[test]
    fn external_table_storage_preserves_explicit_weak_graph_tracing() {
        let mut heap = VmCell::new();
        let weak = heap.alloc_array_table(&[]);
        let weak_metatable = heap.alloc_array_table(&[]);
        let holder = heap.alloc_array_table(&[]);
        let target = heap.alloc_array_table(&[]);
        let mode_key = Value::string_atom(heap.intern_string("__mode"));
        let weak_values = Value::string_atom(heap.intern_string("v"));
        let target_key = Value::string_atom(heap.intern_string("target"));

        heap.table_set(weak_metatable, mode_key, weak_values)
            .expect("weak mode should be writable");
        heap.table_set_metatable(weak, Some(weak_metatable))
            .expect("weak table should accept its metatable");
        heap.table_set(weak, target_key, Value::table(target))
            .expect("weak value should be writable");
        heap.table_set(holder, target_key, Value::table(target))
            .expect("strong value should be writable");

        heap.collect_weak_tables_with_roots(&[Value::table(weak), Value::table(holder)]);
        assert_eq!(
            heap.table_get(weak, target_key).and_then(Value::as_table),
            Some(target)
        );

        heap.table_set(holder, target_key, Value::nil())
            .expect("strong edge should be removable");
        heap.collect_weak_tables_with_roots(&[Value::table(weak)]);
        assert!(heap
            .table_get(weak, target_key)
            .unwrap_or_else(Value::nil)
            .is_nil());
    }

    #[test]
    fn collector_mark_phase_distinguishes_live_and_unreachable_cycles() {
        let mut heap = VmCell::new();

        let live_environment = heap.alloc_array_table(&[]);
        let live_target = heap.alloc_array_table(&[]);
        let live_upvalue = heap
            .alloc_upvalue(Value::table(live_target))
            .expect("live upvalue handle");
        let live_function = heap
            .alloc_native_function_with_upvalues(
                NativeFunction::Assert,
                vec![live_upvalue],
                live_environment,
            )
            .expect("live function handle");
        heap.tables
            .get_mut(live_environment)
            .expect("live environment")
            .array
            .push(Value::function(live_function));
        let live_userdata_target = heap.alloc_array_table(&[]);
        let live_userdata = heap
            .alloc_userdata_with_payload(
                None,
                UserdataPayload::ConformanceVertex {
                    pos: Value::table(live_userdata_target),
                    normal: Value::nil(),
                    uv: Value::nil(),
                },
            )
            .expect("live userdata handle");

        let dead_environment = heap.alloc_array_table(&[]);
        let dead_target = heap.alloc_array_table(&[]);
        let dead_upvalue = heap
            .alloc_upvalue(Value::table(dead_target))
            .expect("dead upvalue handle");
        let dead_function = heap
            .alloc_native_function_with_upvalues(
                NativeFunction::Assert,
                vec![dead_upvalue],
                dead_environment,
            )
            .expect("dead function handle");
        heap.tables
            .get_mut(dead_environment)
            .expect("dead environment")
            .array
            .push(Value::function(dead_function));
        heap.tables
            .get_mut(dead_target)
            .expect("dead target")
            .array
            .push(Value::table(dead_environment));

        let modes = BTreeMap::new();
        let tables = &heap.tables;
        let functions = &heap.functions;
        let upvalues = &heap.upvalues;
        let live = heap.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            mark_heap_reachable(
                &root,
                tables,
                functions,
                upvalues,
                &modes,
                &[
                    Value::function(live_function),
                    Value::userdata(live_userdata),
                ],
            )
        });

        assert!(live.contains_ref(HeapRef::Function(live_function)));
        assert!(live.contains_ref(HeapRef::Table(live_environment)));
        assert!(live.contains_ref(HeapRef::Table(live_target)));
        assert!(live.contains_ref(HeapRef::Userdata(live_userdata)));
        assert!(live.contains_ref(HeapRef::Table(live_userdata_target)));
        assert!(live.upvalues.contains(live_upvalue));

        assert!(!live.contains_ref(HeapRef::Function(dead_function)));
        assert!(!live.contains_ref(HeapRef::Table(dead_environment)));
        assert!(!live.contains_ref(HeapRef::Table(dead_target)));
        assert!(!live.upvalues.contains(dead_upvalue));
    }

    #[test]
    fn ordinary_sweep_releases_unreachable_backing_allocations() {
        let mut heap = VmCell::new();
        let live = heap.alloc_filled_array_table(128, Value::number(1.0));
        let dead = heap.alloc_filled_array_table(4_096, Value::number(2.0));
        let dead_upvalue = heap
            .alloc_upvalue(Value::table(dead))
            .expect("dead upvalue handle");
        let dead_function = heap
            .alloc_native_function_with_upvalues(NativeFunction::Assert, vec![dead_upvalue], dead)
            .expect("dead function handle");
        let dead_buffer = heap
            .alloc_buffer(vec![0xaa; 64 * 1_024])
            .expect("dead buffer handle");

        heap.collect_weak_tables_with_roots(&[Value::table(live)]);

        assert_eq!(heap.tables.get(live).unwrap().array.len(), 128);
        assert!(heap.tables.get(dead).is_none());
        assert!(heap.functions.get(dead_function).is_none());
        assert!(heap.upvalues.get(dead_upvalue).is_none());
        heap.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            assert!(root.buffers.get(dead_buffer).is_none());
        });
    }

    #[test]
    fn ordinary_sweep_reclaims_unreachable_strings_without_stale_atom_aliasing() {
        let mut heap = VmCell::new();
        let dead = heap.intern_string("dead-runtime-string-that-is-not-rooted");
        let live = heap.intern_string("live-runtime-string-that-remains-rooted");
        let holder = heap.alloc_array_table(&[Value::string_atom(live)]);

        heap.collect_weak_tables_with_roots(&[Value::table(holder)]);

        assert_eq!(
            heap.string_bytes(live).as_deref(),
            Some(b"live-runtime-string-that-remains-rooted".as_slice())
        );
        assert!(heap.string_bytes(dead).is_none());
        assert_eq!(heap.string_atom_count(), 1);

        let replacement = heap.intern_string("replacement-runtime-string");
        assert_ne!(replacement, dead);
        assert!(heap.string_bytes(dead).is_none());
        assert_eq!(
            heap.string_bytes(replacement).as_deref(),
            Some(b"replacement-runtime-string".as_slice())
        );
    }

    #[test]
    fn weak_string_keys_keep_their_interned_atoms_live() {
        let mut heap = VmCell::new();
        let weak = heap.alloc_array_table(&[]);
        let metatable = heap.alloc_array_table(&[]);
        let mode_key = Value::short_string(b"__mode").expect("short mode key");
        let weak_keys = Value::short_string(b"k").expect("short weak mode");
        let key = Value::short_string(b"@@@@").expect("short table key");

        heap.table_set(metatable, mode_key, weak_keys)
            .expect("weak mode should be writable");
        heap.table_set_metatable(weak, Some(metatable))
            .expect("weak table should accept its metatable");
        heap.table_set(weak, key, Value::number(7.0))
            .expect("string key should be writable");

        heap.collect_weak_tables_with_roots(&[Value::table(weak)]);

        assert_eq!(
            heap.table_get(weak, key).and_then(Value::as_number),
            Some(7.0)
        );
    }

    #[test]
    fn quiescent_capacity_release_compacts_live_tables() {
        let mut heap = VmCell::new();
        let table = heap.alloc_filled_array_table(4_096, Value::number(1.0));
        heap.table_clear(table).expect("clear table");
        assert_eq!(heap.tables.get(table).unwrap().array.len(), 4_096);

        heap.collect_weak_tables_with_roots(&[Value::table(table)]);
        heap.release_spare_capacity();

        let table = heap.tables.get(table).expect("rooted table remains live");
        assert!(table.array.is_empty());
        assert_eq!(table.array.capacity(), 0);
        let HashPart::Log { entries, index } = &table.hash else {
            panic!("test table should be Log-layout");
        };
        assert!(entries.is_empty());
        assert_eq!(entries.capacity(), 0);
        assert!(index.is_empty());
        assert_eq!(index.capacity(), 0);
    }

    #[test]
    fn quiescent_trailing_nil_pop_bumps_mutation_version() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[
            Value::number(1.0),
            Value::number(2.0),
            Value::nil(),
            Value::number(4.0),
            Value::nil(),
        ]);
        assert_eq!(heap.array_len(table), Some(2));
        let before = heap.table_mutation_version(table).expect("table version");

        heap.collect_weak_tables_with_roots(&[Value::table(table)]);
        heap.release_spare_capacity();

        // The trailing nil pop moves the observable border to 4; the
        // table.sort mutation guard (the live version consumer) must observe
        // the mutation.
        assert_eq!(heap.array_len(table), Some(4));
        assert_ne!(heap.table_mutation_version(table), Some(before));
    }

    #[test]
    fn reusable_slots_reject_stale_handles_and_retire_before_generation_wrap() {
        let mut slots = ReusableSlots::new();
        let first = slots.insert(10_u32).expect("first slot");
        assert_eq!(slot_handle_parts(first), (0, 0));
        assert!(slots.retain_handles(|handle| handle != first));
        assert!(slots.get(first).is_none());

        let mut current = slots.insert(11_u32).expect("reused slot");
        assert_eq!(slot_handle_parts(current), (0, 1));
        assert_ne!(current, first);
        for generation in 1..SLOT_MAX_GENERATION {
            assert_eq!(slot_handle_parts(current), (0, generation));
            assert!(slots.retain_handles(|handle| handle != current));
            assert!(slots.get(current).is_none());
            current = slots.insert(11_u32).expect("slot before retirement");
        }
        assert_eq!(slot_handle_parts(current), (0, SLOT_MAX_GENERATION));
        assert!(slots.retain_handles(|handle| handle != current));

        let after_retirement = slots.insert(12_u32).expect("replacement slot");
        assert_eq!(slot_handle_parts(after_retirement), (1, 0));
        assert_eq!(slots.storage_len(), 2);
        assert!(slots.get(first).is_none());
        assert!(slots.get(current).is_none());
        assert_eq!(slots.get(after_retirement), Some(&12));
    }

    #[test]
    fn heap_reuses_table_slots_without_stale_value_aliasing() {
        let mut heap = VmCell::new();
        let stale = heap.alloc_array_table(&[Value::number(1.0)]);
        heap.collect_weak_tables_with_roots(&[]);
        assert!(!heap.has_table(stale));

        let replacement = heap.alloc_array_table(&[Value::number(2.0)]);
        assert_eq!(slot_handle_parts(stale).0, slot_handle_parts(replacement).0);
        assert_ne!(stale, replacement);
        assert!(!heap.has_table(stale));
        assert_eq!(
            heap.table_get(replacement, Value::number(1.0))
                .and_then(Value::as_number),
            Some(2.0)
        );
        assert!(heap.table_get(stale, Value::number(1.0)).is_none());
    }

    #[test]
    fn every_collectable_store_reuses_slots_with_generation_validation() {
        let mut heap = VmCell::new();
        let environment = heap.alloc_array_table(&[]);
        let old_function = heap
            .alloc_native_function(NativeFunction::Assert, environment)
            .expect("old function");
        let old_upvalue = heap.alloc_upvalue(Value::number(1.0)).expect("old upvalue");
        let old_userdata = heap.alloc_userdata(None).expect("old userdata");
        let old_buffer = heap.alloc_buffer(vec![1, 2, 3]).expect("old buffer");
        let old_vector = heap
            .alloc_vector(VectorData::new(1.0, 2.0, 3.0))
            .expect("old vector");

        heap.collect_weak_tables_with_roots(&[]);

        let new_environment = heap.alloc_array_table(&[]);
        let new_function = heap
            .alloc_native_function(NativeFunction::Error, new_environment)
            .expect("new function");
        let new_upvalue = heap.alloc_upvalue(Value::number(2.0)).expect("new upvalue");
        let new_userdata = heap.alloc_userdata(None).expect("new userdata");
        let new_buffer = heap.alloc_buffer(vec![4, 5, 6]).expect("new buffer");
        let new_vector = heap
            .alloc_vector(VectorData::new(4.0, 5.0, 6.0))
            .expect("new vector");

        for (old, new) in [
            (old_function, new_function),
            (old_upvalue, new_upvalue),
            (old_userdata, new_userdata),
            (old_buffer, new_buffer),
            (old_vector, new_vector),
        ] {
            assert_eq!(slot_handle_parts(old).0, slot_handle_parts(new).0);
            assert_ne!(old, new);
        }
        assert!(heap.function_kind(old_function).is_none());
        assert!(matches!(
            heap.function_kind(new_function),
            Some(FunctionKind::Native(NativeFunction::Error))
        ));
        assert!(heap.upvalue_get(old_upvalue).is_none());
        assert_eq!(
            heap.upvalue_get(new_upvalue).and_then(Value::as_number),
            Some(2.0)
        );
        assert!(!heap.has_userdata(old_userdata));
        assert!(heap.has_userdata(new_userdata));
        assert!(!heap.has_buffer(old_buffer));
        assert!(heap.has_buffer(new_buffer));
        assert!(heap.vector(old_vector).is_none());
        assert_eq!(
            heap.vector(new_vector)
                .map(|vector| (vector.x(), vector.y(), vector.z())),
            Some((4.0, 5.0, 6.0))
        );
    }

    #[test]
    fn weak_value_sweep_cannot_alias_a_reused_table_slot() {
        let mut heap = VmCell::new();
        let weak = heap.alloc_array_table(&[]);
        let metatable = heap.alloc_array_table(&[]);
        let mode = heap.intern_string("__mode");
        let value_mode = heap.intern_string("v");
        heap.table_set(
            metatable,
            Value::string_atom(mode),
            Value::string_atom(value_mode),
        )
        .unwrap();
        heap.table_set_metatable(weak, Some(metatable)).unwrap();
        let target = heap.alloc_array_table(&[Value::number(1.0)]);
        heap.table_set(weak, Value::number(1.0), Value::table(target))
            .unwrap();

        heap.collect_weak_tables_with_roots(&[Value::table(weak)]);
        let replacement = heap.alloc_array_table(&[Value::number(2.0)]);

        assert_eq!(
            slot_handle_parts(target).0,
            slot_handle_parts(replacement).0
        );
        assert_ne!(target, replacement);
        assert!(heap
            .table_get(weak, Value::number(1.0))
            .unwrap_or_else(Value::nil)
            .is_nil());
    }

    #[test]
    fn bounded_table_churn_reuses_a_fixed_slot_population() {
        let mut heap = VmCell::new();
        const BATCH: usize = 128;
        const ROUNDS: usize = 64;

        for _ in 0..ROUNDS {
            for index in 0..BATCH {
                heap.alloc_filled_array_table(32, Value::number(index as f64));
            }
            heap.collect_weak_tables_with_roots(&[]);
            assert_eq!(heap.tables.occupied_len(), 0);
        }

        assert_eq!(heap.tables.storage_len(), BATCH);
    }

    #[test]
    fn vector_keys_keep_generation_valid_backing_slots_alive() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        let vector = heap
            .alloc_vector(VectorData::new(1.0, 2.0, 3.0))
            .expect("vector");
        heap.table_set(table, Value::vector(vector), Value::number(9.0))
            .expect("vector key write");

        heap.collect_weak_tables_with_roots(&[Value::table(table)]);

        let Some(TableNext::Entry(key, value)) =
            heap.table_next_after(table, Value::nil(), TableIterationOrder::Insertion)
        else {
            panic!("vector key should remain iterable");
        };
        assert_eq!(value.as_number(), Some(9.0));
        assert!(key
            .as_vector()
            .and_then(|handle| heap.vector(handle))
            .is_some());
    }

    #[test]
    fn table_clear_releases_compact_hash_index() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        for index in 0..20 {
            let atom = heap.intern_string(&format!("key_{index}"));
            assert_eq!(
                heap.table_set(table, Value::string_atom(atom), Value::number(index as f64)),
                Some(())
            );
        }
        let HashPart::Log { index, .. } = &heap.tables.get(table).unwrap().hash else {
            panic!("test table should be Log-layout");
        };
        assert!(!index.is_empty());

        assert_eq!(heap.table_clear(table), Some(()));
        let table = heap.tables.get(table).unwrap();
        let HashPart::Log { entries, index } = &table.hash else {
            panic!("test table should be Log-layout");
        };
        assert!(entries.is_empty());
        assert!(index.is_empty());
        assert_eq!(index.capacity(), 0);
    }

    // =========================================================================
    // PUC 5.1.5 table layout
    // =========================================================================

    fn puc_heap() -> VmCell {
        VmCell::with_table_layout(TableLayout::Puc51)
    }

    /// Test-only accessor to the raw PUC node vector.
    fn puc_node_snapshot(heap: &VmCell, table: u32) -> Vec<PucNode> {
        let HashPart::Puc(puc) = &heap.tables.get(table).expect("table exists").hash else {
            panic!("expected a PUC-layout table");
        };
        puc.nodes.clone()
    }

    fn puc_array_len(heap: &VmCell, table: u32) -> usize {
        heap.tables.get(table).expect("table exists").array.len()
    }

    fn puc_node_live_entry(node: &PucNode) -> Option<(TableKey, Value)> {
        match node.key {
            PucNodeKey::Live(key) => Some((key, node.value)),
            PucNodeKey::Free | PucNodeKey::Dead(_) => None,
        }
    }

    /// Walks every live key's chain from its own main position, asserting it
    /// reaches its slot (the PUC chain-integrity invariant).
    fn assert_puc_chain_integrity(heap: &VmCell, table: u32) {
        let tables = &heap.tables;
        heap.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = tables.get(table).expect("table exists");
            let HashPart::Puc(puc) = &table.hash else {
                panic!("expected a PUC-layout table");
            };
            for slot in 0..puc.nodes.len() {
                let PucNodeKey::Live(key) = puc.nodes[slot].key else {
                    continue;
                };
                let mut at = puc_main_position(&root, puc.nodes.len(), key);
                loop {
                    assert!(
                        at < puc.nodes.len(),
                        "PUC chain for slot {slot} leaves the node vector"
                    );
                    if at == slot {
                        break;
                    }
                    let next = puc.nodes[at].next;
                    assert!(
                        next >= 0,
                        "PUC key at slot {slot} is unreachable from its main position"
                    );
                    at = next as usize;
                }
            }
        });
    }

    /// Test-only main-position helper for a single key.
    fn puc_test_main_position(heap: &VmCell, nodes_len: usize, key: TableKey) -> usize {
        heap.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            puc_main_position(&root, nodes_len, key)
        })
    }

    fn same_test_value(a: Value, b: Value) -> bool {
        if a.kind() != b.kind() {
            return false;
        }
        match a.kind() {
            ValueKind::Nil => true,
            ValueKind::Boolean => a.as_bool() == b.as_bool(),
            ValueKind::Number => a.as_number() == b.as_number(),
            ValueKind::String => a.as_string_atom() == b.as_string_atom(),
            _ => false,
        }
    }

    fn collect_puc_iteration(heap: &mut VmCell, table: u32) -> Vec<(Value, Value)> {
        let mut entries = Vec::new();
        let mut after = Value::nil();
        loop {
            match heap.table_next_after(table, after, TableIterationOrder::Puc51) {
                Some(TableNext::Entry(key, value)) => {
                    entries.push((key, value));
                    after = key;
                }
                Some(TableNext::End) => return entries,
                Some(TableNext::InvalidKey) => panic!("iteration key became invalid"),
                None => panic!("table handle is missing"),
            }
        }
    }

    #[test]
    fn puc_chain_integrity_and_lookup_fuzz() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        enum FuzzKey {
            Number(u32),
            String(u32),
            Boolean(bool),
        }

        fn number_key_value(key: u32) -> f64 {
            match key % 3 {
                // Small integral keys are array-part candidates; fractional
                // and large keys live in the node part only.
                0 => f64::from(key / 3 + 1),
                1 => f64::from(key) + 0.25,
                _ => 500.0 + f64::from(key * 7),
            }
        }

        let mut heap = puc_heap();
        let table = heap.alloc_array_table(&[]);
        let mut pool: Vec<(FuzzKey, Value)> = Vec::new();
        for index in 0..36_u32 {
            pool.push((
                FuzzKey::Number(index),
                Value::number(number_key_value(index)),
            ));
        }
        for index in 0..36_u32 {
            let atom = heap.intern_string(&format!("fuzz-key-{index}"));
            pool.push((FuzzKey::String(index), Value::string_atom(atom)));
        }
        for boolean in [false, true] {
            pool.push((FuzzKey::Boolean(boolean), Value::boolean(boolean)));
        }

        let mut state = 0x243F_6A88_85A3_08D3_u64;
        let mut rng = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };

        let mut oracle: BTreeMap<FuzzKey, Option<f64>> = BTreeMap::new();
        for op in 0..500_u32 {
            let (key, key_value) = pool[(rng() as usize) % pool.len()];
            if rng() % 3 == 0 {
                assert_eq!(heap.table_set(table, key_value, Value::nil()), Some(()));
                oracle.insert(key, None);
            } else {
                let value = f64::from(op) + 1_000.0;
                assert_eq!(
                    heap.table_set(table, key_value, Value::number(value)),
                    Some(())
                );
                oracle.insert(key, Some(value));
            }

            assert_puc_chain_integrity(&heap, table);
            for (key, expected) in &oracle {
                let (_, key_value) = pool
                    .iter()
                    .find(|(pool_key, _)| pool_key == key)
                    .expect("oracle key is in the pool");
                let got = heap.table_get(table, *key_value);
                match expected {
                    Some(value) => assert_eq!(
                        got.and_then(Value::as_number),
                        Some(*value),
                        "lookup mismatch for {key:?}"
                    ),
                    None => assert!(
                        got.is_none_or(|value| value.is_nil()),
                        "deleted key {key:?} should read absent or nil"
                    ),
                }
            }
        }
    }

    #[test]
    fn puc_tombstone_slot_is_reused_in_place() {
        let mut heap = puc_heap();
        let table = heap.alloc_array_table(&[]);
        // All number main positions collide modulo ((sizenode-1)|1) == 1
        // while the node vector has two slots.
        assert_eq!(
            heap.table_set(table, Value::number(100.0), Value::number(1.0)),
            Some(())
        );
        assert_eq!(
            heap.table_set(table, Value::number(200.0), Value::number(2.0)),
            Some(())
        );
        let nodes = puc_node_snapshot(&heap, table);
        assert_eq!(nodes.len(), 2);
        let slot_a = nodes
            .iter()
            .position(|node| matches!(node.key, PucNodeKey::Live(TableKey::Number(bits)) if f64::from_bits(bits) == 100.0))
            .expect("key 100 node");
        assert_eq!(slot_a, 0);
        assert_eq!(nodes[0].next, 1);

        assert_eq!(
            heap.table_set(table, Value::number(100.0), Value::nil()),
            Some(())
        );
        let key_300 = number_table_key(300.0).expect("number key");
        assert_eq!(
            puc_test_main_position(&heap, nodes.len(), key_300),
            slot_a,
            "test key 300 must collide at key 100's main position"
        );

        assert_eq!(
            heap.table_set(table, Value::number(300.0), Value::number(3.0)),
            Some(())
        );
        let nodes = puc_node_snapshot(&heap, table);
        assert_eq!(
            nodes.len(),
            2,
            "tombstone reuse must not grow the node vector"
        );
        let (key, value) = puc_node_live_entry(&nodes[0]).expect("slot 0 stays live");
        assert!(key == key_300, "slot 0 should hold key 300");
        assert_eq!(value.as_number(), Some(3.0));
        assert_eq!(nodes[0].next, 1, "the stale chain link is preserved");
        let (key, value) = puc_node_live_entry(&nodes[1]).expect("slot 1 stays live");
        assert!(matches!(key, TableKey::Number(bits) if f64::from_bits(bits) == 200.0));
        assert_eq!(value.as_number(), Some(2.0));

        assert!(heap.table_get(table, Value::number(100.0)).is_none());
        assert_eq!(
            heap.table_get(table, Value::number(300.0))
                .and_then(Value::as_number),
            Some(3.0)
        );
        assert_puc_chain_integrity(&heap, table);
    }

    #[test]
    fn puc_nil_store_to_absent_key_creates_tombstone() {
        let mut heap = puc_heap();
        let table = heap.alloc_array_table(&[]);
        let short = Value::short_string(b"tombstone-key").expect("short string");
        assert_eq!(heap.string_atom_count(), 0);

        // PUC interns the key and creates a nil-valued live node even though
        // the store writes nil (luaV_settable always goes through luaH_set).
        assert_eq!(heap.table_set(table, short, Value::nil()), Some(()));
        assert_eq!(heap.string_atom_count(), 1);

        let nodes = puc_node_snapshot(&heap, table);
        assert_eq!(nodes.len(), 1);
        let (key, value) = puc_node_live_entry(&nodes[0]).expect("tombstone keeps a live key");
        let TableKey::String(atom) = key else {
            panic!("tombstone key should be a string");
        };
        assert_eq!(
            heap.string_bytes(atom).as_deref(),
            Some(b"tombstone-key".as_slice())
        );
        assert!(value.is_nil());
        // key_value keeps the original (short-string) Value, mirroring PUC
        // storing the original TString* in the node key.
        assert!(nodes[0]
            .key_value
            .as_short_string()
            .is_some_and(|s| s.as_bytes() == b"tombstone-key"));
    }

    #[test]
    fn puc_border_matches_lua_h_getn() {
        let mut heap = puc_heap();

        // Dense prefix: array part has no trailing nil, empty node part.
        let dense =
            heap.alloc_array_table(&[Value::number(1.0), Value::number(2.0), Value::number(3.0)]);
        assert_eq!(heap.array_len(dense), Some(3));

        // Forcing a rehash with the next integral key grows the array to 4
        // (computesizes: keys 1..=4 all land in the array part).
        assert_eq!(
            heap.table_set(dense, Value::number(4.0), Value::number(4.0)),
            Some(())
        );
        assert_eq!(puc_array_len(&heap, dense), 4);
        assert_eq!(heap.array_len(dense), Some(4));

        // Trailing nils: binary search inside the array part.
        let trailing_one =
            heap.alloc_array_table(&[Value::number(1.0), Value::nil(), Value::nil(), Value::nil()]);
        assert_eq!(heap.array_len(trailing_one), Some(1));
        let trailing_two = heap.alloc_array_table(&[
            Value::number(1.0),
            Value::number(2.0),
            Value::nil(),
            Value::nil(),
        ]);
        assert_eq!(heap.array_len(trailing_two), Some(2));

        // Hash-only overflow: t[1] then t[2] on an empty table grows the
        // array exactly to 2 through successive rehashes.
        let overflow = heap.alloc_array_table(&[]);
        assert_eq!(
            heap.table_set(overflow, Value::number(1.0), Value::number(1.0)),
            Some(())
        );
        assert_eq!(puc_array_len(&heap, overflow), 1);
        assert_eq!(
            heap.table_set(overflow, Value::number(2.0), Value::number(2.0)),
            Some(())
        );
        assert_eq!(puc_array_len(&heap, overflow), 2);
        assert_eq!(heap.array_len(overflow), Some(2));

        // Sparse: a single far-away key stays in the node part, so the
        // border is 0.
        let sparse = heap.alloc_array_table(&[]);
        assert_eq!(
            heap.table_set(sparse, Value::number(100.0), Value::number(1.0)),
            Some(())
        );
        assert_eq!(puc_array_len(&heap, sparse), 0);
        assert_eq!(heap.array_len(sparse), Some(0));
    }

    #[test]
    fn puc_next_walks_array_then_node_slots() {
        let mut heap = puc_heap();
        let table = heap.alloc_array_table(&[]);
        assert_eq!(
            heap.table_set(table, Value::number(1.0), Value::number(10.0)),
            Some(())
        );
        assert_eq!(
            heap.table_set(table, Value::number(2.0), Value::number(20.0)),
            Some(())
        );
        let atom_x = heap.intern_string("x");
        let atom_y = heap.intern_string("y");
        assert_eq!(
            heap.table_set(table, Value::string_atom(atom_x), Value::number(30.0)),
            Some(())
        );
        assert_eq!(
            heap.table_set(table, Value::string_atom(atom_y), Value::number(40.0)),
            Some(())
        );

        // Expected order: array ascending, then node slots ascending.
        let nodes = puc_node_snapshot(&heap, table);
        let mut expected = vec![
            (Value::number(1.0), Value::number(10.0)),
            (Value::number(2.0), Value::number(20.0)),
        ];
        for node in &nodes {
            if !node.value.is_nil() {
                expected.push((node.key_value, node.value));
            }
        }
        assert_eq!(expected.len(), 4);

        let entries = collect_puc_iteration(&mut heap, table);
        assert_eq!(entries.len(), expected.len());
        for (got, want) in entries.iter().zip(&expected) {
            assert!(same_test_value(got.0, want.0), "iteration key mismatch");
            assert!(same_test_value(got.1, want.1), "iteration value mismatch");
        }

        // The requested order is ignored for PUC tables (raw_table_entries
        // passes Insertion and must see the same sequence).
        let mut insertion_entries = Vec::new();
        let mut after = Value::nil();
        loop {
            match heap.table_next_after(table, after, TableIterationOrder::Insertion) {
                Some(TableNext::Entry(key, value)) => {
                    insertion_entries.push((key, value));
                    after = key;
                }
                Some(TableNext::End) => break,
                _ => panic!("unexpected iteration outcome"),
            }
        }
        assert_eq!(insertion_entries.len(), entries.len());
        for (got, want) in insertion_entries.iter().zip(&entries) {
            assert!(same_test_value(got.0, want.0));
            assert!(same_test_value(got.1, want.1));
        }

        // next() from a tombstoned key resumes at the following slot, or ends
        // when the tombstone occupies the last node slot.
        assert_eq!(
            heap.table_set(table, Value::string_atom(atom_x), Value::nil()),
            Some(())
        );
        let tombstone_slot = puc_node_snapshot(&heap, table)
            .iter()
            .position(|node| matches!(node.key, PucNodeKey::Live(TableKey::String(atom)) if atom == atom_x))
            .expect("tombstone keeps its slot");
        let following = puc_first_node_entry(
            &PucHashPart {
                nodes: puc_node_snapshot(&heap, table),
                lastfree: 0,
            },
            tombstone_slot + 1,
        );
        match (
            heap.table_next_after(
                table,
                Value::string_atom(atom_x),
                TableIterationOrder::Puc51,
            ),
            following,
        ) {
            (Some(TableNext::Entry(key, value)), Some((expected_key, expected_value))) => {
                assert!(same_test_value(key, expected_key));
                assert!(same_test_value(value, expected_value));
            }
            (Some(TableNext::End), None) => {}
            (got, expected) => {
                panic!("next after a tombstone mismatch: got {got:?}, expected {expected:?}");
            }
        }

        // next() from an unknown key reports InvalidKey.
        let unknown = heap.intern_string("unknown");
        assert!(matches!(
            heap.table_next_after(
                table,
                Value::string_atom(unknown),
                TableIterationOrder::Puc51
            ),
            Some(TableNext::InvalidKey)
        ));
    }

    #[test]
    fn puc_rehash_sizes_match_computesizes() {
        let mut heap = puc_heap();

        // Inserting keys 1..=3 as the first writes: the third insert rehashes
        // and computesizes picks an array size of 4.
        let table = heap.alloc_array_table(&[]);
        for index in 1..=3_u32 {
            assert_eq!(
                heap.table_set(
                    table,
                    Value::number(f64::from(index)),
                    Value::number(f64::from(index)),
                ),
                Some(())
            );
        }
        assert_eq!(puc_array_len(&heap, table), 4);

        // A lone key 100 has nums[7] = 1 and never fills half of any slice:
        // the array part stays empty and the key lives in a single node.
        let sparse = heap.alloc_array_table(&[]);
        assert_eq!(
            heap.table_set(sparse, Value::number(100.0), Value::number(1.0)),
            Some(())
        );
        assert_eq!(puc_array_len(&heap, sparse), 0);
        assert_eq!(puc_node_snapshot(&heap, sparse).len(), 1);
    }
}
