use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRootSet, Gc, Rootable, Static};
use std::collections::{BTreeSet, HashMap, VecDeque};

use crate::number::table_key_number_bits;
use crate::value::{Value, ValueKind};

const MAX_LUAU_ARRAY_INDEX: usize = 2_147_483_647;
const MAX_LUAU_ARRAY_INDEX_F64: f64 = 2_147_483_647.0;
const INLINE_FUNCTION_UPVALUE_CAPACITY: usize = 4;
const TABLE_HASH_INDEX_MIN_LEN: usize = 16;
const CONCAT_INTEGER_DIRECT_CACHE_LEN: usize = 256;

#[derive(Collect)]
#[collect(no_drop)]
struct RootData<'gc> {
    _roots: DynamicRootSet<'gc>,
    strings: Vec<Gc<'gc, Static<Vec<u8>>>>,
    #[collect(require_static)]
    string_index: HashMap<Vec<u8>, u32>,
    #[collect(require_static)]
    string_string_concat_index: HashMap<(u32, u32), u32>,
    #[collect(require_static)]
    string_string_concat_direct: [Option<ConcatIntegerCacheEntry>; CONCAT_INTEGER_DIRECT_CACHE_LEN],
    #[collect(require_static)]
    string_integer_concat_index: HashMap<u64, u32>,
    #[collect(require_static)]
    integer_string_concat_index: HashMap<u64, u32>,
    #[collect(require_static)]
    string_integer_concat_direct:
        [Option<ConcatIntegerCacheEntry>; CONCAT_INTEGER_DIRECT_CACHE_LEN],
    #[collect(require_static)]
    integer_string_concat_direct:
        [Option<ConcatIntegerCacheEntry>; CONCAT_INTEGER_DIRECT_CACHE_LEN],
    tables: Vec<TableData>,
    functions: Vec<FunctionData>,
    upvalues: Vec<UpvalueData>,
    userdatas: Vec<UserdataData>,
    buffers: Vec<BufferData>,
    vectors: Vec<VectorData>,
    weak_tables_possible: bool,
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
struct TableData {
    array: Vec<Value>,
    hash: Vec<TableEntry>,
    hash_index: Vec<Option<TableHashIndexEntry>>,
    readonly: bool,
    metatable: Option<u32>,
    version: u64,
    shape_version: u64,
}

#[derive(Collect)]
#[collect(no_drop)]
struct FunctionData {
    kind: FunctionKind,
    upvalues: Vec<u32>,
    environment: u32,
}

pub struct FunctionCallMetadata {
    pub kind: FunctionKind,
    pub bytecode_context: Option<FunctionBytecodeContext>,
}

pub struct FunctionBytecodeContext {
    pub upvalues: FunctionUpvalueSnapshot,
    pub environment: u32,
}

pub enum FunctionUpvalueSnapshot {
    Inline {
        len: u8,
        values: [u32; INLINE_FUNCTION_UPVALUE_CAPACITY],
    },
    Heap(Vec<u32>),
}

impl FunctionUpvalueSnapshot {
    fn from_slice(upvalues: &[u32]) -> Self {
        if upvalues.len() <= INLINE_FUNCTION_UPVALUE_CAPACITY {
            let mut values = [0; INLINE_FUNCTION_UPVALUE_CAPACITY];
            values[..upvalues.len()].copy_from_slice(upvalues);
            return Self::Inline {
                len: upvalues.len() as u8,
                values,
            };
        }
        Self::Heap(upvalues.to_vec())
    }

    pub fn as_slice(&self) -> &[u32] {
        match self {
            Self::Inline { len, values } => &values[..usize::from(*len)],
            Self::Heap(values) => values,
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

pub(crate) enum TableConcatOutcome {
    Atom(u32),
    InvalidValue,
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

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
struct TableEntry {
    key: TableKey,
    key_value: Value,
    value: Value,
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
struct TableHashIndexEntry {
    key: TableKey,
    index: usize,
}

#[derive(Clone, Copy, Collect)]
#[collect(no_drop)]
struct ConcatIntegerCacheEntry {
    key: u64,
    atom: u32,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Collect)]
#[collect(no_drop)]
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
pub(crate) enum ArrayWriteStatus {
    Stored,
    Deferred,
    Readonly,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum HashReadStatus {
    Hit(Value),
    Deferred,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HashWriteStatus {
    Stored,
    Deferred,
    Readonly,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TableStringHashSlot {
    pub index: usize,
    pub value: Value,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TableStringHashProbe {
    pub shape_version: u64,
    pub slot: Option<TableStringHashSlot>,
    pub metatable: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TableStringHashSlotRead {
    pub value: Value,
    pub has_metatable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StringIndexWriteStatus {
    Stored { hash_index: Option<usize> },
    Deferred,
    Readonly,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TableIntegerHashSlot {
    pub index: usize,
    pub value: Value,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TableIntegerHashSlotRead {
    pub value: Value,
    pub has_metatable: bool,
}

pub(crate) enum UpvalueAddStatus {
    Stored(Value),
    Deferred(Value, Value),
}

pub(crate) enum UpvalueAddConstStatus {
    Stored(Value),
    Deferred(Value),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TableIterationOrder {
    Insertion,
    LuauHash,
}

type ArenaRoot = Rootable![Gc<'_, RefLock<RootData<'_>>>];
type VmArena = Arena<ArenaRoot>;

pub struct VmCell {
    arena: VmArena,
}

impl VmCell {
    pub fn new() -> Self {
        Self {
            arena: VmArena::new(|mc| {
                Gc::new(
                    mc,
                    RefLock::new(RootData {
                        _roots: DynamicRootSet::new(mc),
                        strings: Vec::new(),
                        string_index: HashMap::new(),
                        string_string_concat_index: HashMap::new(),
                        string_string_concat_direct: [None; CONCAT_INTEGER_DIRECT_CACHE_LEN],
                        string_integer_concat_index: HashMap::new(),
                        integer_string_concat_index: HashMap::new(),
                        string_integer_concat_direct: [None; CONCAT_INTEGER_DIRECT_CACHE_LEN],
                        integer_string_concat_direct: [None; CONCAT_INTEGER_DIRECT_CACHE_LEN],
                        tables: Vec::new(),
                        functions: Vec::new(),
                        upvalues: Vec::new(),
                        userdatas: Vec::new(),
                        buffers: Vec::new(),
                        vectors: Vec::new(),
                        weak_tables_possible: false,
                    }),
                )
            }),
        }
    }

    pub fn finish_collection_cycle(&mut self) {
        self.arena.finish_cycle();
    }

    pub fn collect_weak_tables_with_roots(&mut self, roots: &[Value]) {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if !root.weak_tables_possible {
                return;
            }
            let modes = (0..root.tables.len())
                .map(|index| table_weak_mode(&root, index))
                .collect::<Vec<_>>();
            let live = mark_weak_reachable(&root, &modes, roots);
            for (index, mode) in modes.into_iter().enumerate() {
                if let Some(mode) = mode {
                    if let Some(table) = root.tables.get_mut(index) {
                        sweep_weak_table(table, mode, &live);
                    }
                }
            }
        })
    }

    pub fn collect_debt(&mut self) {
        self.arena.collect_debt();
    }

    pub fn intern_string(&mut self, value: &str) -> u32 {
        self.intern_string_bytes(value.as_bytes())
    }

    pub fn intern_string_bytes(&mut self, value: &[u8]) -> u32 {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(atom) = root.string_index.get(value) {
                return *atom;
            }
            let atom = root.strings.len();
            root.strings.push(Gc::new(mc, Static(value.to_vec())));
            root.string_index.insert(value.to_vec(), atom as u32);
            atom as u32
        })
    }

    pub fn intern_concat_string_atoms(&mut self, lhs: u32, rhs: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let key = concat_integer_key(lhs, rhs);
            if let Some(atom) =
                concat_integer_direct_cache_get(&root.string_string_concat_direct, key)
            {
                return Some(atom);
            }
            if let Some(atom) = root.string_string_concat_index.get(&(lhs, rhs)) {
                let atom = *atom;
                concat_integer_direct_cache_insert(
                    &mut root.string_string_concat_direct,
                    key,
                    atom,
                );
                return Some(atom);
            }
            let lhs_text = root.strings.get(lhs as usize)?.0.as_slice();
            let rhs_text = root.strings.get(rhs as usize)?.0.as_slice();
            let mut value = Vec::with_capacity(lhs_text.len() + rhs_text.len());
            value.extend_from_slice(lhs_text);
            value.extend_from_slice(rhs_text);
            let atom = if let Some(atom) = root.string_index.get(value.as_slice()) {
                *atom
            } else {
                let atom = u32::try_from(root.strings.len()).ok()?;
                root.strings.push(Gc::new(mc, Static(value.clone())));
                root.string_index.insert(value, atom);
                atom
            };
            root.string_string_concat_index.insert((lhs, rhs), atom);
            concat_integer_direct_cache_insert(&mut root.string_string_concat_direct, key, atom);
            Some(atom)
        })
    }

    pub fn intern_concat_string_atom_bytes(&mut self, lhs: u32, rhs: &[u8]) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let lhs = root.strings.get(lhs as usize)?.0.as_slice();
            let mut value = Vec::with_capacity(lhs.len() + rhs.len());
            value.extend_from_slice(lhs);
            value.extend_from_slice(rhs);
            if let Some(atom) = root.string_index.get(value.as_slice()) {
                return Some(*atom);
            }
            let atom = u32::try_from(root.strings.len()).ok()?;
            root.strings.push(Gc::new(mc, Static(value.clone())));
            root.string_index.insert(value, atom);
            Some(atom)
        })
    }

    pub fn intern_concat_string_atom_small_integer(&mut self, lhs: u32, rhs: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.strings.get(lhs as usize)?;
            let key = concat_integer_key(lhs, rhs);
            if let Some(atom) =
                concat_integer_direct_cache_get(&root.string_integer_concat_direct, key)
            {
                return Some(atom);
            }
            if let Some(atom) = root.string_integer_concat_index.get(&key) {
                let atom = *atom;
                concat_integer_direct_cache_insert(
                    &mut root.string_integer_concat_direct,
                    key,
                    atom,
                );
                return Some(atom);
            }
            let lhs_text = root.strings.get(lhs as usize)?.0.as_slice();
            let (rhs_text, rhs_len) = small_positive_integer_bytes(rhs)?;
            let mut value = Vec::with_capacity(lhs_text.len() + rhs_len);
            value.extend_from_slice(lhs_text);
            value.extend_from_slice(&rhs_text[..rhs_len]);
            let atom = if let Some(atom) = root.string_index.get(value.as_slice()) {
                *atom
            } else {
                let atom = u32::try_from(root.strings.len()).ok()?;
                root.strings.push(Gc::new(mc, Static(value.clone())));
                root.string_index.insert(value, atom);
                atom
            };
            root.string_integer_concat_index.insert(key, atom);
            concat_integer_direct_cache_insert(&mut root.string_integer_concat_direct, key, atom);
            Some(atom)
        })
    }

    pub fn intern_concat_bytes_string_atom(&mut self, lhs: &[u8], rhs: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let rhs = root.strings.get(rhs as usize)?.0.as_slice();
            let mut value = Vec::with_capacity(lhs.len() + rhs.len());
            value.extend_from_slice(lhs);
            value.extend_from_slice(rhs);
            if let Some(atom) = root.string_index.get(value.as_slice()) {
                return Some(*atom);
            }
            let atom = u32::try_from(root.strings.len()).ok()?;
            root.strings.push(Gc::new(mc, Static(value.clone())));
            root.string_index.insert(value, atom);
            Some(atom)
        })
    }

