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
}

impl<T> ReusableSlots<T> {
    const fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn insert(&mut self, value: T) -> Option<u32> {
        if let Some(index) = self.free.pop() {
            let slot = self.slots.get_mut(index as usize)?;
            debug_assert!(slot.value.is_none());
            debug_assert!(slot.generation > 0);
            slot.value = Some(value);
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
        Some(slot_handle(index, 0))
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
    }

    fn occupied_len(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.value.is_some())
            .count()
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

#[derive(Clone)]
struct TableData {
    array: Vec<Value>,
    hash: Vec<TableEntry>,
    hash_index: Vec<u32>,
    readonly: bool,
    metatable: Option<u32>,
    shape_version: u64,
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

    pub fn new() -> Self {
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
        }
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
    }

    pub(crate) fn memory_usage(&mut self) -> HeapMemoryUsage {
        let mut usage = HeapMemoryUsage::default();
        usage.add_slots(&self.tables);
        usage.add_slots(&self.functions);
        usage.add_slots(&self.upvalues);
        for table in self.tables.values() {
            usage.add_vec(&table.array);
            usage.add_vec(&table.hash);
            usage.add_vec(&table.hash_index);
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

        let arena_bytes = self.arena.metrics().total_gc_allocation();
        usage.add(arena_bytes, arena_bytes);
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            usage.add_slots(&root.strings);
            usage.add_slots(&root.userdatas);
            usage.add_slots(&root.buffers);
            usage.add_slots(&root.vectors);
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
        })
    }

    pub fn release_spare_capacity(&mut self) {
        for table in self.tables.values_mut() {
            while table.array.last().is_some_and(|value| value.is_nil()) {
                table.array.pop();
            }
            table.array.shrink_to_fit();
            table.hash.shrink_to_fit();
            rebuild_hash_index(table);
            table.hash_index.shrink_to_fit();
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
        let atom = self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(atom) = string_index_lookup(&root, value, hash) {
                return atom;
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
            let stored = ShortString::new(value).map_or_else(
                || {
                    allocated_gc_string = true;
                    InternedString::Owned(Gc::new(mc, Static(value.to_vec())))
                },
                InternedString::Inline,
            );
            let atom = root
                .strings
                .insert(stored)
                .expect("string atom table exceeds compact u32 index address space");
            insert_string_index(&mut root, atom);
            atom
        });
        self.allocation_debt_pending |= allocated_gc_string;
        atom
    }

