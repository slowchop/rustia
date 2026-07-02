use gc_arena::lock::RefLock;
use gc_arena::{Arena, Collect, DynamicRootSet, Gc, Rootable, Static};
use std::collections::{BTreeSet, VecDeque};

use crate::number::table_key_number_bits;
use crate::value::{Value, ValueKind};

const MAX_LUAU_ARRAY_INDEX: usize = 2_147_483_647;
const MAX_LUAU_ARRAY_INDEX_F64: f64 = 2_147_483_647.0;

#[derive(Collect)]
#[collect(no_drop)]
struct RootData<'gc> {
    _roots: DynamicRootSet<'gc>,
    strings: Vec<Gc<'gc, Static<Vec<u8>>>>,
    tables: Vec<TableData>,
    functions: Vec<FunctionData>,
    upvalues: Vec<UpvalueData>,
    userdatas: Vec<UserdataData>,
    buffers: Vec<BufferData>,
    vectors: Vec<VectorData>,
}

#[derive(Clone, Collect)]
#[collect(no_drop)]
struct TableData {
    array: Vec<Value>,
    hash: Vec<TableEntry>,
    readonly: bool,
    metatable: Option<u32>,
}

#[derive(Collect)]
#[collect(no_drop)]
struct FunctionData {
    kind: FunctionKind,
    upvalues: Vec<u32>,
    environment: u32,
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

#[derive(Clone, Copy, Eq, PartialEq, Collect)]
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
pub(crate) enum TableIterationOrder {
    Insertion,
    LuauHash,
}

type ArenaRoot = Rootable![Gc<'_, RefLock<RootData<'_>>>];
type VmArena = Arena<ArenaRoot>;

/// The Send unit for one VM lane.
///
/// `gc-arena` values and retained roots are not independently thread-safe. The design model is
/// one VM per lane: the whole arena, root set, and future retained handles move together.
pub struct VmCell {
    arena: VmArena,
}

// SAFETY: `VmCell` owns the arena and root set exclusively. Moving the cell transfers the whole
// GC graph as one unit; callers cannot extract `Gc` values or `DynamicRoot` handles, so no arena
// allocation or non-atomic root bookkeeping can be touched concurrently from two threads.
unsafe impl Send for VmCell {}

impl VmCell {
    pub fn new() -> Self {
        Self {
            arena: VmArena::new(|mc| {
                Gc::new(
                    mc,
                    RefLock::new(RootData {
                        _roots: DynamicRootSet::new(mc),
                        strings: Vec::new(),
                        tables: Vec::new(),
                        functions: Vec::new(),
                        upvalues: Vec::new(),
                        userdatas: Vec::new(),
                        buffers: Vec::new(),
                        vectors: Vec::new(),
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
            if let Some((index, _)) = root
                .strings
                .iter()
                .enumerate()
                .find(|(_, existing)| existing.0.as_slice() == value)
            {
                return index as u32;
            }
            let atom = root.strings.len();
            root.strings.push(Gc::new(mc, Static(value.to_vec())));
            atom as u32
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

    pub fn alloc_array_table(&mut self, values: &[Value]) -> u32 {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = root.tables.len();
            root.tables.push(TableData {
                array: values.to_vec(),
                hash: Vec::new(),
                readonly: false,
                metatable: None,
            });
            handle as u32
        })
    }

    pub fn alloc_filled_array_table(&mut self, size: usize, value: Value) -> u32 {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let handle = root.tables.len();
            root.tables.push(TableData {
                array: vec![value; size],
                hash: Vec::new(),
                readonly: false,
                metatable: None,
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
        self.arena.mutate(|mc, root| {
            let root = root.unlock(mc).borrow();
            let table = root.tables.get(table as usize)?;
            let table_key = table_key_from_value(&root, key)?;
            if let Some(index) = table_key_array_index(table_key) {
                if let Some(value) = table.array.get(index - 1) {
                    return Some(*value);
                }
            }
            table
                .hash
                .iter()
                .find(|entry| entry.key == table_key)
                .map(|entry| entry.value)
        })
    }

    pub fn table_set(&mut self, table: u32, key: Value, value: Value) -> Option<()> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table_key = table_key_from_value(&root, key)?;
            let key_value = table_key_visible_value(&mut root, table_key, key)?;
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly {
                return None;
            }
            if value.kind() == ValueKind::Nil {
                if let Some(index) = table_key_array_index(table_key) {
                    if let Some(slot) = table.array.get_mut(index - 1) {
                        *slot = Value::nil();
                    }
                }
                if let Some(entry) = table.hash.iter_mut().find(|entry| entry.key == table_key) {
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
                    if let Some(hash_index) =
                        table.hash.iter().position(|entry| entry.key == table_key)
                    {
                        table.hash.remove(hash_index);
                    }
                    return Some(());
                }
            }

            if let Some(entry) = table.hash.iter_mut().find(|entry| entry.key == table_key) {
                entry.value = value;
            } else {
                table.hash.push(TableEntry {
                    key: table_key,
                    key_value,
                    value,
                });
            }
            Some(())
        })
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
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            if table.readonly || one_based_index > MAX_LUAU_ARRAY_INDEX {
                return None;
            }
            let key = number_table_key(one_based_index as f64)?;
            if one_based_index > table.array.len() {
                table.array.resize(one_based_index, Value::nil());
            }
            table.array[one_based_index - 1] = value;
            remove_hash_key(table, key);
            Some(())
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
            table.metatable = metatable;
            Some(())
        })
    }

    pub fn table_clone(&mut self, table: u32) -> Option<u32> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let mut clone = root.tables.get(table as usize)?.clone();
            clone.readonly = false;
            let handle = u32::try_from(root.tables.len()).ok()?;
            root.tables.push(clone);
            Some(handle)
        })
    }

    pub fn array_get(&mut self, table: u32, one_based_index: usize) -> Option<Value> {
        self.table_get(table, Value::number(one_based_index as f64))
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
            table.array.fill(Value::nil());
            table.hash.clear();
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

    pub fn table_freeze(&mut self, table: u32) -> Option<bool> {
        self.arena.mutate(|mc, root| {
            let mut root = root.unlock(mc).borrow_mut();
            let table = root.tables.get_mut(table as usize)?;
            let was_readonly = table.readonly;
            table.readonly = true;
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

    let Some(key) = number_table_key(one_based_index as f64) else {
        return Value::nil();
    };
    table
        .hash
        .iter()
        .find(|entry| entry.key == key)
        .map_or_else(Value::nil, |entry| entry.value)
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
        remove_hash_key(table, key);
    } else if index <= table.array.len() + 1 {
        if index > table.array.len() {
            table.array.resize(index, Value::nil());
        }
        table.array[index - 1] = value;
        remove_hash_key(table, key);
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
    if let Some(entry) = table.hash.iter_mut().find(|entry| entry.key == key) {
        entry.value = value;
    } else if value.kind() != ValueKind::Nil {
        table.hash.push(TableEntry {
            key,
            key_value,
            value,
        });
    }
}

fn remove_hash_key(table: &mut TableData, key: TableKey) {
    if let Some(hash_index) = table.hash.iter().position(|entry| entry.key == key) {
        table.hash.remove(hash_index);
    }
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
    if mode.values {
        for value in &mut table.array {
            if weak_value_is_collectable(*value) && !live.contains_value(*value) {
                *value = Value::nil();
            }
        }
    }
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