    pub fn intern_concat_small_integer_string_atom(&mut self, lhs: u32, rhs: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.strings.get(rhs as usize)?;
            let key = concat_integer_key(lhs, rhs);
            if let Some(atom) =
                concat_integer_direct_cache_get(&root.integer_string_concat_direct, key)
            {
                return Some(atom);
            }
            if let Some(atom) = root.integer_string_concat_index.get(&key) {
                let atom = *atom;
                concat_integer_direct_cache_insert(
                    &mut root.integer_string_concat_direct,
                    key,
                    atom,
                );
                return Some(atom);
            }
            let rhs_text = root.strings.get(rhs as usize)?.0.as_slice();
            let (lhs_text, lhs_len) = small_positive_integer_bytes(lhs)?;
            let mut value = Vec::with_capacity(lhs_len + rhs_text.len());
            value.extend_from_slice(&lhs_text[..lhs_len]);
            value.extend_from_slice(rhs_text);
            let atom = if let Some(atom) = root.string_index.get(value.as_slice()) {
                *atom
            } else {
                let atom = u32::try_from(root.strings.len()).ok()?;
                root.strings.push(Gc::new(mc, Static(value.clone())));
                root.string_index.insert(value, atom);
                atom
            };
            root.integer_string_concat_index.insert(key, atom);
            concat_integer_direct_cache_insert(&mut root.integer_string_concat_direct, key, atom);
            Some(atom)
        })
    }

    pub fn string(&mut self, atom: u32) -> Option<String> {
        self.string_bytes(atom)
            .and_then(|bytes| String::from_utf8(bytes).ok())
    }

    pub fn string_lossy(&mut self, atom: u32) -> Option<String> {
        self.string_bytes(atom)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    pub fn string_bytes(&mut self, atom: u32) -> Option<Vec<u8>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .strings
                .get(atom as usize)
                .map(|value| value.0.clone())
        })
    }

    pub fn append_string_atom_bytes(&mut self, atom: u32, output: &mut Vec<u8>) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let value = root.strings.get(atom as usize)?;
            output.extend_from_slice(&value.0);
            Some(())
        })
    }

    pub fn string_atom_byte_len(&mut self, atom: u32) -> Option<usize> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .strings
                .get(atom as usize)
                .map(|value| value.0.len())
        })
    }

    pub(crate) fn string_atom_byte_at(&mut self, atom: u32, index: usize) -> Option<Option<u8>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .strings
                .get(atom as usize)
                .map(|value| value.0.get(index).copied())
        })
    }

    pub fn intern_string_atom_slice(&mut self, atom: u32, start: usize, end: usize) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let value = {
                let source = root.strings.get(atom as usize)?.0.as_slice();
                let bytes = source.get(start..end)?;
                if let Some(atom) = root.string_index.get(bytes) {
                    return Some(*atom);
                }
                bytes.to_vec()
            };
            let atom = u32::try_from(root.strings.len()).ok()?;
            root.strings.push(Gc::new(mc, Static(value.clone())));
            root.string_index.insert(value, atom);
            Some(atom)
        })
    }

    pub fn alloc_array_table(&mut self, values: &[Value]) -> u32 {
        self.alloc_array_table_with_capacity(values, values.len(), 0)
    }

    pub(crate) fn alloc_array_table_with_capacity(
        &mut self,
        values: &[Value],
        array_capacity_hint: usize,
        hash_capacity_hint: usize,
    ) -> u32 {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = root.tables.len();
            let mut array = Vec::with_capacity(values.len().max(array_capacity_hint));
            array.extend_from_slice(values);
            root.tables.push(TableData {
                array,
                hash: Vec::with_capacity(hash_capacity_hint),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                version: 0,
                shape_version: 0,
            });
            handle as u32
        })
    }

    pub(crate) fn alloc_record_table(&mut self, fields: &[Value]) -> Option<u32> {
        if fields.len() % 2 != 0 {
            return None;
        }
        let field_count = fields.len() / 2;
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let mut table = TableData {
                array: Vec::new(),
                hash: Vec::with_capacity(field_count),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                version: 0,
                shape_version: 0,
            };
            for pair in fields.chunks_exact(2) {
                let atom = pair[0].as_string_atom()?;
                let value = pair[1];
                root.strings.get(atom as usize)?;
                if string_atom_is(&root, atom, b"__mode") {
                    root.weak_tables_possible = true;
                }
                set_hash_key(
                    &mut table,
                    TableKey::String(atom),
                    Value::string_atom(atom),
                    value,
                );
            }
            let handle = u32::try_from(root.tables.len()).ok()?;
            root.tables.push(table);
            Some(handle)
        })
    }

    pub(crate) fn alloc_record_table_with_string_atoms(
        &mut self,
        keys: &[u32],
        values: &[Value],
    ) -> Option<u32> {
        if keys.len() != values.len() {
            return None;
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let mut table = TableData {
                array: Vec::new(),
                hash: Vec::with_capacity(keys.len()),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                version: 0,
                shape_version: 0,
            };
            for (atom, value) in keys.iter().copied().zip(values.iter().copied()) {
                root.strings.get(atom as usize)?;
                if string_atom_is(&root, atom, b"__mode") {
                    root.weak_tables_possible = true;
                }
                set_hash_key(
                    &mut table,
                    TableKey::String(atom),
                    Value::string_atom(atom),
                    value,
                );
            }
            let handle = u32::try_from(root.tables.len()).ok()?;
            root.tables.push(table);
            Some(handle)
        })
    }

    pub(crate) fn alloc_unique_record_table_with_string_atoms(
        &mut self,
        keys: &[u32],
        values: &[Value],
    ) -> Option<u32> {
        if keys.len() != values.len() {
            return None;
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let mut table = TableData {
                array: Vec::new(),
                hash: Vec::with_capacity(keys.len()),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                version: 0,
                shape_version: 0,
            };
            for (atom, value) in keys.iter().copied().zip(values.iter().copied()) {
                root.strings.get(atom as usize)?;
                if string_atom_is(&root, atom, b"__mode") {
                    root.weak_tables_possible = true;
                }
                if value.kind() != ValueKind::Nil {
                    table.hash.push(TableEntry {
                        key: TableKey::String(atom),
                        key_value: Value::string_atom(atom),
                        value,
                    });
                }
            }
            table.shape_version = table.hash.len() as u64;
            if table.hash.len() >= TABLE_HASH_INDEX_MIN_LEN {
                rebuild_hash_index(&mut table);
            }
            let handle = u32::try_from(root.tables.len()).ok()?;
            root.tables.push(table);
            Some(handle)
        })
    }

    pub fn alloc_filled_array_table(&mut self, size: usize, value: Value) -> u32 {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = root.tables.len();
            root.tables.push(TableData {
                array: vec![value; size],
                hash: Vec::new(),
                hash_index: Vec::new(),
                readonly: false,
                metatable: None,
                version: 0,
                shape_version: 0,
            });
            handle as u32
        })
    }

    pub fn alloc_function(
        &mut self,
        program: Option<u32>,
        prototype: u32,
        upvalues: Vec<u32>,
        environment: u32,
    ) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.functions.len()).ok()?;
            root.functions.push(FunctionData {
                kind: FunctionKind::Bytecode { program, prototype },
                upvalues,
                environment,
            });
            Some(handle)
        })
    }

    pub fn alloc_dynamic_main_function(&mut self, program: u32, environment: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.functions.len()).ok()?;
            root.functions.push(FunctionData {
                kind: FunctionKind::DynamicMain { program },
                upvalues: Vec::new(),
                environment,
            });
            Some(handle)
        })
    }

    pub fn alloc_native_function(
        &mut self,
        native: NativeFunction,
        environment: u32,
    ) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.functions.len()).ok()?;
            root.functions.push(FunctionData {
                kind: FunctionKind::Native(native),
                upvalues: Vec::new(),
                environment,
            });
            Some(handle)
        })
    }

    pub fn alloc_host_function(&mut self, host: u32, environment: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            root.tables.get(environment as usize)?;
            let handle = u32::try_from(root.functions.len()).ok()?;
            root.functions.push(FunctionData {
                kind: FunctionKind::Host(host),
                upvalues: Vec::new(),
                environment,
            });
            Some(handle)
        })
    }

    pub fn alloc_native_function_with_upvalues(
        &mut self,
        native: NativeFunction,
        upvalues: Vec<u32>,
        environment: u32,
    ) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            for upvalue in &upvalues {
                root.upvalues.get(*upvalue as usize)?;
            }
            let handle = u32::try_from(root.functions.len()).ok()?;
            root.functions.push(FunctionData {
                kind: FunctionKind::Native(native),
                upvalues,
                environment,
            });
            Some(handle)
        })
    }

    pub fn function_kind(&mut self, handle: u32) -> Option<FunctionKind> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .functions
                .get(handle as usize)
                .map(|function| function.kind)
        })
    }

    pub fn function_upvalues(&mut self, handle: u32) -> Option<Vec<u32>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .functions
                .get(handle as usize)
                .map(|function| function.upvalues.clone())
        })
    }

    pub fn function_call_metadata(&mut self, handle: u32) -> Option<FunctionCallMetadata> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let function = root.functions.get(handle as usize)?;
            let bytecode_context = match function.kind {
                FunctionKind::Bytecode { .. } | FunctionKind::DynamicMain { .. } => {
                    Some(FunctionBytecodeContext {
                        upvalues: FunctionUpvalueSnapshot::from_slice(&function.upvalues),
                        environment: function.environment,
                    })
                }
                FunctionKind::Host(_) | FunctionKind::Native(_) => None,
            };
            Some(FunctionCallMetadata {
                kind: function.kind,
                bytecode_context,
            })
        })
    }

    pub fn function_environment(&mut self, handle: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .functions
                .get(handle as usize)
                .map(|function| function.environment)
        })
    }

    pub fn set_function_environment(&mut self, handle: u32, environment: u32) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let function = root.functions.get_mut(handle as usize)?;
            function.environment = environment;
            Some(())
        })
    }

    pub fn alloc_upvalue(&mut self, value: Value) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = u32::try_from(root.upvalues.len()).ok()?;
            root.upvalues.push(UpvalueData { value });
            Some(handle)
        })
    }

    pub fn upvalue_get(&mut self, handle: u32) -> Option<Value> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .upvalues
                .get(handle as usize)
                .map(|upvalue| upvalue.value)
        })
    }

    pub fn upvalue_get_pair(&mut self, first: u32, second: u32) -> Option<(Value, Value)> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let first = root.upvalues.get(first as usize)?.value;
            let second = root.upvalues.get(second as usize)?.value;
            Some((first, second))
        })
    }

    pub(crate) fn upvalue_add_number_pair(
        &mut self,
        target: u32,
        rhs: u32,
    ) -> Option<UpvalueAddStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let lhs = root.upvalues.get(target as usize)?.value;
            let rhs_value = root.upvalues.get(rhs as usize)?.value;
            let (Some(lhs_number), Some(rhs_number)) = (lhs.as_number(), rhs_value.as_number())
            else {
                return Some(UpvalueAddStatus::Deferred(lhs, rhs_value));
            };
            let result = Value::number(lhs_number + rhs_number);
            root.upvalues.get_mut(target as usize)?.value = result;
            Some(UpvalueAddStatus::Stored(result))
        })
    }

    pub(crate) fn upvalue_add_number_const(
        &mut self,
        target: u32,
        rhs: f64,
    ) -> Option<UpvalueAddConstStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let lhs = root.upvalues.get(target as usize)?.value;
            let Some(lhs_number) = lhs.as_number() else {
                return Some(UpvalueAddConstStatus::Deferred(lhs));
            };
            let result = Value::number(lhs_number + rhs);
            root.upvalues.get_mut(target as usize)?.value = result;
            Some(UpvalueAddConstStatus::Stored(result))
        })
    }

    pub fn upvalue_set(&mut self, handle: u32, value: Value) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let upvalue = root.upvalues.get_mut(handle as usize)?;
            upvalue.value = value;
            Some(())
        })
    }

    pub fn alloc_userdata(&mut self, metatable: Option<u32>) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(table) = metatable {
                root.tables.get(table as usize)?;
            }
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
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(table) = metatable {
                root.tables.get(table as usize)?;
            }
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

    pub fn table_get(&mut self, table: u32, key: Value) -> Option<Value> {
        self.table_slot(table, key).flatten()
    }

    pub(crate) fn table_slot(&mut self, table: u32, key: Value) -> Option<Option<Value>> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if let Some(atom) = key.as_string_atom() {
                return Some(
                    table_hash_entry(table, TableKey::String(atom)).map(|entry| entry.value),
                );
            }
            let Some(table_key) = table_key_from_value(&root, key) else {
                return Some(None);
            };
            if let Some(index) = table_key_array_index(table_key) {
                if let Some(value) = table.array.get(index - 1) {
                    return Some(Some(*value));
                }
            }
            Some(table_hash_entry(table, table_key).map(|entry| entry.value))
        })
    }

    pub fn table_set(&mut self, table: u32, key: Value, value: Value) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(atom) = key.as_string_atom() {
                if string_atom_is(&root, atom, b"__mode") {
                    root.weak_tables_possible = true;
                }
                let table = root.tables.get_mut(table as usize)?;
                if table.readonly {
                    return None;
                }
                bump_table_version(table);
                set_hash_key(table, TableKey::String(atom), key, value);
                return Some(());
            }
            let table_key = table_key_from_value(&root, key)?;
            let key_value = table_key_visible_value(&mut root, table_key, key)?;
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return None;
            }
            bump_table_version(table);
            if value.kind() == ValueKind::Nil {
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
                if index == table.array.len() + 1 {
                    table.array.push(value);
                    remove_hash_key(table, table_key);
                    return Some(());
                }
                if index <= table.array.len() {
                    table.array[index - 1] = value;
                    remove_hash_key(table, table_key);
                    return Some(());
                }
            }

            set_hash_key(table, table_key, key_value, value);
            Some(())
        })
    }

    pub(crate) fn table_set_absent(&mut self, table: u32, key: Value, value: Value) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(atom) = key.as_string_atom() {
                if string_atom_is(&root, atom, b"__mode") {
                    root.weak_tables_possible = true;
                }
                let table = root.tables.get_mut(table as usize)?;
                if table.readonly {
                    return None;
                }
                bump_table_version(table);
                if value.kind() != ValueKind::Nil {
                    insert_hash_key_absent(table, TableKey::String(atom), key, value);
                }
                return Some(());
            }
            let table_key = table_key_from_value(&root, key)?;
            let key_value = table_key_visible_value(&mut root, table_key, key)?;
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return None;
            }
            bump_table_version(table);
            if value.kind() == ValueKind::Nil {
                return Some(());
            }
            if let Some(index) = table_key_array_index(table_key) {
                if index == table.array.len() + 1 {
                    table.array.push(value);
                    return Some(());
                }
            }
            insert_hash_key_absent(table, table_key, key_value, value);
            Some(())
        })
    }

    pub(crate) fn write_string_index_without_metamethod(
        &mut self,
        table: u32,
        atom: u32,
        value: Value,
    ) -> Option<StringIndexWriteStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if string_atom_is(&root, atom, b"__mode") {
                root.weak_tables_possible = true;
            }
            let table = root.tables.get_mut(table as usize)?;
            let has_metatable = table.metatable.is_some();
            let key = TableKey::String(atom);
            if let Some(index) = table_hash_entry_index(table, key) {
                let Some(entry) = table.hash.get_mut(index) else {
                    return Some(StringIndexWriteStatus::Deferred);
                };
                if entry.value.kind() == ValueKind::Nil && has_metatable {
                    return Some(StringIndexWriteStatus::Deferred);
                }
                if table.readonly {
                    return Some(StringIndexWriteStatus::Readonly);
                }
                entry.value = value;
                bump_table_version(table);
                return Some(StringIndexWriteStatus::Stored {
                    hash_index: Some(index),
                });
            }
            if has_metatable {
                return Some(StringIndexWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(StringIndexWriteStatus::Readonly);
            }
            bump_table_version(table);
            if value.kind() != ValueKind::Nil {
                let index = table.hash.len();
                insert_hash_key_absent(table, key, Value::string_atom(atom), value);
                return Some(StringIndexWriteStatus::Stored {
                    hash_index: Some(index),
                });
            }
            Some(StringIndexWriteStatus::Stored { hash_index: None })
        })
    }

    pub(crate) fn write_array_slot(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Option<ArrayWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(ArrayWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return Some(ArrayWriteStatus::Readonly);
            }
            bump_table_version(table);
            if one_based_index == table.array.len().saturating_add(1) {
                table.array.push(value);
            } else if one_based_index > table.array.len() {
                table.array.resize(one_based_index, Value::nil());
                table.array[one_based_index - 1] = value;
            } else {
                table.array[one_based_index - 1] = value;
            }
            if !table.hash.is_empty() {
                let key = number_table_key(one_based_index as f64)?;
                remove_hash_key(table, key);
            }
            Some(ArrayWriteStatus::Stored)
        })
    }

    pub(crate) fn write_integer_array_index_without_metamethod(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Option<ArrayWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(ArrayWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            let array_len = table.array.len();
            if one_based_index > array_len.saturating_add(1) {
                return Some(ArrayWriteStatus::Deferred);
            }
            if one_based_index > array_len && value.kind() == ValueKind::Nil {
                return Some(ArrayWriteStatus::Deferred);
            }
            if table.metatable.is_some() {
                let existing = table.array.get(one_based_index - 1).copied();
                if !existing.is_some_and(|value| value.kind() != ValueKind::Nil) {
                    return Some(ArrayWriteStatus::Deferred);
                }
            }
            if table.readonly {
                return Some(ArrayWriteStatus::Readonly);
            }
            bump_table_version(table);
            if one_based_index == table.array.len().saturating_add(1) {
                table.array.push(value);
            } else {
                table.array[one_based_index - 1] = value;
            }
            if !table.hash.is_empty() {
                let key = number_table_key(one_based_index as f64)?;
                remove_hash_key(table, key);
            }
            Some(ArrayWriteStatus::Stored)
        })
    }

    pub(crate) fn write_integer_index_without_metamethod(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Option<ArrayWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(ArrayWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            let array_len = table.array.len();
            if one_based_index <= array_len.saturating_add(1) {
                if one_based_index > array_len && value.kind() == ValueKind::Nil {
                    return Some(ArrayWriteStatus::Deferred);
                }
                if table.metatable.is_some() {
                    let existing = table.array.get(one_based_index - 1).copied();
                    if !existing.is_some_and(|value| value.kind() != ValueKind::Nil) {
                        return Some(ArrayWriteStatus::Deferred);
                    }
                }
                if table.readonly {
                    return Some(ArrayWriteStatus::Readonly);
                }
                bump_table_version(table);
                if one_based_index == table.array.len().saturating_add(1) {
                    table.array.push(value);
                } else {
                    table.array[one_based_index - 1] = value;
                }
                if !table.hash.is_empty() {
                    let key = number_table_key(one_based_index as f64)?;
                    remove_hash_key(table, key);
                }
                return Some(ArrayWriteStatus::Stored);
            }

            if table.metatable.is_some() {
                return Some(ArrayWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(ArrayWriteStatus::Readonly);
            }
            bump_table_version(table);
            let key = number_table_key(one_based_index as f64)?;
            set_hash_key(table, key, Value::number(one_based_index as f64), value);
            Some(ArrayWriteStatus::Stored)
        })
    }

    pub(crate) fn read_integer_hash_key_without_metamethod(
        &mut self,
        table: u32,
        one_based_index: usize,
    ) -> Option<HashReadStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(HashReadStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if one_based_index <= table.array.len() || table.metatable.is_some() {
                return Some(HashReadStatus::Deferred);
            }
            let key = number_table_key(one_based_index as f64)?;
            let value = table_hash_entry(table, key).map_or_else(Value::nil, |entry| entry.value);
            Some(HashReadStatus::Hit(value))
        })
    }

    pub(crate) fn table_integer_hash_slot(
        &mut self,
        table: u32,
        one_based_index: usize,
    ) -> Option<Option<TableIntegerHashSlot>> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(None);
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if one_based_index <= table.array.len() {
                return Some(None);
            }
            let key = number_table_key(one_based_index as f64)?;
            let Some(index) = table_hash_entry_index(table, key) else {
                return Some(None);
            };
            let Some(entry) = table.hash.get(index) else {
                return Some(None);
            };
            Some(Some(TableIntegerHashSlot {
                index,
                value: entry.value,
            }))
        })
    }

    pub(crate) fn read_integer_hash_slot_at_index(
        &mut self,
        table: u32,
        one_based_index: usize,
        index: usize,
    ) -> Option<Option<TableIntegerHashSlotRead>> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(None);
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if one_based_index <= table.array.len() {
                return Some(None);
            }
            let Some(entry) = table.hash.get(index) else {
                return Some(None);
            };
            let key = number_table_key(one_based_index as f64)?;
            if entry.key != key {
                return Some(None);
            }
            Some(Some(TableIntegerHashSlotRead {
                value: entry.value,
                has_metatable: table.metatable.is_some(),
            }))
        })
    }

    pub(crate) fn read_integer_shape_hash_slot_at_index(
        &mut self,
        table: u32,
        one_based_index: usize,
        shape_version: u64,
        index: usize,
    ) -> Option<Option<TableIntegerHashSlotRead>> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(None);
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if one_based_index <= table.array.len() || table.shape_version != shape_version {
                return Some(None);
            }
            let Some(entry) = table.hash.get(index) else {
                return Some(None);
            };
            Some(Some(TableIntegerHashSlotRead {
                value: entry.value,
                has_metatable: table.metatable.is_some(),
            }))
        })
    }

    pub(crate) fn write_integer_hash_slot_at_index(
        &mut self,
        table: u32,
        one_based_index: usize,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(HashWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if one_based_index <= table.array.len().saturating_add(1) {
                return Some(HashWriteStatus::Deferred);
            }
            let has_metatable = table.metatable.is_some();
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            let key = number_table_key(one_based_index as f64)?;
            if entry.key != key {
                return Some(HashWriteStatus::Deferred);
            }
            if entry.value.kind() == ValueKind::Nil && has_metatable {
                return Some(HashWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn write_integer_shape_hash_slot_at_index(
        &mut self,
        table: u32,
        one_based_index: usize,
        shape_version: u64,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(HashWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if one_based_index <= table.array.len().saturating_add(1)
                || table.shape_version != shape_version
            {
                return Some(HashWriteStatus::Deferred);
            }
            let has_metatable = table.metatable.is_some();
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            if entry.value.kind() == ValueKind::Nil && has_metatable {
                return Some(HashWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn write_integer_hash_slot_at_index_raw(
        &mut self,
        table: u32,
        one_based_index: usize,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(HashWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if one_based_index <= table.array.len().saturating_add(1) {
                return Some(HashWriteStatus::Deferred);
            }
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            let key = number_table_key(one_based_index as f64)?;
            if entry.key != key {
                return Some(HashWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn write_integer_shape_hash_slot_at_index_raw(
        &mut self,
        table: u32,
        one_based_index: usize,
        shape_version: u64,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(HashWriteStatus::Deferred);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if one_based_index <= table.array.len().saturating_add(1)
                || table.shape_version != shape_version
            {
                return Some(HashWriteStatus::Deferred);
            }
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn table_string_hash_slot(
        &mut self,
        table: u32,
        atom: u32,
    ) -> Option<Option<TableStringHashSlot>> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            let key = TableKey::String(atom);
            let Some(index) = table_hash_entry_index(table, key) else {
                return Some(None);
            };
            let Some(entry) = table.hash.get(index) else {
                return Some(None);
            };
            Some(Some(TableStringHashSlot {
                index,
                value: entry.value,
            }))
        })
    }

    pub(crate) fn table_string_hash_probe(
        &mut self,
        table: u32,
        atom: u32,
    ) -> Option<TableStringHashProbe> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            let key = TableKey::String(atom);
            let slot = table_hash_entry_index(table, key).and_then(|index| {
                table.hash.get(index).map(|entry| TableStringHashSlot {
                    index,
                    value: entry.value,
                })
            });
            Some(TableStringHashProbe {
                shape_version: table.shape_version,
                slot,
                metatable: table.metatable,
            })
        })
    }

    pub(crate) fn read_string_hash_slot_at_index(
        &mut self,
        table: u32,
        atom: u32,
        index: usize,
    ) -> Option<Option<TableStringHashSlotRead>> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            let Some(entry) = table.hash.get(index) else {
                return Some(None);
            };
            if entry.key != TableKey::String(atom) {
                return Some(None);
            }
            Some(Some(TableStringHashSlotRead {
                value: entry.value,
                has_metatable: table.metatable.is_some(),
            }))
        })
    }

    pub(crate) fn read_string_shape_hash_slot_at_index(
        &mut self,
        table: u32,
        shape_version: u64,
        index: usize,
    ) -> Option<Option<TableStringHashSlotRead>> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if table.shape_version != shape_version {
                return Some(None);
            }
            let Some(entry) = table.hash.get(index) else {
                return Some(None);
            };
            Some(Some(TableStringHashSlotRead {
                value: entry.value,
                has_metatable: table.metatable.is_some(),
            }))
        })
    }

    pub(crate) fn write_string_hash_slot_at_index(
        &mut self,
        table: u32,
        atom: u32,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if string_atom_is(&root, atom, b"__mode") {
                root.weak_tables_possible = true;
            }
            let table = root.tables.get_mut(table as usize)?;
            let has_metatable = table.metatable.is_some();
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            if entry.key != TableKey::String(atom) {
                return Some(HashWriteStatus::Deferred);
            }
            if entry.value.kind() == ValueKind::Nil && has_metatable {
                return Some(HashWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn write_string_shape_hash_slot_at_index(
        &mut self,
        table: u32,
        atom: u32,
        shape_version: u64,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if string_atom_is(&root, atom, b"__mode") {
                root.weak_tables_possible = true;
            }
            let table = root.tables.get_mut(table as usize)?;
            if table.shape_version != shape_version {
                return Some(HashWriteStatus::Deferred);
            }
            let has_metatable = table.metatable.is_some();
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            if entry.value.kind() == ValueKind::Nil && has_metatable {
                return Some(HashWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn write_string_hash_slot_at_index_raw(
        &mut self,
        table: u32,
        atom: u32,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if string_atom_is(&root, atom, b"__mode") {
                root.weak_tables_possible = true;
            }
            let table = root.tables.get_mut(table as usize)?;
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            if entry.key != TableKey::String(atom) {
                return Some(HashWriteStatus::Deferred);
            }
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub(crate) fn write_string_shape_hash_slot_at_index_raw(
        &mut self,
        table: u32,
        atom: u32,
        shape_version: u64,
        index: usize,
        value: Value,
    ) -> Option<HashWriteStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if string_atom_is(&root, atom, b"__mode") {
                root.weak_tables_possible = true;
            }
            let table = root.tables.get_mut(table as usize)?;
            if table.shape_version != shape_version {
                return Some(HashWriteStatus::Deferred);
            }
            let Some(entry) = table.hash.get_mut(index) else {
                return Some(HashWriteStatus::Deferred);
            };
            if table.readonly {
                return Some(HashWriteStatus::Readonly);
            }
            entry.value = value;
            bump_table_version(table);
            Some(HashWriteStatus::Stored)
        })
    }

    pub fn table_sort_numeric_array(&mut self, table: u32, len: usize) -> Option<bool> {
        if len < 2 {
            return Some(true);
        }
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly || table.array.len() < len {
                return None;
            }
            if table
                .hash
                .iter()
                .any(|entry| table_key_array_index(entry.key).is_some_and(|index| index <= len))
            {
                return Some(false);
            }
            let mut numbers = Vec::with_capacity(len);
            for value in table.array.iter().take(len) {
                let Some(number) = value.as_number().filter(|number| number.is_finite()) else {
                    return Some(false);
                };
                numbers.push(number);
            }
            bump_table_version(table);
            numbers.sort_by(|left, right| {
                left.partial_cmp(right)
                    .expect("numeric table sort validated finite values")
            });
            for (slot, number) in table.array[..len].iter_mut().zip(numbers) {
                *slot = Value::number(number);
            }
            Some(true)
        })
    }

    pub(crate) fn table_array_range_values(
        &mut self,
        table: u32,
        start: usize,
        end: usize,
    ) -> Option<Option<Vec<Value>>> {
        if start == 0 || end < start {
            return Some(None);
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            if end > table.array.len() {
                return Some(None);
            }
            if table.hash.iter().any(|entry| {
                table_key_array_index(entry.key).is_some_and(|index| (start..=end).contains(&index))
            }) {
                return Some(None);
            }
            Some(Some(table.array[start - 1..end].to_vec()))
        })
    }

    pub(crate) fn write_array_prefix(
        &mut self,
        table: u32,
        values: &[Value],
    ) -> Option<ArrayWriteStatus> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return Some(ArrayWriteStatus::Readonly);
            }
            if table.array.len() < values.len() {
                return Some(ArrayWriteStatus::Deferred);
            }
            if !values.is_empty() {
                bump_table_version(table);
                table.array[..values.len()].copy_from_slice(values);
            }
            if !table.hash.is_empty() {
                let keys = table
                    .hash
                    .iter()
                    .filter_map(|entry| {
                        table_key_array_index(entry.key)
                            .is_some_and(|index| index <= values.len())
                            .then_some(entry.key)
                    })
                    .collect::<Vec<_>>();
                for key in keys {
                    remove_hash_key(table, key);
                }
            }
            Some(ArrayWriteStatus::Stored)
        })
    }

    pub(crate) fn table_move_integer_range(
        &mut self,
        source: u32,
        start: usize,
        destination: u32,
        destination_start: usize,
        count: usize,
    ) -> Option<ArrayWriteStatus> {
        if start == 0 || destination_start == 0 {
            return Some(ArrayWriteStatus::Deferred);
        }
        if count == 0 {
            return Some(ArrayWriteStatus::Stored);
        }
        let source_end = start.checked_add(count - 1)?;
        if source_end > MAX_LUAU_ARRAY_INDEX {
            return Some(ArrayWriteStatus::Deferred);
        }
        let destination_end = destination_start.checked_add(count - 1)?;
        if destination_end > MAX_LUAU_ARRAY_INDEX {
            return Some(ArrayWriteStatus::Deferred);
        }

        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let values = {
                let source = root.tables.get(source as usize)?;
                (start..=source_end)
                    .map(|index| table_numeric_get(source, index))
                    .collect::<Vec<_>>()
            };
            let destination = root.tables.get_mut(destination as usize)?;
            if destination.readonly {
                return Some(ArrayWriteStatus::Readonly);
            }
            bump_table_version(destination);
            for (offset, value) in values.into_iter().enumerate() {
                table_numeric_set_array_index(destination, destination_start + offset, value);
            }
            Some(ArrayWriteStatus::Stored)
        })
    }

    pub fn table_metatable(&mut self, table: u32) -> Option<Option<u32>> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .tables
                .get(table as usize)
                .map(|table| table.metatable)
        })
    }

    pub fn table_set_metatable(&mut self, table: u32, metatable: Option<u32>) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            if let Some(metatable) = metatable {
                root.tables.get(metatable as usize)?;
            }
            let table = root.tables.get_mut(table as usize)?;
            bump_table_version(table);
            bump_table_shape_version(table);
            table.metatable = metatable;
            Some(())
        })
    }

    pub fn table_clone(&mut self, table: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let mut clone = root.tables.get(table as usize)?.clone();
            clone.readonly = false;
            clone.version = 0;
            clone.shape_version = 0;
            let handle = u32::try_from(root.tables.len()).ok()?;
            root.tables.push(clone);
            Some(handle)
        })
    }

    pub fn array_get(&mut self, table: u32, one_based_index: usize) -> Option<Value> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return self.table_get(table, Value::number(one_based_index as f64));
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            Some(table_numeric_get(table, one_based_index))
        })
    }

    pub(crate) fn intern_table_concat_range<F>(
        &mut self,
        table: u32,
        start: usize,
        end: usize,
        separator: &[u8],
        mut append_number: F,
    ) -> Option<TableConcatOutcome>
    where
        F: FnMut(f64, &mut Vec<u8>),
    {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc);
            let output = {
                let root = root.borrow();
                let table = root.tables.get(table as usize)?;
                let count = end.saturating_sub(start).saturating_add(1);
                let capacity = count
                    .min(4096)
                    .saturating_mul(separator.len().saturating_add(16));
                let mut output = Vec::with_capacity(capacity);
                if start > 0 && end <= table.array.len() {
                    let values = &table.array[start - 1..end];
                    if separator.is_empty() {
                        for value in values.iter().copied() {
                            match value.kind() {
                                ValueKind::String => {
                                    let atom = value.as_string_atom()?;
                                    let bytes = root.strings.get(atom as usize)?;
                                    output.extend_from_slice(&bytes.0);
                                }
                                ValueKind::Number => {
                                    let number = value.as_number()?;
                                    append_number(number, &mut output);
                                }
                                _ => return Some(TableConcatOutcome::InvalidValue),
                            }
                        }
                    } else {
                        for (offset, value) in values.iter().copied().enumerate() {
                            if offset > 0 {
                                output.extend_from_slice(separator);
                            }
                            match value.kind() {
                                ValueKind::String => {
                                    let atom = value.as_string_atom()?;
                                    let bytes = root.strings.get(atom as usize)?;
                                    output.extend_from_slice(&bytes.0);
                                }
                                ValueKind::Number => {
                                    let number = value.as_number()?;
                                    append_number(number, &mut output);
                                }
                                _ => return Some(TableConcatOutcome::InvalidValue),
                            }
                        }
                    }
                } else {
                    for (offset, index) in (start..=end).enumerate() {
                        if offset > 0 {
                            output.extend_from_slice(separator);
                        }
                        let value = if index == 0 || index > MAX_LUAU_ARRAY_INDEX {
                            let key = number_table_key(index as f64)?;
                            table_hash_entry(table, key)
                                .map_or_else(Value::nil, |entry| entry.value)
                        } else {
                            table_numeric_get(table, index)
                        };
                        match value.kind() {
                            ValueKind::String => {
                                let atom = value.as_string_atom()?;
                                let bytes = root.strings.get(atom as usize)?;
                                output.extend_from_slice(&bytes.0);
                            }
                            ValueKind::Number => {
                                let number = value.as_number()?;
                                append_number(number, &mut output);
                            }
                            _ => return Some(TableConcatOutcome::InvalidValue),
                        }
                    }
                }
                output
            };
            let mut root = root.borrow_mut();
            if let Some(atom) = root.string_index.get(output.as_slice()) {
                return Some(TableConcatOutcome::Atom(*atom));
            }
            let atom = u32::try_from(root.strings.len()).ok()?;
            root.strings.push(Gc::new(mc, Static(output.clone())));
            root.string_index.insert(output, atom);
            Some(TableConcatOutcome::Atom(atom))
        })
    }

    pub fn array_len(&mut self, table: u32) -> Option<usize> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .tables
                .get(table as usize)
                .map(table_array_boundary_len)
        })
    }

    pub fn array_slot(&mut self, table: u32, one_based_index: usize) -> Option<Option<Value>> {
        if one_based_index == 0 || one_based_index > MAX_LUAU_ARRAY_INDEX {
            return Some(None);
        }
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            Some(table.array.get(one_based_index - 1).copied())
        })
    }

    pub fn table_storage_len(&mut self, table: u32) -> Option<usize> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            Some(table.array.len().saturating_add(table.hash.len()))
        })
    }

    pub fn table_clear(&mut self, table: u32) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return None;
            }
            bump_table_version(table);
            bump_table_shape_version(table);
            table.array.fill(Value::nil());
            table.hash.clear();
            table.hash_index.clear();
            Some(())
        })
    }

    pub fn table_insert(&mut self, table: u32, position: i32, value: Value) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return None;
            }
            bump_table_version(table);
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
        })
    }

    pub fn table_remove(&mut self, table: u32, position: i32) -> Option<Option<Value>> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return None;
            }
            let len = table_array_boundary_len(table);
            if !(1..=i32::try_from(len).ok()?).contains(&position) {
                return Some(None);
            }
            bump_table_version(table);
            let position = position as usize;
            let removed = table_numeric_get(table, position);
            for index in position..len {
                let value = table_numeric_get(table, index + 1);
                table_numeric_set_array_index(table, index, value);
            }
            table_numeric_set_array_index(table, len, Value::nil());
            Some(Some(removed))
        })
    }

    pub fn max_numeric_key(&mut self, table: u32) -> Option<f64> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
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
        })
    }

    pub fn table_next_after(
        &mut self,
        table: u32,
        after: Value,
        order: TableIterationOrder,
    ) -> Option<TableNext> {
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            match after.kind() {
                ValueKind::Nil => Some(
                    first_non_nil_table_entry(&root, table, order)
                        .map_or(TableNext::End, |(key, value)| TableNext::Entry(key, value)),
                ),
                ValueKind::Number => {
                    let Some(key) = table_key_from_value(&root, after) else {
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
                    let Some(key) = table_key_from_value(&root, after) else {
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
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .tables
                .get(table as usize)
                .map(|table| table.readonly)
        })
    }

    pub fn table_version(&mut self, table: u32) -> Option<u64> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .tables
                .get(table as usize)
                .map(|table| table.version)
        })
    }

    pub fn table_shape_version(&mut self, table: u32) -> Option<u64> {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .tables
                .get(table as usize)
                .map(|table| table.shape_version)
        })
    }

    pub fn table_freeze(&mut self, table: u32) -> Option<bool> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            let was_readonly = table.readonly;
            if !was_readonly {
                bump_table_version(table);
                table.readonly = true;
            }
            Some(was_readonly)
        })
    }

    pub fn has_table(&mut self, table: u32) -> bool {
        self.arena.mutate(|mc, root| {
            root.unlock(mc)
                .borrow()
                .tables
                .get(table as usize)
                .is_some()
        })
    }
}