    pub(crate) fn intern_shared_string(&mut self, value: SharedProgramString) -> u32 {
        let stable_hash = value.stable_hash();
        if let Some(short) = ShortString::new(value.as_bytes()) {
            return self.intern_inline_string(short, stable_hash);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(atom) = string_index_lookup(&root, value.as_bytes(), stable_hash) {
                if matches!(root.strings.get(atom), Some(InternedString::Owned(_))) {
                    *root
                        .strings
                        .get_mut(atom)
                        .expect("looked-up string atom remains live") =
                        InternedString::Shared(Static(value));
                }
                return atom;
            }
            ensure_string_index_capacity(&mut root);
            let atom = root
                .strings
                .insert(InternedString::Shared(Static(value)))
                .expect("string atom table exceeds compact u32 index address space");
            insert_string_index(&mut root, atom);
            atom
        })
    }

    fn intern_inline_string(&mut self, value: ShortString, stable_hash: u64) -> u32 {
        self.arena.mutate(|_mc, root| {
            let mut root = root.unlock(_mc).borrow_mut();
            if let Some(atom) = string_index_lookup(&root, value.as_bytes(), stable_hash) {
                return atom;
            }
            ensure_string_index_capacity(&mut root);
            let atom = root
                .strings
                .insert(InternedString::Inline(value))
                .expect("string atom table exceeds compact u32 index address space");
            insert_string_index(&mut root, atom);
            atom
        })
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
        self.tables
            .insert(TableData {
                array: values.to_vec(),
                hash: Vec::new(),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                shape_version: 0,
            })
            .expect("table slot limit exceeded")
    }

    #[cfg(test)]
    pub fn alloc_filled_array_table(&mut self, size: usize, value: Value) -> u32 {
        self.tables
            .insert(TableData {
                array: vec![value; size],
                hash: Vec::new(),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                shape_version: 0,
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
        self.tables.insert(TableData {
            array,
            hash: Vec::new(),
            hash_index: Vec::new(),
            readonly: false,
            metatable: None,
            shape_version: 0,
        })
    }

    pub fn alloc_function(
        &mut self,
        program: Option<u32>,
        prototype: u32,
        captures: Vec<FunctionCapture>,
        environment: u32,
    ) -> Option<u32> {
        self.functions.insert(FunctionData {
            kind: FunctionKind::Bytecode { program, prototype },
            captures: captures.into_iter().collect(),
            environment,
        })
    }

    pub fn alloc_dynamic_main_function(&mut self, program: u32, environment: u32) -> Option<u32> {
        self.functions.insert(FunctionData {
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
        self.functions.insert(FunctionData {
            kind: FunctionKind::Native(native),
            captures: SmallVec::new(),
            environment,
        })
    }

    pub fn alloc_host_function(&mut self, host: u32, environment: u32) -> Option<u32> {
        self.tables.get(environment)?;
        self.functions.insert(FunctionData {
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
        self.functions.insert(FunctionData {
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
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.buffers.insert(BufferData { bytes })
        })
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
        let table = self.tables.get(table)?;
        if let Some(index) = table_key_array_index(table_key) {
            if let Some(value) = table.array.get(index - 1) {
                return Some(*value);
            }
        }
        table_hash_entry(table, table_key).map(|entry| entry.value)
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
        let table = self.tables.get_mut(table)?;
        if table.readonly {
            return Some(TableNumericArrayWrite::Readonly);
        }
        if values.len() > table.array.len() {
            return Some(TableNumericArrayWrite::Deferred);
        }

        for (offset, value) in values.iter().copied().enumerate() {
            table.array[offset] = value;
            if table.hash.is_empty() {
                continue;
            }
            let key = number_table_key((offset + 1) as f64)
                .expect("materialized array offsets are valid numeric table keys");
            if value.is_nil() {
                if let Some(entry) = table_hash_entry_mut(table, key) {
                    entry.value = Value::nil();
                }
            } else {
                remove_hash_key(table, key);
            }
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
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(TableNumericArrayWrite::Deferred);
        }
        let table = self.tables.get_mut(table)?;
        let existing = table
            .array
            .get(one_based_index - 1)
            .copied()
            .unwrap_or_else(Value::nil);
        if existing.is_nil() && table.metatable.is_some() {
            return Some(TableNumericArrayWrite::Deferred);
        }
        if table.readonly {
            return Some(TableNumericArrayWrite::Readonly);
        }

        if value.is_nil() {
            let Some(slot) = table.array.get_mut(one_based_index - 1) else {
                // A numeric hash entry can exist beyond the array boundary.
                return Some(TableNumericArrayWrite::Deferred);
            };
            *slot = Value::nil();
            return Some(TableNumericArrayWrite::Stored);
        }
        if one_based_index > table.array.len() + 1 {
            return Some(TableNumericArrayWrite::Deferred);
        }
        if one_based_index > table.array.len() {
            table.array.resize(one_based_index, Value::nil());
        }
        table.array[one_based_index - 1] = value;
        if !table.hash.is_empty() {
            let key = number_table_key(one_based_index as f64)
                .expect("validated Luau array indices are valid numeric table keys");
            remove_hash_key(table, key);
        }
        Some(TableNumericArrayWrite::Stored)
    }

    pub(crate) fn table_string_hash_probe(
        &mut self,
        table: u32,
        atom: u32,
    ) -> Option<Option<(Value, TableStringHashSlot)>> {
        let table = self.tables.get(table)?;
        let Some(index) = table_hash_entry_index(table, TableKey::String(atom)) else {
            return Some(None);
        };
        let Some(entry) = table.hash.get(index) else {
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
        if table.shape_version != slot.shape_version {
            return Some(None);
        }
        let Some(entry) = table.hash.get(slot.index as usize) else {
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
        if table.shape_version != slot.shape_version {
            return Some(TableStringHashWrite::Deferred);
        }
        let Some(entry) = table.hash.get_mut(slot.index as usize) else {
            return Some(TableStringHashWrite::Deferred);
        };
        if entry.key != TableKey::String(atom) || entry.value.is_nil() {
            return Some(TableStringHashWrite::Deferred);
        }
        if table.readonly {
            return Some(TableStringHashWrite::Readonly);
        }
        entry.value = value;
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
                    if value.is_nil() {
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
        let key_value = self.table_key_visible_value(table_key, key_value)?;
        let table = self.tables.get_mut(table)?;
        if table.readonly {
            return None;
        }
        set_table_data(table, table_key, key_value, value);
        Some(())
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
        let heap_before = self.memory_usage().live_bytes;
        let key_value = key;
        let table_key = if let Some(number) = key.as_number() {
            let Some(key) = number_table_key(number) else {
                return QuotaTableWrite::Invalid;
            };
            key
        } else {
            let key = if let Some(short) = key.as_short_string() {
                let stable_hash = stable_bytes_hash(short.as_bytes());
                let atom = self
                    .find_string_atom_bytes(short.as_bytes())
                    .unwrap_or_else(|| self.intern_inline_string(short, stable_hash));
                Value::string_atom(atom)
            } else {
                key
            };
            let Some(key) = self.table_key_from_value(key) else {
                return QuotaTableWrite::Invalid;
            };
            key
        };
        let Some(key_value) = self.table_key_visible_value(table_key, key_value) else {
            return QuotaTableWrite::Invalid;
        };
        let Some(existing) = self.tables.get(table) else {
            return QuotaTableWrite::Invalid;
        };
        if existing.readonly {
            return QuotaTableWrite::Invalid;
        }
        let old_table_bytes = table_data_live_bytes(existing);
        let mut candidate = existing.clone();
        set_table_data(&mut candidate, table_key, key_value, value);
        let table_growth = table_data_live_bytes(&candidate).saturating_sub(old_table_bytes);
        let atom_growth = self.memory_usage().live_bytes.saturating_sub(heap_before);
        if atom_growth.saturating_add(table_growth) > allowed_growth {
            return QuotaTableWrite::WouldExceed;
        }
        *self
            .tables
            .get_mut(table)
            .expect("validated table remains available") = candidate;
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
        let old_table_bytes = table_data_live_bytes(existing);
        let mut candidate = existing.clone();
        let Some(key) = number_table_key(one_based_index as f64) else {
            return QuotaTableWrite::Invalid;
        };
        if one_based_index > candidate.array.len() {
            candidate.array.resize(one_based_index, Value::nil());
        }
        candidate.array[one_based_index - 1] = value;
        if !candidate.hash.is_empty() {
            remove_hash_key(&mut candidate, key);
        }
        let growth = table_data_live_bytes(&candidate).saturating_sub(old_table_bytes);
        if growth > allowed_growth {
            return QuotaTableWrite::WouldExceed;
        }
        *self
            .tables
            .get_mut(table)
            .expect("validated table remains available") = candidate;
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
        let table = self.tables.get_mut(table)?;
        if table.readonly || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return None;
        }
        let key = number_table_key(one_based_index as f64)?;
        if one_based_index > table.array.len() {
            table.array.resize(one_based_index, Value::nil());
        }
        table.array[one_based_index - 1] = value;
        if !table.hash.is_empty() {
            remove_hash_key(table, key);
        }
        Some(())
    }

    pub fn table_metatable(&mut self, table: u32) -> Option<Option<u32>> {
        self.tables.get(table).map(|table| table.metatable)
    }

    pub fn table_set_metatable(&mut self, table: u32, metatable: Option<u32>) -> Option<()> {
        if let Some(metatable) = metatable {
            self.tables.get(metatable)?;
        }
        let table = self.tables.get_mut(table)?;
        table.metatable = metatable;
        Some(())
    }

    pub fn table_clone(&mut self, table: u32) -> Option<u32> {
        let mut clone = self.tables.get(table)?.clone();
        clone.readonly = false;
        self.tables.insert(clone)
    }

    pub fn array_get(&mut self, table: u32, one_based_index: usize) -> Option<Value> {
        self.table_get(table, Value::number(one_based_index as f64))
    }

    pub fn array_len(&mut self, table: u32) -> Option<usize> {
        self.tables.get(table).map(table_array_boundary_len)
    }

    pub fn table_storage_len(&mut self, table: u32) -> Option<usize> {
        let table = self.tables.get(table)?;
        Some(table.array.len().saturating_add(table.hash.len()))
    }

    pub fn table_clear(&mut self, table: u32) -> Option<()> {
        let table = self.tables.get_mut(table)?;
        if table.readonly {
            return None;
        }
        bump_table_shape_version(table);
        table.array.fill(Value::nil());
        table.hash.clear();
        table.hash_index = Vec::new();
        Some(())
    }

    pub fn table_insert(&mut self, table: u32, position: i32, value: Value) -> Option<()> {
        let table = self.tables.get_mut(table)?;
        if table.readonly {
            return None;
        }
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
        Some(())
    }

    pub fn table_remove(&mut self, table: u32, position: i32) -> Option<Option<Value>> {
        let table = self.tables.get_mut(table)?;
        if table.readonly {
            return None;
        }
        let len = table_array_boundary_len(table);
        if !(1..=i32::try_from(len).ok()?).contains(&position) {
            return Some(None);
        }
        let position = position as usize;
        let removed = table_numeric_get(table, position);
        for index in position..len {
            let value = table_numeric_get(table, index + 1);
            table_numeric_set_array_index(table, index, value);
        }
        table_numeric_set_array_index(table, len, Value::nil());
        Some(Some(removed))
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
        Some(table.hash.iter().fold(array_max, |max, entry| {
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
        }))
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
        table.readonly = true;
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

    if let Some(value) = table.array.get(one_based_index - 1) {
        return *value;
    }

    let Some(key) = number_table_key(one_based_index as f64) else {
        return Value::nil();
    };
    table_hash_entry(table, key).map_or_else(Value::nil, |entry| entry.value)
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

    if value.kind() == ValueKind::Nil {
        if let Some(slot) = table.array.get_mut(index - 1) {
            *slot = Value::nil();
        }
        if !table.hash.is_empty() {
            remove_hash_key(table, key);
        }
    } else if index <= table.array.len() + 1 {
        if index > table.array.len() {
            table.array.resize(index, Value::nil());
        }
        table.array[index - 1] = value;
        if !table.hash.is_empty() {
            remove_hash_key(table, key);
        }
    } else {
        set_hash_key(table, key, key_value, value);
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
    set_hash_key(table, key, key_value, value);
}

fn table_data_live_bytes(table: &TableData) -> usize {
    table
        .array
        .len()
        .saturating_mul(size_of::<Value>())
        .saturating_add(table.hash.len().saturating_mul(size_of::<TableEntry>()))
        .saturating_add(table.hash_index.len().saturating_mul(size_of::<u32>()))
}

fn set_table_data(table: &mut TableData, table_key: TableKey, key_value: Value, value: Value) {
    if value.is_nil() {
        if let Some(index) = table_key_array_index(table_key) {
            if let Some(slot) = table.array.get_mut(index - 1) {
                *slot = Value::nil();
            }
        }
        if let Some(entry) = table_hash_entry_mut(table, table_key) {
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
            if !table.hash.is_empty() {
                remove_hash_key(table, table_key);
            }
            return;
        }
    }
    set_hash_key(table, table_key, key_value, value);
}

fn set_hash_key(table: &mut TableData, key: TableKey, key_value: Value, value: Value) {
    match table_hash_lookup(table, key) {
        TableHashLookup::Found(index) => {
            if let Some(entry) = table.hash.get_mut(index) {
                entry.value = value;
            }
        }
        TableHashLookup::Vacant(vacant_hash_index) => {
            if !value.is_nil() {
                let index = table.hash.len();
                table.hash.push(TableEntry {
                    key,
                    key_value,
                    value,
                });
                bump_table_shape_version(table);
                record_hash_insert(table, index, vacant_hash_index);
            }
        }
    }
}

#[inline]
fn remove_hash_key(table: &mut TableData, key: TableKey) {
    if let Some(hash_index) = table_hash_entry_index(table, key) {
        table.hash.remove(hash_index);
        bump_table_shape_version(table);
        record_hash_remove(table);
    }
}

#[derive(Clone, Copy)]
enum TableHashLookup {
    Found(usize),
    Vacant(Option<usize>),
}

#[inline]
fn table_hash_entry(table: &TableData, key: TableKey) -> Option<&TableEntry> {
    let index = table_hash_entry_index(table, key)?;
    table.hash.get(index)
}

#[inline]
fn table_hash_entry_mut(table: &mut TableData, key: TableKey) -> Option<&mut TableEntry> {
    let index = table_hash_entry_index(table, key)?;
    table.hash.get_mut(index)
}

#[inline]
fn table_hash_entry_index(table: &TableData, key: TableKey) -> Option<usize> {
    match table_hash_lookup(table, key) {
        TableHashLookup::Found(index) => Some(index),
        TableHashLookup::Vacant(_) => None,
    }
}

#[inline]
fn table_hash_lookup(table: &TableData, key: TableKey) -> TableHashLookup {
    if table.hash.is_empty() {
        return TableHashLookup::Vacant(None);
    }
    if table.hash_index.is_empty() {
        return match table.hash.iter().position(|entry| entry.key == key) {
            Some(index) => TableHashLookup::Found(index),
            None => TableHashLookup::Vacant(None),
        };
    }

    let mut position = table_hash_index_position(key, table.hash_index.len());
    for _ in 0..table.hash_index.len() {
        let slot = table.hash_index[position];
        if slot == 0 {
            return TableHashLookup::Vacant(Some(position));
        }
        let index = (slot - 1) as usize;
        let indexed_entry = table.hash.get(index);
        debug_assert!(
            indexed_entry.is_some(),
            "table hash side index contains an invalid entry index"
        );
        if indexed_entry.is_some_and(|candidate| candidate.key == key) {
            return TableHashLookup::Found(index);
        }
        position = (position + 1) & (table.hash_index.len() - 1);
    }
    TableHashLookup::Vacant(None)
}

fn record_hash_insert(table: &mut TableData, index: usize, vacant_hash_index: Option<usize>) {
    if table.hash_index.is_empty() {
        if table.hash.len() >= TABLE_HASH_INDEX_MIN_LEN {
            rebuild_hash_index(table);
        }
        return;
    }
    if let Some(position) = vacant_hash_index {
        if table.hash.len().saturating_mul(2) > table.hash_index.len() {
            rebuild_hash_index(table);
        } else if let Some(entry) = table.hash_index.get_mut(position) {
            *entry = compact_hash_index_slot(index);
        }
        return;
    }
    insert_hash_index(table, index);
}

fn insert_hash_index(table: &mut TableData, index: usize) {
    if table.hash.len().saturating_mul(2) > table.hash_index.len() {
        rebuild_hash_index(table);
        return;
    }
    let Some(key) = table.hash.get(index).map(|entry| entry.key) else {
        debug_assert!(
            false,
            "table hash side index insertion references an invalid entry"
        );
        return;
    };
    let mut position = table_hash_index_position(key, table.hash_index.len());
    for _ in 0..table.hash_index.len() {
        let slot = table.hash_index[position];
        if slot == 0 {
            table.hash_index[position] = compact_hash_index_slot(index);
            return;
        }
        let existing_index = (slot - 1) as usize;
        match table.hash.get(existing_index) {
            Some(entry) if entry.key == key => {
                table.hash_index[position] = compact_hash_index_slot(index);
                return;
            }
            _ => position = (position + 1) & (table.hash_index.len() - 1),
        }
    }
    rebuild_hash_index(table);
}

fn record_hash_remove(table: &mut TableData) {
    if table.hash.len() < TABLE_HASH_INDEX_MIN_LEN {
        table.hash_index = Vec::new();
    } else {
        rebuild_hash_index(table);
    }
}

fn rebuild_hash_index(table: &mut TableData) {
    if table.hash.len() < TABLE_HASH_INDEX_MIN_LEN {
        table.hash_index = Vec::new();
        return;
    }
    let capacity = table
        .hash
        .len()
        .saturating_mul(2)
        .next_power_of_two()
        .max(TABLE_HASH_INDEX_MIN_LEN * 2);
    table.hash_index = vec![0; capacity];
    for index in 0..table.hash.len() {
        insert_hash_index(table, index);
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
    let mode = table_hash_entry(metatable, TableKey::String(mode_atom))?.value;
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
                for entry in &table.hash {
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
    if mode.values {
        for value in &mut table.array {
            if weak_value_is_collectable(*value) && !live.contains_value(*value) {
                *value = Value::nil();
            }
        }
    }
    let hash_len = table.hash.len();
    table.hash.retain(|entry| {
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
    if table.hash.len() != hash_len {
        bump_table_shape_version(table);
        rebuild_hash_index(table);
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
    if order == TableIterationOrder::Insertion {
        return table.hash.iter().find_map(|entry| {
            (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
        });
    }
    ordered_hash_entries(root, table, order)
        .into_iter()
        .find_map(|entry| {
            (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
        })
}

fn next_after_hash_key(
    root: &RootData<'_>,
    table: &TableData,
    key: TableKey,
    order: TableIterationOrder,
) -> Option<Option<(Value, Value)>> {
    if order == TableIterationOrder::Insertion {
        let index = table_hash_entry_index(table, key)?;
        return Some(table.hash.iter().skip(index + 1).find_map(|entry| {
            (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
        }));
    }
    let entries = ordered_hash_entries(root, table, order);
    let index = entries.iter().position(|entry| entry.key == key)?;
    Some(entries.into_iter().skip(index + 1).find_map(|entry| {
        (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
    }))
}

fn ordered_hash_entries<'a>(
    root: &RootData<'_>,
    table: &'a TableData,
    order: TableIterationOrder,
) -> Vec<&'a TableEntry> {
    match order {
        TableIterationOrder::Insertion => table.hash.iter().collect(),
        TableIterationOrder::LuauHash => luau_hash_nodes(root, table),
    }
}

fn luau_hash_nodes<'a>(root: &RootData<'_>, table: &'a TableData) -> Vec<&'a TableEntry> {
    let Some(hash_size) = table_hash_size(table) else {
        return Vec::new();
    };
    let mut nodes = vec![LuauHashNode::default(); hash_size];
    let mut last_free = hash_size;
    for entry in &table.hash {
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

fn table_hash_size(table: &TableData) -> Option<usize> {
    if table.hash.is_empty() {
        None
    } else {
        Some(table.hash.len().next_power_of_two())
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
            hash: Vec::new(),
            hash_index: Vec::new(),
            readonly: false,
            metatable: None,
            shape_version: 0,
        }
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
            set_hash_key(
                &mut table,
                TableKey::String(*atom),
                Value::string_atom(*atom),
                Value::number(f64::from(*atom)),
            );
        }
        assert_eq!(table.hash_index.len(), 64);
        for atom in &atoms {
            assert_eq!(
                table_hash_entry(&table, TableKey::String(*atom))
                    .and_then(|entry| entry.value.as_number()),
                Some(f64::from(*atom))
            );
        }

        let removed = atoms.remove(7);
        remove_hash_key(&mut table, TableKey::String(removed));
        assert_eq!(table.hash.len(), atoms.len());
        assert!(!table.hash_index.is_empty());
        assert!(table_hash_entry(&table, TableKey::String(removed)).is_none());
        for atom in &atoms {
            assert!(table_hash_entry(&table, TableKey::String(*atom)).is_some());
        }

        let clone = table.clone();
        for atom in &atoms {
            assert!(table_hash_entry(&clone, TableKey::String(*atom)).is_some());
        }

        while table.hash.len() >= TABLE_HASH_INDEX_MIN_LEN {
            let key = table.hash[0].key;
            remove_hash_key(&mut table, key);
        }
        assert!(table.hash_index.is_empty());
        assert_eq!(table.hash_index.capacity(), 0);
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
        assert!(table.hash.is_empty());
        assert_eq!(table.hash.capacity(), 0);
        assert!(table.hash_index.is_empty());
        assert_eq!(table.hash_index.capacity(), 0);
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
        assert!(!heap.tables.get(table).unwrap().hash_index.is_empty());

        assert_eq!(heap.table_clear(table), Some(()));
        let table = heap.tables.get(table).unwrap();
        assert!(table.hash.is_empty());
        assert!(table.hash_index.is_empty());
        assert_eq!(table.hash_index.capacity(), 0);
    }
}
