use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRootSet, Gc, Rootable, Static};
use smallvec::SmallVec;
use std::collections::{BTreeSet, VecDeque};

use crate::bytecode::{stable_bytes_hash, SharedProgramString};
use crate::number::table_key_number_bits;
use crate::value::{ShortString, Value, ValueKind};

const MAX_LUAU_ARRAY_INDEX: usize = 2_147_483_647;
const TABLE_HASH_INDEX_MIN_LEN: usize = 16;
const STRING_INDEX_MIN_CAPACITY: usize = 16;

#[derive(Collect)]
#[collect(no_drop)]
struct RootData<'gc> {
    _roots: DynamicRootSet<'gc>,
    strings: Vec<InternedString<'gc>>,
    string_index: Vec<u32>,
    userdatas: Vec<UserdataData>,
    buffers: Vec<BufferData>,
    vectors: Vec<VectorData>,
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
    upvalues: SmallVec<[u32; 4]>,
    environment: u32,
}

pub(crate) type FunctionDispatchData = (FunctionKind, Option<(SmallVec<[u32; 4]>, u32)>);

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
    tables: Vec<TableData>,
    // Function metadata contains only stable compact handles, not GC pointers.
    // Keeping it beside the arena avoids entering `Arena::mutate` for every
    // bytecode/native dispatch while weak-table traversal can still inspect it.
    functions: Vec<FunctionData>,
    // Upvalues are compact `Value` handles as well. Their hot read/write path
    // does not need a GC mutation boundary; weak traversal receives this slice.
    upvalues: Vec<UpvalueData>,
}

impl VmCell {
    pub fn new() -> Self {
        let arena = VmArena::new(|mc| {
            Gc::new(
                mc,
                RefLock::new(RootData {
                    _roots: DynamicRootSet::new(mc),
                    strings: Vec::new(),
                    string_index: Vec::new(),
                    userdatas: Vec::new(),
                    buffers: Vec::new(),
                    vectors: Vec::new(),
                }),
            )
        });
        let allocation_debt_pending = arena.metrics().allocation_debt() > 0.0;
        Self {
            arena,
            allocation_debt_pending,
            tables: Vec::new(),
            functions: Vec::new(),
            upvalues: Vec::new(),
        }
    }

    pub fn finish_collection_cycle(&mut self) {
        self.arena.finish_cycle();
        self.allocation_debt_pending = self.arena.metrics().allocation_debt() > 0.0;
    }