fn table_key_from_value(root: &RootData<'_>, value: Value) -> Option<TableKey> {
    match value.kind() {
        ValueKind::Nil => None,
        ValueKind::Boolean => value.as_bool().map(TableKey::Boolean),
        ValueKind::Number => {
            let number = value.as_number()?;
            if number.is_nan() {
                None
            } else {
                value.table_key_number_bits().map(TableKey::Number)
            }
        }
        ValueKind::String => value.as_string_atom().map(TableKey::String),
        ValueKind::Table => value.as_table().map(TableKey::Table),
        ValueKind::Function => value.as_function().map(TableKey::Function),
        ValueKind::Userdata => value.as_userdata().map(TableKey::Userdata),
        ValueKind::Buffer => value.as_buffer().map(TableKey::Buffer),
        ValueKind::Vector => {
            let vector = value.as_vector()?;
            root.vectors
                .get(vector as usize)
                .and_then(|vector| vector.table_key_bits())
                .map(TableKey::Vector)
        }
    }
}

fn table_key_visible_value(
    root: &mut RootData<'_>,
    key: TableKey,
    original: Value,
) -> Option<Value> {
    match key {
        TableKey::Number(bits) => Some(Value::number(f64::from_bits(bits))),
        TableKey::Vector(bits) => {
            let handle = u32::try_from(root.vectors.len()).ok()?;
            root.vectors.push(VectorData::new(
                f32::from_bits(bits[0]),
                f32::from_bits(bits[1]),
                f32::from_bits(bits[2]),
            ));
            Some(Value::vector(handle))
        }
        _ => Some(original),
    }
}