    pub fn collect_weak_tables_with_roots(&mut self, roots: &[Value]) {
        let tables = &mut self.tables;
        let functions = &self.functions;
        let upvalues = &self.upvalues;
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let mode_key = b"__mode";
            let Some(mode_atom) = string_index_lookup(&root, mode_key, stable_bytes_hash(mode_key))
            else {
                return;
            };
            let modes = (0..tables.len())
                .map(|index| table_weak_mode(&root, tables, index, mode_atom))
                .collect::<Vec<_>>();
            if modes.iter().all(Option::is_none) {
                return;
            }
            let live = mark_weak_reachable(&root, tables, functions, upvalues, &modes, roots);
            for (index, mode) in modes.into_iter().enumerate() {
                if let Some(mode) = mode {
                    if let Some(table) = tables.get_mut(index) {
                        sweep_weak_table(table, mode, &live);
                    }
                }
            }
        })
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
            let required = root.strings.len().saturating_add(1);
            if root.string_index.is_empty() || required.saturating_mul(2) > root.string_index.len()
            {
                let capacity = required
                    .saturating_mul(2)
                    .next_power_of_two()
                    .max(STRING_INDEX_MIN_CAPACITY);
                rebuild_string_index(&mut root, capacity);
            }
            let atom = u32::try_from(root.strings.len())
                .ok()
                .and_then(|atom| atom.checked_add(1).map(|_| atom))
                .expect("string atom table exceeds compact u32 index address space");
            let stored = ShortString::new(value).map_or_else(
                || {
                    allocated_gc_string = true;
                    InternedString::Owned(Gc::new(mc, Static(value.to_vec())))
                },
                InternedString::Inline,
            );
            root.strings.push(stored);
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
                if matches!(
                    root.strings.get(atom as usize),
                    Some(InternedString::Owned(_))
                ) {
                    root.strings[atom as usize] = InternedString::Shared(Static(value));
                }
                return atom;
            }
            ensure_string_index_capacity(&mut root);
            let atom = next_string_atom(&root);
            root.strings.push(InternedString::Shared(Static(value)));
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
            let atom = next_string_atom(&root);
            root.strings.push(InternedString::Inline(value));
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
                .get(atom as usize)
                .map(|candidate| candidate.as_bytes() == value)
        })
    }

    #[cfg(test)]
    pub(crate) fn string_atom_count(&mut self) -> usize {
        self.arena
            .mutate(|mc, root| root.unlock(mc).borrow().strings.len())
    }

    pub fn string_bytes(&mut self, atom: u32) -> Option<Vec<u8>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .strings
                .get(atom as usize)
                .map(|value| value.as_bytes().to_vec())
        })
    }

    pub fn alloc_array_table(&mut self, values: &[Value]) -> u32 {
        let handle = self.tables.len();
        self.tables.push(TableData {
            array: values.to_vec(),
            hash: Vec::new(),
            hash_index: Vec::new(),
            readonly: false,
            metatable: None,
            shape_version: 0,
        });
        handle as u32
    }

    pub fn alloc_filled_array_table(&mut self, size: usize, value: Value) -> u32 {
        let handle = self.tables.len();
        self.tables.push(TableData {
            array: vec![value; size],
            hash: Vec::new(),
            hash_index: Vec::new(),
            readonly: false,
            metatable: None,
            shape_version: 0,
        });
        handle as u32
    }

    pub fn alloc_function(
        &mut self,
        program: Option<u32>,
        prototype: u32,
        upvalues: Vec<u32>,
        environment: u32,
    ) -> Option<u32> {
        let handle = u32::try_from(self.functions.len()).ok()?;
        self.functions.push(FunctionData {
            kind: FunctionKind::Bytecode { program, prototype },
            upvalues: upvalues.into_iter().collect(),
            environment,
        });
        Some(handle)
    }

    pub fn alloc_dynamic_main_function(&mut self, program: u32, environment: u32) -> Option<u32> {
        let handle = u32::try_from(self.functions.len()).ok()?;
        self.functions.push(FunctionData {
            kind: FunctionKind::DynamicMain { program },
            upvalues: SmallVec::new(),
            environment,
        });
        Some(handle)
    }

    pub fn alloc_native_function(
        &mut self,
        native: NativeFunction,
        environment: u32,
    ) -> Option<u32> {
        let handle = u32::try_from(self.functions.len()).ok()?;
        self.functions.push(FunctionData {
            kind: FunctionKind::Native(native),
            upvalues: SmallVec::new(),
            environment,
        });
        Some(handle)
    }

    pub fn alloc_host_function(&mut self, host: u32, environment: u32) -> Option<u32> {
        self.tables.get(environment as usize)?;
        let handle = u32::try_from(self.functions.len()).ok()?;
        self.functions.push(FunctionData {
            kind: FunctionKind::Host(host),
            upvalues: SmallVec::new(),
            environment,
        });
        Some(handle)
    }

    pub fn alloc_native_function_with_upvalues(
        &mut self,
        native: NativeFunction,
        upvalues: Vec<u32>,
        environment: u32,
    ) -> Option<u32> {
        for upvalue in &upvalues {
            self.upvalues.get(*upvalue as usize)?;
        }
        let handle = u32::try_from(self.functions.len()).ok()?;
        self.functions.push(FunctionData {
            kind: FunctionKind::Native(native),
            upvalues: upvalues.into_iter().collect(),
            environment,
        });
        Some(handle)
    }

    #[inline]
    pub fn function_kind(&self, handle: u32) -> Option<FunctionKind> {
        self.functions
            .get(handle as usize)
            .map(|function| function.kind)
    }

    /// Returns the data needed to dispatch a function call in one arena access.
    ///
    /// Bytecode calls need their captured upvalues and environment immediately
    /// after inspecting the function kind. Native and host calls deliberately do
    /// not clone that metadata on their dispatch path.
    #[inline]
    pub fn function_dispatch_data(&self, handle: u32) -> Option<FunctionDispatchData> {
        let function = self.functions.get(handle as usize)?;
        let bytecode_data = match function.kind {
            FunctionKind::Bytecode { .. } | FunctionKind::DynamicMain { .. } => {
                Some((function.upvalues.clone(), function.environment))
            }
            FunctionKind::Host(_) | FunctionKind::Native(_) => None,
        };
        Some((function.kind, bytecode_data))
    }

    #[inline]
    pub fn function_upvalues(&self, handle: u32) -> Option<SmallVec<[u32; 4]>> {
        self.functions
            .get(handle as usize)
            .map(|function| function.upvalues.clone())
    }

    #[inline]
    pub fn function_environment(&self, handle: u32) -> Option<u32> {
        self.functions
            .get(handle as usize)
            .map(|function| function.environment)
    }

    pub fn set_function_environment(&mut self, handle: u32, environment: u32) -> Option<()> {
        let function = self.functions.get_mut(handle as usize)?;
        function.environment = environment;
        Some(())
    }

    pub fn alloc_upvalue(&mut self, value: Value) -> Option<u32> {
        let handle = u32::try_from(self.upvalues.len()).ok()?;
        self.upvalues.push(UpvalueData { value });
        Some(handle)
    }

    #[inline]
    pub fn upvalue_get(&self, handle: u32) -> Option<Value> {
        self.upvalues
            .get(handle as usize)
            .map(|upvalue| upvalue.value)
    }

    #[inline]
    pub fn upvalue_set(&mut self, handle: u32, value: Value) -> Option<()> {
        let upvalue = self.upvalues.get_mut(handle as usize)?;
        upvalue.value = value;
        Some(())
    }

    pub fn alloc_userdata(&mut self, metatable: Option<u32>) -> Option<u32> {
        if let Some(table) = metatable {
            self.tables.get(table as usize)?;
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.userdatas.len()).ok()?;
            root.userdatas.push(UserdataData {
                metatable,
                payload: UserdataPayload::None,
            });
            Some(handle)
        })
    }

    pub(crate) fn alloc_userdata_with_payload(
        &mut self,
        metatable: Option<u32>,
        payload: UserdataPayload,
    ) -> Option<u32> {
        if let Some(table) = metatable {
            self.tables.get(table as usize)?;
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.userdatas.len()).ok()?;
            root.userdatas.push(UserdataData { metatable, payload });
            Some(handle)
        })
    }

    pub fn alloc_buffer(&mut self, bytes: Vec<u8>) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.buffers.len()).ok()?;
            root.buffers.push(BufferData { bytes });
            Some(handle)
        })
    }

    pub(crate) fn alloc_vector(&mut self, vector: VectorData) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.vectors.len()).ok()?;
            root.vectors.push(vector);
            Some(handle)
        })
    }

    pub(crate) fn vector(&mut self, vector: u32) -> Option<VectorData> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .vectors
                .get(vector as usize)
                .copied()
        })
    }

    pub fn has_buffer(&mut self, buffer: u32) -> bool {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .buffers
                .get(buffer as usize)
                .is_some()
        })
    }

    pub fn buffer_len(&mut self, buffer: u32) -> Option<usize> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .buffers
                .get(buffer as usize)
                .map(|buffer| buffer.bytes.len())
        })
    }

    pub fn buffer_read(&mut self, buffer: u32, offset: usize, count: usize) -> Option<Vec<u8>> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let buffer = root.buffers.get(buffer as usize)?;
            let end = offset.checked_add(count)?;
            (end <= buffer.bytes.len()).then(|| buffer.bytes[offset..end].to_vec())
        })
    }

    pub fn buffer_write(&mut self, buffer: u32, offset: usize, bytes: &[u8]) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let buffer = root.buffers.get_mut(buffer as usize)?;
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
            let buffer = root.buffers.get_mut(buffer as usize)?;
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
                .get(userdata as usize)
                .map(|userdata| userdata.metatable)
        })
    }

    pub fn has_userdata(&mut self, userdata: u32) -> bool {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .userdatas
                .get(userdata as usize)
                .is_some()
        })
    }

    pub(crate) fn userdata_payload(&mut self, userdata: u32) -> Option<UserdataPayload> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .userdatas
                .get(userdata as usize)
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
            let userdata = root.userdatas.get_mut(userdata as usize)?;
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
            self.tables.get(table as usize)?;
            TableKey::String(self.find_string_atom_bytes(short.as_bytes())?)
        } else if key.kind() == ValueKind::Vector {
            self.tables.get(table as usize)?;
            self.table_key_from_value(key)?
        } else {
            self.table_key_from_value(key)?
        };
        let table = self.tables.get(table as usize)?;
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
            .get(table as usize)?
            .array
            .get(one_based_index - 1)
            .copied()
    }

    pub(crate) fn read_array_prefix(&mut self, table: u32, len: usize) -> Option<Vec<Value>> {
        let table = self.tables.get(table as usize)?;
        Some(table.array.get(..len)?.to_vec())
    }

    pub(crate) fn try_write_array_prefix(
        &mut self,
        table: u32,
        values: &[Value],
    ) -> Option<TableNumericArrayWrite> {
        let table = self.tables.get_mut(table as usize)?;
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
        let table = self.tables.get_mut(table as usize)?;
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
        let table = self.tables.get(table as usize)?;
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
        let table = self.tables.get(table as usize)?;
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
        let table = self.tables.get_mut(table as usize)?;
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
                    let table = self.tables.get(table as usize)?;
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
        let table = self.tables.get_mut(table as usize)?;
        if table.readonly {
            return None;
        }
        if value.is_nil() {
            if let Some(index) = table_key_array_index(table_key) {
                if let Some(slot) = table.array.get_mut(index - 1) {
                    *slot = Value::nil();
                }
            }
            if let Some(entry) = table_hash_entry_mut(table, table_key) {
                entry.value = Value::nil();
            }
            return Some(());
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
                return Some(());
            }
        }

        set_hash_key(table, table_key, key_value, value);
        Some(())
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
        let table = self.tables.get_mut(table as usize)?;
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
        self.tables.get(table as usize).map(|table| table.metatable)
    }

    pub fn table_set_metatable(&mut self, table: u32, metatable: Option<u32>) -> Option<()> {
        if let Some(metatable) = metatable {
            self.tables.get(metatable as usize)?;
        }
        let table = self.tables.get_mut(table as usize)?;
        table.metatable = metatable;
        Some(())
    }

    pub fn table_clone(&mut self, table: u32) -> Option<u32> {
        let mut clone = self.tables.get(table as usize)?.clone();
        clone.readonly = false;
        let handle = u32::try_from(self.tables.len()).ok()?;
        self.tables.push(clone);
        Some(handle)
    }

    pub fn array_get(&mut self, table: u32, one_based_index: usize) -> Option<Value> {
        self.table_get(table, Value::number(one_based_index as f64))
    }

    pub fn array_len(&mut self, table: u32) -> Option<usize> {
        self.tables
            .get(table as usize)
            .map(table_array_boundary_len)
    }

    pub fn table_storage_len(&mut self, table: u32) -> Option<usize> {
        let table = self.tables.get(table as usize)?;
        Some(table.array.len().saturating_add(table.hash.len()))
    }

    pub fn table_clear(&mut self, table: u32) -> Option<()> {
        let table = self.tables.get_mut(table as usize)?;
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
        let table = self.tables.get_mut(table as usize)?;
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
        let table = self.tables.get_mut(table as usize)?;
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
        let table = self.tables.get(table as usize)?;
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
        self.tables.get(table as usize)?;
        let table_key = match after.kind() {
            ValueKind::Nil => None,
            _ => Some(self.table_key_from_value(after)),
        };
        let tables = &self.tables;
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = tables.get(table as usize)?;
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
        self.tables.get(table as usize).map(|table| table.readonly)
    }

    pub fn table_freeze(&mut self, table: u32) -> Option<bool> {
        let table = self.tables.get_mut(table as usize)?;
        let was_readonly = table.readonly;
        table.readonly = true;
        Some(was_readonly)
    }

    pub fn has_table(&mut self, table: u32) -> bool {
        self.tables.get(table as usize).is_some()
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
        let indexed_value = root.strings.get(atom as usize);
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
    let required = root.strings.len().saturating_add(1);
    if root.string_index.is_empty() || required.saturating_mul(2) > root.string_index.len() {
        let capacity = required
            .saturating_mul(2)
            .next_power_of_two()
            .max(STRING_INDEX_MIN_CAPACITY);
        rebuild_string_index(root, capacity);
    }
}

fn next_string_atom(root: &RootData<'_>) -> u32 {
    u32::try_from(root.strings.len())
        .ok()
        .and_then(|atom| atom.checked_add(1).map(|_| atom))
        .expect("string atom table exceeds compact u32 index address space")
}

fn rebuild_string_index(root: &mut RootData<'_>, requested_capacity: usize) {
    let required_capacity = root
        .strings
        .len()
        .saturating_mul(2)
        .next_power_of_two()
        .max(STRING_INDEX_MIN_CAPACITY);
    let capacity = requested_capacity
        .max(required_capacity)
        .next_power_of_two();
    root.string_index = vec![0; capacity];
    for atom in 0..root.strings.len() {
        let atom = u32::try_from(atom)
            .ok()
            .and_then(|atom| atom.checked_add(1).map(|_| atom))
            .expect("string atom table exceeds compact u32 index address space");
        insert_string_index(root, atom);
    }
}

fn insert_string_index(root: &mut RootData<'_>, atom: u32) {
    debug_assert!(!root.string_index.is_empty());
    debug_assert!(root.string_index.len().is_power_of_two());
    let slot = atom
        .checked_add(1)
        .expect("string atom table exceeds compact u32 index address space");
    let value = root
        .strings
        .get(atom as usize)
        .expect("string index insertion references an invalid atom");
    let mut position = string_index_position(value.stable_hash(), root.string_index.len());
    for _ in 0..root.string_index.len() {
        if root.string_index[position] == 0 {
            root.string_index[position] = slot;
            return;
        }
        position = (position + 1) & (root.string_index.len() - 1);
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
enum WeakRef {
    Table(u32),
    Function(u32),
    Userdata(u32),
    Buffer(u32),
}

struct WeakLiveSet {
    refs: BTreeSet<WeakRef>,
}

impl WeakLiveSet {
    fn contains_value(&self, value: Value) -> bool {
        weak_ref_from_value(value).is_some_and(|reference| self.refs.contains(&reference))
    }

    fn contains_key(&self, key: TableKey) -> bool {
        weak_ref_from_key(key).is_some_and(|reference| self.refs.contains(&reference))
    }
}

fn table_weak_mode(
    root: &RootData<'_>,
    tables: &[TableData],
    table: usize,
    mode_atom: u32,
) -> Option<WeakMode> {
    let table = tables.get(table)?;
    let metatable = table.metatable?;
    let metatable = tables.get(metatable as usize)?;
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
        .get(atom as usize)
        .map(|value| value.as_bytes().contains(&needle))
}

fn mark_weak_reachable(
    root: &RootData<'_>,
    tables: &[TableData],
    functions: &[FunctionData],
    upvalues: &[UpvalueData],
    modes: &[Option<WeakMode>],
    roots: &[Value],
) -> WeakLiveSet {
    let mut refs = BTreeSet::new();
    let mut queue = VecDeque::new();

    for value in roots {
        mark_weak_value(*value, &mut refs, &mut queue);
    }

    while let Some(reference) = queue.pop_front() {
        match reference {
            WeakRef::Buffer(_) => {}
            WeakRef::Userdata(handle) => {
                let Some(userdata) = root.userdatas.get(handle as usize) else {
                    continue;
                };
                if let Some(metatable) = userdata.metatable {
                    mark_weak_value(Value::table(metatable), &mut refs, &mut queue);
                }
            }
            WeakRef::Function(handle) => {
                let Some(function) = functions.get(handle as usize) else {
                    continue;
                };
                mark_weak_value(Value::table(function.environment), &mut refs, &mut queue);
                for upvalue in &function.upvalues {
                    if let Some(upvalue) = upvalues.get(*upvalue as usize) {
                        mark_weak_value(upvalue.value, &mut refs, &mut queue);
                    }
                }
            }
            WeakRef::Table(handle) => {
                let Some(table) = tables.get(handle as usize) else {
                    continue;
                };
                if let Some(metatable) = table.metatable {
                    mark_weak_value(Value::table(metatable), &mut refs, &mut queue);
                }
                let mode = modes.get(handle as usize).copied().flatten();
                let weak_keys = mode.is_some_and(|mode| mode.keys);
                let weak_values = mode.is_some_and(|mode| mode.values);
                if !weak_values {
                    for value in &table.array {
                        mark_weak_value(*value, &mut refs, &mut queue);
                    }
                }
                for entry in &table.hash {
                    if !weak_keys {
                        mark_weak_key(entry.key, &mut refs, &mut queue);
                    }
                    if !weak_values {
                        mark_weak_value(entry.value, &mut refs, &mut queue);
                    }
                }
            }
        }
    }

    WeakLiveSet { refs }
}

fn mark_weak_value(value: Value, refs: &mut BTreeSet<WeakRef>, queue: &mut VecDeque<WeakRef>) {
    if let Some(reference) = weak_ref_from_value(value) {
        mark_weak_ref(reference, refs, queue);
    }
}

fn mark_weak_key(key: TableKey, refs: &mut BTreeSet<WeakRef>, queue: &mut VecDeque<WeakRef>) {
    if let Some(reference) = weak_ref_from_key(key) {
        mark_weak_ref(reference, refs, queue);
    }
}

fn mark_weak_ref(reference: WeakRef, refs: &mut BTreeSet<WeakRef>, queue: &mut VecDeque<WeakRef>) {
    if refs.insert(reference) {
        queue.push_back(reference);
    }
}

fn sweep_weak_table(table: &mut TableData, mode: WeakMode, live: &WeakLiveSet) {
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

fn table_key_is_collectable(key: TableKey) -> bool {
    weak_ref_from_key(key).is_some()
}

fn weak_value_is_collectable(value: Value) -> bool {
    weak_ref_from_value(value).is_some()
}

fn weak_ref_from_key(key: TableKey) -> Option<WeakRef> {
    match key {
        TableKey::Table(handle) => Some(WeakRef::Table(handle)),
        TableKey::Function(handle) => Some(WeakRef::Function(handle)),
        TableKey::Userdata(handle) => Some(WeakRef::Userdata(handle)),
        TableKey::Buffer(handle) => Some(WeakRef::Buffer(handle)),
        TableKey::Boolean(_) | TableKey::Number(_) | TableKey::String(_) | TableKey::Vector(_) => {
            None
        }
    }
}

fn weak_ref_from_value(value: Value) -> Option<WeakRef> {
    match value.kind() {
        ValueKind::Table => value.as_table().map(WeakRef::Table),
        ValueKind::Function => value.as_function().map(WeakRef::Function),
        ValueKind::Userdata => value.as_userdata().map(WeakRef::Userdata),
        ValueKind::Buffer => value.as_buffer().map(WeakRef::Buffer),
        ValueKind::Nil
        | ValueKind::Boolean
        | ValueKind::Number
        | ValueKind::String
        | ValueKind::Vector => None,
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
            .get(atom as usize)
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
                root.strings.get(short as usize),
                Some(InternedString::Inline(_))
            ));
            let Some(InternedString::Shared(stored)) = root.strings.get(first_long as usize) else {
                panic!("compiled long string should use shared storage");
            };
            assert_eq!(stored.0.as_bytes().as_ptr(), long_pointer);
        });
        second.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let Some(InternedString::Shared(stored)) = root.strings.get(second_long as usize)
            else {
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
            let Some(InternedString::Shared(stored)) = root.strings.get(runtime_atom as usize)
            else {
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
        assert!(!heap.tables[table as usize].hash_index.is_empty());

        assert_eq!(heap.table_clear(table), Some(()));
        let table = &heap.tables[table as usize];
        assert!(table.hash.is_empty());
        assert!(table.hash_index.is_empty());
        assert_eq!(table.hash_index.capacity(), 0);
    }
}