fn table_key_array_index(key: TableKey) -> Option<usize> {
    let TableKey::Number(bits) = key else {
        return None;
    };
    let value = f64::from_bits(bits);
    if value.is_finite()
        && (1.0..=MAX_LUAU_ARRAY_INDEX_F64).contains(&value)
        && value.fract() == 0.0
    {
        Some(value as usize)
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

    if table.hash.is_empty() {
        return Value::nil();
    }

    let Some(key) = number_table_key(one_based_index as f64) else {
        return Value::nil();
    };
    table_hash_entry(table, key).map_or_else(Value::nil, |entry| entry.value)
}

fn bump_table_version(table: &mut TableData) {
    table.version = table.version.wrapping_add(1);
}

fn bump_table_shape_version(table: &mut TableData) {
    table.shape_version = table.shape_version.wrapping_add(1);
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
    if value.kind() == ValueKind::Nil {
        if let Some(slot) = table.array.get_mut(index - 1) {
            *slot = Value::nil();
        }
        if !table.hash.is_empty() {
            let Some(key) = number_table_key(index as f64) else {
                return;
            };
            remove_hash_key(table, key);
        }
    } else if index == table.array.len() + 1 {
        table.array.push(value);
        if !table.hash.is_empty() {
            let Some(key) = number_table_key(index as f64) else {
                return;
            };
            remove_hash_key(table, key);
        }
    } else if index <= table.array.len() {
        table.array[index - 1] = value;
        if !table.hash.is_empty() {
            let Some(key) = number_table_key(index as f64) else {
                return;
            };
            remove_hash_key(table, key);
        }
    } else {
        set_hash_number_key(table, index as f64, value);
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

fn small_positive_integer_bytes(value: u32) -> Option<([u8; 6], usize)> {
    if value > 999_999 {
        return None;
    }
    if value == 0 {
        let mut buffer = [0; 6];
        buffer[0] = b'0';
        return Some((buffer, 1));
    }

    let mut number = value;
    let mut buffer = [0; 6];
    let mut cursor = buffer.len();
    while number >= 10 {
        cursor -= 1;
        buffer[cursor] = b'0' + (number % 10) as u8;
        number /= 10;
    }
    cursor -= 1;
    buffer[cursor] = b'0' + number as u8;

    let len = buffer.len() - cursor;
    buffer.copy_within(cursor.., 0);
    Some((buffer, len))
}

fn concat_integer_key(lhs: u32, rhs: u32) -> u64 {
    (u64::from(lhs) << 32) | u64::from(rhs)
}

fn concat_integer_direct_cache_get(
    cache: &[Option<ConcatIntegerCacheEntry>],
    key: u64,
) -> Option<u32> {
    let entry = cache.get(concat_integer_direct_cache_slot(key))?.as_ref()?;
    (entry.key == key).then_some(entry.atom)
}

fn concat_integer_direct_cache_insert(
    cache: &mut [Option<ConcatIntegerCacheEntry>],
    key: u64,
    atom: u32,
) {
    let slot = concat_integer_direct_cache_slot(key);
    if let Some(entry) = cache.get_mut(slot) {
        *entry = Some(ConcatIntegerCacheEntry { key, atom });
    }
}

fn concat_integer_direct_cache_slot(key: u64) -> usize {
    ((key ^ (key >> 32)) as usize) & (CONCAT_INTEGER_DIRECT_CACHE_LEN - 1)
}

fn set_hash_key(table: &mut TableData, key: TableKey, key_value: Value, value: Value) {
    if let Some(entry) = table_hash_entry_mut(table, key) {
        entry.value = value;
    } else if value.kind() != ValueKind::Nil {
        insert_hash_key_absent(table, key, key_value, value);
    }
}

fn insert_hash_key_absent(table: &mut TableData, key: TableKey, key_value: Value, value: Value) {
    let index = table.hash.len();
    table.hash.push(TableEntry {
        key,
        key_value,
        value,
    });
    bump_table_shape_version(table);
    record_hash_insert(table, key, index);
}

fn remove_hash_key(table: &mut TableData, key: TableKey) {
    if let Some(hash_index) = table_hash_entry_index(table, key) {
        table.hash.remove(hash_index);
        bump_table_shape_version(table);
        record_hash_remove(table, hash_index);
    }
}

#[inline(always)]
fn table_hash_entry(table: &TableData, key: TableKey) -> Option<&TableEntry> {
    let index = table_hash_entry_index(table, key)?;
    table.hash.get(index)
}

#[inline(always)]
fn table_hash_entry_mut(table: &mut TableData, key: TableKey) -> Option<&mut TableEntry> {
    let index = table_hash_entry_index(table, key)?;
    table.hash.get_mut(index)
}

#[inline(always)]
fn table_hash_entry_index(table: &TableData, key: TableKey) -> Option<usize> {
    if table.hash.is_empty() {
        return None;
    }
    if table.hash_index.is_empty() {
        table.hash.iter().position(|entry| entry.key == key)
    } else {
        let mut position = table_hash_index_position(key, table.hash_index.len());
        for _ in 0..table.hash_index.len() {
            match table.hash_index[position] {
                Some(entry) if entry.key == key => return Some(entry.index),
                Some(_) => position = (position + 1) & (table.hash_index.len() - 1),
                None => return None,
            }
        }
        None
    }
}

fn record_hash_insert(table: &mut TableData, key: TableKey, index: usize) {
    if table.hash_index.is_empty() {
        if table.hash.len() >= TABLE_HASH_INDEX_MIN_LEN {
            rebuild_hash_index(table);
        }
        return;
    }
    insert_hash_index(table, key, index);
}

fn insert_hash_index(table: &mut TableData, key: TableKey, index: usize) {
    if table.hash.len().saturating_mul(2) > table.hash_index.len() {
        rebuild_hash_index(table);
        return;
    }
    let mut position = table_hash_index_position(key, table.hash_index.len());
    for _ in 0..table.hash_index.len() {
        match table.hash_index[position] {
            Some(entry) if entry.key == key => {
                table.hash_index[position] = Some(TableHashIndexEntry { key, index });
                return;
            }
            Some(_) => position = (position + 1) & (table.hash_index.len() - 1),
            None => {
                table.hash_index[position] = Some(TableHashIndexEntry { key, index });
                return;
            }
        }
    }
    rebuild_hash_index(table);
}

fn record_hash_remove(table: &mut TableData, removed_index: usize) {
    let _ = removed_index;
    if table.hash.len() < TABLE_HASH_INDEX_MIN_LEN {
        table.hash_index.clear();
        return;
    }
    rebuild_hash_index(table);
}

fn rebuild_hash_index(table: &mut TableData) {
    table.hash_index.clear();
    let capacity = table
        .hash
        .len()
        .saturating_mul(4)
        .next_power_of_two()
        .max(TABLE_HASH_INDEX_MIN_LEN * 4);
    table.hash_index.resize(capacity, None);
    for index in 0..table.hash.len() {
        let key = table.hash[index].key;
        insert_hash_index(table, key, index);
    }
}

#[inline(always)]
fn table_hash_index_position(key: TableKey, len: usize) -> usize {
    debug_assert!(len.is_power_of_two());
    (table_hash_index_key(key) as usize) & (len - 1)
}

#[inline(always)]
fn table_hash_index_key(key: TableKey) -> u64 {
    let value = match key {
        TableKey::Boolean(value) => u64::from(value),
        TableKey::Number(bits) => bits,
        TableKey::String(handle)
        | TableKey::Table(handle)
        | TableKey::Function(handle)
        | TableKey::Userdata(handle)
        | TableKey::Buffer(handle) => u64::from(handle),
        TableKey::Vector(bits) => bits.into_iter().fold(0x9e37_79b9_u64, |hash, bits| {
            hash ^ u64::from(bits)
                .wrapping_add(0x9e37_79b9)
                .wrapping_add(hash << 6)
                .wrapping_add(hash >> 2)
        }),
    };
    value.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(23) ^ value.rotate_right(17)
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

fn table_weak_mode(root: &RootData<'_>, table: usize) -> Option<WeakMode> {
    let table = root.tables.get(table)?;
    let metatable = table.metatable?;
    let metatable = root.tables.get(metatable as usize)?;
    let mode = metatable.hash.iter().find_map(|entry| {
        if !table_key_is_string(root, entry.key, b"__mode") {
            return None;
        }
        let atom = entry.value.as_string_atom()?;
        root.strings
            .get(atom as usize)
            .map(|value| value.0.as_slice())
    })?;
    let weak = WeakMode {
        keys: mode.contains(&b'k'),
        values: mode.contains(&b'v'),
    };
    (weak.keys || weak.values).then_some(weak)
}

fn table_key_is_string(root: &RootData<'_>, key: TableKey, expected: &[u8]) -> bool {
    let TableKey::String(atom) = key else {
        return false;
    };
    string_atom_is(root, atom, expected)
}

fn string_atom_is(root: &RootData<'_>, atom: u32, expected: &[u8]) -> bool {
    root.strings
        .get(atom as usize)
        .is_some_and(|value| value.0.as_slice() == expected)
}

fn mark_weak_reachable(
    root: &RootData<'_>,
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
                let Some(function) = root.functions.get(handle as usize) else {
                    continue;
                };
                mark_weak_value(Value::table(function.environment), &mut refs, &mut queue);
                for upvalue in &function.upvalues {
                    if let Some(upvalue) = root.upvalues.get(*upvalue as usize) {
                        mark_weak_value(upvalue.value, &mut refs, &mut queue);
                    }
                }
            }
            WeakRef::Table(handle) => {
                let Some(table) = root.tables.get(handle as usize) else {
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
    let mut changed = false;
    if mode.values {
        for value in &mut table.array {
            if weak_value_is_collectable(*value) && !live.contains_value(*value) {
                *value = Value::nil();
                changed = true;
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
        rebuild_hash_index(table);
        bump_table_shape_version(table);
        changed = true;
    }
    if changed {
        bump_table_version(table);
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
    match order {
        TableIterationOrder::Insertion => table.hash.iter().find_map(hash_entry_pair),
        TableIterationOrder::LuauHash => luau_hash_nodes(root, table)
            .into_iter()
            .find_map(hash_entry_pair),
    }
}

fn next_after_hash_key(
    root: &RootData<'_>,
    table: &TableData,
    key: TableKey,
    order: TableIterationOrder,
) -> Option<Option<(Value, Value)>> {
    match order {
        TableIterationOrder::Insertion => {
            let index = table_hash_entry_index(table, key)?;
            Some(
                table
                    .hash
                    .get(index + 1..)
                    .unwrap_or(&[])
                    .iter()
                    .find_map(hash_entry_pair),
            )
        }
        TableIterationOrder::LuauHash => {
            let entries = luau_hash_nodes(root, table);
            let index = entries.iter().position(|entry| entry.key == key)?;
            Some(
                entries
                    .into_iter()
                    .skip(index + 1)
                    .find_map(hash_entry_pair),
            )
        }
    }
}

fn hash_entry_pair(entry: &TableEntry) -> Option<(Value, Value)> {
    (entry.value.kind() != ValueKind::Nil).then_some((entry.key_value, entry.value))
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
            .map_or(atom, |value| luau_string_hash(value.0.as_slice())),
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

    #[test]
    fn table_shape_version_tracks_hash_shape_not_hash_values() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let other_key = heap.intern_string("y");
        let table = heap.alloc_array_table(&[]);

        heap.table_set(table, Value::string_atom(key), Value::number(1.0))
            .unwrap();
        let slot = heap.table_string_hash_slot(table, key).unwrap().unwrap();
        let shape_version = heap.table_shape_version(table).unwrap();

        heap.table_set(table, Value::string_atom(key), Value::number(2.0))
            .unwrap();
        assert_eq!(heap.table_shape_version(table), Some(shape_version));
        let slot_read = heap
            .read_string_hash_slot_at_index(table, key, slot.index)
            .unwrap()
            .unwrap();
        assert_eq!(slot_read.value.as_number(), Some(2.0));

        heap.table_set(table, Value::string_atom(other_key), Value::number(3.0))
            .unwrap();
        assert_ne!(heap.table_shape_version(table), Some(shape_version));
    }

    #[test]
    fn table_shape_version_tracks_numeric_hash_insertions() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        let shape_version = heap.table_shape_version(table).unwrap();

        heap.table_set(table, Value::number(20.0), Value::number(7.0))
            .unwrap();

        assert_ne!(heap.table_shape_version(table), Some(shape_version));
        assert_eq!(
            heap.table_integer_hash_slot(table, 20)
                .unwrap()
                .unwrap()
                .value
                .as_number(),
            Some(7.0)
        );
    }

    #[test]
    fn table_shape_version_tracks_metatable_changes() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        let metatable = heap.alloc_array_table(&[]);
        let shape_version = heap.table_shape_version(table).unwrap();

        heap.table_set_metatable(table, Some(metatable)).unwrap();

        assert_ne!(heap.table_shape_version(table), Some(shape_version));
    }

    #[test]
    fn absent_string_write_fast_path_requires_plain_table() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let table = heap.alloc_array_table(&[]);

        let status = heap.write_string_index_without_metamethod(table, key, Value::number(3.0));
        let Some(StringIndexWriteStatus::Stored {
            hash_index: Some(index),
        }) = status
        else {
            panic!("absent plain string write should insert a hash slot");
        };
        assert_eq!(
            heap.table_get(table, Value::string_atom(key))
                .unwrap()
                .as_number(),
            Some(3.0)
        );
        assert_eq!(
            heap.write_string_index_without_metamethod(table, key, Value::number(4.0)),
            Some(StringIndexWriteStatus::Stored {
                hash_index: Some(index)
            })
        );
        assert_eq!(
            heap.table_get(table, Value::string_atom(key))
                .unwrap()
                .as_number(),
            Some(4.0)
        );
    }

    #[test]
    fn absent_string_write_fast_path_defers_with_metatable() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let table = heap.alloc_array_table(&[]);
        let metatable = heap.alloc_array_table(&[]);
        heap.table_set_metatable(table, Some(metatable)).unwrap();

        assert_eq!(
            heap.write_string_index_without_metamethod(table, key, Value::number(3.0)),
            Some(StringIndexWriteStatus::Deferred)
        );
        assert!(heap.table_get(table, Value::string_atom(key)).is_none());
    }

    #[test]
    fn table_string_hash_probe_reads_shape_slot_and_metatable() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let table = heap.alloc_array_table(&[]);
        let metatable = heap.alloc_array_table(&[]);
        heap.table_set(table, Value::string_atom(key), Value::number(9.0))
            .unwrap();
        heap.table_set_metatable(table, Some(metatable)).unwrap();

        let probe = heap
            .table_string_hash_probe(table, key)
            .expect("probe should find table");

        assert_eq!(probe.metatable, Some(metatable));
        assert_eq!(
            probe.shape_version,
            heap.table_shape_version(table).unwrap()
        );
        assert_eq!(probe.slot.unwrap().value.as_number(), Some(9.0));
    }

    #[test]
    fn table_array_range_values_reads_plain_array_ranges() {
        let mut heap = VmCell::new();
        let table =
            heap.alloc_array_table(&[Value::number(1.0), Value::number(2.0), Value::number(3.0)]);

        let values = heap.table_array_range_values(table, 2, 3).unwrap().unwrap();

        assert_eq!(values.len(), 2);
        assert_eq!(values[0].as_number(), Some(2.0));
        assert_eq!(values[1].as_number(), Some(3.0));
    }

    #[test]
    fn table_sort_numeric_array_sorts_number_prefix_in_place() {
        let mut heap = VmCell::new();
        let table =
            heap.alloc_array_table(&[Value::number(3.0), Value::number(1.0), Value::number(2.0)]);

        assert_eq!(heap.table_sort_numeric_array(table, 3), Some(true));

        assert_eq!(
            heap.array_slot(table, 1).unwrap().unwrap().as_number(),
            Some(1.0)
        );
        assert_eq!(
            heap.array_slot(table, 2).unwrap().unwrap().as_number(),
            Some(2.0)
        );
        assert_eq!(
            heap.array_slot(table, 3).unwrap().unwrap().as_number(),
            Some(3.0)
        );
    }

    #[test]
    fn table_sort_numeric_array_defers_mixed_values() {
        let mut heap = VmCell::new();
        let atom = heap.intern_string("a");
        let table = heap.alloc_array_table(&[
            Value::number(3.0),
            Value::string_atom(atom),
            Value::number(2.0),
        ]);

        assert_eq!(heap.table_sort_numeric_array(table, 3), Some(false));

        assert_eq!(
            heap.array_slot(table, 1).unwrap().unwrap().as_number(),
            Some(3.0)
        );
        assert_eq!(
            heap.array_slot(table, 2).unwrap().unwrap().as_string_atom(),
            Some(atom)
        );
        assert_eq!(
            heap.array_slot(table, 3).unwrap().unwrap().as_number(),
            Some(2.0)
        );
    }

    #[test]
    fn table_array_prefix_write_updates_only_existing_array_prefix() {
        let mut heap = VmCell::new();
        let table =
            heap.alloc_array_table(&[Value::number(1.0), Value::number(2.0), Value::number(3.0)]);
        heap.table_set(table, Value::number(10.0), Value::number(10.0))
            .unwrap();

        assert_eq!(
            heap.write_array_prefix(table, &[Value::number(4.0), Value::number(5.0)]),
            Some(ArrayWriteStatus::Stored)
        );

        assert_eq!(
            heap.array_slot(table, 1).unwrap().unwrap().as_number(),
            Some(4.0)
        );
        assert_eq!(
            heap.array_slot(table, 2).unwrap().unwrap().as_number(),
            Some(5.0)
        );
        assert_eq!(
            heap.array_slot(table, 3).unwrap().unwrap().as_number(),
            Some(3.0)
        );
        assert_eq!(
            heap.table_get(table, Value::number(10.0))
                .unwrap()
                .as_number(),
            Some(10.0)
        );
    }

    #[test]
    fn table_move_integer_range_copies_array_values_in_bulk() {
        let mut heap = VmCell::new();
        let source = heap.alloc_array_table(&[
            Value::number(10.0),
            Value::number(20.0),
            Value::number(30.0),
        ]);
        let destination = heap.alloc_array_table(&[
            Value::number(1.0),
            Value::number(2.0),
            Value::number(3.0),
            Value::number(4.0),
            Value::number(5.0),
        ]);

        assert_eq!(
            heap.table_move_integer_range(source, 1, destination, 2, 3),
            Some(ArrayWriteStatus::Stored)
        );

        let values = heap
            .table_array_range_values(destination, 1, 5)
            .unwrap()
            .unwrap();
        let numbers = values
            .into_iter()
            .map(|value| value.as_number())
            .collect::<Vec<_>>();
        assert_eq!(
            numbers,
            vec![Some(1.0), Some(10.0), Some(20.0), Some(30.0), Some(5.0)]
        );
    }

    #[test]
    fn table_move_integer_range_preserves_self_overlap() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[
            Value::number(10.0),
            Value::number(20.0),
            Value::number(30.0),
        ]);

        assert_eq!(
            heap.table_move_integer_range(table, 1, table, 2, 3),
            Some(ArrayWriteStatus::Stored)
        );

        let values = heap.table_array_range_values(table, 1, 4).unwrap().unwrap();
        let numbers = values
            .into_iter()
            .map(|value| value.as_number())
            .collect::<Vec<_>>();
        assert_eq!(
            numbers,
            vec![Some(10.0), Some(10.0), Some(20.0), Some(30.0)]
        );
    }

    #[test]
    fn unique_record_table_builder_matches_generic_record_shape() {
        let mut heap = VmCell::new();
        let keys = (0..20)
            .map(|index| heap.intern_string(&format!("k{index}")))
            .collect::<Vec<_>>();
        let values = (0..20)
            .map(|index| Value::number(index as f64))
            .collect::<Vec<_>>();

        let generic = heap
            .alloc_record_table_with_string_atoms(&keys, &values)
            .unwrap();
        let direct = heap
            .alloc_unique_record_table_with_string_atoms(&keys, &values)
            .unwrap();

        assert_eq!(heap.table_shape_version(direct), Some(20));
        assert_eq!(
            heap.table_shape_version(direct),
            heap.table_shape_version(generic)
        );
        for (index, key) in keys.iter().copied().enumerate() {
            let generic_slot = heap.table_string_hash_slot(generic, key).unwrap().unwrap();
            let direct_slot = heap.table_string_hash_slot(direct, key).unwrap().unwrap();
            assert_eq!(direct_slot.index, generic_slot.index);
            assert_eq!(direct_slot.value.as_number(), Some(index as f64));
        }
    }

    #[test]
    fn unique_record_table_builder_skips_initial_nil_values() {
        let mut heap = VmCell::new();
        let absent = heap.intern_string("absent");
        let present = heap.intern_string("present");
        let table = heap
            .alloc_unique_record_table_with_string_atoms(
                &[absent, present],
                &[Value::nil(), Value::number(4.0)],
            )
            .unwrap();

        assert_eq!(heap.table_shape_version(table), Some(1));
        assert!(heap
            .table_string_hash_slot(table, absent)
            .unwrap()
            .is_none());
        assert_eq!(
            heap.table_get(table, Value::string_atom(present))
                .unwrap()
                .as_number(),
            Some(4.0)
        );
    }

    #[test]
    fn string_hash_slot_writes_preserve_shape_and_update_value() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let table = heap.alloc_array_table(&[]);
        heap.table_set(table, Value::string_atom(key), Value::number(1.0))
            .unwrap();
        let slot = heap.table_string_hash_slot(table, key).unwrap().unwrap();
        let shape_version = heap.table_shape_version(table).unwrap();

        assert_eq!(
            heap.write_string_hash_slot_at_index(table, key, slot.index, Value::number(2.0)),
            Some(HashWriteStatus::Stored)
        );
        assert_eq!(heap.table_shape_version(table), Some(shape_version));
        assert_eq!(
            heap.table_get(table, Value::string_atom(key))
                .unwrap()
                .as_number(),
            Some(2.0)
        );
    }

    #[test]
    fn string_shape_hash_slot_reads_only_matching_shape() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let other_key = heap.intern_string("y");
        let table = heap.alloc_array_table(&[]);
        heap.table_set(table, Value::string_atom(key), Value::number(1.0))
            .unwrap();
        let slot = heap.table_string_hash_slot(table, key).unwrap().unwrap();
        let shape_version = heap.table_shape_version(table).unwrap();

        heap.table_set(table, Value::string_atom(key), Value::number(2.0))
            .unwrap();
        let slot_read = heap
            .read_string_shape_hash_slot_at_index(table, shape_version, slot.index)
            .unwrap()
            .unwrap();
        assert_eq!(slot_read.value.as_number(), Some(2.0));

        heap.table_set(table, Value::string_atom(other_key), Value::number(3.0))
            .unwrap();
        assert!(heap
            .read_string_shape_hash_slot_at_index(table, shape_version, slot.index)
            .unwrap()
            .is_none());
    }

    #[test]
    fn string_shape_hash_slot_writes_only_matching_shape() {
        let mut heap = VmCell::new();
        let key = heap.intern_string("x");
        let other_key = heap.intern_string("y");
        let table = heap.alloc_array_table(&[]);
        heap.table_set(table, Value::string_atom(key), Value::number(1.0))
            .unwrap();
        let slot = heap.table_string_hash_slot(table, key).unwrap().unwrap();
        let shape_version = heap.table_shape_version(table).unwrap();

        assert_eq!(
            heap.write_string_shape_hash_slot_at_index(
                table,
                key,
                shape_version,
                slot.index,
                Value::number(2.0),
            ),
            Some(HashWriteStatus::Stored)
        );
        assert_eq!(heap.table_shape_version(table), Some(shape_version));
        assert_eq!(
            heap.table_get(table, Value::string_atom(key))
                .unwrap()
                .as_number(),
            Some(2.0)
        );

        heap.table_set(table, Value::string_atom(other_key), Value::number(3.0))
            .unwrap();
        assert_eq!(
            heap.write_string_shape_hash_slot_at_index(
                table,
                key,
                shape_version,
                slot.index,
                Value::number(4.0),
            ),
            Some(HashWriteStatus::Deferred)
        );
        assert_eq!(
            heap.table_get(table, Value::string_atom(key))
                .unwrap()
                .as_number(),
            Some(2.0)
        );
    }

    #[test]
    fn string_hash_slot_indices_can_be_reused_across_matching_tables() {
        let mut heap = VmCell::new();
        let x = heap.intern_string("x");
        let y = heap.intern_string("y");
        let first = heap.alloc_array_table(&[]);
        let second = heap.alloc_array_table(&[]);
        let different = heap.alloc_array_table(&[]);

        for table in [first, second] {
            heap.table_set(table, Value::string_atom(x), Value::number(1.0))
                .unwrap();
            heap.table_set(table, Value::string_atom(y), Value::number(2.0))
                .unwrap();
        }
        heap.table_set(different, Value::string_atom(y), Value::number(3.0))
            .unwrap();
        heap.table_set(different, Value::string_atom(x), Value::number(4.0))
            .unwrap();

        let x_slot = heap.table_string_hash_slot(first, x).unwrap().unwrap();
        let second_read = heap
            .read_string_hash_slot_at_index(second, x, x_slot.index)
            .unwrap()
            .unwrap();
        assert_eq!(second_read.value.as_number(), Some(1.0));
        assert!(heap
            .read_string_hash_slot_at_index(different, x, x_slot.index)
            .unwrap()
            .is_none());
    }

    #[test]
    fn integer_hash_slot_indices_can_be_reused_across_matching_tables() {
        let mut heap = VmCell::new();
        let first = heap.alloc_array_table(&[]);
        let second = heap.alloc_array_table(&[]);
        let different = heap.alloc_array_table(&[]);

        for table in [first, second] {
            assert_eq!(
                heap.write_integer_index_without_metamethod(table, 8, Value::number(1.0)),
                Some(ArrayWriteStatus::Stored)
            );
        }
        assert_eq!(
            heap.write_integer_index_without_metamethod(different, 7, Value::number(2.0)),
            Some(ArrayWriteStatus::Stored)
        );
        assert_eq!(
            heap.write_integer_index_without_metamethod(different, 8, Value::number(3.0)),
            Some(ArrayWriteStatus::Stored)
        );

        let slot = heap.table_integer_hash_slot(first, 8).unwrap().unwrap();
        let second_read = heap
            .read_integer_hash_slot_at_index(second, 8, slot.index)
            .unwrap()
            .unwrap();
        assert_eq!(second_read.value.as_number(), Some(1.0));
        assert!(heap
            .read_integer_hash_slot_at_index(different, 8, slot.index)
            .unwrap()
            .is_none());
    }

    #[test]
    fn integer_array_write_fast_path_appends_dense_slots() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);

        assert_eq!(
            heap.write_integer_array_index_without_metamethod(table, 1, Value::number(7.0)),
            Some(ArrayWriteStatus::Stored)
        );
        assert_eq!(
            heap.array_slot(table, 1).unwrap().unwrap().as_number(),
            Some(7.0)
        );
        assert!(heap.table_integer_hash_slot(table, 1).unwrap().is_none());
    }

    #[test]
    fn integer_array_write_fast_path_defers_sparse_slots() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);

        assert_eq!(
            heap.write_integer_array_index_without_metamethod(table, 8, Value::number(7.0)),
            Some(ArrayWriteStatus::Deferred)
        );
        assert!(heap.table_integer_hash_slot(table, 8).unwrap().is_none());
    }

    #[test]
    fn integer_array_write_fast_path_defers_absent_metatable_slots() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        let metatable = heap.alloc_array_table(&[]);
        heap.table_set_metatable(table, Some(metatable)).unwrap();

        assert_eq!(
            heap.write_integer_array_index_without_metamethod(table, 1, Value::number(7.0)),
            Some(ArrayWriteStatus::Deferred)
        );
        assert!(heap.array_slot(table, 1).unwrap().is_none());
    }

    #[test]
    fn integer_shape_hash_slot_reads_only_matching_shape() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        assert_eq!(
            heap.write_integer_index_without_metamethod(table, 8, Value::number(1.0)),
            Some(ArrayWriteStatus::Stored)
        );
        let slot = heap.table_integer_hash_slot(table, 8).unwrap().unwrap();
        let shape_version = heap.table_shape_version(table).unwrap();

        assert_eq!(
            heap.write_integer_hash_slot_at_index(table, 8, slot.index, Value::number(2.0)),
            Some(HashWriteStatus::Stored)
        );
        let slot_read = heap
            .read_integer_shape_hash_slot_at_index(table, 8, shape_version, slot.index)
            .unwrap()
            .unwrap();
        assert_eq!(slot_read.value.as_number(), Some(2.0));

        assert_eq!(
            heap.write_integer_index_without_metamethod(table, 9, Value::number(3.0)),
            Some(ArrayWriteStatus::Stored)
        );
        assert!(heap
            .read_integer_shape_hash_slot_at_index(table, 8, shape_version, slot.index)
            .unwrap()
            .is_none());
    }

    #[test]
    fn integer_shape_hash_slot_writes_only_matching_shape() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);
        assert_eq!(
            heap.write_integer_index_without_metamethod(table, 8, Value::number(1.0)),
            Some(ArrayWriteStatus::Stored)
        );
        let slot = heap.table_integer_hash_slot(table, 8).unwrap().unwrap();
        let shape_version = heap.table_shape_version(table).unwrap();

        assert_eq!(
            heap.write_integer_shape_hash_slot_at_index(
                table,
                8,
                shape_version,
                slot.index,
                Value::number(2.0),
            ),
            Some(HashWriteStatus::Stored)
        );
        assert_eq!(heap.table_shape_version(table), Some(shape_version));
        assert_eq!(
            heap.table_get(table, Value::number(8.0))
                .unwrap()
                .as_number(),
            Some(2.0)
        );

        assert_eq!(
            heap.write_integer_index_without_metamethod(table, 9, Value::number(3.0)),
            Some(ArrayWriteStatus::Stored)
        );
        assert_eq!(
            heap.write_integer_shape_hash_slot_at_index(
                table,
                8,
                shape_version,
                slot.index,
                Value::number(4.0),
            ),
            Some(HashWriteStatus::Deferred)
        );
        assert_eq!(
            heap.table_get(table, Value::number(8.0))
                .unwrap()
                .as_number(),
            Some(2.0)
        );
    }

    #[test]
    fn integer_hash_slot_indices_defer_when_array_slot_can_take_precedence() {
        let mut heap = VmCell::new();
        let table = heap.alloc_array_table(&[]);

        assert_eq!(
            heap.write_integer_index_without_metamethod(table, 4, Value::number(9.0)),
            Some(ArrayWriteStatus::Stored)
        );
        let slot = heap.table_integer_hash_slot(table, 4).unwrap().unwrap();
        for index in 1..=3 {
            assert_eq!(
                heap.write_integer_index_without_metamethod(
                    table,
                    index,
                    Value::number(index as f64)
                ),
                Some(ArrayWriteStatus::Stored)
            );
        }

        assert_eq!(
            heap.write_integer_hash_slot_at_index(table, 4, slot.index, Value::number(10.0)),
            Some(HashWriteStatus::Deferred)
        );
        assert_eq!(
            heap.write_integer_index_without_metamethod(table, 4, Value::number(4.0)),
            Some(ArrayWriteStatus::Stored)
        );
        assert!(heap
            .read_integer_hash_slot_at_index(table, 4, slot.index)
            .unwrap()
            .is_none());
        assert_eq!(
            heap.array_slot(table, 4).unwrap().unwrap().as_number(),
            Some(4.0)
        );
    }
}
