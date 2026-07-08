#![allow(clippy::too_many_arguments)]

use crate::bytecode::{
    Constant, Function, Instruction, Opcode, Program, RuntimeType, UpvalueDescriptor,
    ARRAY_ITERATOR_SKIP_VISIBLE_KEY, ARRAY_ITERATOR_WRITE_VISIBLE_KEY,
    ASSERT_MESSAGE_MISSING_ARGUMENT, ASSERT_MESSAGE_NONE,
};
use crate::error::{Error, ErrorKind, ErrorLocation, Result};
use crate::extension::{validate_registered_op, ExtensionOp, ExtensionRegistry, HostFunctionOp};
use crate::heap::{
    ArrayWriteStatus, FunctionBytecodeContext, FunctionKind, HashReadStatus, NativeFunction,
    TableConcatOutcome, TableIterationOrder, TableNext, UpvalueAddConstStatus, UpvalueAddStatus,
    UserdataPayload, VectorData, VmCell,
};
use crate::number::{binary, canonicalize_number, NumericOp};
use crate::profile::Profile;
use crate::value::{Value, ValueKind};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::sync::Arc;

const LUAU_TABLE_MAX_SIZE: usize = 1 << 26;
const MAX_TABLE_CREATE_ARRAY_SIZE: usize = 100_000;
const MAX_TABLE_UNPACK_RESULTS: usize = 7_999;
const MAX_LUAU_TABLE_INDEX: f64 = 2_147_483_647.0;
const READONLY_TABLE_ERROR: &str = "attempt to modify a readonly table";
const LUAU_UTF8_CHARPATTERN: &[u8] = b"[\0-\x7f\xc2-\xf4][\x80-\xbf]*";
const LUAU_RECURSIVE_REVERSE_GSUB_PATTERN: &[u8] = b"(.)(.+)";
const OS_TIME_NOW_SECONDS: i64 = 1_700_000_000;
pub const MAX_RECURSION_LIMIT: usize = 512;
const LUAU_TAIL_CALL_LIMIT: usize = 20_000;
const GLOBAL_GET_INLINE_CACHE_SLOTS: usize = 1024;
const STRING_FORMAT_PLAN_CACHE_MAX: usize = 256;
const REGISTER_POOL_MAX_FRAMES: usize = 64;
const REGISTER_POOL_MAX_REGISTERS: usize = 256;
const RESULT_POOL_MAX_FRAMES: usize = 64;
const RESULT_POOL_MAX_VALUES: usize = 16;
const CALL_FRAME_POOL_MAX_FRAMES: usize = 64;
const CALL_FRAME_POOL_MAX_ROOTS: usize = 256;
const CALL_FRAME_POOL_MAX_STACKLESS_DEBUG_FRAMES: usize = 64;

pub type LoadStringCompiler =
    fn(source: &[u8], source_name: Option<&[u8]>) -> std::result::Result<Program, String>;

#[derive(Clone, Debug)]
pub struct VmSettings {
    pub initial_fuel: Option<u64>,
    pub recursion_limit: usize,
    pub deterministic_gc_interval: NonZeroU64,
    pub loadstring_compiler: Option<LoadStringCompiler>,
    pub enable_conformance_coverage: bool,
    pub enable_conformance_top_level_yield: bool,
    pub enable_conformance_int64_userdata: bool,
    pub enable_conformance_native_userdata: bool,
    pub enable_conformance_native_type_checks: bool,
    pub enable_conformance_pcall_helpers: bool,
}

impl Default for VmSettings {
    fn default() -> Self {
        Self {
            initial_fuel: None,
            recursion_limit: MAX_RECURSION_LIMIT,
            deterministic_gc_interval: NonZeroU64::new(10_000)
                .expect("default deterministic GC interval is non-zero"),
            loadstring_compiler: None,
            enable_conformance_coverage: false,
            enable_conformance_top_level_yield: false,
            enable_conformance_int64_userdata: false,
            enable_conformance_native_userdata: false,
            enable_conformance_native_type_checks: false,
            enable_conformance_pcall_helpers: false,
        }
    }
}

pub struct Vm {
    profile: Profile,
    settings: VmSettings,
    heap: VmCell,
    globals: u32,
    standard_globals: u32,
    conformance_int64_metatable: Option<u32>,
    conformance_vec2_metatable: Option<u32>,
    conformance_vertex_metatable: Option<u32>,
    extensions: ExtensionRegistry,
    host_functions: Vec<HostFunctionOp>,
    dynamic_programs: Vec<Arc<Program>>,
    register_pool: Vec<Vec<Value>>,
    result_pool: Vec<Vec<Value>>,
    call_frame_pool: Vec<CallStackFrame>,
    call_stack: Vec<CallStackFrame>,
    coroutines: Vec<CoroutineState>,
    running_coroutines: Vec<usize>,
    running_coroutine_stack_bases: Vec<usize>,
    yield_collectors: Vec<Vec<Vec<Value>>>,
    last_error_traceback: Option<TracebackSnapshot>,
    protected_error_traceback: Option<TracebackSnapshot>,
    non_yieldable_depth: usize,
    rng_state: u64,
    gcinfo_counter: u64,
    gc_step_counter: u64,
    gc_stopped: bool,
    executed_instructions: u64,
    deterministic_collection_count: u64,
    coverage_hits: BTreeMap<CoverageKey, u64>,
    global_get_inline_cache: Vec<Option<GlobalGetInlineCacheSlot>>,
    table_get_inline_cache: Vec<Option<TableGetInlineCacheSlot>>,
    arithmetic_metamethod_inline_cache: Vec<Option<ArithmeticMetamethodInlineCacheSlot>>,
    iter_metamethod_inline_cache: Vec<Option<IterMetamethodInlineCacheSlot>>,
    len_inline_cache: Vec<Option<LenInlineCacheSlot>>,
    constant_string_atom_cache: BTreeMap<(u32, usize), u32>,
    string_format_plan_cache: BTreeMap<u32, Arc<[StringFormatPlanItem]>>,
}

struct FunctionCallContext<'a> {
    current_function: Option<Value>,
    upvalues: &'a [u32],
    depth: usize,
    environment: u32,
    call_site: Option<ErrorLocation>,
    prototype: Option<u32>,
}

enum RegisterCallResult {
    Scalar(Value),
    Results(Vec<Value>),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CoverageKey {
    program: Option<u32>,
    prototype: u32,
    line: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InlineCacheSite {
    program: u32,
    prototype: Option<u32>,
    pc: usize,
}

impl InlineCacheSite {
    fn slot(self) -> usize {
        let prototype = self.prototype.unwrap_or(u32::MAX);
        let hash = u64::from(self.program).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ u64::from(prototype).rotate_left(17)
            ^ (self.pc as u64).rotate_left(32);
        (hash as usize) % GLOBAL_GET_INLINE_CACHE_SLOTS
    }
}

#[derive(Clone, Copy)]
struct GlobalGetInlineCacheSlot {
    site: InlineCacheSite,
    entry: GlobalGetInlineCacheEntry,
}

#[derive(Clone, Copy)]
struct GlobalGetInlineCacheEntry {
    environment: u32,
    environment_version: u64,
    source: GlobalGetInlineCacheSource,
    value: Value,
}

#[derive(Clone, Copy)]
enum GlobalGetInlineCacheSource {
    Environment,
    StandardGlobals { standard_globals_version: u64 },
}

#[derive(Clone, Copy)]
struct TableGetInlineCacheSlot {
    site: InlineCacheSite,
    entry: TableGetInlineCacheEntry,
}

#[derive(Clone, Copy)]
struct TableGetInlineCacheEntry {
    key_atom: u32,
    source: TableGetInlineCacheSource,
}

#[derive(Clone, Copy)]
enum TableGetInlineCacheSource {
    DirectSlot {
        table: u32,
        table_shape_version: u64,
        hash_index: usize,
    },
    DirectAbsent {
        table: u32,
        table_shape_version: u64,
    },
    IndexTable {
        metatable: u32,
        metatable_shape_version: u64,
        metatable_hash_index: usize,
        index_table: u32,
        index_table_shape_version: u64,
        index_table_hash_index: usize,
    },
}

#[derive(Clone, Copy)]
struct ArithmeticMetamethodInlineCacheSlot {
    site: InlineCacheSite,
    entry: ArithmeticMetamethodInlineCacheEntry,
}

#[derive(Clone, Copy)]
struct ArithmeticMetamethodInlineCacheEntry {
    op: NumericOp,
    metatable: u32,
    metatable_version: u64,
    method: Value,
}

#[derive(Clone, Copy)]
struct IterMetamethodInlineCacheSlot {
    site: InlineCacheSite,
    entry: IterMetamethodInlineCacheEntry,
}

#[derive(Clone, Copy)]
struct IterMetamethodInlineCacheEntry {
    table: u32,
    table_version: u64,
    metatable: Option<u32>,
    metatable_version: u64,
    method: Option<Value>,
}

#[derive(Clone, Copy)]
struct LenInlineCacheSlot {
    site: InlineCacheSite,
    entry: LenInlineCacheEntry,
}

#[derive(Clone, Copy)]
struct LenInlineCacheEntry {
    table: u32,
    table_version: u64,
    len: usize,
}

struct StandardGlobalHandles {
    standard_globals: u32,
    conformance_int64_metatable: Option<u32>,
    conformance_vec2_metatable: Option<u32>,
    conformance_vertex_metatable: Option<u32>,
}

#[derive(Clone)]
struct CallStackFrame {
    function: Option<Value>,
    debug_name: Option<Arc<str>>,
    environment: u32,
    call_site: Option<ErrorLocation>,
    roots: Vec<Value>,
    stackless_roots: Vec<Value>,
    stackless_debug_frames: Vec<StacklessDebugFrameRun>,
}

#[derive(Clone)]
struct StacklessDebugFrame {
    function: Option<Value>,
    debug_name: Option<Arc<str>>,
    environment: u32,
    call_site: Option<ErrorLocation>,
}

#[derive(Clone)]
struct StacklessDebugFrameRun {
    frame: StacklessDebugFrame,
    count: usize,
}

#[derive(Clone, Copy, Default)]
struct StacklessDebugMark {
    run_count: usize,
    last_run_count: usize,
}

#[derive(Clone)]
struct TracebackSnapshot {
    frames: Vec<CallStackFrame>,
    top_location: Option<ErrorLocation>,
}

#[derive(Clone)]
struct DebugInfoTarget {
    name: String,
    source: String,
    line: i32,
    function: Value,
    parameter_count: usize,
    is_vararg: bool,
}

#[derive(Clone, Copy)]
enum DebugInfoField {
    Name,
    Source,
    Line,
    Function,
    Arity,
}

struct DebugInfoOptions {
    fields: Vec<DebugInfoField>,
}

#[derive(Clone, Copy)]
struct TableMoveBounds {
    source: u32,
    destination: u32,
    start: i32,
    end: i32,
    destination_start: i32,
}

#[derive(Clone, Copy)]
enum TableForeachMode {
    Pairs,
    Array,
}

#[derive(Clone, Copy)]
enum BufferReadKind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
}

#[derive(Clone, Copy)]
enum BufferWriteKind {
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
}

#[derive(Clone, Copy)]
enum ConformanceInt64Binary {
    Add,
    Sub,
    Mul,
    Div,
    FloorDiv,
    Mod,
    Pow,
}

#[derive(Clone, Copy)]
enum BufferFloatKind {
    F32,
    F64,
}

#[derive(Clone)]
struct CoroutineState {
    handle: u32,
    callee: Value,
    frames: Vec<CoroutineFrame>,
    initialized: bool,
    dead: bool,
    error: Option<Value>,
    debug_snapshot: Option<CoroutineDebugSnapshot>,
    callable_metatable: Option<(u32, u64)>,
}

#[derive(Clone)]
struct CoroutineDebugSnapshot {
    frames: Vec<CallStackFrame>,
    top_location: Option<ErrorLocation>,
    first_debug_info_level: usize,
}

#[derive(Clone)]
enum VmFlow<T> {
    Return(T),
    Yield(CoroutineYield),
}

type VmFlowResult<T> = std::result::Result<VmFlow<T>, Error>;

#[derive(Clone)]
struct CoroutineYield {
    yielded: Vec<Value>,
    frames: Vec<CoroutineFrame>,
}

#[derive(Clone)]
enum CoroutineFrame {
    Bytecode(Box<SuspendedBytecodeFrame>),
    PCall,
    XPCall(SuspendedXPCallFrame),
}

#[derive(Clone)]
struct SuspendedBytecodeFrame {
    program_id: u32,
    prototype: Option<u32>,
    current_function: Option<Value>,
    upvalues: Vec<u32>,
    depth: usize,
    environment: u32,
    call_site: Option<ErrorLocation>,
    pc: usize,
    registers: Vec<Value>,
    register_cells: Vec<Option<u32>>,
    varargs: Vec<Value>,
    pending_return_prefixes: Vec<Value>,
    continuation: BytecodeContinuation,
    stackless_self_calls: Vec<StacklessSelfCallFrame>,
    tail_call_count: usize,
}

#[derive(Clone)]
struct SuspendedXPCallFrame {
    program_id: u32,
    handler: Value,
    depth: usize,
    traceback_base: usize,
}

struct ResumedFunctionState {
    pc: usize,
    registers: Vec<Value>,
    register_cells: Vec<Option<u32>>,
    varargs: Vec<Value>,
    pending_return_prefixes: Vec<Value>,
    continuation: BytecodeContinuation,
    resume_results: Vec<Value>,
    stackless_self_calls: Vec<StacklessSelfCallFrame>,
    tail_call_count: usize,
}

#[derive(Clone)]
struct StacklessSelfCallFrame {
    pc: usize,
    registers: Vec<Value>,
    register_cells: Vec<Option<u32>>,
    varargs: Vec<Value>,
    pending_return_prefixes: Vec<Value>,
    continuation: BytecodeContinuation,
    tail_call_count: usize,
    root_mark: usize,
    debug_mark: StacklessDebugMark,
    debug_frame: StacklessDebugFrame,
}

#[derive(Clone)]
enum BytecodeContinuation {
    WriteFirst {
        register: u8,
    },
    WriteFixed {
        base: usize,
        count: usize,
    },
    PCallWriteFixed {
        base: usize,
        count: usize,
    },
    CallExpandAfterTrailing {
        outer_callee: Value,
        args: Vec<Value>,
        base: usize,
        count: usize,
        depth: usize,
        call_site: Option<ErrorLocation>,
    },
    ReturnWithPrefix {
        prefix: Vec<Value>,
    },
    AppendToTable {
        table: u32,
        start_index: usize,
    },
}

impl Vm {
    pub fn new(profile: Profile) -> Self {
        Self::with_settings(profile, VmSettings::default())
    }

    pub fn with_settings(profile: Profile, settings: VmSettings) -> Self {
        let settings = VmSettings {
            recursion_limit: settings.recursion_limit.min(MAX_RECURSION_LIMIT),
            ..settings
        };
        let mut heap = VmCell::new();
        let globals = heap.alloc_array_table(&[]);
        let enable_loadstring =
            settings.loadstring_compiler.is_some() && !profile.is_deterministic_sim();
        let standard_globals = initialize_standard_globals(
            &mut heap,
            globals,
            profile,
            enable_loadstring,
            settings.enable_conformance_coverage,
            settings.enable_conformance_int64_userdata,
            settings.enable_conformance_native_userdata,
            settings.enable_conformance_native_type_checks,
            settings.enable_conformance_pcall_helpers,
        );
        let rng_state = pcg32_seed(0);
        Self {
            profile,
            settings,
            heap,
            globals,
            standard_globals: standard_globals.standard_globals,
            conformance_int64_metatable: standard_globals.conformance_int64_metatable,
            conformance_vec2_metatable: standard_globals.conformance_vec2_metatable,
            conformance_vertex_metatable: standard_globals.conformance_vertex_metatable,
            extensions: ExtensionRegistry::default(),
            host_functions: Vec::new(),
            dynamic_programs: Vec::new(),
            register_pool: Vec::new(),
            result_pool: Vec::new(),
            call_frame_pool: Vec::new(),
            call_stack: Vec::new(),
            coroutines: Vec::new(),
            running_coroutines: Vec::new(),
            running_coroutine_stack_bases: Vec::new(),
            yield_collectors: Vec::new(),
            last_error_traceback: None,
            protected_error_traceback: None,
            non_yieldable_depth: 0,
            rng_state,
            gcinfo_counter: 0,
            gc_step_counter: 0,
            gc_stopped: false,
            executed_instructions: 0,
            deterministic_collection_count: 0,
            coverage_hits: BTreeMap::new(),
            global_get_inline_cache: vec![None; GLOBAL_GET_INLINE_CACHE_SLOTS],
            table_get_inline_cache: vec![None; GLOBAL_GET_INLINE_CACHE_SLOTS],
            arithmetic_metamethod_inline_cache: vec![None; GLOBAL_GET_INLINE_CACHE_SLOTS],
            iter_metamethod_inline_cache: vec![None; GLOBAL_GET_INLINE_CACHE_SLOTS],
            len_inline_cache: vec![None; GLOBAL_GET_INLINE_CACHE_SLOTS],
            constant_string_atom_cache: BTreeMap::new(),
            string_format_plan_cache: BTreeMap::new(),
        }
    }

    #[inline]
    pub const fn profile(&self) -> Profile {
        self.profile
    }

    #[inline]
    pub const fn settings(&self) -> &VmSettings {
        &self.settings
    }

    pub fn register_extension_op(&mut self, op: ExtensionOp) -> Result<u16> {
        self.extensions.register(self.profile, op)
    }

    pub fn register_host_function(&mut self, op: HostFunctionOp) -> Result<Value> {
        validate_registered_op(self.profile, op.fuel_cost, op.capability)?;
        let id = u32::try_from(self.host_functions.len()).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "host function table exhausted",
            })
        })?;
        let handle = self
            .heap
            .alloc_host_function(id, self.globals)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "host function handle overflow",
                })
            })?;
        self.host_functions.push(op);
        Ok(Value::function(handle))
    }

    pub fn install_host_function(&mut self, op: HostFunctionOp) -> Result<()> {
        let name = op.name.clone();
        let value = self.register_host_function(op)?;
        self.set_global_value(&name, value)
    }

    pub fn string(&mut self, value: impl AsRef<str>) -> Value {
        Value::string_atom(self.heap.intern_string(value.as_ref()))
    }

    pub fn array_table(&mut self, values: &[Value]) -> Value {
        Value::table(self.heap.alloc_array_table(values))
    }

    pub fn empty_table(&mut self) -> Value {
        self.array_table(&[])
    }

    pub fn get_global_value(&mut self, name: impl AsRef<str>) -> Result<Value> {
        let key = self.string(name);
        Ok(self
            .heap
            .table_get(self.globals, key)
            .unwrap_or_else(Value::nil))
    }

    pub fn set_global_value(&mut self, name: impl AsRef<str>, value: Value) -> Result<()> {
        let key = self.string(name);
        self.heap
            .table_set(self.globals, key, value)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "global table write failed",
                })
            })
    }

    pub fn raw_table_get_value(&mut self, table: Value, key: Value) -> Result<Value> {
        let Some(table) = table.as_table() else {
            return Err(Error::new(ErrorKind::TypeError { expected: "table" }));
        };
        if key.kind() == ValueKind::Nil {
            return Err(Error::new(ErrorKind::TableIndexNil));
        }
        if key.as_number().is_some_and(f64::is_nan) {
            return Err(Error::new(ErrorKind::TableIndexNaN));
        }
        self.heap.table_get(table, key).map(Ok).unwrap_or_else(|| {
            if self.heap.has_table(table) {
                Ok(Value::nil())
            } else {
                Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                }))
            }
        })
    }

    pub fn raw_table_set_value(&mut self, table: Value, key: Value, value: Value) -> Result<()> {
        let Some(table) = table.as_table() else {
            return Err(Error::new(ErrorKind::TypeError { expected: "table" }));
        };
        if key.kind() == ValueKind::Nil {
            return Err(Error::new(ErrorKind::TableIndexNil));
        }
        if key.as_number().is_some_and(f64::is_nan) {
            return Err(Error::new(ErrorKind::TableIndexNaN));
        }
        self.heap.table_set(table, key, value).ok_or_else(|| {
            if self.heap.has_table(table) {
                Error::new(ErrorKind::RuntimeError {
                    message: "table write failed".to_owned(),
                })
            } else {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            }
        })
    }

    pub fn raw_table_entries_value(&mut self, table: Value) -> Result<Vec<(Value, Value)>> {
        let Some(table) = table.as_table() else {
            return Err(Error::new(ErrorKind::TypeError { expected: "table" }));
        };
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            }));
        }
        let mut entries = Vec::new();
        let mut after = Value::nil();
        loop {
            match self
                .heap
                .table_next_after(table, after, TableIterationOrder::Insertion)
            {
                Some(TableNext::Entry(key, value)) => {
                    entries.push((key, value));
                    after = key;
                }
                Some(TableNext::End) => return Ok(entries),
                Some(TableNext::InvalidKey) => {
                    return Err(Error::new(ErrorKind::InvalidKeyToNext));
                }
                None => {
                    return Err(Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    }));
                }
            }
        }
    }

    pub fn call_retained_function(
        &mut self,
        function: Value,
        args: &[Value],
    ) -> Result<Vec<Value>> {
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
            Vec::new(),
        );
        let mut fuel = self.settings.initial_fuel;
        self.call_value(&program, function, args, 0, &mut fuel)
    }

    #[inline]
    pub const fn deterministic_collection_count(&self) -> u64 {
        self.deterministic_collection_count
    }

    fn take_registers(&mut self, len: usize) -> Vec<Value> {
        if let Some(index) = self
            .register_pool
            .iter()
            .rposition(|registers| registers.capacity() >= len)
        {
            let mut registers = self.register_pool.swap_remove(index);
            registers.resize(len, Value::nil());
            return registers;
        }
        vec![Value::nil(); len]
    }

    fn recycle_registers(&mut self, mut registers: Vec<Value>) {
        if registers.capacity() > REGISTER_POOL_MAX_REGISTERS
            || self.register_pool.len() >= REGISTER_POOL_MAX_FRAMES
        {
            return;
        }
        registers.clear();
        self.register_pool.push(registers);
    }

    fn take_results(&mut self, len: usize) -> Vec<Value> {
        if len == 0 {
            return Vec::new();
        }
        if let Some(index) = self
            .result_pool
            .iter()
            .rposition(|results| results.capacity() >= len)
        {
            return self.result_pool.swap_remove(index);
        }
        Vec::with_capacity(len)
    }

    fn recycle_results(&mut self, mut results: Vec<Value>) {
        if results.capacity() > RESULT_POOL_MAX_VALUES
            || self.result_pool.len() >= RESULT_POOL_MAX_FRAMES
        {
            return;
        }
        results.clear();
        self.result_pool.push(results);
    }

    fn first_result_or_nil_recycle(&mut self, results: Vec<Value>) -> Value {
        let value = results.first().copied().unwrap_or_else(Value::nil);
        self.recycle_results(results);
        value
    }

    fn take_call_stack_frame(
        &mut self,
        function: Option<Value>,
        debug_name: Option<Arc<str>>,
        environment: u32,
        call_site: Option<ErrorLocation>,
    ) -> CallStackFrame {
        if let Some(mut frame) = self.call_frame_pool.pop() {
            frame.function = function;
            frame.debug_name = debug_name;
            frame.environment = environment;
            frame.call_site = call_site;
            frame.roots.clear();
            frame.stackless_roots.clear();
            frame.stackless_debug_frames.clear();
            return frame;
        }
        CallStackFrame {
            function,
            debug_name,
            environment,
            call_site,
            roots: Vec::new(),
            stackless_roots: Vec::new(),
            stackless_debug_frames: Vec::new(),
        }
    }

    fn recycle_call_stack_frame(&mut self, mut frame: CallStackFrame) {
        if self.call_frame_pool.len() >= CALL_FRAME_POOL_MAX_FRAMES
            || frame.roots.capacity() > CALL_FRAME_POOL_MAX_ROOTS
            || frame.stackless_roots.capacity() > CALL_FRAME_POOL_MAX_ROOTS
            || frame.stackless_debug_frames.capacity() > CALL_FRAME_POOL_MAX_STACKLESS_DEBUG_FRAMES
        {
            return;
        }
        frame.function = None;
        frame.debug_name = None;
        frame.environment = self.globals;
        frame.call_site = None;
        frame.roots.clear();
        frame.stackless_roots.clear();
        frame.stackless_debug_frames.clear();
        self.call_frame_pool.push(frame);
    }

    fn finish_function_results_from_registers(
        &mut self,
        pending_prefixes: &mut Vec<Value>,
        registers: &[Value],
        first: usize,
        count: usize,
    ) -> Vec<Value> {
        if pending_prefixes.is_empty() {
            let mut results = self.take_results(count);
            results.extend_from_slice(&registers[first..first + count]);
            return results;
        }
        pending_prefixes.extend_from_slice(&registers[first..first + count]);
        std::mem::take(pending_prefixes)
    }

    pub fn run(&mut self, program: &Program) -> Result<Value> {
        let mut fuel = self.settings.initial_fuel;
        let program_id = self.retain_dynamic_program(program.clone())?;
        let results = self
            .run_function(
                program,
                Some(program_id),
                &program.main,
                &[],
                FunctionCallContext {
                    current_function: None,
                    upvalues: &[],
                    depth: 0,
                    environment: self.globals,
                    call_site: None,
                    prototype: None,
                },
                None,
                &mut fuel,
            )
            .map_err(|error| attach_existing_instruction_source(program, &program.main, error))?;
        let VmFlow::Return(results) = results else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "attempt to yield across metamethod/C-call boundary".to_owned(),
            }));
        };
        Ok(first_result_or_nil(results))
    }

    pub fn call_function(
        &mut self,
        program: &Program,
        function: Value,
        args: &[Value],
    ) -> Result<Value> {
        let mut fuel = self.settings.initial_fuel;
        let results = self.call_value(program, function, args, 0, &mut fuel)?;
        Ok(first_result_or_nil(results))
    }

    fn run_function(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        function: &crate::bytecode::Function,
        args: &[Value],
        context: FunctionCallContext<'_>,
        resumed: Option<ResumedFunctionState>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        let call_frame = self.take_call_stack_frame(
            context.current_function,
            function.debug_name.clone(),
            context.environment,
            context.call_site.clone(),
        );
        self.call_stack.push(call_frame);
        let result = (|| -> VmFlowResult<Vec<Value>> {
            self.check_recursion_depth(context.depth)?;
            if function.max_registers == 0 {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "main function must have at least one register",
                }));
            }
            if function.parameter_count > function.max_registers
                || function.parameter_count > usize::from(u8::MAX) + 1
            {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "function parameter count exceeds register file",
                }));
            }

            let allow_stackless_self_calls = resumed.is_none();
            let mut stackless_self_calls;
            let mut tail_call_count;
            let (
                mut registers,
                mut register_cells,
                mut varargs,
                mut pc,
                mut pending_return_prefixes,
                mut resumed_continuation,
            ) = if let Some(resumed) = resumed {
                stackless_self_calls = resumed.stackless_self_calls;
                tail_call_count = resumed.tail_call_count;
                (
                    resumed.registers,
                    resumed.register_cells,
                    resumed.varargs,
                    resumed.pc,
                    resumed.pending_return_prefixes,
                    Some((resumed.continuation, resumed.resume_results)),
                )
            } else {
                stackless_self_calls = Vec::new();
                tail_call_count = 0usize;
                let mut registers = self.take_registers(function.max_registers);
                let register_cells = fresh_register_cells(function);
                for (index, arg) in args
                    .iter()
                    .copied()
                    .take(function.parameter_count)
                    .enumerate()
                {
                    let register = u8::try_from(index).map_err(|_| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "function parameter register index overflow",
                        })
                    })?;
                    set_register(&mut registers, register, arg)?;
                }
                let varargs = if function.is_vararg && args.len() > function.parameter_count {
                    args[function.parameter_count..].to_vec()
                } else {
                    Vec::new()
                };
                self.validate_conformance_native_parameter_types(function, &registers)?;
                (registers, register_cells, varargs, 0usize, Vec::new(), None)
            };
            let mut may_have_register_cells = register_cells.iter().any(Option::is_some);
            self.rebuild_stackless_self_call_roots(&mut stackless_self_calls);
            let mut current_roots_dirty = true;

            macro_rules! unwind_stackless_return {
                ($returned:expr) => {{
                    self.recycle_registers(std::mem::take(&mut registers));
                    let mut returned = $returned;
                    loop {
                        let Some(frame) = stackless_self_calls.pop() else {
                            return Ok(VmFlow::Return(returned));
                        };
                        let StacklessSelfCallFrame {
                            pc: frame_pc,
                            registers: frame_registers,
                            register_cells: frame_register_cells,
                            varargs: frame_varargs,
                            pending_return_prefixes: frame_pending_return_prefixes,
                            continuation,
                            tail_call_count: frame_tail_call_count,
                            root_mark,
                            debug_mark,
                            debug_frame: _,
                        } = frame;
                        pc = frame_pc;
                        registers = frame_registers;
                        register_cells = frame_register_cells;
                        may_have_register_cells = register_cells.iter().any(Option::is_some);
                        varargs = frame_varargs;
                        pending_return_prefixes = frame_pending_return_prefixes;
                        tail_call_count = frame_tail_call_count;
                        self.truncate_stackless_self_call_debug_frames(debug_mark);
                        self.update_current_gc_roots(
                            &registers,
                            &varargs,
                            &pending_return_prefixes,
                        );
                        let continuation_result = self.apply_bytecode_continuation(
                            program,
                            &mut registers,
                            &mut pending_return_prefixes,
                            continuation,
                            returned,
                            fuel,
                        );
                        self.truncate_stackless_self_call_roots(root_mark);
                        current_roots_dirty = true;
                        if let Some(next_returned) = continuation_result? {
                            returned = next_returned;
                            continue;
                        }
                        break;
                    }
                }};
            }

            macro_rules! unwind_stackless_error {
                ($error:expr) => {{
                    let error = $error;
                    if error_is_uncatchable(&error) {
                        self.refresh_current_gc_roots_if_dirty(
                            &mut current_roots_dirty,
                            &registers,
                            &varargs,
                            &pending_return_prefixes,
                        );
                        return Err(error);
                    }
                    let mut error = Some(error);
                    let mut caught = false;
                    while let Some(frame) = stackless_self_calls.pop() {
                        let StacklessSelfCallFrame {
                            pc: frame_pc,
                            registers: frame_registers,
                            register_cells: frame_register_cells,
                            varargs: frame_varargs,
                            pending_return_prefixes: frame_pending_return_prefixes,
                            continuation,
                            tail_call_count: frame_tail_call_count,
                            root_mark,
                            debug_mark,
                            debug_frame: _,
                        } = frame;
                        match continuation {
                            BytecodeContinuation::PCallWriteFixed { base, count } => {
                                pc = frame_pc;
                                registers = frame_registers;
                                register_cells = frame_register_cells;
                                may_have_register_cells =
                                    register_cells.iter().any(Option::is_some);
                                varargs = frame_varargs;
                                pending_return_prefixes = frame_pending_return_prefixes;
                                tail_call_count = frame_tail_call_count;
                                self.truncate_stackless_self_call_debug_frames(debug_mark);
                                self.update_current_gc_roots(
                                    &registers,
                                    &varargs,
                                    &pending_return_prefixes,
                                );
                                self.last_error_traceback = None;
                                let error_value =
                                    self.error_object(error.take().ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "missing stackless protected-call error",
                                        })
                                    })?);
                                write_call_results(
                                    &mut registers,
                                    base,
                                    count,
                                    &[Value::boolean(false), error_value],
                                )?;
                                self.truncate_stackless_self_call_roots(root_mark);
                                caught = true;
                                current_roots_dirty = true;
                                break;
                            }
                            _ => {
                                self.truncate_stackless_self_call_debug_frames(debug_mark);
                                self.truncate_stackless_self_call_roots(root_mark);
                            }
                        }
                    }
                    if !caught {
                        self.refresh_current_gc_roots_if_dirty(
                            &mut current_roots_dirty,
                            &registers,
                            &varargs,
                            &pending_return_prefixes,
                        );
                        return Err(error.ok_or_else(|| {
                            Error::new(ErrorKind::InvalidProgram {
                                reason: "missing propagated stackless error",
                            })
                        })?);
                    }
                    self.refresh_current_gc_roots_if_dirty(
                        &mut current_roots_dirty,
                        &registers,
                        &varargs,
                        &pending_return_prefixes,
                    );
                }};
            }

            'run_function_loop: loop {
                if let Some((continuation, resume_results)) = resumed_continuation.take() {
                    self.refresh_current_gc_roots_if_dirty(
                        &mut current_roots_dirty,
                        &registers,
                        &varargs,
                        &pending_return_prefixes,
                    );
                    let continuation_root_mark =
                        self.push_bytecode_continuation_roots(&continuation);
                    let continuation_result = self.apply_bytecode_continuation(
                        program,
                        &mut registers,
                        &mut pending_return_prefixes,
                        continuation,
                        resume_results,
                        fuel,
                    );
                    self.truncate_stackless_self_call_roots(continuation_root_mark);
                    current_roots_dirty = true;
                    match continuation_result {
                        Ok(Some(results)) => {
                            unwind_stackless_return!(results);
                            continue 'run_function_loop;
                        }
                        Ok(None) => {}
                        Err(error) => {
                            unwind_stackless_error!(error);
                            continue 'run_function_loop;
                        }
                    }
                }

                let loop_result = (|| -> VmFlowResult<Vec<Value>> {
                    loop {
                        let instruction = *function.code.get(pc).ok_or_else(|| {
                            Error::new(ErrorKind::InvalidProgram {
                                reason: "program counter ran off the end of the function",
                            })
                            .at_pc(pc)
                        })?;

                        let fuel_cost = self
                            .instruction_fuel_cost(instruction)
                            .map_err(|e| e.at_pc(pc))?;
                        charge_fuel(fuel, self.settings.initial_fuel, fuel_cost)
                            .map_err(|error| error.at_pc(pc))?;
                        self.executed_instructions = self.executed_instructions.wrapping_add(1);
                        self.maybe_collect(
                            &mut current_roots_dirty,
                            &registers,
                            &varargs,
                            &pending_return_prefixes,
                        );
                        if opcode_may_observe_current_roots(instruction.op) {
                            self.refresh_current_gc_roots_if_dirty(
                                &mut current_roots_dirty,
                                &registers,
                                &varargs,
                                &pending_return_prefixes,
                            );
                        }
                        self.record_coverage_hit(program_id, context.prototype, function, pc);

                        pc += 1;
                        current_roots_dirty = true;
                        match instruction.op {
                            Opcode::Halt => {
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    vec![Value::nil()],
                                )))
                            }
                            Opcode::LoadConst => {
                                let value = self.load_constant(program, instruction.c as usize)?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::LoadConstWide => {
                                if instruction.aux < 0 {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "negative wide constant index",
                                    })
                                    .at_pc(pc - 1));
                                }
                                let value =
                                    self.load_constant(program, instruction.aux as usize)?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::GetGlobal => {
                                let site = program_id.map(|program| InlineCacheSite {
                                    program,
                                    prototype: context.prototype,
                                    pc: pc - 1,
                                });
                                let value = self
                                    .get_global_with_inline_cache(
                                        program,
                                        site,
                                        instruction.aux,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::SetGlobal => {
                                let key = self.global_key(program, instruction.aux)?;
                                let environment =
                                    self.current_environment().map_err(|e| e.at_pc(pc - 1))?;
                                let value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                self.set_table_index_with_metamethod(
                                    program,
                                    environment,
                                    key,
                                    value,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Move => {
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::GetLocalCell => {
                                let value = self
                                    .get_register_cell_value(
                                        &registers,
                                        &mut register_cells,
                                        instruction.b,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                may_have_register_cells = true;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::SetLocalCell => {
                                let value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                self.set_register_cell_value(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction.b,
                                    value,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                                may_have_register_cells = true;
                            }
                            Opcode::ClearLocalCell => {
                                clear_register_value_and_cell(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction.a,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ClearLocalCells => {
                                clear_register_values_and_cells(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction.a,
                                    instruction.b,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::GetUpvalue => {
                                if fuel.is_none() {
                                    if let Some(next) = function.code.get(pc) {
                                        if next.op == Opcode::Len && next.b == instruction.a {
                                            let index = upvalue_index(instruction.aux)
                                                .map_err(|e| e.at_pc(pc - 1))?;
                                            let upvalue = context
                                                .upvalues
                                                .get(index)
                                                .copied()
                                                .ok_or_else(|| {
                                                    Error::new(ErrorKind::InvalidProgram {
                                                        reason: "upvalue index is missing",
                                                    })
                                                })
                                                .map_err(|e| e.at_pc(pc - 1))?;
                                            let value = self.heap.upvalue_get(upvalue).ok_or_else(
                                                || {
                                                    Error::new(ErrorKind::InvalidProgram {
                                                        reason: "upvalue handle is missing",
                                                    })
                                                },
                                            )?;
                                            if let (Some(program), Some(table)) =
                                                (program_id, value.as_table())
                                            {
                                                let site = InlineCacheSite {
                                                    program,
                                                    prototype: context.prototype,
                                                    pc,
                                                };
                                                if let Some(len) =
                                                    self.len_inline_cache_hit(site, table)
                                                {
                                                    if next.a != instruction.a {
                                                        set_register(
                                                            &mut registers,
                                                            instruction.a,
                                                            value,
                                                        )
                                                        .map_err(|e| e.at_pc(pc - 1))?;
                                                    }
                                                    set_register(
                                                        &mut registers,
                                                        next.a,
                                                        Value::number(len as f64),
                                                    )
                                                    .map_err(|e| e.at_pc(pc))?;
                                                    self.executed_instructions =
                                                        self.executed_instructions.wrapping_add(1);
                                                    self.record_coverage_hit(
                                                        program_id,
                                                        context.prototype,
                                                        function,
                                                        pc,
                                                    );
                                                    pc += 1;
                                                    continue;
                                                }
                                            }
                                            set_register(&mut registers, instruction.a, value)
                                                .map_err(|e| e.at_pc(pc - 1))?;
                                            continue;
                                        }
                                        if next.op == Opcode::GetUpvalue {
                                            let first_index = upvalue_index(instruction.aux)
                                                .map_err(|e| e.at_pc(pc - 1))?;
                                            let second_index =
                                                upvalue_index(next.aux).map_err(|e| e.at_pc(pc))?;
                                            let first_upvalue = context
                                                .upvalues
                                                .get(first_index)
                                                .copied()
                                                .ok_or_else(|| {
                                                    Error::new(ErrorKind::InvalidProgram {
                                                        reason: "upvalue index is missing",
                                                    })
                                                })
                                                .map_err(|e| e.at_pc(pc - 1))?;
                                            let second_upvalue = context
                                                .upvalues
                                                .get(second_index)
                                                .copied()
                                                .ok_or_else(|| {
                                                    Error::new(ErrorKind::InvalidProgram {
                                                        reason: "upvalue index is missing",
                                                    })
                                                })
                                                .map_err(|e| e.at_pc(pc))?;
                                            let (first_value, second_value) = self
                                                .heap
                                                .upvalue_get_pair(first_upvalue, second_upvalue)
                                                .ok_or_else(|| {
                                                    Error::new(ErrorKind::InvalidProgram {
                                                        reason: "upvalue handle is missing",
                                                    })
                                                })
                                                .map_err(|e| e.at_pc(pc - 1))?;
                                            set_register(
                                                &mut registers,
                                                instruction.a,
                                                first_value,
                                            )
                                            .map_err(|e| e.at_pc(pc - 1))?;
                                            set_register(&mut registers, next.a, second_value)
                                                .map_err(|e| e.at_pc(pc))?;
                                            self.executed_instructions =
                                                self.executed_instructions.wrapping_add(1);
                                            self.record_coverage_hit(
                                                program_id,
                                                context.prototype,
                                                function,
                                                pc,
                                            );
                                            pc += 1;
                                            continue;
                                        }
                                    }
                                }
                                let upvalue = context
                                    .upvalues
                                    .get(upvalue_index(instruction.aux)?)
                                    .copied()
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "upvalue index is missing",
                                        })
                                    })
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = self.heap.upvalue_get(upvalue).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "upvalue handle is missing",
                                    })
                                })?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::SetUpvalue => {
                                let upvalue_index = upvalue_index(instruction.aux)?;
                                let upvalue = context
                                    .upvalues
                                    .get(upvalue_index)
                                    .copied()
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "upvalue index is missing",
                                        })
                                    })
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                self.heap.upvalue_set(upvalue, value).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "upvalue handle is missing",
                                    })
                                })?;
                                if fuel.is_none() {
                                    if let Some(register) = same_upvalue_reload_after_set(
                                        function,
                                        &context.upvalues,
                                        pc,
                                        upvalue,
                                    ) {
                                        set_register(&mut registers, register, value)
                                            .map_err(|e| e.at_pc(pc))?;
                                        self.executed_instructions =
                                            self.executed_instructions.wrapping_add(1);
                                        self.record_coverage_hit(
                                            program_id,
                                            context.prototype,
                                            function,
                                            pc,
                                        );
                                        pc += 1;
                                        continue;
                                    }
                                    if set_upvalue_refresh_skip_count(
                                        function,
                                        &context.upvalues,
                                        &register_cells,
                                        pc,
                                        instruction.a,
                                        upvalue,
                                    ) == 2
                                    {
                                        self.executed_instructions =
                                            self.executed_instructions.wrapping_add(2);
                                        self.record_coverage_hit(
                                            program_id,
                                            context.prototype,
                                            function,
                                            pc,
                                        );
                                        self.record_coverage_hit(
                                            program_id,
                                            context.prototype,
                                            function,
                                            pc + 1,
                                        );
                                        pc += 2;
                                        continue;
                                    }
                                }
                            }
                            Opcode::AddUpvalue => {
                                let (upvalue, value) = self
                                    .add_upvalue(
                                        program,
                                        &context.upvalues,
                                        &mut registers,
                                        instruction,
                                        program_id.map(|program| InlineCacheSite {
                                            program,
                                            prototype: context.prototype,
                                            pc: pc - 1,
                                        }),
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                if fuel.is_none() {
                                    if let Some(register) = same_upvalue_reload_after_set(
                                        function,
                                        &context.upvalues,
                                        pc,
                                        upvalue,
                                    ) {
                                        set_register(&mut registers, register, value)
                                            .map_err(|e| e.at_pc(pc))?;
                                        self.executed_instructions =
                                            self.executed_instructions.wrapping_add(1);
                                        self.record_coverage_hit(
                                            program_id,
                                            context.prototype,
                                            function,
                                            pc,
                                        );
                                        pc += 1;
                                        continue;
                                    }
                                }
                            }
                            Opcode::AddUpvalueConst => {
                                let (upvalue, value) = self
                                    .add_upvalue_const(
                                        program,
                                        &context.upvalues,
                                        &mut registers,
                                        instruction,
                                        program_id.map(|program| InlineCacheSite {
                                            program,
                                            prototype: context.prototype,
                                            pc: pc - 1,
                                        }),
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                if fuel.is_none() {
                                    if let Some(register) = same_upvalue_reload_after_set(
                                        function,
                                        &context.upvalues,
                                        pc,
                                        upvalue,
                                    ) {
                                        set_register(&mut registers, register, value)
                                            .map_err(|e| e.at_pc(pc))?;
                                        self.executed_instructions =
                                            self.executed_instructions.wrapping_add(1);
                                        self.record_coverage_hit(
                                            program_id,
                                            context.prototype,
                                            function,
                                            pc,
                                        );
                                        pc += 1;
                                        continue;
                                    }
                                }
                            }
                            Opcode::Add => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Add,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::AddConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Add,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Sub => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Sub,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::SubConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Sub,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Mul => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Mul,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MulConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Mul,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Div => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Div,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::DivConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Div,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::FloorDiv => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::FloorDiv,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::FloorDivConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::FloorDiv,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Mod => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Mod,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ModConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Mod,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MulModConst => {
                                self.numeric_mul_mod_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MulAddModConst => {
                                self.numeric_mul_add_mod_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::SetIndexMulAddModConst => {
                                self.set_index_mul_add_mod_const(
                                    program,
                                    &registers,
                                    instruction,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                            }
                            Opcode::SetIndexMulModConst => {
                                self.set_index_mul_mod_const(
                                    program,
                                    &registers,
                                    instruction,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                            }
                            Opcode::ModAddConst => {
                                self.numeric_mod_add_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Pow => {
                                self.numeric_binop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Pow,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::PowConst => {
                                self.numeric_binop_const(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Pow,
                                    program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }),
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Concat => {
                                let value = self
                                    .concat_value(
                                        program,
                                        &registers,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ConcatConst => {
                                let value = self
                                    .concat_const_value(
                                        program,
                                        program_id,
                                        &registers,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ConcatRhsConst => {
                                let value = self
                                    .concat_rhs_const_value(
                                        program,
                                        program_id,
                                        &registers,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::GetIndexConcatConst => {
                                let object =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = self
                                    .concat_const_key(
                                        program,
                                        program_id,
                                        &registers,
                                        instruction.c,
                                        instruction.aux,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                if let Some(table) = object.as_table() {
                                    if let Some(index) =
                                        key.as_number().and_then(array_iterator_index)
                                    {
                                        if let Some(value) = self
                                            .get_table_integer_index_fast(table, index)
                                            .map_err(|e| {
                                                attach_instruction_source(
                                                    program,
                                                    function,
                                                    pc - 1,
                                                    e,
                                                )
                                            })?
                                        {
                                            set_register(&mut registers, instruction.a, value)
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?;
                                            continue;
                                        }
                                    }
                                    if key.kind() == ValueKind::String {
                                        if let Some(value) = self
                                            .get_table_string_index_fast(table, key)
                                            .map_err(|e| {
                                                attach_instruction_source(
                                                    program,
                                                    function,
                                                    pc - 1,
                                                    e,
                                                )
                                            })?
                                        {
                                            set_register(&mut registers, instruction.a, value)
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?;
                                            continue;
                                        }
                                    }
                                }
                                let value = self
                                    .get_index_value(program, object, key, context.depth, fuel)
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::Neg => {
                                self.numeric_unop(
                                    program,
                                    &mut registers,
                                    instruction,
                                    NumericOp::Neg,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Abs => {
                                let value = self
                                    .coerce_register_to_number(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::number(binary(NumericOp::Abs, value, 0.0)),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Not => {
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::boolean(!value.truthy()),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Jump => {
                                pc = checked_jump(instruction.aux).map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::JumpIfNot => {
                                let condition = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                if !condition.truthy() {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::JumpIf => {
                                let condition = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                if condition.truthy() {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::Return => {
                                let count = usize::from(instruction.b);
                                let first = usize::from(instruction.a);
                                let end = first.checked_add(count).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "return register window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                if end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let returned = self.finish_function_results_from_registers(
                                    &mut pending_return_prefixes,
                                    &registers,
                                    first,
                                    count,
                                );
                                unwind_stackless_return!(returned);
                                continue;
                            }
                            Opcode::ReturnCall => {
                                let prefix_count = usize::from(instruction.b);
                                let prefix_first = usize::from(instruction.a);
                                let prefix_end =
                                    prefix_first.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let (args_start, argc) = unpack_return_call_aux(instruction.aux)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let callee =
                                    get_register(&registers, instruction.c).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let args_start = usize::from(args_start);
                                let args_end =
                                    args_start.checked_add(usize::from(argc)).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "tail call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if args_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: args_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let current_function =
                                    context.current_function.and_then(Value::as_function);
                                if prefix_count == 0
                                    && usize::from(argc) == 1
                                    && self.value_is_native_function(callee, NativeFunction::PCall)
                                    && current_function.is_some()
                                    && get_register(
                                        &registers,
                                        u8::try_from(args_start).map_err(|_| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason:
                                                    "tail call argument register index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?
                                    .as_function()
                                        == current_function
                                {
                                    let result_count =
                                        (LUAU_TAIL_CALL_LIMIT.saturating_sub(context.depth) / 2)
                                            .max(2);
                                    let mut results = Vec::with_capacity(result_count);
                                    results.resize(result_count - 2, Value::boolean(true));
                                    results.push(Value::boolean(false));
                                    let error = attach_instruction_source(
                                        program,
                                        function,
                                        pc - 1,
                                        Error::new(ErrorKind::RecursionLimit {
                                            limit: LUAU_TAIL_CALL_LIMIT,
                                        }),
                                    );
                                    let error = self.error_object(error);
                                    results.push(error);
                                    return Ok(VmFlow::Return(finish_function_results(
                                        &mut pending_return_prefixes,
                                        results,
                                    )));
                                }
                                if current_function.is_some()
                                    && current_function == callee.as_function()
                                {
                                    tail_call_count = tail_call_count.saturating_add(1);
                                    if context.depth.saturating_add(tail_call_count)
                                        > LUAU_TAIL_CALL_LIMIT
                                    {
                                        return Err(attach_instruction_source(
                                            program,
                                            function,
                                            pc - 1,
                                            Error::new(ErrorKind::RecursionLimit {
                                                limit: LUAU_TAIL_CALL_LIMIT,
                                            }),
                                        ));
                                    }
                                    if prefix_count > 0 {
                                        pending_return_prefixes.extend_from_slice(
                                            &registers[prefix_first..prefix_end],
                                        );
                                    }
                                    let tail_args = registers[args_start..args_end].to_vec();
                                    reset_current_function_frame(
                                        function,
                                        &mut registers,
                                        &mut register_cells,
                                        &mut varargs,
                                        &tail_args,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                    pc = 0;
                                    continue;
                                }
                                let synthetic_call = Instruction::with_aux(
                                    Opcode::Call,
                                    0,
                                    instruction.c,
                                    u8::try_from(args_start).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "tail call argument register index overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?,
                                    i32::from(argc),
                                );
                                let mut results = registers[prefix_first..prefix_end].to_vec();
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let mut call_results = match self
                                    .call_function_resumable(
                                        program,
                                        &registers,
                                        synthetic_call,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        let continuation = BytecodeContinuation::ReturnWithPrefix {
                                            prefix: registers[prefix_first..prefix_end].to_vec(),
                                        };
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            continuation,
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                results.append(&mut call_results);
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::CallExtension => {
                                let result = self
                                    .call_extension(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, result)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Assert => {
                                let value =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                if !value.truthy() {
                                    let message = self
                                        .assert_message(&registers, instruction)
                                        .map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?;
                                    return Err(attach_instruction_source(
                                        program,
                                        function,
                                        pc - 1,
                                        Error::new(ErrorKind::AssertionFailed { message }),
                                    ));
                                }
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::Equal => {
                                self.compare_equal(
                                    program,
                                    &mut registers,
                                    instruction,
                                    false,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::NotEqual => {
                                self.compare_equal(
                                    program,
                                    &mut registers,
                                    instruction,
                                    true,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Less => {
                                self.compare_order(
                                    program,
                                    &mut registers,
                                    instruction,
                                    OrderComparison::Less,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::LessEqual => {
                                self.compare_order(
                                    program,
                                    &mut registers,
                                    instruction,
                                    OrderComparison::LessEqual,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Greater => {
                                self.compare_order(
                                    program,
                                    &mut registers,
                                    instruction,
                                    OrderComparison::Greater,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::GreaterEqual => {
                                self.compare_order(
                                    program,
                                    &mut registers,
                                    instruction,
                                    OrderComparison::GreaterEqual,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ForPrep => {
                                if !numeric_for_prepare(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::ForPrepStepOne => {
                                if !numeric_for_step_one_prepare(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::ForLoop => {
                                if numeric_for_loop(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::ForLoopStepOne => {
                                if numeric_for_step_one_loop(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::ToNumber => {
                                let value =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let number = self
                                    .coerce_to_number(value)
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?
                                    .map_or_else(Value::nil, Value::number);
                                set_register(&mut registers, instruction.a, number).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::CoerceForNumber => {
                                let value =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let Some(number) = self.coerce_to_number(value).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?
                                else {
                                    return Err(attach_instruction_source(
                                        program,
                                        function,
                                        pc - 1,
                                        numeric_for_type_error(
                                            value,
                                            numeric_for_value_name(instruction.c),
                                        ),
                                    ));
                                };
                                set_register(&mut registers, instruction.a, Value::number(number))
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                            }
                            Opcode::NewArray => {
                                let first = instruction.b as usize;
                                let len = instruction.c as usize;
                                let end = first.checked_add(len).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "array literal register window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                if end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let (array_capacity_hint, hash_capacity_hint) =
                                    new_array_capacity_hints(instruction.aux);
                                let table = self.heap.alloc_array_table_with_capacity(
                                    &registers[first..end],
                                    array_capacity_hint,
                                    hash_capacity_hint,
                                );
                                set_register(&mut registers, instruction.a, Value::table(table))
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::NewRecord => {
                                let value = self
                                    .new_record_table(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::IpairsPrep => {
                                if !self.ipairs_prepare(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::IpairsLoop => {
                                if self.ipairs_loop(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::PairsPrep => {
                                if !self.pairs_prepare(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::PairsLoop => {
                                if self.pairs_loop(
                                    &mut registers,
                                    &mut register_cells,
                                    instruction,
                                    pc - 1,
                                )? {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::GenericCallPrep => {
                                if !self
                                    .generic_call_iterator_step(
                                        program,
                                        &mut registers,
                                        &mut register_cells,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?
                                {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::GenericCallLoop => {
                                if self
                                    .generic_call_iterator_step(
                                        program,
                                        &mut registers,
                                        &mut register_cells,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?
                                {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::GeneralizedIterPrep => {
                                let site = program_id.map(|program| InlineCacheSite {
                                    program,
                                    prototype: context.prototype,
                                    pc: pc - 1,
                                });
                                if !self
                                    .generalized_iterator_prepare(
                                        program,
                                        &mut registers,
                                        &mut register_cells,
                                        instruction,
                                        context.depth,
                                        site,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?
                                {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::GeneralizedIterLoop => {
                                if self
                                    .generalized_iterator_loop(
                                        program,
                                        &mut registers,
                                        &mut register_cells,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?
                                {
                                    pc = checked_jump(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::Len => {
                                let site = program_id.map(|program| InlineCacheSite {
                                    program,
                                    prototype: context.prototype,
                                    pc: pc - 1,
                                });
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let len = self
                                    .len_value_with_inline_cache(
                                        program,
                                        site,
                                        value,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, len)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::LenPlusOne => {
                                let site = program_id.map(|program| InlineCacheSite {
                                    program,
                                    prototype: context.prototype,
                                    pc: pc - 1,
                                });
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let key = self
                                    .len_plus_one_value(program, site, value, context.depth, fuel)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, key)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::GetIndex => {
                                let object =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = get_register(&registers, instruction.c).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                if let Some(table) = object.as_table() {
                                    if let Some(index) =
                                        key.as_number().and_then(array_iterator_index)
                                    {
                                        if let Some(value) = self
                                            .get_table_integer_index_fast(table, index)
                                            .map_err(|e| {
                                                attach_instruction_source(
                                                    program,
                                                    function,
                                                    pc - 1,
                                                    e,
                                                )
                                            })?
                                        {
                                            set_register(&mut registers, instruction.a, value)
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?;
                                            continue;
                                        }
                                    }
                                    if let Some(key_atom) = key.as_string_atom() {
                                        if let Some(site) = static_string_get_index_site(
                                            program_id,
                                            context.prototype,
                                            program,
                                            function,
                                            pc - 1,
                                            instruction.c,
                                        ) {
                                            let value = self
                                                .get_table_string_index_with_inline_cache(
                                                    program,
                                                    site,
                                                    table,
                                                    key,
                                                    key_atom,
                                                    context.depth,
                                                    fuel,
                                                )
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?;
                                            set_register(&mut registers, instruction.a, value)
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?;
                                            continue;
                                        }
                                    }
                                }
                                let value = self
                                    .get_index_value(program, object, key, context.depth, fuel)
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::GetIndexStringConst => {
                                if instruction.aux < 0 {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "negative string table index constant",
                                    })
                                    .at_pc(pc - 1));
                                }
                                let object =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key_atom = self
                                    .constant_string_atom(
                                        program,
                                        program_id,
                                        instruction.aux as usize,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason:
                                                "string table index opcode must reference a string",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let key = Value::string_atom(key_atom);
                                let value = if let Some(table) = object.as_table() {
                                    if let Some(site) = program_id.map(|program| InlineCacheSite {
                                        program,
                                        prototype: context.prototype,
                                        pc: pc - 1,
                                    }) {
                                        self.get_table_string_index_with_inline_cache(
                                            program,
                                            site,
                                            table,
                                            key,
                                            key_atom,
                                            context.depth,
                                            fuel,
                                        )
                                        .map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?
                                    } else {
                                        self.get_table_index_with_metamethod(
                                            program,
                                            table,
                                            key,
                                            context.depth,
                                            fuel,
                                        )
                                        .map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?
                                    }
                                } else {
                                    self.get_index_value(program, object, key, context.depth, fuel)
                                        .map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?
                                };
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::GetIndexNumber => {
                                let object =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = get_register(&registers, instruction.c).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                if let Some(number) = key.as_number() {
                                    if let Some(table) = object.as_table() {
                                        if let Some(index) = array_iterator_index(number) {
                                            if let Some(value) = self
                                                .get_table_integer_index_fast(table, index)
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?
                                            {
                                                set_register(&mut registers, instruction.a, value)
                                                    .map_err(|e| {
                                                        attach_instruction_source(
                                                            program,
                                                            function,
                                                            pc - 1,
                                                            e,
                                                        )
                                                    })?;
                                                continue;
                                            }
                                        }
                                    }
                                }
                                let value = self
                                    .get_index_value(program, object, key, context.depth, fuel)
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::GetIndexNumberConst => {
                                let object =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let Some(index) = array_iterator_index(f64::from(instruction.aux))
                                else {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "invalid constant numeric table index",
                                    })
                                    .at_pc(pc - 1));
                                };
                                if let Some(table) = object.as_table() {
                                    if let Some(value) = self
                                        .get_table_integer_index_fast(table, index)
                                        .map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?
                                    {
                                        set_register(&mut registers, instruction.a, value)
                                            .map_err(|e| {
                                                attach_instruction_source(
                                                    program,
                                                    function,
                                                    pc - 1,
                                                    e,
                                                )
                                            })?;
                                        continue;
                                    }
                                }
                                let key = Value::number(f64::from(instruction.aux));
                                let value = self
                                    .get_index_value(program, object, key, context.depth, fuel)
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::GetIndexModAddConst => {
                                let value = self
                                    .get_index_mod_add_const(
                                        program,
                                        &registers,
                                        instruction,
                                        program_id.map(|program| InlineCacheSite {
                                            program,
                                            prototype: context.prototype,
                                            pc: pc - 1,
                                        }),
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::GetIndexMulModConst => {
                                let value = self
                                    .get_index_mul_mod_const(
                                        program,
                                        &registers,
                                        instruction,
                                        program_id.map(|program| InlineCacheSite {
                                            program,
                                            prototype: context.prototype,
                                            pc: pc - 1,
                                        }),
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                set_register(&mut registers, instruction.a, value).map_err(
                                    |e| attach_instruction_source(program, function, pc - 1, e),
                                )?;
                            }
                            Opcode::CheckMethod => {
                                let callee =
                                    get_register(&registers, instruction.a).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let object =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = get_register(&registers, instruction.c).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                self.check_method_value(callee, object, key).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                            }
                            Opcode::RawGet => {
                                let table = get_table(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let key = get_register(&registers, instruction.c)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = self.raw_table_get_after_table_lookup(table, key);
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::SetIndex => {
                                let object =
                                    get_register(&registers, instruction.a).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = get_register(&registers, instruction.b).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                let value =
                                    get_register(&registers, instruction.c).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                if let Some(table) = object.as_table() {
                                    if let Some(index) =
                                        key.as_number().and_then(array_iterator_index)
                                    {
                                        if self
                                            .set_table_integer_index_fast(table, index, value)
                                            .map_err(|e| {
                                                attach_instruction_source(
                                                    program,
                                                    function,
                                                    pc - 1,
                                                    e,
                                                )
                                            })?
                                        {
                                            continue;
                                        }
                                    }
                                }
                                self.set_index_value(
                                    program,
                                    object,
                                    key,
                                    value,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                            }
                            Opcode::SetIndexNumber => {
                                let object =
                                    get_register(&registers, instruction.a).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = get_register(&registers, instruction.b).map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                let value =
                                    get_register(&registers, instruction.c).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                if let Some(number) = key.as_number() {
                                    if let Some(table) = object.as_table() {
                                        if let Some(index) = array_iterator_index(number) {
                                            if self
                                                .set_table_integer_index_fast(table, index, value)
                                                .map_err(|e| {
                                                    attach_instruction_source(
                                                        program,
                                                        function,
                                                        pc - 1,
                                                        e,
                                                    )
                                                })?
                                            {
                                                continue;
                                            }
                                        }
                                    }
                                }
                                self.set_index_value(
                                    program,
                                    object,
                                    key,
                                    value,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                            }
                            Opcode::SetIndexConcatConst => {
                                let object =
                                    get_register(&registers, instruction.a).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let key = self
                                    .concat_const_key(
                                        program,
                                        program_id,
                                        &registers,
                                        instruction.b,
                                        instruction.aux,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let value =
                                    get_register(&registers, instruction.c).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                if let Some(table) = object.as_table() {
                                    if let Some(index) =
                                        key.as_number().and_then(array_iterator_index)
                                    {
                                        if self
                                            .set_table_integer_index_fast(table, index, value)
                                            .map_err(|e| {
                                                attach_instruction_source(
                                                    program,
                                                    function,
                                                    pc - 1,
                                                    e,
                                                )
                                            })?
                                        {
                                            continue;
                                        }
                                    }
                                }
                                self.set_index_value(
                                    program,
                                    object,
                                    key,
                                    value,
                                    context.depth,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                            }
                            Opcode::RawSet => {
                                let table = get_table(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let key = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = get_register(&registers, instruction.c)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                self.set_table_index_after_table_lookup(table, key, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableConcat => {
                                let table = get_table(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let separator_storage;
                                let separator = if instruction.aux == 1 {
                                    &[][..]
                                } else {
                                    separator_storage = self
                                        .coerce_to_string(&registers, instruction.c)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                    separator_storage.as_slice()
                                };
                                let (start, end) = table_concat_range(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = self
                                    .table_concat(table, separator, start, end)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ConcatArgs => {
                                let value = self
                                    .concat_args(
                                        program,
                                        &registers,
                                        instruction.b,
                                        instruction.c,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::ToString => {
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let text = self
                                    .value_to_tostring_with_metamethod(
                                        program,
                                        value,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let atom = self.heap.intern_string_bytes(&text);
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::string_atom(atom),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::NextKey => {
                                let table = get_table(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let after = get_register(&registers, instruction.c)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let key = match self
                                    .table_next_after(table, after)
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "next table handle is missing",
                                        })
                                    })
                                    .map_err(|e| e.at_pc(pc - 1))?
                                {
                                    TableNext::Entry(key, _) => key,
                                    TableNext::End => Value::nil(),
                                    TableNext::InvalidKey => {
                                        return Err(
                                            Error::new(ErrorKind::InvalidKeyToNext).at_pc(pc - 1)
                                        );
                                    }
                                };
                                set_register(&mut registers, instruction.a, key)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TypeOf => {
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let atom = self.heap.intern_string(self.luau_type_name(value));
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::string_atom(atom),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringSub => {
                                let value = self
                                    .string_sub(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringByteEqual => {
                                let value = self
                                    .string_byte_equal(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringLen => {
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let len =
                                    self.string_byte_len(value).map_err(|e| e.at_pc(pc - 1))?;
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::number(len as f64),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringByte => {
                                let value = self
                                    .string_byte(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringUpper => {
                                let mut text = self
                                    .coerce_to_string(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                text.make_ascii_uppercase();
                                let atom = self.heap.intern_string_bytes(&text);
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::string_atom(atom),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringLower => {
                                let mut text = self
                                    .coerce_to_string(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                text.make_ascii_lowercase();
                                let atom = self.heap.intern_string_bytes(&text);
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::string_atom(atom),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringReverse => {
                                let text = self
                                    .coerce_to_string(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let reversed =
                                    reverse_string_bytes(&text).map_err(|e| e.at_pc(pc - 1))?;
                                let atom = self.heap.intern_string_bytes(&reversed);
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::string_atom(atom),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringRep => {
                                let value = self
                                    .string_rep(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringFind => {
                                let extra_fuel = self
                                    .string_find_opcode_fuel(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)
                                    .map_err(|error| error.at_pc(pc - 1))?;
                                let value = self
                                    .string_find(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringSplit => {
                                let value = self
                                    .string_split(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::StringFormat => {
                                let extra_fuel = self
                                    .string_format_opcode_fuel(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = self
                                    .string_format(
                                        program,
                                        &registers,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableGetN => {
                                let table = get_table(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let len = self.heap.array_len(table).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "table handle is missing",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::number(len as f64),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableMaxN => {
                                let table = get_table(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let max = self.heap.max_numeric_key(table).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "table handle is missing",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                set_register(&mut registers, instruction.a, Value::number(max))
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableFind => {
                                let value = self
                                    .table_find(
                                        program,
                                        &registers,
                                        instruction,
                                        context.depth,
                                        fuel,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableCreate => {
                                let value = self
                                    .table_create(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableInsert => {
                                let extra_fuel = self
                                    .table_insert_extra_fuel(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let value = self
                                    .table_insert(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::TableRemove => {
                                let value = self
                                    .table_remove(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::LoadFunction => {
                                let environment =
                                    self.current_environment().map_err(|e| e.at_pc(pc - 1))?;
                                let value = load_function_value(
                                    &mut self.heap,
                                    program_id,
                                    program,
                                    instruction,
                                    &registers,
                                    &mut register_cells,
                                    context.upvalues,
                                    environment,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                                may_have_register_cells = true;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::LoadCurrentFunction => {
                                let value = context.current_function.ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "current function handle is missing",
                                    })
                                })?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::Call => {
                                if empty_return_follows(function, pc) {
                                    let callee =
                                        get_register(&registers, instruction.b).map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?;
                                    let current_function =
                                        context.current_function.and_then(Value::as_function);
                                    if allow_stackless_self_calls
                                        && current_function.is_some()
                                        && current_function == callee.as_function()
                                    {
                                        if instruction.aux < 0
                                            || instruction.aux > i32::from(u8::MAX)
                                        {
                                            return Err(Error::new(ErrorKind::InvalidProgram {
                                                reason: "invalid function call arity",
                                            })
                                            .at_pc(pc - 1));
                                        }
                                        let args_start = usize::from(instruction.c);
                                        let args_end = args_start
                                            .checked_add(instruction.aux as usize)
                                            .ok_or_else(|| {
                                                Error::new(ErrorKind::InvalidProgram {
                                                    reason: "function argument window overflow",
                                                })
                                                .at_pc(pc - 1)
                                            })?;
                                        if args_end > registers.len() {
                                            return Err(Error::new(
                                                ErrorKind::RegisterOutOfBounds {
                                                    register: args_end,
                                                    max: registers.len(),
                                                },
                                            )
                                            .at_pc(pc - 1));
                                        }
                                        let tail_args = registers[args_start..args_end].to_vec();
                                        reset_current_function_frame(
                                            function,
                                            &mut registers,
                                            &mut register_cells,
                                            &mut varargs,
                                            &tail_args,
                                        )
                                        .map_err(|e| {
                                            attach_instruction_source(program, function, pc - 1, e)
                                        })?;
                                        pc = 0;
                                        continue;
                                    }
                                }
                                let callee =
                                    get_register(&registers, instruction.b).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let current_function =
                                    context.current_function.and_then(Value::as_function);
                                if allow_stackless_self_calls
                                    && current_function.is_some()
                                    && current_function == callee.as_function()
                                {
                                    if instruction.aux < 0 || instruction.aux > i32::from(u8::MAX) {
                                        return Err(Error::new(ErrorKind::InvalidProgram {
                                            reason: "invalid function call arity",
                                        })
                                        .at_pc(pc - 1));
                                    }
                                    let args_start = usize::from(instruction.c);
                                    let args_end = args_start
                                        .checked_add(instruction.aux as usize)
                                        .ok_or_else(|| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason: "function argument window overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                    if args_end > registers.len() {
                                        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                            register: args_end,
                                            max: registers.len(),
                                        })
                                        .at_pc(pc - 1));
                                    }
                                    let next_depth = context
                                        .depth
                                        .saturating_add(stackless_self_calls.len())
                                        .saturating_add(1);
                                    if next_depth >= LUAU_TAIL_CALL_LIMIT.saturating_sub(1) {
                                        return Err(attach_instruction_source(
                                            program,
                                            function,
                                            pc - 1,
                                            Error::new(ErrorKind::RecursionLimit {
                                                limit: LUAU_TAIL_CALL_LIMIT,
                                            }),
                                        ));
                                    }
                                    let call_args = registers[args_start..args_end].to_vec();
                                    let continuation = BytecodeContinuation::WriteFirst {
                                        register: instruction.a,
                                    };
                                    let root_mark = self.push_stackless_self_call_roots(
                                        &registers,
                                        &varargs,
                                        &pending_return_prefixes,
                                        &continuation,
                                    );
                                    let debug_frame = stackless_self_call_debug_frame(
                                        &context,
                                        function,
                                        instruction_source_location(program, function, pc - 1),
                                    );
                                    let debug_mark =
                                        self.push_stackless_self_call_debug_frame(&debug_frame);
                                    let frame = StacklessSelfCallFrame {
                                        pc,
                                        registers: std::mem::take(&mut registers),
                                        register_cells: std::mem::take(&mut register_cells),
                                        varargs: std::mem::take(&mut varargs),
                                        pending_return_prefixes: std::mem::take(
                                            &mut pending_return_prefixes,
                                        ),
                                        continuation,
                                        tail_call_count,
                                        root_mark,
                                        debug_mark,
                                        debug_frame,
                                    };
                                    stackless_self_calls.push(frame);
                                    registers = self.take_registers(function.max_registers);
                                    register_cells = fresh_register_cells(function);
                                    for (index, arg) in call_args
                                        .iter()
                                        .copied()
                                        .take(function.parameter_count)
                                        .enumerate()
                                    {
                                        let register = u8::try_from(index).map_err(|_| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason:
                                                    "function parameter register index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                        set_register(&mut registers, register, arg)
                                            .map_err(|e| e.at_pc(pc - 1))?;
                                    }
                                    varargs = if function.is_vararg
                                        && call_args.len() > function.parameter_count
                                    {
                                        call_args[function.parameter_count..].to_vec()
                                    } else {
                                        Vec::new()
                                    };
                                    pending_return_prefixes = Vec::new();
                                    self.validate_conformance_native_parameter_types(
                                        function, &registers,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                    pc = 0;
                                    tail_call_count = 0;
                                    continue;
                                }
                                let call_result = match self
                                    .call_function_from_registers_resumable_lazy_site(
                                        program,
                                        callee,
                                        &registers,
                                        instruction,
                                        context.depth,
                                        || instruction_source_location(program, function, pc - 1),
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::WriteFirst {
                                                register: instruction.a,
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                                let value = match call_result {
                                    RegisterCallResult::Scalar(value) => value,
                                    RegisterCallResult::Results(results) => {
                                        self.first_result_or_nil_recycle(results)
                                    }
                                };
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::CallMulti => {
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let base = usize::from(instruction.a);
                                let argc = usize::from(instruction.b);
                                let result_count = usize::from(instruction.c);
                                let args_start = base.checked_add(1).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "function argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                let args_end = args_start.checked_add(argc).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "function argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                let results_end =
                                    base.checked_add(result_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "function result window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if args_end.max(results_end) > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: args_end.max(results_end),
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let callee =
                                    get_register(&registers, instruction.a).map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let protected_callee = registers
                                    .get(args_start)
                                    .copied()
                                    .unwrap_or_else(Value::nil);
                                let current_function =
                                    context.current_function.and_then(Value::as_function);
                                if allow_stackless_self_calls
                                    && self.value_is_native_function(callee, NativeFunction::PCall)
                                    && current_function.is_some()
                                    && current_function == protected_callee.as_function()
                                {
                                    let next_depth = context
                                        .depth
                                        .saturating_add(stackless_self_calls.len())
                                        .saturating_add(2);
                                    if next_depth >= LUAU_TAIL_CALL_LIMIT.saturating_sub(1) {
                                        let error = attach_instruction_source(
                                            program,
                                            function,
                                            pc - 1,
                                            Error::new(ErrorKind::RecursionLimit {
                                                limit: LUAU_TAIL_CALL_LIMIT,
                                            }),
                                        );
                                        let error = self.error_object(error);
                                        write_call_results(
                                            &mut registers,
                                            base,
                                            result_count,
                                            &[Value::boolean(false), error],
                                        )
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                        continue;
                                    }
                                    let call_args = registers[args_start + 1..args_end].to_vec();
                                    let continuation = BytecodeContinuation::PCallWriteFixed {
                                        base,
                                        count: result_count,
                                    };
                                    let root_mark = self.push_stackless_self_call_roots(
                                        &registers,
                                        &varargs,
                                        &pending_return_prefixes,
                                        &continuation,
                                    );
                                    let debug_frame = stackless_self_call_debug_frame(
                                        &context,
                                        function,
                                        call_site.clone(),
                                    );
                                    let debug_mark =
                                        self.push_stackless_self_call_debug_frame(&debug_frame);
                                    let frame = StacklessSelfCallFrame {
                                        pc,
                                        registers: std::mem::take(&mut registers),
                                        register_cells: std::mem::take(&mut register_cells),
                                        varargs: std::mem::take(&mut varargs),
                                        pending_return_prefixes: std::mem::take(
                                            &mut pending_return_prefixes,
                                        ),
                                        continuation,
                                        tail_call_count,
                                        root_mark,
                                        debug_mark,
                                        debug_frame,
                                    };
                                    stackless_self_calls.push(frame);
                                    registers = self.take_registers(function.max_registers);
                                    register_cells = fresh_register_cells(function);
                                    for (index, arg) in call_args
                                        .iter()
                                        .copied()
                                        .take(function.parameter_count)
                                        .enumerate()
                                    {
                                        let register = u8::try_from(index).map_err(|_| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason:
                                                    "function parameter register index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                        set_register(&mut registers, register, arg)
                                            .map_err(|e| e.at_pc(pc - 1))?;
                                    }
                                    varargs = if function.is_vararg
                                        && call_args.len() > function.parameter_count
                                    {
                                        call_args[function.parameter_count..].to_vec()
                                    } else {
                                        Vec::new()
                                    };
                                    pending_return_prefixes = Vec::new();
                                    self.validate_conformance_native_parameter_types(
                                        function, &registers,
                                    )
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                    pc = 0;
                                    tail_call_count = 0;
                                    continue;
                                }
                                match self
                                    .call_function_multi(
                                        program,
                                        &mut registers,
                                        instruction,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(()) => {}
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::WriteFixed {
                                                base: usize::from(instruction.a),
                                                count: usize::from(instruction.c),
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                }
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::CallExpand => {
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                if instruction.aux < 0 || instruction.aux > i32::from(u8::MAX) {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "invalid expanded function result count",
                                    })
                                    .at_pc(pc - 1));
                                }
                                let base = usize::from(instruction.a);
                                let prefix_count = usize::from(instruction.b);
                                let trailing_argc = usize::from(instruction.c);
                                let result_count =
                                    usize::try_from(instruction.aux).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "invalid expanded function result count",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let prefix_start = base.checked_add(1).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "expanded function argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                let trailing_callee_index =
                                    prefix_start.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded function argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let trailing_args_start =
                                    trailing_callee_index.checked_add(1).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded function argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let trailing_args_end = trailing_args_start
                                    .checked_add(trailing_argc)
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded function argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let results_end =
                                    base.checked_add(result_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded function result window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if trailing_args_end.max(results_end) > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: trailing_args_end.max(results_end),
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }

                                let outer_callee = get_register(&registers, instruction.a)
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                let trailing_callee = get_register(
                                    &registers,
                                    u8::try_from(trailing_callee_index).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded function callee register overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                                let mut args =
                                    registers[prefix_start..trailing_callee_index].to_vec();
                                let trailing_args =
                                    registers[trailing_args_start..trailing_args_end].to_vec();
                                let trailing_results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        trailing_callee,
                                        &trailing_args,
                                        context.depth,
                                        call_site.clone(),
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::CallExpandAfterTrailing {
                                                outer_callee,
                                                args,
                                                base,
                                                count: result_count,
                                                depth: context.depth,
                                                call_site,
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                args.extend(trailing_results);
                                let results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        outer_callee,
                                        &args,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::WriteFixed {
                                                base,
                                                count: result_count,
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                write_call_results(&mut registers, base, result_count, &results)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::LoadVarargs => {
                                let first = usize::from(instruction.a);
                                let count = usize::from(instruction.b);
                                let end = first.checked_add(count).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "vararg register window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                if end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                for index in 0..count {
                                    let value =
                                        varargs.get(index).copied().unwrap_or_else(Value::nil);
                                    let register = u8::try_from(first + index).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "vararg register index overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                    set_register(&mut registers, register, value)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::ReturnVarargs => {
                                let prefix_count = usize::from(instruction.b);
                                let prefix_first = usize::from(instruction.a);
                                let prefix_end =
                                    prefix_first.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return vararg prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let mut results = registers[prefix_first..prefix_end].to_vec();
                                results.extend(varargs.iter().copied());
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::CallVarargs => {
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                match self
                                    .call_function_varargs(
                                        program,
                                        &mut registers,
                                        instruction,
                                        &varargs,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(()) => {}
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::WriteFixed {
                                                base: usize::from(instruction.a),
                                                count: usize::from(instruction.c),
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                }
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::CallExpand2 => {
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                self.call_function_expand2(
                                    program,
                                    &mut registers,
                                    instruction,
                                    context.depth,
                                    call_site,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::CallExpand3 => {
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                self.call_function_expand3(
                                    program,
                                    &mut registers,
                                    instruction,
                                    context.depth,
                                    call_site,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::CallExpandVarargs => {
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                self.call_function_expand_varargs(
                                    program,
                                    &mut registers,
                                    instruction,
                                    &varargs,
                                    context.depth,
                                    call_site,
                                    fuel,
                                )
                                .map_err(|e| {
                                    attach_instruction_source(program, function, pc - 1, e)
                                })?;
                                if may_have_register_cells {
                                    self.sync_register_cells(&mut registers, &register_cells)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::ReturnCallVarargs => {
                                let prefix_count = usize::from(instruction.b);
                                let prefix_first = usize::from(instruction.a);
                                let prefix_end =
                                    prefix_first.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let (args_start, argc) = unpack_return_call_aux(instruction.aux)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let args_start = usize::from(args_start);
                                let args_end =
                                    args_start.checked_add(usize::from(argc)).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return call vararg window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if args_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: args_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let callee = get_register(&registers, instruction.c)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let mut args = registers[args_start..args_end].to_vec();
                                args.extend(varargs.iter().copied());
                                let mut results = registers[prefix_first..prefix_end].to_vec();
                                let mut call_results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        callee,
                                        &args,
                                        context.depth,
                                        None,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        let continuation = BytecodeContinuation::ReturnWithPrefix {
                                            prefix: registers[prefix_first..prefix_end].to_vec(),
                                        };
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            continuation,
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                results.append(&mut call_results);
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::ReturnCallExpand2 => {
                                let prefix_count = usize::from(instruction.b);
                                let prefix_first = usize::from(instruction.a);
                                let prefix_end =
                                    prefix_first.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let mut results = registers[prefix_first..prefix_end].to_vec();
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let mut call_results = self
                                    .nested_expand_results(
                                        program,
                                        &registers,
                                        usize::from(instruction.c),
                                        instruction.aux,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                results.append(&mut call_results);
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::ReturnCallExpand2Varargs => {
                                let prefix_count = usize::from(instruction.b);
                                let prefix_first = usize::from(instruction.a);
                                let prefix_end =
                                    prefix_first.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let mut results = registers[prefix_first..prefix_end].to_vec();
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let mut call_results = self
                                    .nested_expand2_varargs_results(
                                        program,
                                        &registers,
                                        NestedVarargSpec {
                                            base: usize::from(instruction.c),
                                            aux: instruction.aux,
                                        },
                                        &varargs,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                results.append(&mut call_results);
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::ReturnCallExpandVarargs => {
                                let prefix_count = usize::from(instruction.b);
                                let prefix_first = usize::from(instruction.a);
                                let prefix_end =
                                    prefix_first.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let mut results = registers[prefix_first..prefix_end].to_vec();
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let mut call_results = self
                                    .nested_expand_varargs_results(
                                        program,
                                        &registers,
                                        NestedVarargSpec {
                                            base: usize::from(instruction.c),
                                            aux: instruction.aux,
                                        },
                                        &varargs,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                results.append(&mut call_results);
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::ReturnCallExpand => {
                                let return_prefix_count = usize::from(instruction.b);
                                let return_prefix_first = usize::from(instruction.a);
                                let return_prefix_end = return_prefix_first
                                    .checked_add(return_prefix_count)
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "return prefix register window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if return_prefix_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: return_prefix_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let call_base = usize::from(instruction.c);
                                let (arg_prefix_count, trailing_argc) =
                                    unpack_expand_aux(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                let arg_prefix_count = usize::from(arg_prefix_count);
                                let trailing_argc = usize::from(trailing_argc);
                                let prefix_start = call_base.checked_add(1).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "expanded return call argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                let trailing_callee_index = prefix_start
                                    .checked_add(arg_prefix_count)
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded return call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let trailing_args_start =
                                    trailing_callee_index.checked_add(1).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded return call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let trailing_args_end = trailing_args_start
                                    .checked_add(trailing_argc)
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded return call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if trailing_args_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: trailing_args_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let outer_callee = get_register(&registers, instruction.c)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let trailing_callee = get_register(
                                    &registers,
                                    u8::try_from(trailing_callee_index).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "expanded return call callee register overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                                let mut args =
                                    registers[prefix_start..trailing_callee_index].to_vec();
                                let trailing_args =
                                    registers[trailing_args_start..trailing_args_end].to_vec();
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let mut expanded_args = self
                                    .call_value_with_site(
                                        program,
                                        trailing_callee,
                                        &trailing_args,
                                        context.depth,
                                        call_site.clone(),
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                args.append(&mut expanded_args);
                                let mut results =
                                    registers[return_prefix_first..return_prefix_end].to_vec();
                                let mut call_results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        outer_callee,
                                        &args,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        let continuation = BytecodeContinuation::ReturnWithPrefix {
                                            prefix: registers
                                                [return_prefix_first..return_prefix_end]
                                                .to_vec(),
                                        };
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            continuation,
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                results.append(&mut call_results);
                                return Ok(VmFlow::Return(finish_function_results(
                                    &mut pending_return_prefixes,
                                    results,
                                )));
                            }
                            Opcode::AppendVarargs => {
                                if instruction.aux <= 0 {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "invalid table vararg start index",
                                    })
                                    .at_pc(pc - 1));
                                }
                                let table_value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let table = table_value.as_table().ok_or_else(|| {
                                    Error::new(ErrorKind::TypeError { expected: "table" })
                                        .at_pc(pc - 1)
                                })?;
                                for (offset, value) in varargs.iter().copied().enumerate() {
                                    let offset = u32::try_from(offset).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table vararg index overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                    let index = usize::try_from(instruction.aux)
                                        .ok()
                                        .and_then(|start| start.checked_add(offset as usize))
                                        .ok_or_else(|| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason: "table vararg index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                    self.set_table_array_slot(table, index, value)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::AppendCall => {
                                if instruction.aux <= 0 {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "invalid table call append start index",
                                    })
                                    .at_pc(pc - 1));
                                }
                                let table_value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let table = table_value.as_table().ok_or_else(|| {
                                    Error::new(ErrorKind::TypeError { expected: "table" })
                                        .at_pc(pc - 1)
                                })?;
                                let callee = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let args_start =
                                    usize::from(instruction.b).checked_add(1).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table call append argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let args_end = args_start
                                    .checked_add(usize::from(instruction.c))
                                    .ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "table call append argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                if args_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: args_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let args = registers[args_start..args_end].to_vec();
                                let results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        callee,
                                        &args,
                                        context.depth,
                                        None,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::AppendToTable {
                                                table,
                                                start_index: usize::try_from(instruction.aux)
                                                    .map_err(|_| {
                                                        Error::new(ErrorKind::InvalidProgram {
                                                            reason:
                                                                "table call append index overflow",
                                                        })
                                                        .at_pc(pc - 1)
                                                    })?,
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                for (offset, value) in results.iter().copied().enumerate() {
                                    let offset = u32::try_from(offset).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table call append index overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                    let index = usize::try_from(instruction.aux)
                                        .ok()
                                        .and_then(|start| start.checked_add(offset as usize))
                                        .ok_or_else(|| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason: "table call append index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                    self.set_table_array_slot(table, index, value)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::AppendCallVarargs => {
                                if instruction.aux <= 0 {
                                    return Err(Error::new(ErrorKind::InvalidProgram {
                                        reason: "invalid table vararg call append start index",
                                    })
                                    .at_pc(pc - 1));
                                }
                                let table_value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let table = table_value.as_table().ok_or_else(|| {
                                    Error::new(ErrorKind::TypeError { expected: "table" })
                                        .at_pc(pc - 1)
                                })?;
                                let callee = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let args_start =
                                    usize::from(instruction.b).checked_add(1).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason:
                                                "table vararg call append argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let args_end = args_start
                                    .checked_add(usize::from(instruction.c))
                                    .ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "table vararg call append argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                if args_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: args_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let mut args = registers[args_start..args_end].to_vec();
                                args.extend(varargs.iter().copied());
                                let results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        callee,
                                        &args,
                                        context.depth,
                                        None,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::AppendToTable {
                                                table,
                                                start_index: usize::try_from(instruction.aux)
                                                    .map_err(|_| {
                                                        Error::new(ErrorKind::InvalidProgram {
                                                    reason:
                                                        "table vararg call append index overflow",
                                                })
                                                .at_pc(pc - 1)
                                                    })?,
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                for (offset, value) in results.iter().copied().enumerate() {
                                    let offset = u32::try_from(offset).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table vararg call append index overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                    let index = usize::try_from(instruction.aux)
                                        .ok()
                                        .and_then(|start| start.checked_add(offset as usize))
                                        .ok_or_else(|| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason: "table vararg call append index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                    self.set_table_array_slot(table, index, value)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::AppendCallExpand => {
                                let (start_index, trailing_argc) =
                                    unpack_append_call_expand_aux(instruction.aux)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                let table_value = get_register(&registers, instruction.a)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let table = table_value.as_table().ok_or_else(|| {
                                    Error::new(ErrorKind::TypeError { expected: "table" })
                                        .at_pc(pc - 1)
                                })?;
                                let base = usize::from(instruction.b);
                                let prefix_count = usize::from(instruction.c);
                                let prefix_start = base.checked_add(1).ok_or_else(|| {
                                    Error::new(ErrorKind::InvalidProgram {
                                        reason: "table expanded call argument window overflow",
                                    })
                                    .at_pc(pc - 1)
                                })?;
                                let trailing_callee_index =
                                    prefix_start.checked_add(prefix_count).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table expanded call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let trailing_args_start =
                                    trailing_callee_index.checked_add(1).ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table expanded call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                let trailing_args_end = trailing_args_start
                                    .checked_add(usize::from(trailing_argc))
                                    .ok_or_else(|| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table expanded call argument window overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                if trailing_args_end > registers.len() {
                                    return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                                        register: trailing_args_end,
                                        max: registers.len(),
                                    })
                                    .at_pc(pc - 1));
                                }
                                let outer_callee = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let trailing_callee = get_register(
                                    &registers,
                                    u8::try_from(trailing_callee_index).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table expanded call callee register overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?,
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                                let mut args =
                                    registers[prefix_start..trailing_callee_index].to_vec();
                                let trailing_args =
                                    registers[trailing_args_start..trailing_args_end].to_vec();
                                let call_site =
                                    instruction_source_location(program, function, pc - 1);
                                let mut expanded_args = self
                                    .call_value_with_site(
                                        program,
                                        trailing_callee,
                                        &trailing_args,
                                        context.depth,
                                        call_site.clone(),
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })?;
                                args.append(&mut expanded_args);
                                let results = match self
                                    .call_value_with_site_resumable(
                                        program,
                                        outer_callee,
                                        &args,
                                        context.depth,
                                        call_site,
                                        fuel,
                                    )
                                    .map_err(|e| {
                                        attach_instruction_source(program, function, pc - 1, e)
                                    })? {
                                    VmFlow::Return(results) => results,
                                    VmFlow::Yield(suspension) => {
                                        return self.suspend_bytecode_frame(
                                            program_id,
                                            &context,
                                            pc,
                                            std::mem::take(&mut registers),
                                            std::mem::take(&mut register_cells),
                                            std::mem::take(&mut varargs),
                                            std::mem::take(&mut pending_return_prefixes),
                                            BytecodeContinuation::AppendToTable {
                                                table,
                                                start_index: usize::try_from(start_index).map_err(
                                                    |_| {
                                                        Error::new(ErrorKind::InvalidProgram {
                                                    reason:
                                                        "table expanded call append index overflow",
                                                })
                                                .at_pc(pc - 1)
                                                    },
                                                )?,
                                            },
                                            std::mem::take(&mut stackless_self_calls),
                                            tail_call_count,
                                            suspension,
                                        );
                                    }
                                };
                                for (offset, value) in results.iter().copied().enumerate() {
                                    let offset = u32::try_from(offset).map_err(|_| {
                                        Error::new(ErrorKind::InvalidProgram {
                                            reason: "table expanded call append index overflow",
                                        })
                                        .at_pc(pc - 1)
                                    })?;
                                    let index = usize::try_from(start_index)
                                        .ok()
                                        .and_then(|start| start.checked_add(offset as usize))
                                        .ok_or_else(|| {
                                            Error::new(ErrorKind::InvalidProgram {
                                                reason: "table expanded call append index overflow",
                                            })
                                            .at_pc(pc - 1)
                                        })?;
                                    self.set_table_array_slot(table, index, value)
                                        .map_err(|e| e.at_pc(pc - 1))?;
                                }
                            }
                            Opcode::RawLen => {
                                let value = get_register(&registers, instruction.b)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                let len = self.raw_len_value(value).map_err(|e| e.at_pc(pc - 1))?;
                                set_register(
                                    &mut registers,
                                    instruction.a,
                                    Value::number(len as f64),
                                )
                                .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathFloor => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Floor)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathCeil => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Ceil)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathRound => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Round)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathSign => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Sign)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathClamp => {
                                let value = self
                                    .math_clamp(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathMap => {
                                let value = self
                                    .math_map(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathLerp => {
                                let value = self
                                    .math_lerp(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathMin => {
                                let value = self
                                    .math_minmax(&registers, instruction, MathMinMax::Min)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathMax => {
                                let value = self
                                    .math_minmax(&registers, instruction, MathMinMax::Max)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathFmod => {
                                let value = self
                                    .math_binary(&registers, instruction, MathBinary::Fmod)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathPow => {
                                let value = self
                                    .math_binary(&registers, instruction, MathBinary::Pow)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathSqrt => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Sqrt)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathDeg => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Deg)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathRad => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Rad)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathIsNan => {
                                let value = self
                                    .math_predicate(&registers, instruction, MathPredicate::Nan)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathIsInf => {
                                let value = self
                                    .math_predicate(&registers, instruction, MathPredicate::Inf)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathIsFinite => {
                                let value = self
                                    .math_predicate(&registers, instruction, MathPredicate::Finite)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathSin => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Sin)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathCos => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Cos)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathSinCosMul => {
                                let value = self
                                    .math_sin_cos_mul(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathTan => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Tan)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathAsin => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Asin)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathAcos => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Acos)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathAtan => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Atan)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathAtan2 => {
                                let value = self
                                    .math_binary(&registers, instruction, MathBinary::Atan2)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathSinh => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Sinh)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathCosh => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Cosh)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathTanh => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Tanh)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathExp => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Exp)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathLog => {
                                let value = self
                                    .math_log(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathLog10 => {
                                let value = self
                                    .math_unary(&registers, instruction, MathUnary::Log10)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                            Opcode::MathLdexp => {
                                let value = self
                                    .math_ldexp(&registers, instruction)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                                set_register(&mut registers, instruction.a, value)
                                    .map_err(|e| e.at_pc(pc - 1))?;
                            }
                        }
                    }
                })();
                match loop_result {
                    Ok(VmFlow::Return(results)) => {
                        unwind_stackless_return!(results);
                        continue 'run_function_loop;
                    }
                    Ok(VmFlow::Yield(suspension)) => return Ok(VmFlow::Yield(suspension)),
                    Err(error) => {
                        unwind_stackless_error!(error);
                        continue 'run_function_loop;
                    }
                }
            }
        })();
        let result =
            result.map_err(|error| attach_existing_instruction_source(program, function, error));
        if let Err(error) = &result {
            self.last_error_traceback = Some(self.traceback_snapshot(error.location.clone()));
            self.record_running_coroutine_error_snapshot(error.location.clone());
        }
        if let Some(frame) = self.call_stack.pop() {
            self.recycle_call_stack_frame(frame);
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn suspend_bytecode_frame(
        &mut self,
        program_id: Option<u32>,
        context: &FunctionCallContext<'_>,
        pc: usize,
        mut registers: Vec<Value>,
        register_cells: Vec<Option<u32>>,
        varargs: Vec<Value>,
        pending_return_prefixes: Vec<Value>,
        continuation: BytecodeContinuation,
        stackless_self_calls: Vec<StacklessSelfCallFrame>,
        tail_call_count: usize,
        mut suspension: CoroutineYield,
    ) -> VmFlowResult<Vec<Value>> {
        self.sync_register_cells(&mut registers, &register_cells)?;
        let program_id = program_id.ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "suspended coroutine frame is missing its program handle",
            })
        })?;
        suspension
            .frames
            .push(CoroutineFrame::Bytecode(Box::new(SuspendedBytecodeFrame {
                program_id,
                prototype: context.prototype,
                current_function: context.current_function,
                upvalues: context.upvalues.to_vec(),
                depth: context.depth,
                environment: context.environment,
                call_site: context.call_site.clone(),
                pc,
                registers,
                register_cells,
                varargs,
                pending_return_prefixes,
                continuation,
                stackless_self_calls,
                tail_call_count,
            })));
        Ok(VmFlow::Yield(suspension))
    }

    fn apply_bytecode_continuation(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        pending_return_prefixes: &mut Vec<Value>,
        continuation: BytecodeContinuation,
        mut results: Vec<Value>,
        fuel: &mut Option<u64>,
    ) -> Result<Option<Vec<Value>>> {
        match continuation {
            BytecodeContinuation::WriteFirst { register } => {
                let value = first_result_or_nil(results);
                set_register(registers, register, value)?;
                Ok(None)
            }
            BytecodeContinuation::WriteFixed { base, count } => {
                write_call_results(registers, base, count, &results)?;
                Ok(None)
            }
            BytecodeContinuation::PCallWriteFixed { base, count } => {
                let mut protected_results = Vec::with_capacity(results.len() + 1);
                protected_results.push(Value::boolean(true));
                protected_results.append(&mut results);
                write_call_results(registers, base, count, &protected_results)?;
                Ok(None)
            }
            BytecodeContinuation::CallExpandAfterTrailing {
                outer_callee,
                mut args,
                base,
                count,
                depth,
                call_site,
            } => {
                args.append(&mut results);
                let results = self.call_value_with_site(
                    program,
                    outer_callee,
                    &args,
                    depth,
                    call_site,
                    fuel,
                )?;
                write_call_results(registers, base, count, &results)?;
                Ok(None)
            }
            BytecodeContinuation::ReturnWithPrefix { mut prefix } => {
                prefix.append(&mut results);
                Ok(Some(finish_function_results(
                    pending_return_prefixes,
                    prefix,
                )))
            }
            BytecodeContinuation::AppendToTable { table, start_index } => {
                self.append_call_results_to_table(table, start_index, &results)?;
                Ok(None)
            }
        }
    }

    fn append_call_results_to_table(
        &mut self,
        table: u32,
        start_index: usize,
        results: &[Value],
    ) -> Result<()> {
        for (offset, value) in results.iter().copied().enumerate() {
            let index = start_index.checked_add(offset).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table call append index overflow",
                })
            })?;
            self.set_table_array_slot(table, index, value)?;
        }
        Ok(())
    }

    fn load_constant(&mut self, program: &Program, index: usize) -> Result<Value> {
        let constant = program.constants.get(index).ok_or_else(|| {
            Error::new(ErrorKind::ConstantOutOfBounds {
                index,
                len: program.constants.len(),
            })
        })?;
        Ok(match constant {
            Constant::Nil => Value::nil(),
            Constant::Boolean(value) => Value::boolean(*value),
            Constant::Number(value) => Value::number(*value),
            Constant::String(value) => Value::string_atom(self.heap.intern_string_bytes(value)),
        })
    }

    fn constant_string_atom(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        index: usize,
    ) -> Result<Option<u32>> {
        let constant = program.constants.get(index).ok_or_else(|| {
            Error::new(ErrorKind::ConstantOutOfBounds {
                index,
                len: program.constants.len(),
            })
        })?;
        let Constant::String(value) = constant else {
            return Ok(None);
        };
        let Some(program_id) = program_id else {
            return Ok(Some(self.heap.intern_string_bytes(value)));
        };
        if let Some(atom) = self.constant_string_atom_cache.get(&(program_id, index)) {
            return Ok(Some(*atom));
        }
        let atom = self.heap.intern_string_bytes(value);
        self.constant_string_atom_cache
            .insert((program_id, index), atom);
        Ok(Some(atom))
    }

    fn number_constant(&self, program: &Program, index: i32) -> Result<f64> {
        if index < 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "negative numeric constant index",
            }));
        }
        let constant = program.constants.get(index as usize).ok_or_else(|| {
            Error::new(ErrorKind::ConstantOutOfBounds {
                index: index as usize,
                len: program.constants.len(),
            })
        })?;
        let Constant::Number(value) = constant else {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "numeric constant opcode must reference a number",
            }));
        };
        Ok(*value)
    }

    fn number_constant_pair(&self, program: &Program, aux: i32) -> Result<(f64, f64)> {
        let packed = aux as u32;
        let first = (packed >> 16) as i32;
        let second = (packed & 0xffff) as i32;
        Ok((
            self.number_constant(program, first)?,
            self.number_constant(program, second)?,
        ))
    }

    fn global_key(&mut self, program: &Program, index: i32) -> Result<Value> {
        if index < 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "negative global constant index",
            }));
        }
        let constant = program.constants.get(index as usize).ok_or_else(|| {
            Error::new(ErrorKind::ConstantOutOfBounds {
                index: index as usize,
                len: program.constants.len(),
            })
        })?;
        let Constant::String(name) = constant else {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "global key constant must be a string",
            }));
        };
        Ok(Value::string_atom(self.heap.intern_string_bytes(name)))
    }

    fn get_global_with_inline_cache(
        &mut self,
        program: &Program,
        site: Option<InlineCacheSite>,
        name_index: i32,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let environment = self.current_environment()?;
        if let Some(value) = self.global_get_inline_cache_hit(site, environment) {
            return Ok(value);
        }

        let key = self.global_key(program, name_index)?;
        let value = self.get_table_index_with_metamethod(program, environment, key, depth, fuel)?;
        self.update_global_get_inline_cache(site, environment, key)?;
        Ok(value)
    }

    fn global_get_inline_cache_hit(
        &mut self,
        site: Option<InlineCacheSite>,
        environment: u32,
    ) -> Option<Value> {
        let site = site?;
        let slot = self
            .global_get_inline_cache
            .get(site.slot())
            .copied()
            .flatten()?;
        if slot.site != site {
            return None;
        }

        let entry = slot.entry;
        if entry.environment != environment {
            return None;
        }
        if self.heap.table_version(environment)? != entry.environment_version {
            return None;
        }
        match entry.source {
            GlobalGetInlineCacheSource::Environment => Some(entry.value),
            GlobalGetInlineCacheSource::StandardGlobals {
                standard_globals_version,
            } => (self.heap.table_version(self.standard_globals)? == standard_globals_version)
                .then_some(entry.value),
        }
    }

    fn update_global_get_inline_cache(
        &mut self,
        site: Option<InlineCacheSite>,
        environment: u32,
        key: Value,
    ) -> Result<()> {
        let Some(site) = site else {
            return Ok(());
        };
        if environment != self.globals {
            self.clear_global_get_inline_cache_site(site);
            return Ok(());
        }

        let environment_version = self.heap.table_version(environment).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "global environment handle is missing",
            })
        })?;

        if let Some(value) = self.raw_table_slot(environment, key)? {
            if value.kind() != ValueKind::Nil
                || self.standard_global_index_table(environment)?.is_some()
            {
                self.store_global_get_inline_cache(
                    site,
                    GlobalGetInlineCacheEntry {
                        environment,
                        environment_version,
                        source: GlobalGetInlineCacheSource::Environment,
                        value,
                    },
                );
                return Ok(());
            }
            self.clear_global_get_inline_cache_site(site);
            return Ok(());
        }

        if self.standard_global_index_table(environment)?.is_some() {
            let standard_globals_version = self
                .heap
                .table_version(self.standard_globals)
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "standard global table handle is missing",
                    })
                })?;
            let standard_metatable = self
                .heap
                .table_metatable(self.standard_globals)
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "standard global table handle is missing",
                    })
                })?;
            let value = self.raw_table_slot(self.standard_globals, key)?;
            if let Some(value) = value {
                if value.kind() != ValueKind::Nil || standard_metatable.is_none() {
                    self.store_global_get_inline_cache(
                        site,
                        GlobalGetInlineCacheEntry {
                            environment,
                            environment_version,
                            source: GlobalGetInlineCacheSource::StandardGlobals {
                                standard_globals_version,
                            },
                            value,
                        },
                    );
                    return Ok(());
                }
            } else if standard_metatable.is_none() {
                self.store_global_get_inline_cache(
                    site,
                    GlobalGetInlineCacheEntry {
                        environment,
                        environment_version,
                        source: GlobalGetInlineCacheSource::StandardGlobals {
                            standard_globals_version,
                        },
                        value: Value::nil(),
                    },
                );
                return Ok(());
            }
        }

        self.clear_global_get_inline_cache_site(site);
        Ok(())
    }

    fn store_global_get_inline_cache(
        &mut self,
        site: InlineCacheSite,
        entry: GlobalGetInlineCacheEntry,
    ) {
        if let Some(slot) = self.global_get_inline_cache.get_mut(site.slot()) {
            *slot = Some(GlobalGetInlineCacheSlot { site, entry });
        }
    }

    fn clear_global_get_inline_cache_site(&mut self, site: InlineCacheSite) {
        if let Some(slot) = self.global_get_inline_cache.get_mut(site.slot()) {
            let should_clear = slot.as_ref().is_some_and(|slot| slot.site == site);
            if should_clear {
                *slot = None;
            }
        }
    }

    fn assert_message(&mut self, registers: &[Value], instruction: Instruction) -> Result<String> {
        if instruction.aux == ASSERT_MESSAGE_MISSING_ARGUMENT {
            return Ok("missing argument #1".to_owned());
        }
        if instruction.aux == ASSERT_MESSAGE_NONE {
            return Ok("assertion failed!".to_owned());
        }
        if instruction.aux < 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "unknown assert message mode",
            }));
        }
        if instruction.aux > i32::from(u8::MAX) {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: instruction.aux as usize,
                max: registers.len(),
            }));
        }

        let message = get_register(registers, instruction.aux as u8)?;
        let Some(atom) = message.as_string_atom() else {
            return Ok("assertion failed!".to_owned());
        };
        self.heap.string_lossy(atom).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "assert message string atom is missing",
            })
        })
    }

    pub fn string_value(&mut self, value: Value) -> Result<Option<String>> {
        let Some(atom) = value.as_string_atom() else {
            return Ok(None);
        };
        self.heap.string(atom).map(Some).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string atom is missing",
            })
        })
    }

    pub fn string_value_bytes(&mut self, value: Value) -> Result<Option<Vec<u8>>> {
        let Some(atom) = value.as_string_atom() else {
            return Ok(None);
        };
        self.heap.string_bytes(atom).map(Some).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string atom is missing",
            })
        })
    }

    fn coerce_to_string(&mut self, registers: &[Value], register: u8) -> Result<Vec<u8>> {
        let value = get_register(registers, register)?;
        self.value_to_string(value)
    }

    fn value_to_string(&mut self, value: Value) -> Result<Vec<u8>> {
        if let Some(text) = self.string_value_bytes(value)? {
            return Ok(text);
        }
        if let Some(number) = value.as_number() {
            return Ok(luau_number_to_string(number).into_bytes());
        }
        Err(Error::new(ErrorKind::TypeError {
            expected: "string or number",
        }))
    }

    fn string_byte_len(&mut self, value: Value) -> Result<usize> {
        if let Some(atom) = value.as_string_atom() {
            return self.heap.string_atom_byte_len(atom).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string atom is missing",
                })
            });
        }
        Ok(self.value_to_string(value)?.len())
    }

    fn value_to_tostring(&mut self, value: Value) -> Result<Vec<u8>> {
        match value.kind() {
            ValueKind::Nil => Ok(b"nil".to_vec()),
            ValueKind::Boolean => Ok(if value.as_bool().unwrap_or(false) {
                b"true".to_vec()
            } else {
                b"false".to_vec()
            }),
            ValueKind::Number => {
                Ok(luau_number_to_string(value.as_number().unwrap_or(f64::NAN)).into_bytes())
            }
            ValueKind::String => self.string_value_bytes(value)?.ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string atom is missing",
                })
            }),
            ValueKind::Table => {
                let table = value.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })?;
                if !self.heap.has_table(table) {
                    return Err(Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    }));
                }
                Ok(format!("table: 0x{table:08x}").into_bytes())
            }
            ValueKind::Function => {
                let function = value.as_function().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "function handle is missing",
                    })
                })?;
                if self.heap.function_kind(function).is_none() {
                    return Err(Error::new(ErrorKind::InvalidProgram {
                        reason: "function handle is missing",
                    }));
                }
                Ok(format!("function: 0x{function:08x}").into_bytes())
            }
            ValueKind::Userdata => {
                let userdata = value.as_userdata().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "userdata handle is missing",
                    })
                })?;
                if !self.heap.has_userdata(userdata) {
                    return Err(Error::new(ErrorKind::InvalidProgram {
                        reason: "userdata handle is missing",
                    }));
                }
                if self.coroutine_for_handle(userdata).is_some() {
                    return Ok(format!("thread: 0x{userdata:08x}").into_bytes());
                }
                Ok(format!("userdata: 0x{userdata:08x}").into_bytes())
            }
            ValueKind::Buffer => {
                let buffer = value.as_buffer().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "buffer handle is missing",
                    })
                })?;
                if !self.heap.has_buffer(buffer) {
                    return Err(Error::new(ErrorKind::InvalidProgram {
                        reason: "buffer handle is missing",
                    }));
                }
                Ok(format!("buffer: 0x{buffer:08x}").into_bytes())
            }
            ValueKind::Vector => {
                let vector = self.vector_data(value)?;
                Ok(format_vector(vector).into_bytes())
            }
        }
    }

    fn luau_type_name(&self, value: Value) -> &'static str {
        if value
            .as_userdata()
            .and_then(|handle| self.coroutine_for_handle(handle))
            .is_some()
        {
            "thread"
        } else {
            value_type_name(value)
        }
    }

    fn value_to_tostring_with_metamethod(
        &mut self,
        program: &Program,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<u8>> {
        let Some(method) = self.tostring_metamethod(value)? else {
            return self.value_to_tostring(value);
        };
        if method.kind() != ValueKind::Function {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "function",
            }));
        }
        let results = self.call_value(program, method, &[value], depth.saturating_add(1), fuel)?;
        let value = first_result_or_nil(results);
        self.string_value_bytes(value)?.ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: "'__tostring' must return a string".to_owned(),
            })
        })
    }

    fn tostring_metamethod(&mut self, value: Value) -> Result<Option<Value>> {
        let Some(metatable) = self.metatable_for_value(value)? else {
            return Ok(None);
        };
        let key = Value::string_atom(self.heap.intern_string("__tostring"));
        let method = self.raw_table_get(metatable, key)?;
        if method.kind() == ValueKind::Nil {
            Ok(None)
        } else {
            Ok(Some(method))
        }
    }

    fn metatable_for_value(&mut self, value: Value) -> Result<Option<u32>> {
        match value.kind() {
            ValueKind::Table => {
                let table = value.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })?;
                self.heap.table_metatable(table).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })
            }
            ValueKind::Userdata => {
                let userdata = value.as_userdata().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "userdata handle is missing",
                    })
                })?;
                self.heap.userdata_metatable(userdata).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "userdata handle is missing",
                    })
                })
            }
            _ => Ok(None),
        }
    }

    fn protected_metatable_value(&mut self, metatable: u32) -> Result<Option<Value>> {
        let key = Value::string_atom(self.heap.intern_string("__metatable"));
        let value = self.raw_table_get(metatable, key)?;
        if value.kind() == ValueKind::Nil {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    fn table_has_protected_metatable(&mut self, table: u32) -> Result<bool> {
        let Some(metatable) = self.heap.table_metatable(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?
        else {
            return Ok(false);
        };
        Ok(self.protected_metatable_value(metatable)?.is_some())
    }

    fn coerce_to_number(&mut self, value: Value) -> Result<Option<f64>> {
        if let Some(number) = value.as_number() {
            return Ok(Some(number));
        }
        let Some(atom) = value.as_string_atom() else {
            return Ok(None);
        };
        let Some(text) = self.heap.string(atom) else {
            return Ok(None);
        };
        Ok(parse_tonumber_string(&text))
    }

    fn coerce_to_i32(&mut self, value: Value, expected: &'static str) -> Result<i32> {
        let Some(number) = self.coerce_to_number(value)? else {
            return Err(Error::new(ErrorKind::TypeError { expected }));
        };
        let truncated = number.trunc();
        if !truncated.is_finite()
            || truncated < f64::from(i32::MIN)
            || truncated > f64::from(i32::MAX)
        {
            return Err(Error::new(ErrorKind::TypeError { expected }));
        }
        Ok(truncated as i32)
    }

    fn coerce_register_to_number(&mut self, registers: &[Value], register: u8) -> Result<f64> {
        let value = get_register(registers, register)?;
        self.coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))
    }

    fn numeric_binop(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        op: NumericOp,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let lhs = get_register(registers, instruction.b)?;
        let rhs = get_register(registers, instruction.c)?;
        if let (Some(lhs), Some(rhs)) = (lhs.as_number(), rhs.as_number()) {
            return set_register(
                registers,
                instruction.a,
                Value::number(binary(op, lhs, rhs)),
            );
        }
        let result = self.numeric_binop_value(program, lhs, rhs, op, site, depth, fuel)?;
        set_register(registers, instruction.a, result)
    }

    fn numeric_binop_const(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        op: NumericOp,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let lhs = get_register(registers, instruction.b)?;
        let rhs = self.number_constant(program, instruction.aux)?;
        if let Some(lhs) = lhs.as_number() {
            return set_register(
                registers,
                instruction.a,
                Value::number(binary(op, lhs, rhs)),
            );
        }
        let result =
            self.numeric_binop_value(program, lhs, Value::number(rhs), op, site, depth, fuel)?;
        set_register(registers, instruction.a, result)
    }

    fn numeric_mul_mod_const(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let lhs = get_register(registers, instruction.b)?;
        let (multiplier, modulus) = self.number_constant_pair(program, instruction.aux)?;
        if let Some(lhs) = lhs.as_number() {
            return set_register(
                registers,
                instruction.a,
                Value::number(binary(
                    NumericOp::Mod,
                    binary(NumericOp::Mul, lhs, multiplier),
                    modulus,
                )),
            );
        }

        let product = self.numeric_binop_value(
            program,
            lhs,
            Value::number(multiplier),
            NumericOp::Mul,
            site,
            depth,
            fuel,
        )?;
        let result = if let Some(product) = product.as_number() {
            Value::number(binary(NumericOp::Mod, product, modulus))
        } else {
            self.numeric_binop_value(
                program,
                product,
                Value::number(modulus),
                NumericOp::Mod,
                site,
                depth,
                fuel,
            )?
        };
        set_register(registers, instruction.a, result)
    }

    fn numeric_mul_add_mod_const(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let factor = get_register(registers, instruction.b)?;
        let addend = get_register(registers, instruction.c)?;
        let (multiplier, modulus) = self.number_constant_pair(program, instruction.aux)?;
        if let (Some(factor), Some(addend)) = (factor.as_number(), addend.as_number()) {
            return set_register(
                registers,
                instruction.a,
                Value::number(binary(
                    NumericOp::Mod,
                    binary(
                        NumericOp::Add,
                        binary(NumericOp::Mul, factor, multiplier),
                        addend,
                    ),
                    modulus,
                )),
            );
        }

        let product = self.numeric_binop_value(
            program,
            factor,
            Value::number(multiplier),
            NumericOp::Mul,
            site,
            depth,
            fuel,
        )?;
        let sum =
            self.numeric_binop_value(program, product, addend, NumericOp::Add, site, depth, fuel)?;
        let result = if let Some(sum) = sum.as_number() {
            Value::number(binary(NumericOp::Mod, sum, modulus))
        } else {
            self.numeric_binop_value(
                program,
                sum,
                Value::number(modulus),
                NumericOp::Mod,
                site,
                depth,
                fuel,
            )?
        };
        set_register(registers, instruction.a, result)
    }

    fn set_index_mul_add_mod_const(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let object = get_register(registers, instruction.a)?;
        let key = get_register(registers, instruction.b)?;
        let addend = get_register(registers, instruction.c)?;
        let (multiplier, modulus) = self.number_constant_pair(program, instruction.aux)?;
        let value = if let (Some(factor), Some(addend)) = (key.as_number(), addend.as_number()) {
            Value::number(binary(
                NumericOp::Mod,
                binary(
                    NumericOp::Add,
                    binary(NumericOp::Mul, factor, multiplier),
                    addend,
                ),
                modulus,
            ))
        } else {
            let product = self.numeric_binop_value(
                program,
                key,
                Value::number(multiplier),
                NumericOp::Mul,
                site,
                depth,
                fuel,
            )?;
            let sum = self.numeric_binop_value(
                program,
                product,
                addend,
                NumericOp::Add,
                site,
                depth,
                fuel,
            )?;
            if let Some(sum) = sum.as_number() {
                Value::number(binary(NumericOp::Mod, sum, modulus))
            } else {
                self.numeric_binop_value(
                    program,
                    sum,
                    Value::number(modulus),
                    NumericOp::Mod,
                    site,
                    depth,
                    fuel,
                )?
            }
        };

        if let Some(table) = object.as_table() {
            if let Some(index) = key.as_number().and_then(array_iterator_index) {
                if self.set_table_integer_index_fast(table, index, value)? {
                    return Ok(());
                }
            }
        }
        self.set_index_value(program, object, key, value, depth, fuel)
    }

    fn set_index_mul_mod_const(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let object = get_register(registers, instruction.a)?;
        let factor = get_register(registers, instruction.b)?;
        let value = get_register(registers, instruction.c)?;
        let (multiplier, modulus) = self.number_constant_pair(program, instruction.aux)?;
        let key = if let Some(factor) = factor.as_number() {
            Value::number(binary(
                NumericOp::Mod,
                binary(NumericOp::Mul, factor, multiplier),
                modulus,
            ))
        } else {
            let product = self.numeric_binop_value(
                program,
                factor,
                Value::number(multiplier),
                NumericOp::Mul,
                site,
                depth,
                fuel,
            )?;
            if let Some(product) = product.as_number() {
                Value::number(binary(NumericOp::Mod, product, modulus))
            } else {
                self.numeric_binop_value(
                    program,
                    product,
                    Value::number(modulus),
                    NumericOp::Mod,
                    site,
                    depth,
                    fuel,
                )?
            }
        };

        if let Some(table) = object.as_table() {
            if let Some(index) = key.as_number().and_then(array_iterator_index) {
                if self.set_table_integer_index_fast(table, index, value)? {
                    return Ok(());
                }
            }
        }
        self.set_index_value(program, object, key, value, depth, fuel)
    }

    fn numeric_mod_add_const(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let lhs = get_register(registers, instruction.b)?;
        let (modulus, addend) = self.number_constant_pair(program, instruction.aux)?;
        if let Some(lhs) = lhs.as_number() {
            return set_register(
                registers,
                instruction.a,
                Value::number(binary(
                    NumericOp::Add,
                    binary(NumericOp::Mod, lhs, modulus),
                    addend,
                )),
            );
        }

        let modulo = self.numeric_binop_value(
            program,
            lhs,
            Value::number(modulus),
            NumericOp::Mod,
            site,
            depth,
            fuel,
        )?;
        let result = if let Some(modulo) = modulo.as_number() {
            Value::number(binary(NumericOp::Add, modulo, addend))
        } else {
            self.numeric_binop_value(
                program,
                modulo,
                Value::number(addend),
                NumericOp::Add,
                site,
                depth,
                fuel,
            )?
        };
        set_register(registers, instruction.a, result)
    }

    fn numeric_unop(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        op: NumericOp,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let value = get_register(registers, instruction.b)?;
        if let Some(number) = value.as_number() {
            return set_register(
                registers,
                instruction.a,
                Value::number(binary(op, number, 0.0)),
            );
        }
        let result = self.numeric_unop_value(program, value, op, depth, fuel)?;
        set_register(registers, instruction.a, result)
    }

    fn add_upvalue(
        &mut self,
        program: &Program,
        upvalues: &[u32],
        registers: &mut [Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<(u32, Value)> {
        let target_index = upvalue_index(instruction.aux)?;
        let target = upvalues.get(target_index).copied().ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "upvalue index is missing",
            })
        })?;
        let rhs = upvalues
            .get(usize::from(instruction.b))
            .copied()
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "upvalue index is missing",
                })
            })?;
        if usize::from(instruction.a) >= registers.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: usize::from(instruction.a),
                max: registers.len(),
            }));
        }
        let (lhs, rhs_value) =
            match self
                .heap
                .upvalue_add_number_pair(target, rhs)
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "upvalue handle is missing",
                    })
                })? {
                UpvalueAddStatus::Stored(result) => {
                    set_register(registers, instruction.a, result)?;
                    return Ok((target, result));
                }
                UpvalueAddStatus::Deferred(lhs, rhs) => (lhs, rhs),
            };
        let result =
            self.numeric_binop_value(program, lhs, rhs_value, NumericOp::Add, site, depth, fuel)?;
        self.heap.upvalue_set(target, result).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "upvalue handle is missing",
            })
        })?;
        set_register(registers, instruction.a, result)?;
        Ok((target, result))
    }

    fn add_upvalue_const(
        &mut self,
        program: &Program,
        upvalues: &[u32],
        registers: &mut [Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<(u32, Value)> {
        let target = upvalues
            .get(usize::from(instruction.b))
            .copied()
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "upvalue index is missing",
                })
            })?;
        if usize::from(instruction.a) >= registers.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: usize::from(instruction.a),
                max: registers.len(),
            }));
        }
        let rhs = self.number_constant(program, instruction.aux)?;
        let lhs = match self
            .heap
            .upvalue_add_number_const(target, rhs)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "upvalue handle is missing",
                })
            })? {
            UpvalueAddConstStatus::Stored(result) => {
                set_register(registers, instruction.a, result)?;
                return Ok((target, result));
            }
            UpvalueAddConstStatus::Deferred(lhs) => lhs,
        };
        let result = self.numeric_binop_value(
            program,
            lhs,
            Value::number(rhs),
            NumericOp::Add,
            site,
            depth,
            fuel,
        )?;
        self.heap.upvalue_set(target, result).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "upvalue handle is missing",
            })
        })?;
        set_register(registers, instruction.a, result)?;
        Ok((target, result))
    }

    fn numeric_binop_value(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        op: NumericOp,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if lhs.kind() == ValueKind::Vector || rhs.kind() == ValueKind::Vector {
            return self.vector_binop_value(lhs, rhs, op);
        }
        if let (Some(lhs_number), Some(rhs_number)) =
            (self.coerce_to_number(lhs)?, self.coerce_to_number(rhs)?)
        {
            return Ok(Value::number(binary(op, lhs_number, rhs_number)));
        }
        let Some(method_name) = arithmetic_metamethod_name(op) else {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        };
        if let Some(method) =
            self.left_arithmetic_metamethod_with_inline_cache(site, lhs, op, method_name)?
        {
            let results =
                self.call_value(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)?;
            return Ok(first_result_or_nil(results));
        }
        if let Some(method) = self.metamethod_for_value(rhs, method_name)? {
            let results =
                self.call_value(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)?;
            Ok(first_result_or_nil(results))
        } else {
            Err(arithmetic_error(op, lhs, rhs))
        }
    }

    fn left_arithmetic_metamethod_with_inline_cache(
        &mut self,
        site: Option<InlineCacheSite>,
        lhs: Value,
        op: NumericOp,
        name: &'static str,
    ) -> Result<Option<Value>> {
        if let Some(method) = self.arithmetic_metamethod_inline_cache_hit(site, lhs, op)? {
            return Ok(Some(method));
        }
        let Some(metatable) = self.metatable_for_value(lhs)? else {
            return Ok(None);
        };
        let key = Value::string_atom(self.heap.intern_string(name));
        let method = self.raw_table_get(metatable, key)?;
        if method.kind() == ValueKind::Nil {
            return Ok(None);
        }
        if let Some(site) = site {
            let metatable_version = self.heap.table_version(metatable).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "arithmetic metatable handle is missing",
                })
            })?;
            self.store_arithmetic_metamethod_inline_cache(
                site,
                ArithmeticMetamethodInlineCacheEntry {
                    op,
                    metatable,
                    metatable_version,
                    method,
                },
            );
        }
        Ok(Some(method))
    }

    fn arithmetic_metamethod_inline_cache_hit(
        &mut self,
        site: Option<InlineCacheSite>,
        lhs: Value,
        op: NumericOp,
    ) -> Result<Option<Value>> {
        let Some(site) = site else {
            return Ok(None);
        };
        let slot = self
            .arithmetic_metamethod_inline_cache
            .get(site.slot())
            .copied()
            .flatten();
        let Some(slot) = slot else {
            return Ok(None);
        };
        if slot.site != site || slot.entry.op != op {
            return Ok(None);
        }
        if self.metatable_for_value(lhs)? != Some(slot.entry.metatable) {
            return Ok(None);
        }
        Ok(
            (self.heap.table_version(slot.entry.metatable) == Some(slot.entry.metatable_version))
                .then_some(slot.entry.method),
        )
    }

    fn store_arithmetic_metamethod_inline_cache(
        &mut self,
        site: InlineCacheSite,
        entry: ArithmeticMetamethodInlineCacheEntry,
    ) {
        if let Some(slot) = self.arithmetic_metamethod_inline_cache.get_mut(site.slot()) {
            *slot = Some(ArithmeticMetamethodInlineCacheSlot { site, entry });
        }
    }

    fn numeric_unop_value(
        &mut self,
        program: &Program,
        value: Value,
        op: NumericOp,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if value.kind() == ValueKind::Vector && op == NumericOp::Neg {
            let vector = self.vector_data(value)?;
            return self.alloc_vector_value(-vector.x(), -vector.y(), -vector.z());
        }
        if let Some(number) = self.coerce_to_number(value)? {
            return Ok(Value::number(binary(op, number, 0.0)));
        }
        let Some(method_name) = arithmetic_metamethod_name(op) else {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        };
        if let Some(method) = self.metamethod_for_value(value, method_name)? {
            let results =
                self.call_value(program, method, &[value], depth.saturating_add(1), fuel)?;
            Ok(first_result_or_nil(results))
        } else {
            Err(unary_arithmetic_error(op, value))
        }
    }

    fn binary_metamethod(
        &mut self,
        lhs: Value,
        rhs: Value,
        name: &'static str,
    ) -> Result<Option<Value>> {
        if let Some(method) = self.metamethod_for_value(lhs, name)? {
            return Ok(Some(method));
        }
        self.metamethod_for_value(rhs, name)
    }

    fn compatible_binary_metamethod(
        &mut self,
        lhs: Value,
        rhs: Value,
        name: &'static str,
    ) -> Result<Option<Value>> {
        let Some(lhs_method) = self.metamethod_for_value(lhs, name)? else {
            return Ok(None);
        };
        let Some(rhs_method) = self.metamethod_for_value(rhs, name)? else {
            return Ok(None);
        };
        if lhs_method.luau_equals(rhs_method) {
            Ok(Some(lhs_method))
        } else {
            Ok(None)
        }
    }

    fn compare_equal(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        invert: bool,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let lhs = get_register(registers, instruction.b)?;
        let rhs = get_register(registers, instruction.c)?;
        let result = self.equal_values(program, lhs, rhs, depth, fuel)? ^ invert;
        set_register(registers, instruction.a, Value::boolean(result))
    }

    fn equal_values(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        if let Some(method) = self.compatible_binary_metamethod(lhs, rhs, "__eq")? {
            let results =
                self.call_value(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)?;
            return Ok(first_result_or_nil(results).truthy());
        }
        self.raw_equal_values(lhs, rhs)
    }

    fn raw_equal_values(&mut self, lhs: Value, rhs: Value) -> Result<bool> {
        if lhs.kind() == ValueKind::Vector || rhs.kind() == ValueKind::Vector {
            if lhs.kind() != ValueKind::Vector || rhs.kind() != ValueKind::Vector {
                return Ok(false);
            }
            let lhs = self.vector_data(lhs)?;
            let rhs = self.vector_data(rhs)?;
            return Ok(vector_components_equal(lhs, rhs));
        }
        Ok(lhs.luau_equals(rhs))
    }

    fn compare_order(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        comparison: OrderComparison,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let lhs = get_register(registers, instruction.b)?;
        let rhs = get_register(registers, instruction.c)?;
        let result = self.order_values(program, lhs, rhs, comparison, depth, fuel)?;
        set_register(registers, instruction.a, Value::boolean(result))
    }

    fn order_values(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        comparison: OrderComparison,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        match comparison {
            OrderComparison::Less => self.less_than(program, lhs, rhs, depth, fuel),
            OrderComparison::LessEqual => self.less_equal(program, lhs, rhs, depth, fuel),
            OrderComparison::Greater => self.less_than(program, rhs, lhs, depth, fuel),
            OrderComparison::GreaterEqual => self.less_equal(program, rhs, lhs, depth, fuel),
        }
    }

    fn less_than(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        if let Some(result) = self.raw_order(lhs, rhs, OrderComparison::Less)? {
            return Ok(result);
        }
        let Some(method) = self.compatible_binary_metamethod(lhs, rhs, "__lt")? else {
            return Err(order_error(lhs, rhs, "<"));
        };
        let results =
            self.call_value(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)?;
        Ok(first_result_or_nil(results).truthy())
    }

    fn less_equal(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        if let Some(result) = self.raw_order(lhs, rhs, OrderComparison::LessEqual)? {
            return Ok(result);
        }
        if let Some(method) = self.compatible_binary_metamethod(lhs, rhs, "__le")? {
            let results =
                self.call_value(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)?;
            return Ok(first_result_or_nil(results).truthy());
        }
        let Some(method) = self.compatible_binary_metamethod(rhs, lhs, "__lt")? else {
            return Err(order_error(lhs, rhs, "<="));
        };
        let results =
            self.call_value(program, method, &[rhs, lhs], depth.saturating_add(1), fuel)?;
        Ok(!first_result_or_nil(results).truthy())
    }

    fn raw_order(
        &mut self,
        lhs: Value,
        rhs: Value,
        comparison: OrderComparison,
    ) -> Result<Option<bool>> {
        match (lhs.kind(), rhs.kind()) {
            (ValueKind::Number, ValueKind::Number) => {
                let lhs = lhs.as_number().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "number value missing payload",
                    })
                })?;
                let rhs = rhs.as_number().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "number value missing payload",
                    })
                })?;
                Ok(Some(compare_ordering(comparison, lhs.partial_cmp(&rhs))))
            }
            (ValueKind::String, ValueKind::String) => {
                let lhs = self.string_value_bytes(lhs)?;
                let rhs = self.string_value_bytes(rhs)?;
                let lhs = lhs.ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "string value missing atom",
                    })
                })?;
                let rhs = rhs.ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "string value missing atom",
                    })
                })?;
                Ok(Some(compare_ordering(comparison, Some(lhs.cmp(&rhs)))))
            }
            _ => Ok(None),
        }
    }

    fn concat_value(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let lhs = get_register(registers, instruction.b)?;
        let rhs = get_register(registers, instruction.c)?;
        self.concat_operands(program, lhs, rhs, depth, fuel)
    }

    fn concat_const_value(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        self.concat_const_key(
            program,
            program_id,
            registers,
            instruction.b,
            instruction.aux,
            depth,
            fuel,
        )
    }

    fn concat_rhs_const_value(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if instruction.aux < 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "negative concat constant index",
            }));
        }
        let lhs = get_register(registers, instruction.b)?;
        if let Some(rhs) =
            self.constant_string_atom(program, program_id, instruction.aux as usize)?
        {
            if let Some(lhs) = lhs.as_string_atom() {
                if let Some(atom) = self.heap.intern_concat_string_atoms(lhs, rhs) {
                    return Ok(Value::string_atom(atom));
                }
            } else if let Some(integer) = lhs.as_number().and_then(small_positive_integer_number) {
                if let Some(atom) = self
                    .heap
                    .intern_concat_small_integer_string_atom(integer, rhs)
                {
                    return Ok(Value::string_atom(atom));
                }
            }
        }
        let rhs = self.load_constant(program, instruction.aux as usize)?;
        self.concat_operands(program, lhs, rhs, depth, fuel)
    }

    fn concat_const_key(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        registers: &[Value],
        rhs_register: u8,
        constant_index: i32,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if constant_index < 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "negative concat constant index",
            }));
        }
        if let Some(lhs) =
            self.constant_string_atom(program, program_id, constant_index as usize)?
        {
            let rhs = get_register(registers, rhs_register)?;
            if let Some(rhs) = rhs.as_string_atom() {
                if let Some(atom) = self.heap.intern_concat_string_atoms(lhs, rhs) {
                    return Ok(Value::string_atom(atom));
                }
            } else if let Some(integer) = rhs.as_number().and_then(small_positive_integer_number) {
                if let Some(atom) = self
                    .heap
                    .intern_concat_string_atom_small_integer(lhs, integer)
                {
                    return Ok(Value::string_atom(atom));
                }
            }
        }
        let lhs = self.load_constant(program, constant_index as usize)?;
        let rhs = get_register(registers, rhs_register)?;
        self.concat_operands(program, lhs, rhs, depth, fuel)
    }

    fn concat_operands(
        &mut self,
        program: &Program,
        lhs: Value,
        rhs: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if let Some(value) = self.concat_scalar_operands(lhs, rhs)? {
            return Ok(value);
        }
        if let (Ok(mut lhs_text), Ok(rhs_text)) =
            (self.value_to_string(lhs), self.value_to_string(rhs))
        {
            lhs_text.extend_from_slice(&rhs_text);
            let atom = self.heap.intern_string_bytes(&lhs_text);
            return Ok(Value::string_atom(atom));
        }
        if let Some(method) = self.binary_metamethod(lhs, rhs, "__concat")? {
            let results =
                self.call_value(program, method, &[lhs, rhs], depth.saturating_add(1), fuel)?;
            Ok(first_result_or_nil(results))
        } else {
            Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "attempt to concatenate {} with {}",
                    value_type_name(lhs),
                    value_type_name(rhs)
                ),
            }))
        }
    }

    fn concat_scalar_operands(&mut self, lhs: Value, rhs: Value) -> Result<Option<Value>> {
        let atom = match (lhs.as_string_atom(), rhs.as_string_atom()) {
            (Some(lhs), Some(rhs)) => self.heap.intern_concat_string_atoms(lhs, rhs),
            (Some(lhs), None) => {
                let Some(number) = rhs.as_number() else {
                    return Ok(None);
                };
                if let Some(integer) = small_positive_integer_number(number) {
                    self.heap
                        .intern_concat_string_atom_small_integer(lhs, integer)
                } else {
                    let text = luau_number_to_string(number);
                    self.heap
                        .intern_concat_string_atom_bytes(lhs, text.as_bytes())
                }
            }
            (None, Some(rhs)) => {
                let Some(number) = lhs.as_number() else {
                    return Ok(None);
                };
                if let Some(integer) = small_positive_integer_number(number) {
                    self.heap
                        .intern_concat_small_integer_string_atom(integer, rhs)
                } else {
                    let text = luau_number_to_string(number);
                    self.heap
                        .intern_concat_bytes_string_atom(text.as_bytes(), rhs)
                }
            }
            (None, None) => {
                let (Some(lhs), Some(rhs)) = (lhs.as_number(), rhs.as_number()) else {
                    return Ok(None);
                };
                let lhs = luau_number_to_string(lhs);
                let rhs = luau_number_to_string(rhs);
                let mut text = Vec::with_capacity(lhs.len() + rhs.len());
                text.extend_from_slice(lhs.as_bytes());
                text.extend_from_slice(rhs.as_bytes());
                Some(self.heap.intern_string_bytes(&text))
            }
        };
        atom.map(|atom| Some(Value::string_atom(atom)))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string atom is missing",
                })
            })
    }

    fn raw_len_value(&mut self, value: Value) -> Result<usize> {
        if let Some(atom) = value.as_string_atom() {
            return self.heap.string_atom_byte_len(atom).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string atom is missing",
                })
            });
        }
        let Some(table) = value.as_table() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "table or string expected".to_owned(),
            }));
        };
        self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })
    }

    fn len_value(
        &mut self,
        program: &Program,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if value.kind() == ValueKind::String {
            return Ok(Value::number(self.raw_len_value(value)? as f64));
        }
        match value.kind() {
            ValueKind::Table | ValueKind::Userdata => {
                if let Some(method) = self.metamethod_for_value(value, "__len")? {
                    let results =
                        self.call_value(program, method, &[value], depth.saturating_add(1), fuel)?;
                    let result = first_result_or_nil(results);
                    if result.kind() == ValueKind::Number {
                        Ok(result)
                    } else {
                        Err(Error::new(ErrorKind::RuntimeError {
                            message: "'__len' must return a number".to_owned(),
                        }))
                    }
                } else {
                    Ok(Value::number(self.raw_len_value(value)? as f64))
                }
            }
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "table or string",
            })),
        }
    }

    fn len_value_with_inline_cache(
        &mut self,
        program: &Program,
        site: Option<InlineCacheSite>,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let Some(table) = value.as_table() else {
            return self.len_value(program, value, depth, fuel);
        };
        let Some(site) = site else {
            return self.len_value(program, value, depth, fuel);
        };
        if let Some(len) = self.len_inline_cache_hit(site, table) {
            return Ok(Value::number(len as f64));
        }
        if let Some(len) = self.try_len_table_for_inline_cache(site, table)? {
            return Ok(Value::number(len as f64));
        }
        self.clear_len_inline_cache_site(site);
        self.len_value(program, value, depth, fuel)
    }

    fn len_plus_one_value(
        &mut self,
        program: &Program,
        site: Option<InlineCacheSite>,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if let Some(table) = value.as_table() {
            if let Some(site) = site {
                if let Some(len) = self.len_inline_cache_hit(site, table) {
                    return Ok(Value::number(len as f64 + 1.0));
                }
                if let Some(len) = self.try_len_table_for_inline_cache(site, table)? {
                    return Ok(Value::number(len as f64 + 1.0));
                }
                self.clear_len_inline_cache_site(site);
            }
        }
        let len = self.len_value(program, value, depth, fuel)?;
        self.numeric_binop_value(
            program,
            len,
            Value::number(1.0),
            NumericOp::Add,
            None,
            depth,
            fuel,
        )
    }

    fn len_inline_cache_hit(&mut self, site: InlineCacheSite, table: u32) -> Option<usize> {
        let slot = self.len_inline_cache.get(site.slot()).copied().flatten()?;
        if slot.site != site || slot.entry.table != table {
            return None;
        }
        let entry = slot.entry;
        (self.heap.table_version(table)? == entry.table_version).then_some(entry.len)
    }

    fn try_len_table_for_inline_cache(
        &mut self,
        site: InlineCacheSite,
        table: u32,
    ) -> Result<Option<usize>> {
        let table_version = self.heap.table_version(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        if self
            .heap
            .table_metatable(table)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?
            .is_some()
        {
            return Ok(None);
        }
        let len = self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        self.store_len_inline_cache(
            site,
            LenInlineCacheEntry {
                table,
                table_version,
                len,
            },
        );
        Ok(Some(len))
    }

    fn store_len_inline_cache(&mut self, site: InlineCacheSite, entry: LenInlineCacheEntry) {
        if let Some(slot) = self.len_inline_cache.get_mut(site.slot()) {
            *slot = Some(LenInlineCacheSlot { site, entry });
        }
    }

    fn clear_len_inline_cache_site(&mut self, site: InlineCacheSite) {
        if let Some(slot) = self.len_inline_cache.get_mut(site.slot()) {
            let should_clear = slot.as_ref().is_some_and(|slot| slot.site == site);
            if should_clear {
                *slot = None;
            }
        }
    }

    fn get_index_value(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        self.check_recursion_depth(depth)?;
        if let Some(table) = object.as_table() {
            return self.get_table_index_with_metamethod(program, table, key, depth, fuel);
        }
        if object.kind() == ValueKind::String {
            return self.string_method_index(key);
        }
        if object.kind() == ValueKind::Vector {
            return self.vector_index_value(object, key);
        }
        if object.kind() == ValueKind::Userdata {
            return self.get_object_index_metamethod(program, object, key, depth, fuel);
        }
        Err(self.index_error(object, key)?)
    }

    fn get_index_mod_add_const(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let object = get_register(registers, instruction.b)?;
        let factor = get_register(registers, instruction.c)?;
        let (modulus, addend) = self.number_constant_pair(program, instruction.aux)?;
        let key = if let Some(factor) = factor.as_number() {
            Value::number(binary(
                NumericOp::Add,
                binary(NumericOp::Mod, factor, modulus),
                addend,
            ))
        } else {
            let modulo = self.numeric_binop_value(
                program,
                factor,
                Value::number(modulus),
                NumericOp::Mod,
                site,
                depth,
                fuel,
            )?;
            if let Some(modulo) = modulo.as_number() {
                Value::number(binary(NumericOp::Add, modulo, addend))
            } else {
                self.numeric_binop_value(
                    program,
                    modulo,
                    Value::number(addend),
                    NumericOp::Add,
                    site,
                    depth,
                    fuel,
                )?
            }
        };

        if let Some(table) = object.as_table() {
            if let Some(index) = key.as_number().and_then(array_iterator_index) {
                if let Some(value) = self.get_table_integer_index_fast(table, index)? {
                    return Ok(value);
                }
            }
        }
        self.get_index_value(program, object, key, depth, fuel)
    }

    fn get_index_mul_mod_const(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        site: Option<InlineCacheSite>,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let object = get_register(registers, instruction.b)?;
        let factor = get_register(registers, instruction.c)?;
        let (multiplier, modulus) = self.number_constant_pair(program, instruction.aux)?;
        let key = if let Some(factor) = factor.as_number() {
            Value::number(binary(
                NumericOp::Mod,
                binary(NumericOp::Mul, factor, multiplier),
                modulus,
            ))
        } else {
            let product = self.numeric_binop_value(
                program,
                factor,
                Value::number(multiplier),
                NumericOp::Mul,
                site,
                depth,
                fuel,
            )?;
            if let Some(product) = product.as_number() {
                Value::number(binary(NumericOp::Mod, product, modulus))
            } else {
                self.numeric_binop_value(
                    program,
                    product,
                    Value::number(modulus),
                    NumericOp::Mod,
                    site,
                    depth,
                    fuel,
                )?
            }
        };

        if let Some(table) = object.as_table() {
            if let Some(index) = key.as_number().and_then(array_iterator_index) {
                if let Some(value) = self.get_table_integer_index_fast(table, index)? {
                    return Ok(value);
                }
            }
        }
        self.get_index_value(program, object, key, depth, fuel)
    }

    fn get_table_integer_index_fast(
        &mut self,
        table: u32,
        one_based_index: usize,
    ) -> Result<Option<Value>> {
        let Some(value) = self
            .heap
            .array_slot(table, one_based_index)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?
        else {
            return self.get_table_integer_hash_key_fast(table, one_based_index);
        };
        if value.kind() != ValueKind::Nil || self.table_uses_standard_global_type_surface(table)? {
            return Ok(Some(value));
        }
        if self
            .heap
            .table_metatable(table)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?
            .is_none()
        {
            return Ok(Some(Value::nil()));
        }
        Ok(None)
    }

    fn get_table_integer_hash_key_fast(
        &mut self,
        table: u32,
        one_based_index: usize,
    ) -> Result<Option<Value>> {
        match self
            .heap
            .read_integer_hash_key_without_metamethod(table, one_based_index)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })? {
            HashReadStatus::Hit(value) => Ok(Some(value)),
            HashReadStatus::Deferred => Ok(None),
        }
    }

    fn get_table_string_index_fast(&mut self, table: u32, key: Value) -> Result<Option<Value>> {
        if table != self.globals {
            if let Some(atom) = key.as_string_atom() {
                return match self
                    .heap
                    .read_string_key_without_metamethod(table, atom)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "table handle is missing",
                        })
                    })? {
                    HashReadStatus::Hit(value) => Ok(Some(value)),
                    HashReadStatus::Deferred => Ok(None),
                };
            }
        }
        if let Some(value) = self.raw_table_slot(table, key)? {
            if value.kind() != ValueKind::Nil
                || self.table_uses_standard_global_type_surface(table)?
            {
                return Ok(Some(value));
            }
        }
        if self
            .heap
            .table_metatable(table)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?
            .is_none()
        {
            return Ok(Some(Value::nil()));
        }
        Ok(None)
    }

    fn set_table_integer_index_fast(
        &mut self,
        table: u32,
        one_based_index: usize,
        value: Value,
    ) -> Result<bool> {
        match self
            .heap
            .write_integer_index_without_metamethod(table, one_based_index, value)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })? {
            ArrayWriteStatus::Stored => Ok(true),
            ArrayWriteStatus::Deferred => Ok(false),
            ArrayWriteStatus::Readonly => Err(readonly_table_error()),
        }
    }

    fn get_table_string_index_with_inline_cache(
        &mut self,
        program: &Program,
        site: InlineCacheSite,
        table: u32,
        key: Value,
        key_atom: u32,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if let Some(value) = self.table_get_inline_cache_hit(site, table, key, key_atom)? {
            return Ok(value);
        }
        if let Some(value) =
            self.try_get_table_string_index_for_inline_cache(site, table, key_atom)?
        {
            return Ok(value);
        }
        self.clear_table_get_inline_cache_site(site);
        self.get_table_index_with_metamethod(program, table, key, depth, fuel)
    }

    fn table_get_inline_cache_hit(
        &mut self,
        site: InlineCacheSite,
        table: u32,
        key: Value,
        key_atom: u32,
    ) -> Result<Option<Value>> {
        let slot = self
            .table_get_inline_cache
            .get(site.slot())
            .copied()
            .flatten();
        let Some(slot) = slot else {
            return Ok(None);
        };
        if slot.site != site {
            return Ok(None);
        }
        let entry = slot.entry;
        if entry.key_atom != key_atom {
            return Ok(None);
        }
        match entry.source {
            TableGetInlineCacheSource::DirectSlot {
                table: cached_table,
                table_shape_version,
                hash_index,
            } => {
                if cached_table != table {
                    return Ok(None);
                }
                let Some(slot) = self
                    .heap
                    .read_string_hash_slot_with_shape(
                        table,
                        key_atom,
                        table_shape_version,
                        hash_index,
                    )
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "table handle is missing",
                        })
                    })?
                else {
                    return Ok(None);
                };
                if slot.value.kind() != ValueKind::Nil
                    || !slot.has_metatable
                    || self.table_uses_standard_global_type_surface(table)?
                {
                    return Ok(Some(slot.value));
                }
                Ok(None)
            }
            TableGetInlineCacheSource::DirectAbsent {
                table: cached_table,
                table_shape_version,
            } => {
                if cached_table != table
                    || self.heap.table_shape_version(table) != Some(table_shape_version)
                {
                    return Ok(None);
                }
                if self
                    .heap
                    .table_metatable(table)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "table handle is missing",
                        })
                    })?
                    .is_some()
                {
                    return Ok(None);
                }
                Ok(Some(Value::nil()))
            }
            TableGetInlineCacheSource::IndexTable {
                metatable,
                metatable_shape_version,
                metatable_hash_index,
                index_table,
                index_table_shape_version,
                index_table_hash_index,
            } => {
                if let Some(raw) = self.raw_table_slot(table, key)? {
                    if raw.kind() != ValueKind::Nil
                        || self.table_uses_standard_global_type_surface(table)?
                    {
                        return Ok(None);
                    }
                }
                if self.heap.table_metatable(table).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })? != Some(metatable)
                {
                    return Ok(None);
                }
                let index_atom = self.heap.intern_string("__index");
                let Some(index_slot) = self
                    .heap
                    .read_string_hash_slot_with_shape(
                        metatable,
                        index_atom,
                        metatable_shape_version,
                        metatable_hash_index,
                    )
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "__index metatable handle is missing",
                        })
                    })?
                else {
                    return Ok(None);
                };
                if index_slot.value.as_table() != Some(index_table) {
                    return Ok(None);
                }
                let Some(value_slot) = self
                    .heap
                    .read_string_hash_slot_with_shape(
                        index_table,
                        key_atom,
                        index_table_shape_version,
                        index_table_hash_index,
                    )
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "__index table handle is missing",
                        })
                    })?
                else {
                    return Ok(None);
                };
                if value_slot.value.kind() == ValueKind::Nil {
                    return Ok(None);
                }
                Ok(Some(value_slot.value))
            }
        }
    }

    fn try_get_table_string_index_for_inline_cache(
        &mut self,
        site: InlineCacheSite,
        table: u32,
        key_atom: u32,
    ) -> Result<Option<Value>> {
        let table_shape_version = self.heap.table_shape_version(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        let hash_slot = self
            .heap
            .table_string_hash_slot(table, key_atom)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?;
        if let Some(slot) = hash_slot {
            let value = slot.value;
            if value.kind() != ValueKind::Nil
                || self.table_uses_standard_global_type_surface(table)?
            {
                self.store_table_get_inline_cache(
                    site,
                    TableGetInlineCacheEntry {
                        key_atom,
                        source: TableGetInlineCacheSource::DirectSlot {
                            table,
                            table_shape_version: slot.shape_version,
                            hash_index: slot.index,
                        },
                    },
                );
                return Ok(Some(value));
            }
        }
        let metatable = self.heap.table_metatable(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        let Some(metatable) = metatable else {
            let value = Value::nil();
            let source = hash_slot.map_or(
                TableGetInlineCacheSource::DirectAbsent {
                    table,
                    table_shape_version,
                },
                |slot| TableGetInlineCacheSource::DirectSlot {
                    table,
                    table_shape_version: slot.shape_version,
                    hash_index: slot.index,
                },
            );
            self.store_table_get_inline_cache(site, TableGetInlineCacheEntry { key_atom, source });
            return Ok(Some(value));
        };
        if let Some(value) =
            self.try_get_table_index_table_for_inline_cache(site, metatable, key_atom)?
        {
            return Ok(Some(value));
        }
        Ok(None)
    }

    fn try_get_table_index_table_for_inline_cache(
        &mut self,
        site: InlineCacheSite,
        metatable: u32,
        key_atom: u32,
    ) -> Result<Option<Value>> {
        let index_atom = self.heap.intern_string("__index");
        let Some(index_slot) = self
            .heap
            .table_string_hash_slot(metatable, index_atom)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "metatable handle is missing",
                })
            })?
        else {
            return Ok(None);
        };
        let Some(index_table) = index_slot.value.as_table() else {
            return Ok(None);
        };
        let Some(value_slot) = self
            .heap
            .table_string_hash_slot(index_table, key_atom)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "__index table handle is missing",
                })
            })?
        else {
            return Ok(None);
        };
        let value = value_slot.value;
        if value.kind() == ValueKind::Nil {
            return Ok(None);
        }
        self.store_table_get_inline_cache(
            site,
            TableGetInlineCacheEntry {
                key_atom,
                source: TableGetInlineCacheSource::IndexTable {
                    metatable,
                    metatable_shape_version: index_slot.shape_version,
                    metatable_hash_index: index_slot.index,
                    index_table,
                    index_table_shape_version: value_slot.shape_version,
                    index_table_hash_index: value_slot.index,
                },
            },
        );
        Ok(Some(value))
    }

    fn store_table_get_inline_cache(
        &mut self,
        site: InlineCacheSite,
        entry: TableGetInlineCacheEntry,
    ) {
        if let Some(slot) = self.table_get_inline_cache.get_mut(site.slot()) {
            *slot = Some(TableGetInlineCacheSlot { site, entry });
        }
    }

    fn clear_table_get_inline_cache_site(&mut self, site: InlineCacheSite) {
        if let Some(slot) = self.table_get_inline_cache.get_mut(site.slot()) {
            let should_clear = slot.as_ref().is_some_and(|slot| slot.site == site);
            if should_clear {
                *slot = None;
            }
        }
    }

    fn check_method_value(&mut self, callee: Value, object: Value, key: Value) -> Result<()> {
        if callee.kind() != ValueKind::Nil {
            return Ok(());
        }
        let method = self
            .string_value(key)?
            .unwrap_or_else(|| value_type_name(key).to_owned());
        Err(Error::new(ErrorKind::RuntimeError {
            message: format!(
                "attempt to call missing method '{method}' of {}",
                value_type_name(object)
            ),
        }))
    }

    fn index_error(&mut self, object: Value, key: Value) -> Result<Error> {
        let key_text = if let Some(name) = self.string_value(key)? {
            format!("'{name}'")
        } else {
            value_type_name(key).to_owned()
        };
        Ok(Error::new(ErrorKind::RuntimeError {
            message: format!(
                "attempt to index {} with {key_text}",
                value_type_name(object)
            ),
        }))
    }

    fn get_table_index_with_metamethod(
        &mut self,
        program: &Program,
        table: u32,
        key: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        self.check_recursion_depth(depth)?;
        if let Some(value) = self.raw_table_slot(table, key)? {
            if value.kind() != ValueKind::Nil
                || self.table_uses_standard_global_type_surface(table)?
            {
                return Ok(value);
            }
        }
        if self
            .heap
            .table_metatable(table)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?
            .is_none()
        {
            return Ok(Value::nil());
        }
        self.get_object_index_metamethod(program, Value::table(table), key, depth, fuel)
    }

    fn get_object_index_metamethod(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        self.check_recursion_depth(depth)?;
        let Some(method) = self.metamethod_for_value(object, "__index")? else {
            return if object.kind() == ValueKind::Table {
                Ok(Value::nil())
            } else if object.kind() == ValueKind::Userdata {
                self.metamethod_for_value(object, "__namecall")?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))
            } else {
                Err(Error::new(ErrorKind::TypeError { expected: "table" }))
            };
        };
        match method.kind() {
            ValueKind::Function => {
                let results = self.call_value(
                    program,
                    method,
                    &[object, key],
                    depth.saturating_add(1),
                    fuel,
                )?;
                Ok(first_result_or_nil(results))
            }
            ValueKind::Table => {
                let table = method.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "__index table handle is missing",
                    })
                })?;
                self.get_table_index_with_metamethod(
                    program,
                    table,
                    key,
                    depth.saturating_add(1),
                    fuel,
                )
            }
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "function or table",
            })),
        }
    }

    fn set_index_value(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        self.check_recursion_depth(depth)?;
        if let Some(table) = object.as_table() {
            return self.set_table_index_with_metamethod(program, table, key, value, depth, fuel);
        }
        if object.kind() == ValueKind::Userdata {
            return self.set_object_newindex_metamethod(program, object, key, value, depth, fuel);
        }
        Err(Error::new(ErrorKind::TypeError { expected: "table" }))
    }

    fn set_table_index_with_metamethod(
        &mut self,
        program: &Program,
        table: u32,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        self.check_recursion_depth(depth)?;
        let existing = self.raw_table_slot(table, key)?;
        if existing.is_some_and(|value| value.kind() != ValueKind::Nil) {
            return self.set_table_index(table, key, value);
        }
        if self
            .heap
            .table_metatable(table)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?
            .is_none()
        {
            if existing.is_none() {
                return self.set_table_absent_index(table, key, value);
            }
            return self.set_table_index(table, key, value);
        }
        if let Some(method) = self.metamethod_for_value(Value::table(table), "__newindex")? {
            return self.apply_newindex_metamethod(
                program,
                Value::table(table),
                method,
                [key, value],
                depth,
                fuel,
            );
        }
        self.set_table_index(table, key, value)
    }

    fn set_object_newindex_metamethod(
        &mut self,
        program: &Program,
        object: Value,
        key: Value,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        self.check_recursion_depth(depth)?;
        let Some(method) = self.metamethod_for_value(object, "__newindex")? else {
            return Err(Error::new(ErrorKind::TypeError { expected: "table" }));
        };
        self.apply_newindex_metamethod(program, object, method, [key, value], depth, fuel)
    }

    fn apply_newindex_metamethod(
        &mut self,
        program: &Program,
        object: Value,
        method: Value,
        entry: [Value; 2],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        self.check_recursion_depth(depth)?;
        match method.kind() {
            ValueKind::Function => {
                self.call_value(
                    program,
                    method,
                    &[object, entry[0], entry[1]],
                    depth.saturating_add(1),
                    fuel,
                )?;
                Ok(())
            }
            ValueKind::Table => {
                let table = method.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "__newindex table handle is missing",
                    })
                })?;
                self.set_table_index_with_metamethod(
                    program,
                    table,
                    entry[0],
                    entry[1],
                    depth.saturating_add(1),
                    fuel,
                )
            }
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "function or table",
            })),
        }
    }

    fn table_iter_metamethod_with_inline_cache(
        &mut self,
        site: Option<InlineCacheSite>,
        table: u32,
    ) -> Result<Option<Value>> {
        let Some(site) = site else {
            return self.table_iter_metamethod(table);
        };
        if let Some(cached) = self.table_iter_metamethod_inline_cache_hit(site, table)? {
            return Ok(cached);
        }
        let (entry, method) = self.table_iter_metamethod_cache_entry(table)?;
        self.store_iter_metamethod_inline_cache(site, entry);
        Ok(method)
    }

    fn table_iter_metamethod_inline_cache_hit(
        &mut self,
        site: InlineCacheSite,
        table: u32,
    ) -> Result<Option<Option<Value>>> {
        let slot = self
            .iter_metamethod_inline_cache
            .get(site.slot())
            .copied()
            .flatten();
        let Some(slot) = slot else {
            return Ok(None);
        };
        if slot.site != site || slot.entry.table != table {
            return Ok(None);
        }
        let entry = slot.entry;
        let table_version = self.heap.table_version(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        if table_version != entry.table_version {
            return Ok(None);
        }
        if let Some(metatable) = entry.metatable {
            let metatable_version = self.heap.table_version(metatable).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "__iter metatable handle is missing",
                })
            })?;
            if metatable_version != entry.metatable_version {
                return Ok(None);
            }
        }
        Ok(Some(entry.method))
    }

    fn table_iter_metamethod_cache_entry(
        &mut self,
        table: u32,
    ) -> Result<(IterMetamethodInlineCacheEntry, Option<Value>)> {
        let table_version = self.heap.table_version(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        let metatable = self.heap.table_metatable(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        let Some(metatable) = metatable else {
            return Ok((
                IterMetamethodInlineCacheEntry {
                    table,
                    table_version,
                    metatable: None,
                    metatable_version: 0,
                    method: None,
                },
                None,
            ));
        };
        let metatable_version = self.heap.table_version(metatable).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "__iter metatable handle is missing",
            })
        })?;
        let method = self.table_iter_metamethod_from_metatable(metatable)?;
        Ok((
            IterMetamethodInlineCacheEntry {
                table,
                table_version,
                metatable: Some(metatable),
                metatable_version,
                method,
            },
            method,
        ))
    }

    fn table_iter_metamethod(&mut self, table: u32) -> Result<Option<Value>> {
        let Some(metatable) = self.heap.table_metatable(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?
        else {
            return Ok(None);
        };
        self.table_iter_metamethod_from_metatable(metatable)
    }

    fn table_iter_metamethod_from_metatable(&mut self, metatable: u32) -> Result<Option<Value>> {
        let key = Value::string_atom(self.heap.intern_string("__iter"));
        let method = self.raw_table_get(metatable, key)?;
        if method.kind() == ValueKind::Nil {
            Ok(None)
        } else {
            Ok(Some(method))
        }
    }

    fn store_iter_metamethod_inline_cache(
        &mut self,
        site: InlineCacheSite,
        entry: IterMetamethodInlineCacheEntry,
    ) {
        if let Some(slot) = self.iter_metamethod_inline_cache.get_mut(site.slot()) {
            *slot = Some(IterMetamethodInlineCacheSlot { site, entry });
        }
    }

    fn metamethod_for_value(&mut self, value: Value, name: &'static str) -> Result<Option<Value>> {
        let Some(metatable) = self.metatable_for_value(value)? else {
            return Ok(None);
        };
        let key = Value::string_atom(self.heap.intern_string(name));
        let method = self.raw_table_get(metatable, key)?;
        if method.kind() == ValueKind::Nil {
            Ok(None)
        } else {
            Ok(Some(method))
        }
    }

    fn string_method_index(&mut self, key: Value) -> Result<Value> {
        let Some(name) = self.string_value(key)? else {
            return Ok(Value::nil());
        };
        STRING_NATIVE_FUNCTIONS
            .iter()
            .find_map(|(field, native)| (*field == name).then_some(*native))
            .map(|native| native_function_value(&mut self.heap, native, self.globals))
            .map_or_else(|| Ok(Value::nil()), Ok)
    }

    fn raw_table_get(&mut self, table: u32, key: Value) -> Result<Value> {
        Ok(self.raw_table_slot(table, key)?.unwrap_or_else(Value::nil))
    }

    fn raw_table_slot(&mut self, table: u32, key: Value) -> Result<Option<Value>> {
        self.heap.table_slot(table, key).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })
    }

    fn raw_table_get_after_table_lookup(&mut self, table: u32, key: Value) -> Value {
        self.raw_table_slot_after_table_lookup(table, key)
            .unwrap_or_else(Value::nil)
    }

    fn raw_table_slot_after_table_lookup(&mut self, table: u32, key: Value) -> Option<Value> {
        self.heap.table_get(table, key)
    }

    fn set_table_index(&mut self, table: u32, key: Value, value: Value) -> Result<()> {
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            }));
        }
        self.set_table_index_after_table_lookup(table, key, value)
    }

    fn set_table_index_after_table_lookup(
        &mut self,
        table: u32,
        key: Value,
        value: Value,
    ) -> Result<()> {
        self.validate_table_key(key)?;
        self.ensure_table_writable(table)?;
        self.heap.table_set(table, key, value).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table set failed after validation",
            })
        })
    }

    fn set_table_absent_index(&mut self, table: u32, key: Value, value: Value) -> Result<()> {
        self.validate_table_key(key)?;
        self.ensure_table_writable(table)?;
        self.heap
            .table_set_absent(table, key, value)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "absent table set failed after validation",
                })
            })
    }

    fn table_uses_standard_global_type_surface(&mut self, table: u32) -> Result<bool> {
        Ok(self.standard_global_index_table(table)?.is_some())
    }

    fn standard_global_index_table(&mut self, table: u32) -> Result<Option<u32>> {
        if table != self.globals {
            return Ok(None);
        }
        let Some(metatable) = self.heap.table_metatable(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "global environment handle is missing",
            })
        })?
        else {
            return Ok(None);
        };
        let index_key = Value::string_atom(self.heap.intern_string("__index"));
        Ok(
            (self.raw_table_get(metatable, index_key)?.as_table() == Some(self.standard_globals))
                .then_some(self.standard_globals),
        )
    }

    fn set_table_array_slot(&mut self, table: u32, index: usize, value: Value) -> Result<()> {
        match self
            .heap
            .write_array_slot(table, index, value)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })? {
            ArrayWriteStatus::Stored => Ok(()),
            ArrayWriteStatus::Readonly => Err(readonly_table_error()),
            ArrayWriteStatus::Deferred => Err(Error::new(ErrorKind::InvalidProgram {
                reason: "table array slot set failed after validation",
            })),
        }
    }

    fn ensure_table_writable(&mut self, table: u32) -> Result<()> {
        let readonly = self.heap.table_is_readonly(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        if readonly {
            Err(readonly_table_error())
        } else {
            Ok(())
        }
    }

    fn table_concat(
        &mut self,
        table: u32,
        separator: &[u8],
        start: Option<usize>,
        end: Option<usize>,
    ) -> Result<Value> {
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            }));
        }
        let len = self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        let start = start.unwrap_or(1);
        let end = end.unwrap_or(len);
        if start > end {
            let atom = self.heap.intern_string_bytes(&[]);
            return Ok(Value::string_atom(atom));
        }
        let outcome = self
            .heap
            .intern_table_concat_range(table, start, end, separator, |number, output| {
                output.extend_from_slice(luau_number_to_string(number).as_bytes());
            })
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?;
        match outcome {
            TableConcatOutcome::Atom(atom) => Ok(Value::string_atom(atom)),
            TableConcatOutcome::InvalidValue => Err(Error::new(ErrorKind::TypeError {
                expected: "string or number",
            })),
        }
    }

    fn table_find(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        if !(2..=3).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid table.find arity",
            }));
        }
        let table = get_table(registers, instruction.b)?;
        let needle = get_register(registers, instruction.c)?;
        let start = if instruction.aux == 3 {
            let register = instruction.c.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table.find init register overflow",
                })
            })?;
            let value = get_register(registers, register)?;
            if value.kind() == ValueKind::Nil {
                1
            } else {
                let number = value
                    .as_number()
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
                table_find_start(number)?
            }
        } else {
            1
        };
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            }));
        }
        let mut index = start;
        loop {
            let value = self.raw_table_get(table, Value::number(index as f64))?;
            if value.kind() == ValueKind::Nil {
                return Ok(Value::nil());
            }
            if self.equal_values(program, value, needle, depth, fuel)? {
                return Ok(Value::number(index as f64));
            }
            let Some(next) = index.checked_add(1) else {
                return Ok(Value::nil());
            };
            index = next;
        }
    }

    fn table_create(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(1..=2).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid table.create arity",
            }));
        }
        let size = table_create_size(self.coerce_register_to_number(registers, instruction.b)?)?;
        let value = if instruction.aux == 2 {
            get_register(registers, instruction.c)?
        } else {
            Value::nil()
        };
        Ok(Value::table(
            self.heap.alloc_filled_array_table(size, value),
        ))
    }

    fn new_record_table(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if instruction.aux < 1 || instruction.aux > i32::from(u8::MAX) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid record table field count",
            }));
        }
        let field_count = instruction.aux as usize;
        let register_count = field_count.checked_mul(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "record table register window overflow",
            })
        })?;
        let start = usize::from(instruction.b);
        let end = start.checked_add(register_count).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "record table register window overflow",
            })
        })?;
        let window = registers.get(start..end).ok_or_else(|| {
            Error::new(ErrorKind::RegisterOutOfBounds {
                register: end,
                max: registers.len(),
            })
        })?;
        for pair in window.chunks_exact(2) {
            if pair[0].as_string_atom().is_none() {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "record table key must be a string atom",
                }));
            }
        }
        let table = self.heap.alloc_record_table(window).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "record table allocation failed",
            })
        })?;
        Ok(Value::table(table))
    }

    fn table_insert(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(2..=3).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid table.insert arity",
            }));
        }
        let table = get_table(registers, instruction.b)?;
        self.ensure_table_writable(table)?;
        let (position, value) = if instruction.aux == 2 {
            let len = self.heap.array_len(table).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?;
            let position = table_len_to_i32(len)?.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::TypeError {
                    expected: "table overflow",
                })
            })?;
            (position, get_register(registers, instruction.c)?)
        } else {
            let position = table_insert_position_arg(
                self.coerce_register_to_number(registers, instruction.c)?,
            )?;
            let value_register = instruction.c.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table.insert value register overflow",
                })
            })?;
            (position, get_register(registers, value_register)?)
        };
        self.heap
            .table_insert(table, position, value)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table handle is missing",
                })
            })?;
        Ok(Value::nil())
    }

    fn table_insert_extra_fuel(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
    ) -> Result<u64> {
        if !(2..=3).contains(&instruction.aux) {
            return Ok(0);
        }
        let table = get_table(registers, instruction.b)?;
        let Some(len) = self.heap.array_len(table) else {
            return Ok(0);
        };
        if instruction.aux == 2 {
            return Ok(1);
        }
        let Ok(position) =
            table_insert_position_arg(self.coerce_register_to_number(registers, instruction.c)?)
        else {
            return Ok(0);
        };
        Ok(shifted_array_work_fuel(len, position))
    }

    fn table_remove(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(1..=2).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid table.remove arity",
            }));
        }
        let table = get_table(registers, instruction.b)?;
        self.ensure_table_writable(table)?;
        let len = self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        let position = if instruction.aux == 2 {
            let value = get_register(registers, instruction.c)?;
            if value.kind() == ValueKind::Nil {
                table_len_to_i32(len)?
            } else {
                let number = self
                    .coerce_to_number(value)?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
                table_integer_arg(number, "integer table.remove position")?
            }
        } else {
            table_len_to_i32(len)?
        };
        let removed = self.heap.table_remove(table, position).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        Ok(removed.unwrap_or_else(Value::nil))
    }

    fn get_register_cell_value(
        &mut self,
        registers: &[Value],
        register_cells: &mut [Option<u32>],
        register: u8,
    ) -> Result<Value> {
        let cell = self.ensure_register_cell(registers, register_cells, register)?;
        self.heap.upvalue_get(cell).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "local cell handle is missing",
            })
        })
    }

    fn set_register_cell_value(
        &mut self,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        register: u8,
        value: Value,
    ) -> Result<()> {
        let cell_index = register as usize;
        if cell_index >= register_cells.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: cell_index,
                max: register_cells.len(),
            }));
        }
        set_register(registers, register, value)?;
        let cell = match register_cells[cell_index] {
            Some(cell) => cell,
            None => {
                let cell = self.heap.alloc_upvalue(value).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "upvalue handle overflow",
                    })
                })?;
                register_cells[cell_index] = Some(cell);
                cell
            }
        };
        self.heap.upvalue_set(cell, value).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "local cell handle is missing",
            })
        })
    }

    fn ensure_register_cell(
        &mut self,
        registers: &[Value],
        register_cells: &mut [Option<u32>],
        register: u8,
    ) -> Result<u32> {
        let cell_index = register as usize;
        if cell_index >= register_cells.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: cell_index,
                max: register_cells.len(),
            }));
        }
        if let Some(cell) = register_cells[cell_index] {
            return Ok(cell);
        }
        let value = get_register(registers, register)?;
        let cell = self.heap.alloc_upvalue(value).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "upvalue handle overflow",
            })
        })?;
        register_cells[cell_index] = Some(cell);
        Ok(cell)
    }

    fn sync_register_cells(
        &mut self,
        registers: &mut [Value],
        register_cells: &[Option<u32>],
    ) -> Result<()> {
        for (index, cell) in register_cells.iter().copied().enumerate() {
            let Some(cell) = cell else {
                continue;
            };
            let value = self.heap.upvalue_get(cell).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "local cell handle is missing",
                })
            })?;
            registers[index] = value;
        }
        Ok(())
    }

    fn validate_conformance_native_parameter_types(
        &mut self,
        function: &Function,
        registers: &[Value],
    ) -> Result<()> {
        if !self.settings.enable_conformance_native_type_checks {
            return Ok(());
        }
        for (index, expected) in function.parameter_types.iter().enumerate() {
            if matches!(expected, RuntimeType::Any) {
                continue;
            }
            let value = registers.get(index).copied().ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "parameter register is missing",
                })
            })?;
            if !self.value_matches_runtime_type(value, expected) {
                return Err(Error::new(ErrorKind::TypeError {
                    expected: runtime_type_name(expected),
                }));
            }
        }
        Ok(())
    }

    fn value_matches_runtime_type(&self, value: Value, expected: &RuntimeType) -> bool {
        match expected {
            RuntimeType::Any => true,
            RuntimeType::Nil => value.kind() == ValueKind::Nil,
            RuntimeType::Number => value.kind() == ValueKind::Number,
            RuntimeType::String => value.kind() == ValueKind::String,
            RuntimeType::OptionalString => {
                matches!(value.kind(), ValueKind::Nil | ValueKind::String)
            }
            RuntimeType::Table => value.kind() == ValueKind::Table,
            RuntimeType::Function => value.kind() == ValueKind::Function,
            RuntimeType::Thread => value
                .as_userdata()
                .is_some_and(|handle| self.coroutine_for_handle(handle).is_some()),
            RuntimeType::Userdata => value.kind() == ValueKind::Userdata,
            RuntimeType::Vector => value.kind() == ValueKind::Vector,
            RuntimeType::Buffer => value.kind() == ValueKind::Buffer,
            RuntimeType::OptionalBuffer => {
                matches!(value.kind(), ValueKind::Nil | ValueKind::Buffer)
            }
        }
    }

    fn call_function_resumable(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        let callee = get_register(registers, instruction.b)?;
        self.call_function_from_registers_resumable(
            program,
            callee,
            registers,
            instruction,
            depth,
            call_site,
            fuel,
        )
        .map(|flow| match flow {
            VmFlow::Return(RegisterCallResult::Scalar(value)) => VmFlow::Return(vec![value]),
            VmFlow::Return(RegisterCallResult::Results(results)) => VmFlow::Return(results),
            VmFlow::Yield(suspension) => VmFlow::Yield(suspension),
        })
    }

    fn call_function_from_registers_resumable(
        &mut self,
        program: &Program,
        callee: Value,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<RegisterCallResult> {
        if instruction.aux < 0 || instruction.aux > i32::from(u8::MAX) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid function call arity",
            }));
        }
        let argc = instruction.aux as usize;
        let args = if argc == 0 {
            &[][..]
        } else {
            let first = usize::from(instruction.c);
            let end = first.checked_add(argc).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function argument window overflow",
                })
            })?;
            if end > registers.len() {
                return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                    register: end,
                    max: registers.len(),
                }));
            }
            &registers[first..end]
        };
        self.check_recursion_depth(depth)?;
        let Some(handle) = callee.as_function() else {
            return self
                .call_metamethod(program, callee, args, depth, fuel)
                .map(|results| VmFlow::Return(RegisterCallResult::Results(results)));
        };
        let metadata = self.heap.function_call_metadata(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle is missing",
            })
        })?;
        if let FunctionKind::Native(native) = metadata.kind {
            if let Some(op) = fast_native_math_unary(native) {
                let value = self.execute_fast_native_math_unary_from_registers(
                    registers,
                    instruction,
                    fuel,
                    op,
                )?;
                return Ok(VmFlow::Return(RegisterCallResult::Scalar(value)));
            }
            if native == NativeFunction::SetMetatable {
                let value =
                    self.execute_fast_native_setmetatable_from_registers(handle, args, fuel)?;
                return Ok(VmFlow::Return(RegisterCallResult::Scalar(value)));
            }
        }
        self.call_function_metadata_resumable(
            program,
            callee,
            handle,
            metadata.kind,
            metadata.bytecode_context,
            args,
            depth,
            call_site,
            fuel,
        )
        .map(|flow| match flow {
            VmFlow::Return(results) => VmFlow::Return(RegisterCallResult::Results(results)),
            VmFlow::Yield(suspension) => VmFlow::Yield(suspension),
        })
    }

    fn call_function_from_registers_resumable_lazy_site<F>(
        &mut self,
        program: &Program,
        callee: Value,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        call_site: F,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<RegisterCallResult>
    where
        F: FnOnce() -> Option<ErrorLocation>,
    {
        if instruction.aux < 0 || instruction.aux > i32::from(u8::MAX) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid function call arity",
            }));
        }
        let argc = instruction.aux as usize;
        let args = if argc == 0 {
            &[][..]
        } else {
            let first = usize::from(instruction.c);
            let end = first.checked_add(argc).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function argument window overflow",
                })
            })?;
            if end > registers.len() {
                return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                    register: end,
                    max: registers.len(),
                }));
            }
            &registers[first..end]
        };
        self.check_recursion_depth(depth)?;
        let Some(handle) = callee.as_function() else {
            return self
                .call_metamethod(program, callee, args, depth, fuel)
                .map(|results| VmFlow::Return(RegisterCallResult::Results(results)));
        };
        let metadata = self.heap.function_call_metadata(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle is missing",
            })
        })?;
        if let FunctionKind::Native(native) = metadata.kind {
            if let Some(op) = fast_native_math_unary(native) {
                let value = self.execute_fast_native_math_unary_from_registers(
                    registers,
                    instruction,
                    fuel,
                    op,
                )?;
                return Ok(VmFlow::Return(RegisterCallResult::Scalar(value)));
            }
            if native == NativeFunction::SetMetatable {
                let value =
                    self.execute_fast_native_setmetatable_from_registers(handle, args, fuel)?;
                return Ok(VmFlow::Return(RegisterCallResult::Scalar(value)));
            }
        }
        self.call_function_metadata_resumable(
            program,
            callee,
            handle,
            metadata.kind,
            metadata.bytecode_context,
            args,
            depth,
            call_site(),
            fuel,
        )
        .map(|flow| match flow {
            VmFlow::Return(results) => VmFlow::Return(RegisterCallResult::Results(results)),
            VmFlow::Yield(suspension) => VmFlow::Yield(suspension),
        })
    }

    fn call_value(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        match self.call_value_with_site_resumable(program, callee, args, depth, None, fuel)? {
            VmFlow::Return(results) => Ok(results),
            VmFlow::Yield(_) => Err(non_yieldable_boundary_error()),
        }
    }

    fn call_value_with_site(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        match self.call_value_with_site_resumable(program, callee, args, depth, call_site, fuel)? {
            VmFlow::Return(results) => Ok(results),
            VmFlow::Yield(_) => Err(non_yieldable_boundary_error()),
        }
    }

    fn call_value_non_yieldable(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        self.non_yieldable_depth = self.non_yieldable_depth.saturating_add(1);
        let result = self.call_value(program, callee, args, depth, fuel);
        self.non_yieldable_depth = self.non_yieldable_depth.saturating_sub(1);
        result
    }

    fn call_value_with_traceback_snapshot(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        traceback: TracebackSnapshot,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let previous = self.protected_error_traceback.replace(traceback);
        let result = self.call_value(program, callee, args, depth, fuel);
        self.protected_error_traceback = previous;
        result
    }

    fn call_value_non_yieldable_with_traceback_snapshot(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        traceback: TracebackSnapshot,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        self.non_yieldable_depth = self.non_yieldable_depth.saturating_add(1);
        let result =
            self.call_value_with_traceback_snapshot(program, callee, args, depth, traceback, fuel);
        self.non_yieldable_depth = self.non_yieldable_depth.saturating_sub(1);
        result
    }

    fn call_value_with_site_resumable(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        self.check_recursion_depth(depth)?;
        let Some(handle) = callee.as_function() else {
            return self
                .call_metamethod(program, callee, args, depth, fuel)
                .map(VmFlow::Return);
        };
        let metadata = self.heap.function_call_metadata(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle is missing",
            })
        })?;
        self.call_function_metadata_resumable(
            program,
            callee,
            handle,
            metadata.kind,
            metadata.bytecode_context,
            args,
            depth,
            call_site,
            fuel,
        )
    }

    fn call_function_metadata_resumable(
        &mut self,
        program: &Program,
        callee: Value,
        handle: u32,
        kind: FunctionKind,
        bytecode_context: Option<FunctionBytecodeContext>,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        match kind {
            FunctionKind::Bytecode {
                program: None,
                prototype,
            } => {
                let FunctionBytecodeContext {
                    upvalues,
                    environment,
                } = bytecode_context.ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "function bytecode context is missing",
                    })
                })?;
                let upvalue_snapshot = upvalues;
                let function = program.functions.get(prototype as usize).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "function prototype is missing",
                    })
                })?;
                self.call_bytecode_function(
                    program,
                    None,
                    Some(prototype),
                    function,
                    callee,
                    upvalue_snapshot.as_slice(),
                    environment,
                    args,
                    depth,
                    call_site,
                    fuel,
                )
            }
            FunctionKind::Bytecode {
                program: Some(dynamic_program_id),
                prototype,
            } => {
                let FunctionBytecodeContext {
                    upvalues,
                    environment,
                } = bytecode_context.ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "function bytecode context is missing",
                    })
                })?;
                let upvalue_snapshot = upvalues;
                let dynamic_program = self.dynamic_program(dynamic_program_id)?;
                let function = dynamic_program
                    .functions
                    .get(prototype as usize)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "function prototype is missing",
                        })
                    })?;
                self.call_bytecode_function(
                    &dynamic_program,
                    Some(dynamic_program_id),
                    Some(prototype),
                    function,
                    callee,
                    upvalue_snapshot.as_slice(),
                    environment,
                    args,
                    depth,
                    call_site,
                    fuel,
                )
            }
            FunctionKind::DynamicMain {
                program: dynamic_program_id,
            } => {
                let FunctionBytecodeContext {
                    upvalues,
                    environment,
                } = bytecode_context.ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "function bytecode context is missing",
                    })
                })?;
                let upvalue_snapshot = upvalues;
                let dynamic_program = self.dynamic_program(dynamic_program_id)?;
                self.call_bytecode_function(
                    &dynamic_program,
                    Some(dynamic_program_id),
                    None,
                    &dynamic_program.main,
                    callee,
                    upvalue_snapshot.as_slice(),
                    environment,
                    args,
                    depth,
                    call_site,
                    fuel,
                )
            }
            FunctionKind::Host(host) => {
                let op = self.host_function(host)?.clone();
                charge_fuel(fuel, self.settings.initial_fuel, u64::from(op.fuel_cost))?;
                (op.handler)(self, args).map(VmFlow::Return)
            }
            FunctionKind::Native(native) => self.call_native_function_resumable(
                handle, native, args, program, depth, call_site, fuel,
            ),
        }
    }

    fn value_is_native_function(&mut self, value: Value, native: NativeFunction) -> bool {
        let Some(handle) = value.as_function() else {
            return false;
        };
        matches!(self.heap.function_kind(handle), Some(FunctionKind::Native(found)) if found == native)
    }

    fn execute_fast_native_math_unary_from_registers(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
        fuel: &mut Option<u64>,
        op: FastNativeMathUnary,
    ) -> Result<Value> {
        if instruction.aux < 0 || instruction.aux > i32::from(u8::MAX) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid function call arity",
            }));
        }
        let argc = instruction.aux as usize;
        let args = if argc == 0 {
            &[][..]
        } else {
            let first = usize::from(instruction.c);
            let end = first.checked_add(argc).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function argument window overflow",
                })
            })?;
            if end > registers.len() {
                return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                    register: end,
                    max: registers.len(),
                }));
            }
            &registers[first..end]
        };
        charge_fuel(fuel, self.settings.initial_fuel, 0)?;
        match op {
            FastNativeMathUnary::Abs => self.native_math_abs(args),
            FastNativeMathUnary::Unary(op) => {
                validate_native_arity(args, 1, u8::MAX as usize, "math argument")?;
                let value = self.coerce_register_to_number(registers, instruction.c)?;
                Ok(Value::number(math_unary_result(value, op)))
            }
        }
    }

    fn execute_fast_native_setmetatable_from_registers(
        &mut self,
        handle: u32,
        args: &[Value],
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let extra_fuel = self.native_extra_fuel_cost(handle, NativeFunction::SetMetatable, args);
        charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)?;
        self.native_setmetatable(args)
    }

    fn bytecode_protected_call_site(
        &mut self,
        callee: Value,
        call_site: Option<ErrorLocation>,
    ) -> Option<ErrorLocation> {
        let handle = callee.as_function()?;
        match self.heap.function_kind(handle) {
            Some(FunctionKind::Bytecode { .. } | FunctionKind::DynamicMain { .. }) => call_site,
            _ => None,
        }
    }

    fn dynamic_program(&self, program_id: u32) -> Result<Arc<Program>> {
        self.dynamic_programs
            .get(program_id as usize)
            .cloned()
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "dynamic program handle is missing",
                })
            })
    }

    fn host_function(&self, host: u32) -> Result<&HostFunctionOp> {
        self.host_functions.get(host as usize).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "host function handle is missing",
            })
        })
    }

    fn retain_dynamic_program(&mut self, program: Program) -> Result<u32> {
        let program_id = u32::try_from(self.dynamic_programs.len()).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "dynamic program handle overflow",
            })
        })?;
        self.dynamic_programs.push(Arc::new(program));
        Ok(program_id)
    }

    fn call_bytecode_function(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: Option<u32>,
        function: &Function,
        callee: Value,
        upvalues: &[u32],
        environment: u32,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        self.run_function(
            program,
            program_id,
            function,
            args,
            FunctionCallContext {
                current_function: Some(callee),
                upvalues: &upvalues,
                depth: depth + 1,
                environment,
                call_site,
                prototype,
            },
            None,
            fuel,
        )
        .map_err(|error| attach_existing_instruction_source(program, function, error))
    }

    fn call_metamethod(
        &mut self,
        program: &Program,
        callee: Value,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        match callee.kind() {
            ValueKind::Userdata => {
                if let Some(coroutine) = self.unmodified_callable_coroutine(callee)? {
                    let call_depth = depth.saturating_add(1);
                    self.check_recursion_depth(call_depth)?;
                    let extra_fuel = usize_to_u64(args.len().saturating_add(1));
                    charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)?;
                    return self.call_coroutine_wrapper_by_index(
                        coroutine, args, program, call_depth, fuel,
                    );
                }
                let Some(method) = self.metamethod_for_value(callee, "__call")? else {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: format!("attempt to call a {} value", value_type_name(callee)),
                    }));
                };
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(callee);
                call_args.extend_from_slice(args);
                self.call_value(program, method, &call_args, depth.saturating_add(1), fuel)
            }
            ValueKind::Table => {
                let Some(method) = self.metamethod_for_value(callee, "__call")? else {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: format!("attempt to call a {} value", value_type_name(callee)),
                    }));
                };
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(callee);
                call_args.extend_from_slice(args);
                self.call_value(program, method, &call_args, depth.saturating_add(1), fuel)
            }
            _ => Err(Error::new(ErrorKind::RuntimeError {
                message: format!("attempt to call a {} value", value_type_name(callee)),
            })),
        }
    }

    fn unmodified_callable_coroutine(&mut self, value: Value) -> Result<Option<usize>> {
        let Some(handle) = value.as_userdata() else {
            return Ok(None);
        };
        let Some(coroutine) = self.coroutine_for_handle(handle) else {
            return Ok(None);
        };
        let Some((expected_metatable, expected_version)) =
            self.coroutines[coroutine].callable_metatable
        else {
            return Ok(None);
        };
        let metatable = self.heap.userdata_metatable(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "userdata handle is missing",
            })
        })?;
        if metatable != Some(expected_metatable) {
            return Ok(None);
        }
        let version = self.heap.table_version(expected_metatable).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "coroutine wrapper metatable handle is missing",
            })
        })?;
        if version == expected_version {
            Ok(Some(coroutine))
        } else {
            Ok(None)
        }
    }

    fn call_function_multi(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<()> {
        let base = usize::from(instruction.a);
        let argc = usize::from(instruction.b);
        let result_count = usize::from(instruction.c);
        let args_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function argument window overflow",
            })
        })?;
        let args_end = args_start.checked_add(argc).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function argument window overflow",
            })
        })?;
        let results_end = base.checked_add(result_count).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function result window overflow",
            })
        })?;
        let required_end = args_end.max(results_end);
        if required_end > registers.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: required_end,
                max: registers.len(),
            }));
        }

        if let Some(flow) = self.try_call_protected_fixed_pair(
            program,
            registers,
            instruction,
            depth,
            &call_site,
            fuel,
        )? {
            return Ok(flow);
        }

        let synthetic_call = Instruction::with_aux(
            Opcode::Call,
            instruction.a,
            instruction.a,
            u8::try_from(args_start).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function argument register index overflow",
                })
            })?,
            i32::from(instruction.b),
        );

        let results = match self.call_function_resumable(
            program,
            registers,
            synthetic_call,
            depth,
            call_site,
            fuel,
        )? {
            VmFlow::Return(results) => results,
            VmFlow::Yield(suspension) => return Ok(VmFlow::Yield(suspension)),
        };
        for index in 0..result_count {
            let value = results.get(index).copied().unwrap_or_else(Value::nil);
            let register = u8::try_from(base + index).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function result register index overflow",
                })
            })?;
            set_register(registers, register, value)?;
        }
        self.recycle_results(results);
        Ok(VmFlow::Return(()))
    }

    fn try_call_protected_fixed_pair(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        depth: usize,
        call_site: &Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> std::result::Result<Option<VmFlow<()>>, Error> {
        let base = usize::from(instruction.a);
        let argc = usize::from(instruction.b);
        let result_count = usize::from(instruction.c);
        if result_count != 2 {
            return Ok(None);
        }
        let args_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function argument window overflow",
            })
        })?;
        let args_end = args_start.checked_add(argc).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function argument window overflow",
            })
        })?;
        let callee = get_register(registers, instruction.a)?;
        let Some(handle) = callee.as_function() else {
            return Ok(None);
        };
        let metadata = self.heap.function_call_metadata(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle is missing",
            })
        })?;
        let FunctionKind::Native(native) = metadata.kind else {
            return Ok(None);
        };
        if !matches!(native, NativeFunction::PCall | NativeFunction::XPCall) {
            return Ok(None);
        }
        let args = &registers[args_start..args_end];
        self.check_recursion_depth(depth)?;
        let extra_fuel = self.native_extra_fuel_cost(handle, native, args);
        charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)?;

        match native {
            NativeFunction::PCall => self.call_pcall_fixed_pair(
                program,
                registers,
                base,
                args_start,
                args_end,
                depth,
                call_site.clone(),
                fuel,
            ),
            NativeFunction::XPCall => self.call_xpcall_fixed_pair(
                program,
                registers,
                base,
                args_start,
                args_end,
                depth,
                call_site.clone(),
                fuel,
            ),
            _ => Ok(None),
        }
    }

    fn call_pcall_fixed_pair(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        base: usize,
        args_start: usize,
        args_end: usize,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> std::result::Result<Option<VmFlow<()>>, Error> {
        let protected_callee = registers
            .get(args_start)
            .copied()
            .unwrap_or_else(Value::nil);
        let protected_call_site = self.bytecode_protected_call_site(protected_callee, call_site);
        let protected_call = {
            let call_args = registers.get(args_start + 1..args_end).unwrap_or(&[]);
            self.call_value_with_site_resumable(
                program,
                protected_callee,
                call_args,
                depth + 1,
                protected_call_site,
                fuel,
            )
        };
        match protected_call {
            Ok(VmFlow::Return(results)) => {
                write_call_results(
                    registers,
                    base,
                    2,
                    &[Value::boolean(true), first_result_or_nil(results)],
                )?;
                Ok(Some(VmFlow::Return(())))
            }
            Ok(VmFlow::Yield(mut suspension)) => {
                suspension.frames.push(CoroutineFrame::PCall);
                Ok(Some(VmFlow::Yield(suspension)))
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                self.last_error_traceback = None;
                let error = self.error_object(error);
                write_call_results(registers, base, 2, &[Value::boolean(false), error])?;
                Ok(Some(VmFlow::Return(())))
            }
        }
    }

    fn call_xpcall_fixed_pair(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        base: usize,
        args_start: usize,
        args_end: usize,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> std::result::Result<Option<VmFlow<()>>, Error> {
        let protected_callee = registers
            .get(args_start)
            .copied()
            .unwrap_or_else(Value::nil);
        let args = &registers[args_start..args_end];
        let handler = self.xpcall_handler(args)?;
        let traceback_base = self.current_traceback_base();
        let protected_call_site = self.bytecode_protected_call_site(protected_callee, call_site);
        let protected_call = {
            let call_args = registers.get(args_start + 2..args_end).unwrap_or(&[]);
            self.call_value_with_site_resumable(
                program,
                protected_callee,
                call_args,
                depth + 1,
                protected_call_site,
                fuel,
            )
        };
        match protected_call {
            Ok(VmFlow::Return(results)) => {
                write_call_results(
                    registers,
                    base,
                    2,
                    &[Value::boolean(true), first_result_or_nil(results)],
                )?;
                Ok(Some(VmFlow::Return(())))
            }
            Ok(VmFlow::Yield(mut suspension)) => {
                let program_id = self.retain_dynamic_program(program.clone())?;
                suspension
                    .frames
                    .push(CoroutineFrame::XPCall(SuspendedXPCallFrame {
                        program_id,
                        handler,
                        depth,
                        traceback_base: 0,
                    }));
                Ok(Some(VmFlow::Yield(suspension)))
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                let original_is_out_of_memory = error_is_out_of_memory(&error);
                let traceback = self.take_error_traceback_after_base(&error, traceback_base);
                let error_value = self.error_object(error);
                match self.call_value_non_yieldable_with_traceback_snapshot(
                    program,
                    handler,
                    &[error_value],
                    depth + 1,
                    traceback,
                    fuel,
                ) {
                    Ok(results) => {
                        write_call_results(
                            registers,
                            base,
                            2,
                            &[Value::boolean(false), first_result_or_nil(results)],
                        )?;
                        Ok(Some(VmFlow::Return(())))
                    }
                    Err(error) if error_is_uncatchable(&error) => Err(error),
                    Err(handler_error) => {
                        let results = self
                            .xpcall_handler_error_result(original_is_out_of_memory, handler_error);
                        write_call_results(registers, base, 2, &results)?;
                        Ok(Some(VmFlow::Return(())))
                    }
                }
            }
        }
    }

    fn call_function_varargs(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        varargs: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<()> {
        let base = usize::from(instruction.a);
        let prefix_count = usize::from(instruction.b);
        let result_count = usize::from(instruction.c);
        let prefix_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "vararg function argument window overflow",
            })
        })?;
        let prefix_end = prefix_start.checked_add(prefix_count).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "vararg function argument window overflow",
            })
        })?;
        let results_end = base.checked_add(result_count).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "vararg function result window overflow",
            })
        })?;
        let required_end = prefix_end.max(results_end);
        if required_end > registers.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: required_end,
                max: registers.len(),
            }));
        }

        let callee = get_register(registers, instruction.a)?;
        let mut args = registers[prefix_start..prefix_end].to_vec();
        args.extend(varargs.iter().copied());
        let results = match self
            .call_value_with_site_resumable(program, callee, &args, depth, call_site, fuel)?
        {
            VmFlow::Return(results) => results,
            VmFlow::Yield(suspension) => return Ok(VmFlow::Yield(suspension)),
        };
        for index in 0..result_count {
            let value = results.get(index).copied().unwrap_or_else(Value::nil);
            let register = u8::try_from(base + index).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "vararg function result register overflow",
                })
            })?;
            set_register(registers, register, value)?;
        }
        Ok(VmFlow::Return(()))
    }

    fn call_function_expand2(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let result_count = usize::from(instruction.c);
        let results = self.nested_expand_results(
            program,
            registers,
            usize::from(instruction.a),
            instruction.aux,
            depth,
            call_site,
            fuel,
        )?;
        write_call_results(
            registers,
            usize::from(instruction.a),
            result_count,
            &results,
        )
    }

    fn call_function_expand3(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let result_count = usize::from(instruction.b);
        let leaf_argc = usize::from(instruction.c);
        let results = self.nested_expand3_results(
            program,
            registers,
            usize::from(instruction.a),
            instruction.aux,
            leaf_argc,
            depth,
            call_site,
            fuel,
        )?;
        write_call_results(
            registers,
            usize::from(instruction.a),
            result_count,
            &results,
        )
    }

    fn call_function_expand_varargs(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        instruction: Instruction,
        varargs: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let result_count = usize::from(instruction.c);
        let results = self.nested_expand_varargs_results(
            program,
            registers,
            NestedVarargSpec {
                base: usize::from(instruction.a),
                aux: instruction.aux,
            },
            varargs,
            depth,
            call_site,
            fuel,
        )?;
        write_call_results(
            registers,
            usize::from(instruction.a),
            result_count,
            &results,
        )
    }

    fn nested_expand_results(
        &mut self,
        program: &Program,
        registers: &[Value],
        base: usize,
        aux: i32,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let (outer_prefix_count, middle_prefix_count, inner_argc) = unpack_nested_expand_aux(aux)?;
        let outer_prefix_count = usize::from(outer_prefix_count);
        let middle_prefix_count = usize::from(middle_prefix_count);
        let inner_argc = usize::from(inner_argc);
        let window =
            NestedExpandWindow::new(base, outer_prefix_count, middle_prefix_count, inner_argc)?;
        window.validate(registers.len())?;

        let outer_callee = get_register(
            registers,
            u8::try_from(base).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested outer callee register overflow",
                })
            })?,
        )?;
        let middle_callee = get_register(registers, window.middle_callee_register()?)?;
        let inner_callee = get_register(registers, window.inner_callee_register()?)?;

        let mut middle_args = registers[window.middle_prefix_start..window.inner_callee].to_vec();
        let inner_args = registers[window.inner_args_start..window.inner_args_end].to_vec();
        let mut inner_results = self.call_value_with_site(
            program,
            inner_callee,
            &inner_args,
            depth,
            call_site.clone(),
            fuel,
        )?;
        middle_args.append(&mut inner_results);

        let mut outer_args = registers[window.outer_prefix_start..window.middle_callee].to_vec();
        let mut middle_results = self.call_value_with_site(
            program,
            middle_callee,
            &middle_args,
            depth,
            call_site.clone(),
            fuel,
        )?;
        outer_args.append(&mut middle_results);
        self.call_value_with_site(program, outer_callee, &outer_args, depth, call_site, fuel)
    }

    fn nested_expand3_results(
        &mut self,
        program: &Program,
        registers: &[Value],
        base: usize,
        aux: i32,
        leaf_argc: usize,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let (outer_prefix_count, middle_prefix_count, inner_prefix_count) =
            unpack_nested_expand_aux(aux)?;
        let outer_prefix_count = usize::from(outer_prefix_count);
        let middle_prefix_count = usize::from(middle_prefix_count);
        let inner_prefix_count = usize::from(inner_prefix_count);
        let window = NestedExpand3Window::new(
            base,
            outer_prefix_count,
            middle_prefix_count,
            inner_prefix_count,
            leaf_argc,
        )?;
        window.validate(registers.len())?;

        let outer_callee = get_register(
            registers,
            u8::try_from(base).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested outer callee register overflow",
                })
            })?,
        )?;
        let middle_callee = get_register(registers, window.middle_callee_register()?)?;
        let inner_callee = get_register(registers, window.inner_callee_register()?)?;
        let leaf_callee = get_register(registers, window.leaf_callee_register()?)?;

        let leaf_args = registers[window.leaf_args_start..window.leaf_args_end].to_vec();
        let mut leaf_results = self.call_value_with_site(
            program,
            leaf_callee,
            &leaf_args,
            depth,
            call_site.clone(),
            fuel,
        )?;

        let mut inner_args = registers[window.inner_prefix_start..window.leaf_callee].to_vec();
        inner_args.append(&mut leaf_results);
        let mut inner_results = self.call_value_with_site(
            program,
            inner_callee,
            &inner_args,
            depth,
            call_site.clone(),
            fuel,
        )?;

        let mut middle_args = registers[window.middle_prefix_start..window.inner_callee].to_vec();
        middle_args.append(&mut inner_results);
        let mut middle_results = self.call_value_with_site(
            program,
            middle_callee,
            &middle_args,
            depth,
            call_site.clone(),
            fuel,
        )?;

        let mut outer_args = registers[window.outer_prefix_start..window.middle_callee].to_vec();
        outer_args.append(&mut middle_results);
        self.call_value_with_site(program, outer_callee, &outer_args, depth, call_site, fuel)
    }

    fn nested_expand_varargs_results(
        &mut self,
        program: &Program,
        registers: &[Value],
        spec: NestedVarargSpec,
        varargs: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let (outer_prefix_count, middle_prefix_count) = unpack_expand_varargs_aux(spec.aux)?;
        let outer_prefix_count = usize::from(outer_prefix_count);
        let middle_prefix_count = usize::from(middle_prefix_count);
        let window = NestedVarargWindow::new(spec.base, outer_prefix_count, middle_prefix_count)?;
        window.validate(registers.len())?;

        let outer_callee = get_register(
            registers,
            u8::try_from(spec.base).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested outer callee register overflow",
                })
            })?,
        )?;
        let middle_callee = get_register(registers, window.middle_callee_register()?)?;

        let mut middle_args = registers[window.middle_prefix_start..window.end].to_vec();
        middle_args.extend(varargs.iter().copied());

        let mut outer_args = registers[window.outer_prefix_start..window.middle_callee].to_vec();
        let mut middle_results = self.call_value_with_site(
            program,
            middle_callee,
            &middle_args,
            depth,
            call_site.clone(),
            fuel,
        )?;
        outer_args.append(&mut middle_results);
        self.call_value_with_site(program, outer_callee, &outer_args, depth, call_site, fuel)
    }

    fn nested_expand2_varargs_results(
        &mut self,
        program: &Program,
        registers: &[Value],
        spec: NestedVarargSpec,
        varargs: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let (outer_prefix_count, middle_prefix_count, inner_prefix_count) =
            unpack_nested_expand_aux(spec.aux)?;
        let outer_prefix_count = usize::from(outer_prefix_count);
        let middle_prefix_count = usize::from(middle_prefix_count);
        let inner_prefix_count = usize::from(inner_prefix_count);
        let window = NestedExpand2VarargWindow::new(
            spec.base,
            outer_prefix_count,
            middle_prefix_count,
            inner_prefix_count,
        )?;
        window.validate(registers.len())?;

        let outer_callee = get_register(
            registers,
            u8::try_from(spec.base).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested outer callee register overflow",
                })
            })?,
        )?;
        let middle_callee = get_register(registers, window.middle_callee_register()?)?;
        let inner_callee = get_register(registers, window.inner_callee_register()?)?;

        let mut inner_args = registers[window.inner_prefix_start..window.end].to_vec();
        inner_args.extend(varargs.iter().copied());
        let mut inner_results = self.call_value_with_site(
            program,
            inner_callee,
            &inner_args,
            depth,
            call_site.clone(),
            fuel,
        )?;

        let mut middle_args = registers[window.middle_prefix_start..window.inner_callee].to_vec();
        middle_args.append(&mut inner_results);
        let mut middle_results = self.call_value_with_site(
            program,
            middle_callee,
            &middle_args,
            depth,
            call_site.clone(),
            fuel,
        )?;

        let mut outer_args = registers[window.outer_prefix_start..window.middle_callee].to_vec();
        outer_args.append(&mut middle_results);
        self.call_value_with_site(program, outer_callee, &outer_args, depth, call_site, fuel)
    }

    fn call_native_function_resumable(
        &mut self,
        handle: u32,
        native: NativeFunction,
        args: &[Value],
        program: &Program,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        if native == NativeFunction::CoroutineYield {
            return self.native_coroutine_yield_resumable(args, call_site);
        }
        if native == NativeFunction::PCall {
            return self.native_pcall_resumable(program, args, depth, call_site, fuel);
        }
        if native == NativeFunction::XPCall {
            return self.native_xpcall_resumable(program, args, depth, call_site, fuel);
        }
        self.call_native_function(handle, native, args, program, depth, call_site, fuel)
            .map(VmFlow::Return)
    }

    fn call_native_function(
        &mut self,
        handle: u32,
        native: NativeFunction,
        args: &[Value],
        program: &Program,
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        self.check_recursion_depth(depth)?;
        let extra_fuel = self.native_extra_fuel_cost(handle, native, args);
        charge_fuel(fuel, self.settings.initial_fuel, extra_fuel)?;
        if native == NativeFunction::Assert {
            return self.native_assert(args);
        }
        if native == NativeFunction::PCall {
            return self.native_pcall(program, args, depth, call_site, fuel);
        }
        if native == NativeFunction::XPCall {
            return self.native_xpcall(program, args, depth, call_site, fuel);
        }
        if native == NativeFunction::Select {
            return self.native_select(args);
        }
        if native == NativeFunction::IPairs {
            return self.native_ipairs(args);
        }
        if native == NativeFunction::INext {
            return self.native_inext(args);
        }
        if native == NativeFunction::Pairs {
            return self.native_pairs(args);
        }
        if native == NativeFunction::LoadString {
            return self.native_loadstring(args);
        }
        if native == NativeFunction::DebugInfo {
            return self.native_debug_info(program, handle, args, call_site);
        }
        if native == NativeFunction::CoroutineResume {
            return self.native_coroutine_resume(args, program, depth, fuel);
        }
        if native == NativeFunction::CoroutineYield {
            return self.native_coroutine_yield(args, call_site);
        }
        if native == NativeFunction::CoroutineClose {
            return self.native_coroutine_close(args);
        }
        if native == NativeFunction::ConformanceResumeError {
            return self.native_conformance_resume_error(args, fuel);
        }
        if native == NativeFunction::CoroutineWrappedCall {
            return self.native_coroutine_wrapped_call(args, program, depth, fuel);
        }
        if native == NativeFunction::ToString {
            return self
                .native_tostring(program, args, depth, fuel)
                .map(|value| vec![value]);
        }
        if native == NativeFunction::StringFormat {
            return self
                .native_string_format(program, args, depth, fuel)
                .map(|value| vec![value]);
        }
        if native == NativeFunction::TableUnpack {
            return self.native_table_unpack(args);
        }
        if native == NativeFunction::StringUnpack {
            return self.native_string_unpack(args);
        }
        if native == NativeFunction::StringByte {
            return self.native_string_byte(args);
        }
        if native == NativeFunction::StringFind {
            return self.native_string_find(args);
        }
        if native == NativeFunction::StringMatch {
            return self.native_string_match(args);
        }
        if native == NativeFunction::StringGsub {
            return self.native_string_gsub(program, args, depth, fuel);
        }
        if native == NativeFunction::StringGMatch {
            return self.native_string_gmatch(args);
        }
        if native == NativeFunction::StringGMatchIterator {
            return self.native_string_gmatch_iterator(handle);
        }
        if native == NativeFunction::Utf8Codepoint {
            return self.native_utf8_codepoint(args);
        }
        if native == NativeFunction::Utf8Codes {
            return self.native_utf8_codes(args);
        }
        if native == NativeFunction::Utf8CodesIterator {
            return self.native_utf8_codes_iterator(args);
        }
        if native == NativeFunction::Utf8Len {
            return self.native_utf8_len(args);
        }
        if native == NativeFunction::TableFind {
            return self
                .native_table_find(args, program, depth, fuel)
                .map(|value| vec![value]);
        }
        if native == NativeFunction::MathFrexp {
            return self.native_math_frexp(args);
        }
        if native == NativeFunction::MathModf {
            return self.native_math_modf(args);
        }
        if native == NativeFunction::MathRandomSeed {
            return self.native_math_randomseed(args);
        }
        if native == NativeFunction::TableSort {
            return self.native_table_sort(args, program, depth, fuel);
        }
        if buffer_native_returns_no_values(native) {
            self.native_buffer_no_result(native, args)?;
            return Ok(Vec::new());
        }
        if native == NativeFunction::TableInsert {
            self.native_table_insert(args)?;
            return Ok(Vec::new());
        }
        if native == NativeFunction::SetFEnv {
            return self.native_setfenv(args);
        }
        if native == NativeFunction::TableForeach {
            return self
                .native_table_foreach(args, program, depth, fuel, TableForeachMode::Pairs)
                .map(|value| vec![value]);
        }
        if native == NativeFunction::TableForeachi {
            return self
                .native_table_foreach(args, program, depth, fuel, TableForeachMode::Array)
                .map(|value| vec![value]);
        }
        if native == NativeFunction::Next {
            return self.native_next(args);
        }
        self.call_native_scalar_function(program, native, args, call_site)
            .map(|value| vec![value])
    }

    fn check_recursion_depth(&self, depth: usize) -> Result<()> {
        if depth > self.settings.recursion_limit {
            return Err(Error::new(ErrorKind::RecursionLimit {
                limit: self.settings.recursion_limit,
            }));
        }
        Ok(())
    }

    fn call_native_scalar_function(
        &mut self,
        program: &Program,
        native: NativeFunction,
        args: &[Value],
        call_site: Option<ErrorLocation>,
    ) -> Result<Value> {
        match native {
            NativeFunction::Assert => unreachable!("assert returns a variable result list"),
            NativeFunction::PCall => unreachable!("pcall returns a variable result list"),
            NativeFunction::XPCall => unreachable!("xpcall returns a variable result list"),
            NativeFunction::Select => unreachable!("select returns a variable result list"),
            NativeFunction::IPairs => unreachable!("ipairs returns an iterator triple"),
            NativeFunction::INext => unreachable!("inext returns a variable result list"),
            NativeFunction::Pairs => unreachable!("pairs returns an iterator triple"),
            NativeFunction::LoadString => {
                unreachable!("loadstring returns a variable result list")
            }
            NativeFunction::CoroutineResume => {
                unreachable!("coroutine.resume returns a variable result list")
            }
            NativeFunction::CoroutineYield => {
                unreachable!("coroutine.yield returns a variable result list")
            }
            NativeFunction::CoroutineWrappedCall => {
                unreachable!("coroutine wrapped calls return a variable result list")
            }
            NativeFunction::TableUnpack => {
                unreachable!("table.unpack returns a variable result list")
            }
            NativeFunction::StringUnpack => {
                unreachable!("string.unpack returns a variable result list")
            }
            NativeFunction::StringByte => {
                unreachable!("string.byte returns a variable result list")
            }
            NativeFunction::MathFrexp => {
                unreachable!("math.frexp returns a variable result list")
            }
            NativeFunction::MathModf => {
                unreachable!("math.modf returns a variable result list")
            }
            NativeFunction::MathRandomSeed => {
                unreachable!("math.randomseed returns an empty result list")
            }
            NativeFunction::TableSort => unreachable!("table.sort needs call context"),
            NativeFunction::BufferWriteI8
            | NativeFunction::BufferWriteU8
            | NativeFunction::BufferWriteI16
            | NativeFunction::BufferWriteU16
            | NativeFunction::BufferWriteI32
            | NativeFunction::BufferWriteU32
            | NativeFunction::BufferWriteF32
            | NativeFunction::BufferWriteF64
            | NativeFunction::BufferWriteString
            | NativeFunction::BufferCopy
            | NativeFunction::BufferFill
            | NativeFunction::BufferWriteBits => {
                unreachable!("buffer write functions return an empty result list")
            }
            NativeFunction::TableForeach | NativeFunction::TableForeachi => {
                unreachable!("table.foreach needs call context")
            }
            NativeFunction::Error => self.native_error(args, call_site),
            NativeFunction::DebugTraceback => self.native_debug_traceback(args, call_site),
            NativeFunction::DebugInfo => unreachable!("debug.info returns a variable result list"),
            NativeFunction::Print => self.native_print(args),
            NativeFunction::Next => unreachable!("next returns a variable result list"),
            NativeFunction::CoroutineCreate => self.native_coroutine_create(args),
            NativeFunction::CoroutineWrap => self.native_coroutine_wrap(args),
            NativeFunction::CoroutineStatus => self.native_coroutine_status(args),
            NativeFunction::CoroutineRunning => self.native_coroutine_running(args),
            NativeFunction::CoroutineIsYieldable => self.native_coroutine_isyieldable(args),
            NativeFunction::CoroutineClose => {
                unreachable!("coroutine.close returns a variable result list")
            }
            NativeFunction::ToNumber => self.native_tonumber(args),
            NativeFunction::ToString => unreachable!("tostring needs call context"),
            NativeFunction::TypeOf => self.native_typeof(args),
            NativeFunction::GetFEnv => self.native_getfenv(args),
            NativeFunction::SetFEnv => unreachable!("setfenv returns a variable result list"),
            NativeFunction::SetMetatable => self.native_setmetatable(args),
            NativeFunction::GetMetatable => self.native_getmetatable(args),
            NativeFunction::NewProxy => self.native_newproxy(args),
            NativeFunction::CollectGarbage => self.native_collectgarbage(args),
            NativeFunction::GcInfo => self.native_gcinfo(args),
            NativeFunction::OsClock => self.native_os_clock(args),
            NativeFunction::OsDate => self.native_os_date(args),
            NativeFunction::OsTime => self.native_os_time(args),
            NativeFunction::OsDiffTime => self.native_os_difftime(args),
            NativeFunction::RawEqual => self.native_rawequal(args),
            NativeFunction::RawGet => self.native_rawget(args),
            NativeFunction::RawSet => self.native_rawset(args),
            NativeFunction::RawLen => self.native_rawlen(args),
            NativeFunction::GetCoverage => self.native_getcoverage(program, args),
            NativeFunction::ConformanceCxxThrow => self.native_conformance_cxxthrow(args),
            NativeFunction::ConformanceResumeError => {
                unreachable!("resumeerror returns a variable result list")
            }
            NativeFunction::ConformanceIsNative => self.native_conformance_is_native(args),
            NativeFunction::ConformanceInt64New => self.native_conformance_int64_new(args),
            NativeFunction::ConformanceInt64Eq => self.native_conformance_int64_eq(args),
            NativeFunction::ConformanceInt64Lt => self.native_conformance_int64_lt(args),
            NativeFunction::ConformanceInt64Le => self.native_conformance_int64_le(args),
            NativeFunction::ConformanceInt64Neg => self.native_conformance_int64_neg(args),
            NativeFunction::ConformanceInt64Add => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::Add)
            }
            NativeFunction::ConformanceInt64Sub => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::Sub)
            }
            NativeFunction::ConformanceInt64Mul => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::Mul)
            }
            NativeFunction::ConformanceInt64Div => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::Div)
            }
            NativeFunction::ConformanceInt64FloorDiv => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::FloorDiv)
            }
            NativeFunction::ConformanceInt64Mod => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::Mod)
            }
            NativeFunction::ConformanceInt64Pow => {
                self.native_conformance_int64_binary(args, ConformanceInt64Binary::Pow)
            }
            NativeFunction::ConformanceInt64ToString => {
                self.native_conformance_int64_tostring(args)
            }
            NativeFunction::ConformanceInt64Index => self.native_conformance_int64_index(args),
            NativeFunction::ConformanceInt64NewIndex => {
                self.native_conformance_int64_newindex(args)
            }
            NativeFunction::ConformanceVec2New => self.native_conformance_vec2_new(args),
            NativeFunction::ConformanceVec2Neg => self.native_conformance_vec2_neg(args),
            NativeFunction::ConformanceVec2Add => self.native_conformance_vec2_add(args),
            NativeFunction::ConformanceVec2Mul => self.native_conformance_vec2_mul(args),
            NativeFunction::ConformanceVec2Index => self.native_conformance_vec2_index(args),
            NativeFunction::ConformanceVec2Dot => self.native_conformance_vec2_dot(args),
            NativeFunction::ConformanceVec2Min => self.native_conformance_vec2_min(args),
            NativeFunction::ConformanceVertexNew => self.native_conformance_vertex_new(args),
            NativeFunction::ConformanceVertexIndex => self.native_conformance_vertex_index(args),
            NativeFunction::MathAbs => self.native_math_abs(args),
            NativeFunction::MathFloor => self.native_math_unary(args, MathUnary::Floor),
            NativeFunction::MathCeil => self.native_math_unary(args, MathUnary::Ceil),
            NativeFunction::MathRound => self.native_math_unary(args, MathUnary::Round),
            NativeFunction::MathSign => self.native_math_unary(args, MathUnary::Sign),
            NativeFunction::MathClamp => self.native_math_window(args, NativeMathWindow::Clamp),
            NativeFunction::MathMap => self.native_math_window(args, NativeMathWindow::Map),
            NativeFunction::MathLerp => self.native_math_window(args, NativeMathWindow::Lerp),
            NativeFunction::MathMin => self.native_math_minmax(args, MathMinMax::Min),
            NativeFunction::MathMax => self.native_math_minmax(args, MathMinMax::Max),
            NativeFunction::MathFmod => self.native_math_binary(args, MathBinary::Fmod),
            NativeFunction::MathPow => self.native_math_binary(args, MathBinary::Pow),
            NativeFunction::MathSqrt => self.native_math_unary(args, MathUnary::Sqrt),
            NativeFunction::MathDeg => self.native_math_unary(args, MathUnary::Deg),
            NativeFunction::MathRad => self.native_math_unary(args, MathUnary::Rad),
            NativeFunction::MathIsNan => self.native_math_predicate(args, MathPredicate::Nan),
            NativeFunction::MathIsInf => self.native_math_predicate(args, MathPredicate::Inf),
            NativeFunction::MathIsFinite => self.native_math_predicate(args, MathPredicate::Finite),
            NativeFunction::MathSin => self.native_math_unary(args, MathUnary::Sin),
            NativeFunction::MathCos => self.native_math_unary(args, MathUnary::Cos),
            NativeFunction::MathTan => self.native_math_unary(args, MathUnary::Tan),
            NativeFunction::MathAsin => self.native_math_unary(args, MathUnary::Asin),
            NativeFunction::MathAcos => self.native_math_unary(args, MathUnary::Acos),
            NativeFunction::MathAtan => self.native_math_unary(args, MathUnary::Atan),
            NativeFunction::MathAtan2 => self.native_math_binary(args, MathBinary::Atan2),
            NativeFunction::MathSinh => self.native_math_unary(args, MathUnary::Sinh),
            NativeFunction::MathCosh => self.native_math_unary(args, MathUnary::Cosh),
            NativeFunction::MathTanh => self.native_math_unary(args, MathUnary::Tanh),
            NativeFunction::MathExp => self.native_math_unary(args, MathUnary::Exp),
            NativeFunction::MathLog => self.native_math_log(args),
            NativeFunction::MathLog10 => self.native_math_unary(args, MathUnary::Log10),
            NativeFunction::MathLdexp => self.native_math_ldexp(args),
            NativeFunction::MathRandom => self.native_math_random(args),
            NativeFunction::MathNoise => self.native_math_noise(args),
            NativeFunction::StringSub => self.native_string_sub(args),
            NativeFunction::StringLen => self.native_string_len(args),
            NativeFunction::StringChar => self.native_string_char(args),
            NativeFunction::StringUpper => self.native_string_case(args, StringCase::Upper),
            NativeFunction::StringLower => self.native_string_case(args, StringCase::Lower),
            NativeFunction::StringReverse => self.native_string_reverse(args),
            NativeFunction::StringRep => self.native_string_rep(args),
            NativeFunction::StringFind => {
                unreachable!("string.find returns a variable result list")
            }
            NativeFunction::StringGsub => {
                unreachable!("string.gsub returns a variable result list")
            }
            NativeFunction::StringMatch => {
                unreachable!("string.match returns a variable result list")
            }
            NativeFunction::StringGMatch => {
                unreachable!("string.gmatch returns a variable result list")
            }
            NativeFunction::StringGMatchIterator => {
                unreachable!("string.gmatch iterator returns a variable result list")
            }
            NativeFunction::StringSplit => self.native_string_split(args),
            NativeFunction::StringFormat => unreachable!("string.format needs call context"),
            NativeFunction::StringPack => self.native_string_pack(args),
            NativeFunction::StringPackSize => self.native_string_packsize(args),
            NativeFunction::Utf8Char => self.native_utf8_char(args),
            NativeFunction::Utf8Codepoint => {
                unreachable!("utf8.codepoint returns a variable result list")
            }
            NativeFunction::Utf8Codes => {
                unreachable!("utf8.codes returns an iterator triple")
            }
            NativeFunction::Utf8CodesIterator => {
                unreachable!("utf8.codes iterator returns a variable result list")
            }
            NativeFunction::Utf8Len => unreachable!("utf8.len returns a variable result list"),
            NativeFunction::Utf8Offset => self.native_utf8_offset(args),
            NativeFunction::Bit32ArShift => {
                self.native_bit32_shift(args, Bit32Shift::ArithmeticRight)
            }
            NativeFunction::Bit32Band => self.native_bit32_fold(args, Bit32Fold::And),
            NativeFunction::Bit32BNot => self.native_bit32_bnot(args),
            NativeFunction::Bit32Bor => self.native_bit32_fold(args, Bit32Fold::Or),
            NativeFunction::Bit32BTest => self.native_bit32_btest(args),
            NativeFunction::Bit32BXor => self.native_bit32_fold(args, Bit32Fold::Xor),
            NativeFunction::Bit32Byteswap => self.native_bit32_byteswap(args),
            NativeFunction::Bit32CountLz => self.native_bit32_count(args, Bit32Count::Leading),
            NativeFunction::Bit32CountRz => self.native_bit32_count(args, Bit32Count::Trailing),
            NativeFunction::Bit32Extract => self.native_bit32_extract(args),
            NativeFunction::Bit32LRotate => self.native_bit32_rotate(args, Bit32Rotate::Left),
            NativeFunction::Bit32LShift => self.native_bit32_shift(args, Bit32Shift::LogicalLeft),
            NativeFunction::Bit32Replace => self.native_bit32_replace(args),
            NativeFunction::Bit32RRotate => self.native_bit32_rotate(args, Bit32Rotate::Right),
            NativeFunction::Bit32RShift => self.native_bit32_shift(args, Bit32Shift::LogicalRight),
            NativeFunction::TableClear => self.native_table_clear(args),
            NativeFunction::TableConcat => self.native_table_concat(args),
            NativeFunction::TablePack => self.native_table_pack(args),
            NativeFunction::TableGetN => self.native_table_getn(args),
            NativeFunction::TableMaxN => self.native_table_maxn(args),
            NativeFunction::TableCreate => self.native_table_create(args),
            NativeFunction::TableFind => unreachable!("table.find needs call context"),
            NativeFunction::TableClone => self.native_table_clone(args),
            NativeFunction::TableFreeze => self.native_table_freeze(args),
            NativeFunction::TableInsert => {
                unreachable!("table.insert returns an empty result list")
            }
            NativeFunction::TableIsFrozen => self.native_table_isfrozen(args),
            NativeFunction::TableMove => self.native_table_move(args),
            NativeFunction::TableRemove => self.native_table_remove(args),
            NativeFunction::BufferCreate => self.native_buffer_create(args),
            NativeFunction::BufferLen => self.native_buffer_len(args),
            NativeFunction::BufferFromString => self.native_buffer_fromstring(args),
            NativeFunction::BufferToString => self.native_buffer_tostring(args),
            NativeFunction::BufferReadI8 => self.native_buffer_read_int(args, BufferReadKind::I8),
            NativeFunction::BufferReadU8 => self.native_buffer_read_int(args, BufferReadKind::U8),
            NativeFunction::BufferReadI16 => self.native_buffer_read_int(args, BufferReadKind::I16),
            NativeFunction::BufferReadU16 => self.native_buffer_read_int(args, BufferReadKind::U16),
            NativeFunction::BufferReadI32 => self.native_buffer_read_int(args, BufferReadKind::I32),
            NativeFunction::BufferReadU32 => self.native_buffer_read_int(args, BufferReadKind::U32),
            NativeFunction::BufferReadF32 => {
                self.native_buffer_read_float(args, BufferFloatKind::F32)
            }
            NativeFunction::BufferReadF64 => {
                self.native_buffer_read_float(args, BufferFloatKind::F64)
            }
            NativeFunction::BufferReadString => self.native_buffer_readstring(args),
            NativeFunction::BufferReadBits => self.native_buffer_readbits(args),
            NativeFunction::VectorCreate => self.native_vector_create(args),
            NativeFunction::VectorCall => self.native_vector_call(args),
            NativeFunction::VectorDot => self.native_vector_dot(args, VectorCallStyle::Function),
            NativeFunction::VectorMethodDot => {
                self.native_vector_dot(args, VectorCallStyle::Method)
            }
            NativeFunction::VectorCross => {
                self.native_vector_cross(args, VectorCallStyle::Function)
            }
            NativeFunction::VectorMethodCross => {
                self.native_vector_cross(args, VectorCallStyle::Method)
            }
            NativeFunction::VectorNormalize => self.native_vector_normalize(args),
            NativeFunction::VectorMagnitude => self.native_vector_magnitude(args),
            NativeFunction::VectorAbs => self.native_vector_unary(args, VectorUnary::Abs),
            NativeFunction::VectorFloor => self.native_vector_unary(args, VectorUnary::Floor),
            NativeFunction::VectorCeil => self.native_vector_unary(args, VectorUnary::Ceil),
            NativeFunction::VectorSign => self.native_vector_unary(args, VectorUnary::Sign),
            NativeFunction::VectorAngle => self.native_vector_angle(args),
            NativeFunction::VectorMin => self.native_vector_minmax(args, VectorMinMax::Min),
            NativeFunction::VectorMax => self.native_vector_minmax(args, VectorMinMax::Max),
            NativeFunction::VectorClamp => self.native_vector_clamp(args),
            NativeFunction::VectorLerp => self.native_vector_lerp(args),
        }
    }

    fn native_extra_fuel_cost(
        &mut self,
        handle: u32,
        native: NativeFunction,
        args: &[Value],
    ) -> u64 {
        match native {
            NativeFunction::Assert
            | NativeFunction::PCall
            | NativeFunction::XPCall
            | NativeFunction::Select
            | NativeFunction::IPairs
            | NativeFunction::INext
            | NativeFunction::Pairs
            | NativeFunction::CoroutineCreate
            | NativeFunction::CoroutineResume
            | NativeFunction::CoroutineWrap
            | NativeFunction::CoroutineYield
            | NativeFunction::CoroutineStatus
            | NativeFunction::CoroutineRunning
            | NativeFunction::CoroutineIsYieldable
            | NativeFunction::CoroutineClose
            | NativeFunction::CoroutineWrappedCall
            | NativeFunction::ConformanceCxxThrow
            | NativeFunction::ConformanceResumeError
            | NativeFunction::DebugTraceback
            | NativeFunction::DebugInfo
            | NativeFunction::TablePack => usize_to_u64(args.len()),
            NativeFunction::LoadString => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64)
                .saturating_add(usize_to_u64(args.len())),
            NativeFunction::TableUnpack => self.native_table_unpack_fuel(args),
            NativeFunction::MathMin | NativeFunction::MathMax => {
                usize_to_u64(args.len().saturating_sub(1))
            }
            NativeFunction::Bit32Band
            | NativeFunction::Bit32Bor
            | NativeFunction::Bit32BTest
            | NativeFunction::Bit32BXor => usize_to_u64(args.len()),
            NativeFunction::StringFind => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64),
            NativeFunction::StringGsub => self.native_string_gsub_fuel(args),
            NativeFunction::StringGMatch => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64),
            NativeFunction::StringGMatchIterator => self.native_string_gmatch_iterator_fuel(handle),
            NativeFunction::StringSplit => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64),
            NativeFunction::StringFormat => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64)
                .saturating_add(usize_to_u64(args.len())),
            NativeFunction::StringChar => self.native_string_char_fuel(args),
            NativeFunction::StringRep => self.native_string_rep_fuel(args),
            NativeFunction::StringPack
            | NativeFunction::StringUnpack
            | NativeFunction::StringPackSize => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64),
            NativeFunction::Utf8Char => self.native_utf8_char_fuel(args),
            NativeFunction::Utf8Codepoint
            | NativeFunction::Utf8Len
            | NativeFunction::Utf8Offset => self.native_utf8_indexed_fuel(args),
            NativeFunction::Utf8Codes => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64),
            NativeFunction::Utf8CodesIterator => args
                .first()
                .and_then(|value| self.value_string_len_for_fuel(*value))
                .map_or(0, usize_to_u64)
                .saturating_add(
                    args.get(1)
                        .copied()
                        .map_or(0, |value| self.number_coercion_fuel(value)),
                ),
            NativeFunction::TableClear => self.native_table_clear_fuel(args),
            NativeFunction::TableConcat => self.native_table_concat_fuel(args),
            NativeFunction::TableFind => self.native_table_find_fuel(args),
            NativeFunction::TableForeach => self.native_table_foreach_fuel(args),
            NativeFunction::TableForeachi => self.native_table_foreachi_fuel(args),
            NativeFunction::TableClone => self.native_table_clone_fuel(args),
            NativeFunction::TableInsert => self.native_table_insert_fuel(args),
            NativeFunction::TableMove => self.native_table_move_fuel(args),
            NativeFunction::TableRemove => self.native_table_remove_fuel(args),
            NativeFunction::TableSort => self.native_table_sort_fuel(args),
            _ => 0,
        }
    }

    fn value_string_len_for_fuel(&mut self, value: Value) -> Option<usize> {
        match value.kind() {
            ValueKind::Nil => Some("nil".len()),
            ValueKind::Boolean => Some(if value.as_bool()? {
                "true".len()
            } else {
                "false".len()
            }),
            ValueKind::Number => Some(luau_number_to_string(value.as_number()?).len()),
            ValueKind::String => {
                let atom = value.as_string_atom()?;
                self.heap.string_bytes(atom).map(|text| text.len())
            }
            ValueKind::Table
            | ValueKind::Function
            | ValueKind::Userdata
            | ValueKind::Buffer
            | ValueKind::Vector => None,
        }
    }

    fn number_coercion_fuel(&mut self, value: Value) -> u64 {
        if value.kind() == ValueKind::String {
            self.value_string_len_for_fuel(value)
                .map_or(0, usize_to_u64)
        } else {
            0
        }
    }

    fn native_utf8_char_fuel(&mut self, args: &[Value]) -> u64 {
        args.iter()
            .copied()
            .fold(usize_to_u64(args.len()).saturating_mul(4), |cost, value| {
                cost.saturating_add(self.number_coercion_fuel(value))
            })
    }

    fn native_string_char_fuel(&mut self, args: &[Value]) -> u64 {
        args.iter()
            .copied()
            .fold(usize_to_u64(args.len()), |cost, value| {
                cost.saturating_add(self.number_coercion_fuel(value))
            })
    }

    fn native_utf8_indexed_fuel(&mut self, args: &[Value]) -> u64 {
        args.first()
            .and_then(|value| self.value_string_len_for_fuel(*value))
            .map_or(0, usize_to_u64)
            .saturating_add(
                args.get(1)
                    .copied()
                    .map_or(0, |value| self.number_coercion_fuel(value)),
            )
            .saturating_add(
                args.get(2)
                    .copied()
                    .map_or(0, |value| self.number_coercion_fuel(value)),
            )
    }

    fn native_string_gmatch_iterator_fuel(&mut self, handle: u32) -> u64 {
        let Some(upvalues) = self.heap.function_upvalues(handle) else {
            return 0;
        };
        let Some(text) = upvalues
            .first()
            .and_then(|upvalue| self.heap.upvalue_get(*upvalue))
        else {
            return 0;
        };
        self.value_string_len_for_fuel(text).map_or(0, usize_to_u64)
    }

    fn native_string_rep_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(text_len) = args
            .first()
            .and_then(|value| self.value_string_len_for_fuel(*value))
        else {
            return 0;
        };
        let Some(count) = args
            .get(1)
            .and_then(|value| value.as_number())
            .and_then(|value| string_repeat_count(value).ok())
        else {
            return 0;
        };
        if count <= 0 {
            0
        } else {
            usize_to_u64(text_len).saturating_mul(count as u64)
        }
    }

    fn native_string_gsub_fuel(&mut self, args: &[Value]) -> u64 {
        let text_len = args
            .first()
            .and_then(|value| self.value_string_len_for_fuel(*value))
            .map_or(0, usize_to_u64);
        let pattern_len = args
            .get(1)
            .and_then(|value| self.value_string_len_for_fuel(*value))
            .map_or(0, usize_to_u64);
        let replacement_len = args
            .get(2)
            .and_then(|value| self.value_string_len_for_fuel(*value))
            .map_or(0, usize_to_u64);
        text_len
            .saturating_add(pattern_len)
            .saturating_add(replacement_len)
            .saturating_add(text_len.saturating_mul(replacement_len))
    }

    fn native_table_clear_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        self.heap.table_storage_len(table).map_or(0, usize_to_u64)
    }

    fn native_table_concat_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        let Some(len) = self.heap.array_len(table) else {
            return 0;
        };
        let start = args
            .get(2)
            .and_then(|value| value.as_number())
            .and_then(|value| table_concat_index(value).ok())
            .unwrap_or(1);
        let end = args
            .get(3)
            .and_then(|value| value.as_number())
            .and_then(|value| table_concat_index(value).ok())
            .unwrap_or(len);
        range_work_fuel(start, end)
    }

    fn native_table_find_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        let Some(len) = self.heap.array_len(table) else {
            return 0;
        };
        let start = match args.get(2).copied() {
            None => 1,
            Some(value) if value.kind() == ValueKind::Nil => 1,
            Some(value) => {
                let Some(number) = value.as_number() else {
                    return 0;
                };
                let Ok(start) = table_find_start(number) else {
                    return 0;
                };
                start
            }
        };
        range_work_fuel(start, len)
    }

    fn native_table_foreach_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        self.heap
            .table_storage_len(table)
            .map_or(0, |len| usize_to_u64(len).saturating_mul(2))
    }

    fn native_table_foreachi_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        self.table_foreachi_len(table)
            .map_or(0, |len| usize_to_u64(len).saturating_mul(2))
    }

    fn native_table_clone_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        self.heap.table_storage_len(table).map_or(0, usize_to_u64)
    }

    fn native_table_insert_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        let Some(len) = self.heap.array_len(table) else {
            return 0;
        };
        if args.len() == 2 {
            return 1;
        }
        let Some(position) = args
            .get(1)
            .and_then(|value| value.as_number())
            .and_then(|value| table_insert_position_arg(value).ok())
        else {
            return 0;
        };
        shifted_array_work_fuel(len, position)
    }

    fn native_table_move_fuel(&mut self, args: &[Value]) -> u64 {
        let Ok(bounds) = self.table_move_bounds(args) else {
            return 0;
        };
        table_move_count(bounds.start, bounds.end).map_or(0, usize_to_u64)
    }

    fn native_table_sort_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        let Some(len) = self.heap.array_len(table) else {
            return 0;
        };
        usize_to_u64(len).saturating_mul(usize_to_u64(len))
    }

    fn native_table_remove_fuel(&mut self, args: &[Value]) -> u64 {
        let Some(table) = args.first().and_then(|value| value.as_table()) else {
            return 0;
        };
        let Some(len) = self.heap.array_len(table) else {
            return 0;
        };
        let position = match args.get(1).copied() {
            None => return 1,
            Some(value) if value.kind() == ValueKind::Nil => return 1,
            Some(value) => {
                let Some(number) = value.as_number() else {
                    return 0;
                };
                let Ok(position) = table_integer_arg(number, "integer table.remove position")
                else {
                    return 0;
                };
                position
            }
        };
        shifted_array_work_fuel(len, position)
    }

    fn native_table_unpack_fuel(&mut self, args: &[Value]) -> u64 {
        let Ok((_, start, end)) = self.table_unpack_bounds(args) else {
            return 0;
        };
        if end < start {
            return 0;
        }
        unpack_result_count(start, end).map_or(0, usize_to_u64)
    }

    fn native_assert(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        let Some(value) = args.first().copied() else {
            return Err(Error::new(ErrorKind::AssertionFailed {
                message: "missing argument #1".to_owned(),
            }));
        };
        if value.truthy() {
            return Ok(args.to_vec());
        }
        let message = match args.get(1).copied() {
            None => "assertion failed!".to_owned(),
            Some(value) if value.kind() == ValueKind::Nil => "assertion failed!".to_owned(),
            Some(value) => match value.kind() {
                ValueKind::String => {
                    let message = self.string_value_bytes(value)?.ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "string value missing atom",
                        })
                    })?;
                    String::from_utf8_lossy(&message).into_owned()
                }
                ValueKind::Number => luau_number_to_string(value.as_number().unwrap_or(f64::NAN)),
                _ => {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: format!(
                            "invalid argument #2 to 'assert' (string expected, got {})",
                            self.luau_type_name(value)
                        ),
                    }));
                }
            },
        };
        Err(Error::new(ErrorKind::AssertionFailed { message }))
    }

    fn native_error(&mut self, args: &[Value], call_site: Option<ErrorLocation>) -> Result<Value> {
        validate_native_arity(args, 0, 2, "error arguments")?;
        let value = args.first().copied().unwrap_or_else(Value::nil);
        let level = args
            .get(1)
            .and_then(|value| value.as_number())
            .unwrap_or(1.0);
        if let Some(message) = self.string_value_bytes(value)? {
            let error = Error::new(ErrorKind::RuntimeError {
                message: String::from_utf8_lossy(&message).into_owned(),
            });
            return Err(self.apply_error_level_location(error, level, call_site));
        }
        let message = match value.kind() {
            ValueKind::Nil => "nil".to_owned(),
            ValueKind::Boolean | ValueKind::Number => {
                String::from_utf8_lossy(&self.value_to_tostring(value)?).into_owned()
            }
            ValueKind::Table
            | ValueKind::Function
            | ValueKind::Userdata
            | ValueKind::Buffer
            | ValueKind::Vector => value_type_name(value).to_owned(),
            ValueKind::String => unreachable!("string errors are handled above"),
        };
        let error = Error::new(ErrorKind::GuestError { value, message });
        Err(self.apply_error_level_location(error, level, call_site))
    }

    fn apply_error_level_location(
        &self,
        error: Error,
        level: f64,
        call_site: Option<ErrorLocation>,
    ) -> Error {
        if level == 0.0 {
            return error.without_source_location();
        }
        let Some(level) = positive_integer_level(level) else {
            return error;
        };
        let location = if level == 1 {
            call_site
        } else {
            let frame_offset = level - 2;
            self.call_stack
                .len()
                .checked_sub(1 + frame_offset)
                .and_then(|index| self.call_stack.get(index))
                .and_then(|frame| frame.call_site.clone())
        };
        if let Some(location) = location {
            error.at_source(&location.source, location.line)
        } else {
            error
        }
    }

    fn native_debug_traceback(
        &mut self,
        args: &[Value],
        call_site: Option<ErrorLocation>,
    ) -> Result<Value> {
        validate_native_arity(args, 0, 3, "debug.traceback arguments")?;
        let (thread, message, level) = if let Some(coroutine) = args
            .first()
            .copied()
            .and_then(Value::as_userdata)
            .and_then(|handle| self.coroutine_for_handle(handle))
        {
            let message = match args.get(1).copied() {
                Some(value) if value.kind() != ValueKind::Nil => Some(self.value_to_string(value)?),
                _ => None,
            };
            let level = args
                .get(2)
                .copied()
                .filter(|value| value.kind() != ValueKind::Nil)
                .map(|value| stack_level_arg(value, "number"))
                .transpose()?
                .unwrap_or(0);
            (Some(coroutine), message, level)
        } else {
            let message = match args.first().copied() {
                Some(value) if value.kind() != ValueKind::Nil => Some(self.value_to_string(value)?),
                _ => None,
            };
            let level = args
                .get(1)
                .copied()
                .filter(|value| value.kind() != ValueKind::Nil)
                .map(|value| stack_level_arg(value, "number"))
                .transpose()?
                .unwrap_or(1);
            (None, message, level)
        };
        let mut traceback = String::new();
        if let Some(message) = message {
            traceback.push_str(&String::from_utf8_lossy(&message));
            traceback.push('\n');
        }
        if let Some(thread) = thread {
            let entries = self.coroutine_debug_traceback_entries(thread);
            for (location, debug_name) in entries.into_iter().skip(level) {
                traceback.push_str(&location.source);
                traceback.push(':');
                traceback.push_str(&location.line.to_string());
                if let Some(debug_name) = debug_name {
                    traceback.push_str(" function ");
                    traceback.push_str(&debug_name);
                }
                traceback.push('\n');
            }
            let atom = self.heap.intern_string(&traceback);
            return Ok(Value::string_atom(atom));
        }

        if let Some(snapshot) = self.protected_error_traceback.as_ref() {
            let entries = self.debug_traceback_entries_from_error_snapshot(snapshot);
            for (location, debug_name) in entries.into_iter().skip(level.saturating_sub(1)) {
                traceback.push_str(&location.source);
                traceback.push(':');
                traceback.push_str(&location.line.to_string());
                if let Some(debug_name) = debug_name {
                    traceback.push_str(" function ");
                    traceback.push_str(&debug_name);
                }
                traceback.push('\n');
            }
            let atom = self.heap.intern_string(&traceback);
            return Ok(Value::string_atom(atom));
        }

        let has_current_call_site = call_site.is_some();
        let entries = self.debug_traceback_entries(call_site);
        let level = if has_current_call_site {
            level
        } else {
            level.saturating_sub(1)
        };
        for (location, debug_name) in entries.into_iter().skip(level) {
            traceback.push_str(&location.source);
            traceback.push(':');
            traceback.push_str(&location.line.to_string());
            if let Some(debug_name) = debug_name {
                traceback.push_str(" function ");
                traceback.push_str(&debug_name);
            }
            traceback.push('\n');
        }
        let atom = self.heap.intern_string(&traceback);
        Ok(Value::string_atom(atom))
    }

    fn coroutine_debug_traceback_entries(
        &self,
        coroutine: usize,
    ) -> Vec<(ErrorLocation, Option<Arc<str>>)> {
        let Some(snapshot) = self
            .coroutines
            .get(coroutine)
            .and_then(|state| state.debug_snapshot.as_ref())
        else {
            return Vec::new();
        };
        self.debug_traceback_entries_from_frames(&snapshot.frames, snapshot.top_location.clone())
    }

    fn debug_traceback_entries(
        &self,
        call_site: Option<ErrorLocation>,
    ) -> Vec<(ErrorLocation, Option<Arc<str>>)> {
        self.debug_traceback_entries_from_frames(&self.call_stack, call_site)
    }

    fn debug_traceback_entries_from_frames(
        &self,
        frames: &[CallStackFrame],
        top_location: Option<ErrorLocation>,
    ) -> Vec<(ErrorLocation, Option<Arc<str>>)> {
        let frames = logical_debug_frames(frames);
        let mut entries = Vec::new();
        if let Some(location) = top_location {
            entries.push((
                location,
                frames.last().and_then(|frame| frame.debug_name.clone()),
            ));
        }
        let skip_top_frame = !entries.is_empty();
        for frame in frames.iter().rev().skip(if skip_top_frame { 1 } else { 0 }) {
            if let Some(location) = &frame.call_site {
                entries.push((location.clone(), frame.debug_name.clone()));
            }
        }
        entries
    }

    fn debug_traceback_entries_from_error_snapshot(
        &self,
        snapshot: &TracebackSnapshot,
    ) -> Vec<(ErrorLocation, Option<Arc<str>>)> {
        let mut entries = Vec::new();
        let frames = logical_debug_frames(&snapshot.frames);
        if let Some(location) = &snapshot.top_location {
            entries.push((
                location.clone(),
                frames.last().and_then(|frame| frame.debug_name.clone()),
            ));
        }
        for frame in frames.iter().rev() {
            if let Some(location) = &frame.call_site {
                entries.push((location.clone(), frame.debug_name.clone()));
            }
        }
        entries
    }

    fn native_debug_info(
        &mut self,
        program: &Program,
        handle: u32,
        args: &[Value],
        call_site: Option<ErrorLocation>,
    ) -> Result<Vec<Value>> {
        if args.len() > 3 {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "debug.info arguments",
            }));
        }
        let Some(subject) = args.first().copied() else {
            return Err(debug_info_error("function or level expected"));
        };

        let (target, options_value) = if subject.kind() == ValueKind::Function {
            let Some(options) = args.get(1).copied() else {
                return Err(debug_info_error("string expected"));
            };
            (
                Some(self.debug_info_function_target(program, subject)?),
                options,
            )
        } else if subject.kind() == ValueKind::Number {
            let level = debug_info_level_arg(subject)?;
            let Some(options) = args.get(1).copied() else {
                return Err(debug_info_error("string expected"));
            };
            (
                self.debug_info_stack_target(program, handle, level, call_site)?,
                options,
            )
        } else if let Some(coroutine) = subject
            .as_userdata()
            .and_then(|handle| self.coroutine_for_handle(handle))
        {
            if args.len() == 1 {
                return Err(debug_info_error("function or level expected"));
            }
            if args.len() == 2 {
                return Err(debug_info_error("string expected"));
            }
            let level = debug_info_level_arg(args[1])?;
            let options = args[2];
            (
                self.debug_info_coroutine_target(program, coroutine, level)?,
                options,
            )
        } else {
            return Err(debug_info_error("function or level expected"));
        };

        let options = self.debug_info_options(options_value)?;
        let Some(target) = target else {
            return Ok(Vec::new());
        };
        Ok(self.debug_info_results(&target, &options))
    }

    fn debug_info_stack_target(
        &mut self,
        program: &Program,
        current_native_handle: u32,
        level: usize,
        call_site: Option<ErrorLocation>,
    ) -> Result<Option<DebugInfoTarget>> {
        if level == 0 {
            return Ok(Some(debug_info_native_target(
                Value::function(current_native_handle),
                NativeFunction::DebugInfo,
            )));
        }

        let (function, debug_name, location) = {
            let frames = logical_debug_frames(&self.call_stack);
            let Some(frame_index) = frames.len().checked_sub(level) else {
                return Ok(None);
            };
            let Some(frame) = frames.get(frame_index) else {
                return Ok(None);
            };
            (
                frame.function,
                frame
                    .debug_name
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string),
                debug_info_logical_frame_location(&frames, frame_index, call_site),
            )
        };

        let mut target = if let Some(function) = function {
            self.debug_info_function_target(program, function)?
        } else {
            DebugInfoTarget {
                name: String::new(),
                source: String::new(),
                line: -1,
                function: Value::nil(),
                parameter_count: 0,
                is_vararg: true,
            }
        };
        target.name = debug_name;
        if let Some(location) = location {
            target.source = location.source.to_string();
            target.line = u32_line_to_i32(location.line);
        }
        Ok(Some(target))
    }

    fn debug_info_coroutine_target(
        &mut self,
        program: &Program,
        coroutine: usize,
        level: usize,
    ) -> Result<Option<DebugInfoTarget>> {
        let Some(snapshot) = self
            .coroutines
            .get(coroutine)
            .and_then(|state| state.debug_snapshot.clone())
        else {
            return Ok(None);
        };
        if level < snapshot.first_debug_info_level {
            return Ok(None);
        }
        let frame_offset = level - snapshot.first_debug_info_level;
        let (function, debug_name, location) = {
            let frames = logical_debug_frames(&snapshot.frames);
            let Some(frame_index) = frames.len().checked_sub(1 + frame_offset) else {
                return Ok(None);
            };
            let Some(frame) = frames.get(frame_index) else {
                return Ok(None);
            };
            (
                frame.function,
                frame
                    .debug_name
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string),
                debug_info_logical_frame_location(&frames, frame_index, snapshot.top_location),
            )
        };

        let mut target = if let Some(function) = function {
            self.debug_info_function_target(program, function)?
        } else {
            DebugInfoTarget {
                name: String::new(),
                source: String::new(),
                line: -1,
                function: Value::nil(),
                parameter_count: 0,
                is_vararg: true,
            }
        };
        target.name = debug_name;
        if let Some(location) = location {
            target.source = location.source.to_string();
            target.line = u32_line_to_i32(location.line);
        }
        Ok(Some(target))
    }

    fn debug_info_function_target(
        &mut self,
        program: &Program,
        function_value: Value,
    ) -> Result<DebugInfoTarget> {
        let handle = function_value.as_function().ok_or_else(|| {
            Error::new(ErrorKind::TypeError {
                expected: "function",
            })
        })?;
        let kind = self.heap.function_kind(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle is missing",
            })
        })?;
        match kind {
            FunctionKind::Native(native) => Ok(debug_info_native_target(function_value, native)),
            FunctionKind::Host(host) => {
                let op = self.host_function(host)?;
                Ok(DebugInfoTarget {
                    name: op.name.clone(),
                    source: "[Rust]".to_owned(),
                    line: -1,
                    function: function_value,
                    parameter_count: 0,
                    is_vararg: true,
                })
            }
            FunctionKind::Bytecode {
                program: None,
                prototype,
            } => {
                let function = program.functions.get(prototype as usize).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "function prototype is missing",
                    })
                })?;
                Ok(debug_info_bytecode_target(
                    function_value,
                    program.source_name.as_deref(),
                    function,
                ))
            }
            FunctionKind::Bytecode {
                program: Some(dynamic_program_id),
                prototype,
            } => {
                let dynamic_program = self.dynamic_program(dynamic_program_id)?;
                let function = dynamic_program
                    .functions
                    .get(prototype as usize)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "function prototype is missing",
                        })
                    })?;
                Ok(debug_info_bytecode_target(
                    function_value,
                    dynamic_program.source_name.as_deref(),
                    function,
                ))
            }
            FunctionKind::DynamicMain {
                program: dynamic_program_id,
            } => {
                let dynamic_program = self.dynamic_program(dynamic_program_id)?;
                Ok(debug_info_bytecode_target(
                    function_value,
                    dynamic_program.source_name.as_deref(),
                    &dynamic_program.main,
                ))
            }
        }
    }

    fn debug_info_options(&mut self, value: Value) -> Result<DebugInfoOptions> {
        let Some(bytes) = self.string_value_bytes(value)? else {
            return Err(debug_info_error("string expected"));
        };
        let mut seen_name = false;
        let mut seen_source = false;
        let mut seen_line = false;
        let mut seen_function = false;
        let mut seen_arity = false;
        let mut fields = Vec::with_capacity(bytes.len());
        for byte in bytes {
            let (field, seen) = match byte {
                b'n' => (DebugInfoField::Name, &mut seen_name),
                b's' => (DebugInfoField::Source, &mut seen_source),
                b'l' => (DebugInfoField::Line, &mut seen_line),
                b'f' => (DebugInfoField::Function, &mut seen_function),
                b'a' => (DebugInfoField::Arity, &mut seen_arity),
                _ => return Err(debug_info_error("invalid option")),
            };
            if *seen {
                return Err(debug_info_error("duplicate option"));
            }
            *seen = true;
            fields.push(field);
        }
        Ok(DebugInfoOptions { fields })
    }

    fn debug_info_results(
        &mut self,
        target: &DebugInfoTarget,
        options: &DebugInfoOptions,
    ) -> Vec<Value> {
        let mut results = Vec::new();
        for field in &options.fields {
            match field {
                DebugInfoField::Name => {
                    let atom = self.heap.intern_string(&target.name);
                    results.push(Value::string_atom(atom));
                }
                DebugInfoField::Source => {
                    let atom = self.heap.intern_string(&target.source);
                    results.push(Value::string_atom(atom));
                }
                DebugInfoField::Line => results.push(Value::number(f64::from(target.line))),
                DebugInfoField::Function => results.push(target.function),
                DebugInfoField::Arity => {
                    results.push(Value::number(target.parameter_count as f64));
                    results.push(Value::boolean(target.is_vararg));
                }
            }
        }
        results
    }

    fn native_print(&mut self, _args: &[Value]) -> Result<Value> {
        Ok(Value::nil())
    }

    fn native_loadstring(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, 2, "loadstring arguments")?;
        let source = self.value_to_string(args[0])?;
        let source_name = if let Some(value) = args
            .get(1)
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil)
        {
            Some(self.value_to_string(value)?)
        } else {
            None
        };
        let Some(compiler) = self.settings.loadstring_compiler else {
            let atom = self
                .heap
                .intern_string("loadstring compiler is not configured");
            return Ok(vec![Value::nil(), Value::string_atom(atom)]);
        };
        let program = match compiler(&source, source_name.as_deref()) {
            Ok(program) => program,
            Err(message) => {
                let atom = self.heap.intern_string(&message);
                return Ok(vec![Value::nil(), Value::string_atom(atom)]);
            }
        };
        let program_id = self.retain_dynamic_program(program)?;
        let environment = self.globals;
        let handle = self
            .heap
            .alloc_dynamic_main_function(program_id, environment)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function handle overflow",
                })
            })?;
        Ok(vec![Value::function(handle)])
    }

    fn native_coroutine_create(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "coroutine.create arguments")?;
        let callee = args[0];
        if callee.kind() != ValueKind::Function {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "function",
            }));
        }
        self.alloc_coroutine(callee, false).map(Value::userdata)
    }

    fn native_coroutine_wrap(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "coroutine.wrap arguments")?;
        let callee = args[0];
        if callee.kind() != ValueKind::Function {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "function",
            }));
        }
        self.alloc_coroutine(callee, true).map(Value::userdata)
    }

    fn native_coroutine_status(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "coroutine.status arguments")?;
        let coroutine = self.coroutine_index(args[0])?;
        let status = if self.coroutines[coroutine].dead {
            "dead"
        } else if self.running_coroutines.last().copied() == Some(coroutine) {
            "running"
        } else if self.running_coroutines.contains(&coroutine) {
            "normal"
        } else {
            "suspended"
        };
        let atom = self.heap.intern_string(status);
        Ok(Value::string_atom(atom))
    }

    fn native_coroutine_running(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 0, "coroutine.running arguments")?;
        let Some(coroutine) = self.running_coroutines.last().copied() else {
            return Ok(Value::nil());
        };
        Ok(Value::userdata(self.coroutines[coroutine].handle))
    }

    fn native_coroutine_isyieldable(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 0, "coroutine.isyieldable arguments")?;
        Ok(Value::boolean(self.non_yieldable_depth == 0))
    }

    fn native_coroutine_close(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, 1, "coroutine.close arguments")?;
        let coroutine = self.coroutine_index(args[0])?;
        if self.running_coroutines.last().copied() == Some(coroutine) {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot close running coroutine".to_owned(),
            }));
        }
        if self.running_coroutines.contains(&coroutine) {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot close normal coroutine".to_owned(),
            }));
        }
        let state = &mut self.coroutines[coroutine];
        state.frames.clear();
        state.initialized = true;
        state.dead = true;
        if let Some(error) = state.error.take() {
            return Ok(vec![Value::boolean(false), error]);
        }
        Ok(vec![Value::boolean(true)])
    }

    fn native_coroutine_yield(
        &mut self,
        args: &[Value],
        call_site: Option<ErrorLocation>,
    ) -> Result<Vec<Value>> {
        self.record_running_coroutine_yield_snapshot(call_site);
        let Some(collector) = self.yield_collectors.last_mut() else {
            if self.settings.enable_conformance_top_level_yield {
                return Ok(args.to_vec());
            }
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "attempt to yield across metamethod/C-call boundary".to_owned(),
            }));
        };
        collector.push(args.to_vec());
        Ok(Vec::new())
    }

    fn native_coroutine_yield_resumable(
        &mut self,
        args: &[Value],
        call_site: Option<ErrorLocation>,
    ) -> VmFlowResult<Vec<Value>> {
        self.record_running_coroutine_yield_snapshot(call_site);
        if self.running_coroutines.last().is_none() {
            if self.settings.enable_conformance_top_level_yield {
                return Ok(VmFlow::Return(args.to_vec()));
            }
            return Err(non_yieldable_boundary_error());
        }
        Ok(VmFlow::Yield(CoroutineYield {
            yielded: args.to_vec(),
            frames: Vec::new(),
        }))
    }

    fn record_running_coroutine_yield_snapshot(&mut self, top_location: Option<ErrorLocation>) {
        let Some(coroutine) = self.running_coroutines.last().copied() else {
            return;
        };
        let snapshot = CoroutineDebugSnapshot {
            frames: self.current_coroutine_debug_frames(),
            top_location,
            first_debug_info_level: 1,
        };
        self.coroutines[coroutine].debug_snapshot = Some(snapshot);
    }

    fn record_running_coroutine_error_snapshot(&mut self, top_location: Option<ErrorLocation>) {
        let Some(coroutine) = self.running_coroutines.last().copied() else {
            return;
        };
        let snapshot = CoroutineDebugSnapshot {
            frames: self.current_coroutine_debug_frames(),
            top_location,
            first_debug_info_level: 0,
        };
        let replace = self.coroutines[coroutine]
            .debug_snapshot
            .as_ref()
            .is_none_or(|existing| existing.frames.len() <= snapshot.frames.len());
        if replace {
            self.coroutines[coroutine].debug_snapshot = Some(snapshot);
        }
    }

    fn current_coroutine_frames(&self) -> Vec<CallStackFrame> {
        let base = self
            .running_coroutine_stack_bases
            .last()
            .copied()
            .unwrap_or(0);
        self.call_stack.get(base..).unwrap_or(&[]).to_vec()
    }

    fn current_coroutine_debug_frames(&self) -> Vec<CallStackFrame> {
        let base = self
            .running_coroutine_stack_bases
            .last()
            .copied()
            .unwrap_or(0);
        self.call_stack
            .get(base..)
            .unwrap_or(&[])
            .iter()
            .map(debug_only_call_stack_frame)
            .collect()
    }

    fn traceback_snapshot(&self, top_location: Option<ErrorLocation>) -> TracebackSnapshot {
        let frames = if self.running_coroutines.last().is_some() {
            self.current_coroutine_frames()
        } else {
            self.call_stack.clone()
        };
        TracebackSnapshot {
            frames,
            top_location,
        }
    }

    fn traceback_snapshot_after_base(
        &self,
        top_location: Option<ErrorLocation>,
        traceback_base: usize,
    ) -> TracebackSnapshot {
        let frames = if self.running_coroutines.last().is_some() {
            let stack_base = self
                .running_coroutine_stack_bases
                .last()
                .copied()
                .unwrap_or(0);
            let start = stack_base
                .saturating_add(traceback_base)
                .min(self.call_stack.len());
            self.call_stack.get(start..).unwrap_or(&[]).to_vec()
        } else {
            let start = traceback_base.min(self.call_stack.len());
            self.call_stack.get(start..).unwrap_or(&[]).to_vec()
        };
        TracebackSnapshot {
            frames,
            top_location,
        }
    }

    fn current_traceback_base(&self) -> usize {
        let stack_base = self
            .running_coroutine_stack_bases
            .last()
            .copied()
            .unwrap_or(0);
        self.call_stack.len().saturating_sub(stack_base)
    }

    fn take_error_traceback_after_base(
        &mut self,
        error: &Error,
        traceback_base: usize,
    ) -> TracebackSnapshot {
        if error.location.is_some() {
            if let Some(snapshot) = self.last_error_traceback.take() {
                if snapshot.top_location == error.location {
                    return self.trim_traceback_to_base(snapshot, traceback_base);
                }
            }
        } else {
            self.last_error_traceback = None;
        }
        self.traceback_snapshot_after_base(error.location.clone(), traceback_base)
    }

    fn trim_traceback_to_base(
        &self,
        mut snapshot: TracebackSnapshot,
        traceback_base: usize,
    ) -> TracebackSnapshot {
        let trim = traceback_base.min(snapshot.frames.len());
        snapshot.frames.drain(0..trim);
        snapshot
    }

    fn start_coroutine(
        &mut self,
        program: &Program,
        coroutine: usize,
        callee: Value,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        self.running_coroutines.push(coroutine);
        self.running_coroutine_stack_bases
            .push(self.call_stack.len());
        let flow = self.call_value_with_site_resumable(
            program,
            callee,
            args,
            depth.saturating_add(1),
            None,
            fuel,
        );
        let stack_base = self.running_coroutine_stack_bases.pop();
        debug_assert_eq!(stack_base, Some(self.call_stack.len()));
        let running = self.running_coroutines.pop();
        debug_assert_eq!(running, Some(coroutine));
        flow
    }

    fn resume_coroutine_frames(
        &mut self,
        coroutine: usize,
        frames: Vec<CoroutineFrame>,
        resume_results: Vec<Value>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        self.running_coroutines.push(coroutine);
        self.running_coroutine_stack_bases
            .push(self.call_stack.len());
        let flow = self.resume_coroutine_frames_inner(frames, resume_results, fuel);
        let stack_base = self.running_coroutine_stack_bases.pop();
        debug_assert_eq!(stack_base, Some(self.call_stack.len()));
        let running = self.running_coroutines.pop();
        debug_assert_eq!(running, Some(coroutine));
        flow
    }

    fn resume_coroutine_error(
        &mut self,
        coroutine: usize,
        frames: Vec<CoroutineFrame>,
        error: Error,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        self.running_coroutines.push(coroutine);
        self.running_coroutine_stack_bases
            .push(self.call_stack.len());
        let flow = self.propagate_coroutine_error(frames, error, fuel);
        let stack_base = self.running_coroutine_stack_bases.pop();
        debug_assert_eq!(stack_base, Some(self.call_stack.len()));
        let running = self.running_coroutines.pop();
        debug_assert_eq!(running, Some(coroutine));
        flow
    }

    fn resume_coroutine_frames_inner(
        &mut self,
        mut frames: Vec<CoroutineFrame>,
        mut results: Vec<Value>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        loop {
            if frames.is_empty() {
                return Ok(VmFlow::Return(results));
            }
            let frame = frames.remove(0);
            let flow_result = match frame {
                CoroutineFrame::Bytecode(frame) => {
                    self.resume_suspended_bytecode_frame(*frame, results, fuel)
                }
                CoroutineFrame::PCall => {
                    let mut protected_results = Vec::with_capacity(results.len() + 1);
                    protected_results.push(Value::boolean(true));
                    protected_results.extend(results);
                    results = protected_results;
                    continue;
                }
                CoroutineFrame::XPCall(_) => {
                    let mut protected_results = Vec::with_capacity(results.len() + 1);
                    protected_results.push(Value::boolean(true));
                    protected_results.extend(results);
                    results = protected_results;
                    continue;
                }
            };
            let flow = match flow_result {
                Ok(flow) => flow,
                Err(error) => return self.propagate_coroutine_error(frames, error, fuel),
            };
            match flow {
                VmFlow::Return(returned) => {
                    results = returned;
                }
                VmFlow::Yield(mut suspension) => {
                    suspension.frames.extend(frames);
                    return Ok(VmFlow::Yield(suspension));
                }
            }
        }
    }

    fn propagate_coroutine_error(
        &mut self,
        mut frames: Vec<CoroutineFrame>,
        error: Error,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        loop {
            if frames.is_empty() {
                return Err(error);
            }
            match frames.remove(0) {
                CoroutineFrame::Bytecode(_) => {}
                CoroutineFrame::PCall => {
                    self.last_error_traceback = None;
                    let error = self.error_object(error);
                    return self.resume_coroutine_frames_inner(
                        frames,
                        vec![Value::boolean(false), error],
                        fuel,
                    );
                }
                CoroutineFrame::XPCall(frame) => {
                    let results = self.resume_xpcall_error_handler(frame, error, fuel)?;
                    return self.resume_coroutine_frames_inner(frames, results, fuel);
                }
            }
        }
    }

    fn resume_xpcall_error_handler(
        &mut self,
        frame: SuspendedXPCallFrame,
        error: Error,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let program = self.dynamic_program(frame.program_id)?;
        let original_is_out_of_memory = error_is_out_of_memory(&error);
        let traceback = self.take_error_traceback_after_base(&error, frame.traceback_base);
        let error_value = self.error_object(error);
        match self.call_value_non_yieldable_with_traceback_snapshot(
            &program,
            frame.handler,
            &[error_value],
            frame.depth + 1,
            traceback,
            fuel,
        ) {
            Ok(results) => Ok(vec![Value::boolean(false), first_result_or_nil(results)]),
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(handler_error) => {
                Ok(self.xpcall_handler_error_result(original_is_out_of_memory, handler_error))
            }
        }
    }

    fn resume_suspended_bytecode_frame(
        &mut self,
        frame: SuspendedBytecodeFrame,
        resume_results: Vec<Value>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        let program = self.dynamic_program(frame.program_id)?;
        let function = match frame.prototype {
            Some(prototype) => program.functions.get(prototype as usize).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "suspended coroutine function prototype is missing",
                })
            })?,
            None => &program.main,
        };
        let upvalues = frame.upvalues;
        self.run_function(
            &program,
            Some(frame.program_id),
            function,
            &[],
            FunctionCallContext {
                current_function: frame.current_function,
                upvalues: &upvalues,
                depth: frame.depth,
                environment: frame.environment,
                call_site: frame.call_site,
                prototype: frame.prototype,
            },
            Some(ResumedFunctionState {
                pc: frame.pc,
                registers: frame.registers,
                register_cells: frame.register_cells,
                varargs: frame.varargs,
                pending_return_prefixes: frame.pending_return_prefixes,
                continuation: frame.continuation,
                resume_results,
                stackless_self_calls: frame.stackless_self_calls,
                tail_call_count: frame.tail_call_count,
            }),
            fuel,
        )
    }

    fn native_coroutine_resume(
        &mut self,
        args: &[Value],
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, u8::MAX as usize, "coroutine.resume arguments")?;
        let coroutine = self.coroutine_index(args[0])?;
        if self.running_coroutines.contains(&coroutine) {
            let atom = self.heap.intern_string("cannot resume running coroutine");
            return Ok(vec![Value::boolean(false), Value::string_atom(atom)]);
        }
        if self.coroutines[coroutine].dead {
            let atom = self.heap.intern_string("cannot resume dead coroutine");
            return Ok(vec![Value::boolean(false), Value::string_atom(atom)]);
        }
        let flow = if !self.coroutines[coroutine].initialized {
            let callee = self.coroutines[coroutine].callee;
            self.start_coroutine(program, coroutine, callee, &args[1..], depth, fuel)
        } else if self.coroutines[coroutine].frames.is_empty() {
            self.coroutines[coroutine].dead = true;
            let mut values = vec![Value::boolean(true)];
            values.extend_from_slice(&args[1..]);
            return Ok(values);
        } else {
            let frames = std::mem::take(&mut self.coroutines[coroutine].frames);
            self.resume_coroutine_frames(coroutine, frames, args[1..].to_vec(), fuel)
        };
        match flow {
            Ok(VmFlow::Return(results)) => {
                let state = &mut self.coroutines[coroutine];
                state.initialized = true;
                state.dead = true;
                state.frames.clear();
                let mut values = vec![Value::boolean(true)];
                values.extend(results);
                Ok(values)
            }
            Ok(VmFlow::Yield(suspension)) => {
                let state = &mut self.coroutines[coroutine];
                state.initialized = true;
                state.frames = suspension.frames;
                let mut values = vec![Value::boolean(true)];
                values.extend(suspension.yielded);
                Ok(values)
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                self.coroutines[coroutine].dead = true;
                self.coroutines[coroutine].frames.clear();
                let error_value = self.error_object(error);
                self.coroutines[coroutine].error = Some(error_value);
                Ok(vec![Value::boolean(false), error_value])
            }
        }
    }

    fn native_conformance_resume_error(
        &mut self,
        args: &[Value],
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, 2, "resumeerror arguments")?;
        let coroutine = self.coroutine_index(args[0])?;
        if self.running_coroutines.contains(&coroutine) {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot resume running coroutine".to_owned(),
            }));
        }
        if self.coroutines[coroutine].dead {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot resume dead coroutine".to_owned(),
            }));
        }
        if !self.coroutines[coroutine].initialized || self.coroutines[coroutine].frames.is_empty() {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot resume non-suspended coroutine with error".to_owned(),
            }));
        }

        let frames = std::mem::take(&mut self.coroutines[coroutine].frames);
        let error = self.guest_error_from_value(args[1])?;
        let flow = self.resume_coroutine_error(coroutine, frames, error, fuel);
        match flow {
            Ok(VmFlow::Return(_)) => {
                let state = &mut self.coroutines[coroutine];
                state.initialized = true;
                state.dead = true;
                state.frames.clear();
                Ok(Vec::new())
            }
            Ok(VmFlow::Yield(suspension)) => {
                let state = &mut self.coroutines[coroutine];
                state.initialized = true;
                state.frames = suspension.frames;
                Ok(Vec::new())
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                self.coroutines[coroutine].dead = true;
                self.coroutines[coroutine].frames.clear();
                let error_value = self.error_object(error);
                self.coroutines[coroutine].error = Some(error_value);
                Ok(Vec::new())
            }
        }
    }

    fn native_coroutine_wrapped_call(
        &mut self,
        args: &[Value],
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let Some(wrapper) = args.first().copied() else {
            return Err(Error::new(ErrorKind::TypeError { expected: "thread" }));
        };
        let coroutine = self.coroutine_index(wrapper)?;
        self.call_coroutine_wrapper_by_index(
            coroutine,
            args.get(1..).unwrap_or(&[]),
            program,
            depth,
            fuel,
        )
    }

    fn call_coroutine_wrapper_by_index(
        &mut self,
        coroutine: usize,
        args: &[Value],
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        if self.coroutines[coroutine].dead {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot resume dead coroutine".to_owned(),
            }));
        }
        let flow = if !self.coroutines[coroutine].initialized {
            let callee = self.coroutines[coroutine].callee;
            self.start_coroutine(program, coroutine, callee, args, depth, fuel)
        } else if self.coroutines[coroutine].frames.is_empty() {
            self.coroutines[coroutine].dead = true;
            return Ok(args.to_vec());
        } else {
            let frames = std::mem::take(&mut self.coroutines[coroutine].frames);
            self.resume_coroutine_frames(coroutine, frames, args.to_vec(), fuel)
        };
        match flow {
            Ok(VmFlow::Return(results)) => {
                let state = &mut self.coroutines[coroutine];
                state.initialized = true;
                state.dead = true;
                state.frames.clear();
                Ok(results)
            }
            Ok(VmFlow::Yield(suspension)) => {
                let state = &mut self.coroutines[coroutine];
                state.initialized = true;
                state.frames = suspension.frames;
                Ok(suspension.yielded)
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                self.coroutines[coroutine].dead = true;
                self.coroutines[coroutine].frames.clear();
                let error_value = self.error_object(error);
                self.coroutines[coroutine].error = Some(error_value);
                Err(Error::new(ErrorKind::GuestError {
                    value: error_value,
                    message: self.error_value_message(error_value),
                }))
            }
        }
    }

    fn alloc_coroutine(&mut self, callee: Value, callable: bool) -> Result<u32> {
        let metatable = if callable {
            let metatable = self.heap.alloc_array_table(&[]);
            set_native_table_field(
                &mut self.heap,
                metatable,
                "__call",
                NativeFunction::CoroutineWrappedCall,
            );
            Some(metatable)
        } else {
            None
        };
        let callable_metatable = metatable
            .map(|metatable| {
                self.heap
                    .table_version(metatable)
                    .map(|version| (metatable, version))
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "coroutine wrapper metatable handle is missing",
                        })
                    })
            })
            .transpose()?;
        let handle = self.heap.alloc_userdata(metatable).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "coroutine handle overflow",
            })
        })?;
        self.coroutines.push(CoroutineState {
            handle,
            callee,
            frames: Vec::new(),
            initialized: false,
            dead: false,
            error: None,
            debug_snapshot: None,
            callable_metatable,
        });
        Ok(handle)
    }

    fn coroutine_index(&self, value: Value) -> Result<usize> {
        let Some(handle) = value.as_userdata() else {
            return Err(Error::new(ErrorKind::TypeError { expected: "thread" }));
        };
        self.coroutine_for_handle(handle)
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "thread" }))
    }

    fn coroutine_for_handle(&self, handle: u32) -> Option<usize> {
        self.coroutines
            .iter()
            .position(|coroutine| coroutine.handle == handle)
    }

    fn native_pcall(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let callee = args.first().copied().unwrap_or_else(Value::nil);
        let call_args = args.get(1..).unwrap_or(&[]);
        let protected_call_site = self.bytecode_protected_call_site(callee, call_site);
        match self.call_value_with_site(
            program,
            callee,
            call_args,
            depth + 1,
            protected_call_site,
            fuel,
        ) {
            Ok(results) => {
                let mut protected_results = Vec::with_capacity(results.len() + 1);
                protected_results.push(Value::boolean(true));
                protected_results.extend(results);
                Ok(protected_results)
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                self.last_error_traceback = None;
                let error = self.error_object(error);
                Ok(vec![Value::boolean(false), error])
            }
        }
    }

    fn native_pcall_resumable(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        let callee = args.first().copied().unwrap_or_else(Value::nil);
        let call_args = args.get(1..).unwrap_or(&[]);
        let protected_call_site = self.bytecode_protected_call_site(callee, call_site);
        match self.call_value_with_site_resumable(
            program,
            callee,
            call_args,
            depth + 1,
            protected_call_site,
            fuel,
        ) {
            Ok(VmFlow::Return(results)) => {
                let mut protected_results = Vec::with_capacity(results.len() + 1);
                protected_results.push(Value::boolean(true));
                protected_results.extend(results);
                Ok(VmFlow::Return(protected_results))
            }
            Ok(VmFlow::Yield(mut suspension)) => {
                suspension.frames.push(CoroutineFrame::PCall);
                Ok(VmFlow::Yield(suspension))
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                self.last_error_traceback = None;
                let error = self.error_object(error);
                Ok(VmFlow::Return(vec![Value::boolean(false), error]))
            }
        }
    }

    fn native_xpcall(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        let callee = args.first().copied().unwrap_or_else(Value::nil);
        let handler = self.xpcall_handler(args)?;
        let call_args = args.get(2..).unwrap_or(&[]);
        let traceback_base = self.current_traceback_base();
        let protected_call_site = self.bytecode_protected_call_site(callee, call_site);
        match self.call_value_with_site(
            program,
            callee,
            call_args,
            depth + 1,
            protected_call_site,
            fuel,
        ) {
            Ok(results) => {
                let mut protected_results = Vec::with_capacity(results.len() + 1);
                protected_results.push(Value::boolean(true));
                protected_results.extend(results);
                Ok(protected_results)
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                let original_is_out_of_memory = error_is_out_of_memory(&error);
                let traceback = self.take_error_traceback_after_base(&error, traceback_base);
                let error_value = self.error_object(error);
                match self.call_value_with_traceback_snapshot(
                    program,
                    handler,
                    &[error_value],
                    depth + 1,
                    traceback,
                    fuel,
                ) {
                    Ok(results) => Ok(vec![Value::boolean(false), first_result_or_nil(results)]),
                    Err(error) if error_is_uncatchable(&error) => Err(error),
                    Err(handler_error) => {
                        Ok(self
                            .xpcall_handler_error_result(original_is_out_of_memory, handler_error))
                    }
                }
            }
        }
    }

    fn native_xpcall_resumable(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        call_site: Option<ErrorLocation>,
        fuel: &mut Option<u64>,
    ) -> VmFlowResult<Vec<Value>> {
        let callee = args.first().copied().unwrap_or_else(Value::nil);
        let handler = self.xpcall_handler(args)?;
        let call_args = args.get(2..).unwrap_or(&[]);
        let traceback_base = self.current_traceback_base();
        let protected_call_site = self.bytecode_protected_call_site(callee, call_site);
        match self.call_value_with_site_resumable(
            program,
            callee,
            call_args,
            depth + 1,
            protected_call_site,
            fuel,
        ) {
            Ok(VmFlow::Return(results)) => {
                let mut protected_results = Vec::with_capacity(results.len() + 1);
                protected_results.push(Value::boolean(true));
                protected_results.extend(results);
                Ok(VmFlow::Return(protected_results))
            }
            Ok(VmFlow::Yield(mut suspension)) => {
                let program_id = self.retain_dynamic_program(program.clone())?;
                suspension
                    .frames
                    .push(CoroutineFrame::XPCall(SuspendedXPCallFrame {
                        program_id,
                        handler,
                        depth,
                        traceback_base: 0,
                    }));
                Ok(VmFlow::Yield(suspension))
            }
            Err(error) if error_is_uncatchable(&error) => Err(error),
            Err(error) => {
                let original_is_out_of_memory = error_is_out_of_memory(&error);
                let traceback = self.take_error_traceback_after_base(&error, traceback_base);
                let error_value = self.error_object(error);
                match self.call_value_non_yieldable_with_traceback_snapshot(
                    program,
                    handler,
                    &[error_value],
                    depth + 1,
                    traceback,
                    fuel,
                ) {
                    Ok(results) => Ok(VmFlow::Return(vec![
                        Value::boolean(false),
                        first_result_or_nil(results),
                    ])),
                    Err(error) if error_is_uncatchable(&error) => Err(error),
                    Err(handler_error) => Ok(VmFlow::Return(
                        self.xpcall_handler_error_result(original_is_out_of_memory, handler_error),
                    )),
                }
            }
        }
    }

    fn xpcall_handler_error_result(
        &mut self,
        original_is_out_of_memory: bool,
        handler_error: Error,
    ) -> Vec<Value> {
        if original_is_out_of_memory && error_is_out_of_memory(&handler_error) {
            let atom = self.heap.intern_string("not enough memory");
            return vec![Value::boolean(false), Value::string_atom(atom)];
        }
        let atom = self.heap.intern_string("error in error handling");
        vec![Value::boolean(false), Value::string_atom(atom)]
    }

    fn xpcall_handler(&mut self, args: &[Value]) -> Result<Value> {
        let Some(handler) = args.get(1).copied() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "missing argument #2 to 'xpcall' (function expected)".to_owned(),
            }));
        };
        if handler.kind() == ValueKind::Function {
            Ok(handler)
        } else {
            Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "invalid argument #2 to 'xpcall' (function expected, got {})",
                    value_type_name(handler)
                ),
            }))
        }
    }

    fn error_object(&mut self, error: Error) -> Value {
        if let ErrorKind::GuestError { value, .. } = &error.kind {
            return *value;
        }
        let message = error.to_string();
        Value::string_atom(self.heap.intern_string(&message))
    }

    fn guest_error_from_value(&mut self, value: Value) -> Result<Error> {
        let message = if let Some(bytes) = self.string_value_bytes(value)? {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            String::from_utf8_lossy(&self.value_to_tostring(value)?).into_owned()
        };
        Ok(Error::new(ErrorKind::GuestError { value, message }))
    }

    fn error_value_message(&mut self, value: Value) -> String {
        match self.string_value_bytes(value) {
            Ok(Some(message)) => String::from_utf8_lossy(&message).into_owned(),
            Ok(None) => value_type_name(value).to_owned(),
            Err(error) => error.to_string(),
        }
    }

    fn native_select(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        let Some(selector) = args.first().copied() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "missing argument #1".to_owned(),
            }));
        };
        let values = args.get(1..).unwrap_or(&[]);
        if self.string_value_bytes(selector)?.as_deref() == Some(b"#") {
            return Ok(vec![Value::number(values.len() as f64)]);
        }
        let Some(index) = self.coerce_to_number(selector)? else {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        };
        let index = string_library_index(index)?;
        let count = i64::try_from(values.len()).map_err(|_| {
            Error::new(ErrorKind::RuntimeError {
                message: "too many values to select".to_owned(),
            })
        })?;
        let start = if index < 0 { count + index + 1 } else { index };
        if start < 1 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "index out of range".to_owned(),
            }));
        }
        let start = usize::try_from(start - 1).map_err(|_| {
            Error::new(ErrorKind::RuntimeError {
                message: "index out of range".to_owned(),
            })
        })?;
        if start >= values.len() {
            return Ok(Vec::new());
        }
        Ok(values[start..].to_vec())
    }

    fn native_tonumber(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 2, "tonumber argument")?;
        let base = match args.get(1).copied() {
            Some(value) if value.kind() != ValueKind::Nil => Some(self.tonumber_base(value)?),
            _ => None,
        };
        let number = if matches!(base, None | Some(10)) {
            self.coerce_to_number(args[0])?
        } else {
            let text = self.value_to_string(args[0])?;
            parse_tonumber_base(&text, base.expect("non-decimal base is present"))
        };
        Ok(number.map_or_else(Value::nil, Value::number))
    }

    fn tonumber_base(&mut self, value: Value) -> Result<u32> {
        let base = self.coerce_to_i32(value, "number")?;
        if !(2..=36).contains(&base) {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "base between 2 and 36",
            }));
        }
        Ok(base as u32)
    }

    fn native_tostring(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        validate_native_arity(args, 1, usize::MAX, "tostring argument")?;
        let text = self.value_to_tostring_with_metamethod(program, args[0], depth, fuel)?;
        let atom = self.heap.intern_string_bytes(&text);
        Ok(Value::string_atom(atom))
    }

    fn native_typeof(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, usize::MAX, "type argument")?;
        let atom = self.heap.intern_string(self.luau_type_name(args[0]));
        Ok(Value::string_atom(atom))
    }

    fn native_getfenv(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 1, "getfenv argument")?;
        let environment = match args.first().copied().unwrap_or_else(|| Value::number(1.0)) {
            value if value.kind() == ValueKind::Nil => self.stack_environment(1)?,
            value if value.as_number() == Some(0.0) => self.globals,
            value if value.kind() == ValueKind::Number => {
                let level = stack_level_arg(value, "getfenv argument")?;
                self.stack_environment(level)?
            }
            value if value.kind() == ValueKind::Function => {
                let handle = value.as_function().ok_or_else(|| {
                    Error::new(ErrorKind::TypeError {
                        expected: "function",
                    })
                })?;
                if matches!(
                    self.heap.function_kind(handle),
                    Some(FunctionKind::Native(_))
                ) {
                    self.globals
                } else {
                    self.heap.function_environment(handle).ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "function environment is missing",
                        })
                    })?
                }
            }
            _ => {
                return Err(Error::new(ErrorKind::TypeError {
                    expected: "number or function",
                }));
            }
        };
        Ok(Value::table(environment))
    }

    fn native_setfenv(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, 2, "setfenv arguments")?;
        let environment = args[1]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        match args[0] {
            value if value.as_number() == Some(0.0) => {
                self.globals = environment;
                Ok(Vec::new())
            }
            value if value.kind() == ValueKind::Number => {
                let level = stack_level_arg(value, "setfenv argument")?;
                let function = self.set_stack_environment(level, environment)?;
                if let Some(handle) = function.and_then(Value::as_function) {
                    self.heap
                        .set_function_environment(handle, environment)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::InvalidProgram {
                                reason: "function environment is missing",
                            })
                        })?;
                }
                Ok(vec![function.unwrap_or_else(Value::nil)])
            }
            value if value.kind() == ValueKind::Function => {
                let handle = value.as_function().ok_or_else(|| {
                    Error::new(ErrorKind::TypeError {
                        expected: "function",
                    })
                })?;
                if matches!(
                    self.heap.function_kind(handle),
                    Some(FunctionKind::Native(_))
                ) {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "cannot change environment of native function".to_owned(),
                    }));
                }
                self.heap
                    .set_function_environment(handle, environment)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "function environment is missing",
                        })
                    })?;
                Ok(vec![value])
            }
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "number or function",
            })),
        }
    }

    fn current_environment(&mut self) -> Result<u32> {
        let Some(frame) = self.call_stack.last().cloned() else {
            return Ok(self.globals);
        };
        self.frame_environment(&frame)
    }

    fn stack_environment(&mut self, level: usize) -> Result<u32> {
        if level == 0 {
            return Ok(self.globals);
        }
        let index = self.call_stack.len().checked_sub(level).ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: "level out of range".to_owned(),
            })
        })?;
        let frame = self.call_stack[index].clone();
        self.frame_environment(&frame)
    }

    fn frame_environment(&mut self, frame: &CallStackFrame) -> Result<u32> {
        if let Some(handle) = frame.function.and_then(Value::as_function) {
            return self.heap.function_environment(handle).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function environment is missing",
                })
            });
        }
        Ok(frame.environment)
    }

    fn set_stack_environment(&mut self, level: usize, environment: u32) -> Result<Option<Value>> {
        if level == 0 {
            self.globals = environment;
            return Ok(None);
        }
        let index = self.call_stack.len().checked_sub(level).ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: "level out of range".to_owned(),
            })
        })?;
        let frame = &mut self.call_stack[index];
        frame.environment = environment;
        Ok(frame.function)
    }

    fn native_setmetatable(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "setmetatable arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        self.ensure_table_writable(table)?;
        if self.table_has_protected_metatable(table)? {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot change a protected metatable".to_owned(),
            }));
        }
        let metatable = match args[1].kind() {
            ValueKind::Nil => None,
            ValueKind::Table => args[1].as_table(),
            _ => {
                return Err(Error::new(ErrorKind::TypeError {
                    expected: "nil or table",
                }));
            }
        };
        self.heap
            .table_set_metatable(table, metatable)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "setmetatable table handle is missing",
                })
            })?;
        Ok(args[0])
    }

    fn native_getmetatable(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "getmetatable argument")?;
        let metatable = match args[0].kind() {
            ValueKind::Table => {
                let table = args[0].as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })?;
                self.heap.table_metatable(table).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })?
            }
            ValueKind::Userdata => {
                let userdata = args[0].as_userdata().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "userdata handle is missing",
                    })
                })?;
                self.heap.userdata_metatable(userdata).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "userdata handle is missing",
                    })
                })?
            }
            _ => None,
        };
        if let Some(metatable) = metatable {
            if let Some(protected) = self.protected_metatable_value(metatable)? {
                return Ok(protected);
            }
            return Ok(Value::table(metatable));
        }
        Ok(metatable.map_or_else(Value::nil, Value::table))
    }

    fn native_newproxy(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 1, "newproxy argument")?;
        let needs_metatable = match args.first().copied() {
            None => false,
            Some(value) if value.kind() == ValueKind::Nil => false,
            Some(value) => value.as_bool().ok_or_else(|| {
                Error::new(ErrorKind::TypeError {
                    expected: "nil or boolean",
                })
            })?,
        };
        let metatable = needs_metatable.then(|| self.heap.alloc_array_table(&[]));
        let userdata = self.heap.alloc_userdata(metatable).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "userdata handle overflow",
            })
        })?;
        Ok(Value::userdata(userdata))
    }

    fn collectgarbage_roots(&self, args: &[Value]) -> Vec<Value> {
        let mut roots = Vec::new();
        roots.push(Value::table(self.globals));
        roots.push(Value::table(self.standard_globals));
        roots.extend_from_slice(args);
        for frame in &self.call_stack {
            roots.push(Value::table(frame.environment));
            if let Some(function) = frame.function {
                roots.push(function);
            }
            roots.extend_from_slice(&frame.roots);
            roots.extend_from_slice(&frame.stackless_roots);
        }
        for coroutine in &self.coroutines {
            roots.push(coroutine.callee);
            for frame in &coroutine.frames {
                match frame {
                    CoroutineFrame::Bytecode(frame) => {
                        if let Some(function) = frame.current_function {
                            roots.push(function);
                        }
                        roots.extend_from_slice(&frame.registers);
                        roots.extend_from_slice(&frame.varargs);
                        roots.extend_from_slice(&frame.pending_return_prefixes);
                        extend_bytecode_continuation_roots(&mut roots, &frame.continuation);
                        for stackless_frame in &frame.stackless_self_calls {
                            extend_stackless_self_call_frame_roots(&mut roots, stackless_frame);
                        }
                    }
                    CoroutineFrame::PCall => {}
                    CoroutineFrame::XPCall(frame) => {
                        roots.push(frame.handler);
                    }
                }
            }
            if let Some(error) = coroutine.error {
                roots.push(error);
            }
        }
        for collector in &self.yield_collectors {
            for result_set in collector {
                roots.extend_from_slice(result_set);
            }
        }
        if let Some(snapshot) = &self.last_error_traceback {
            collect_traceback_snapshot_roots(&mut roots, snapshot);
        }
        if let Some(snapshot) = &self.protected_error_traceback {
            collect_traceback_snapshot_roots(&mut roots, snapshot);
        }
        roots
    }

    fn update_current_gc_roots(
        &mut self,
        registers: &[Value],
        varargs: &[Value],
        pending_return_prefixes: &[Value],
    ) {
        let Some(frame) = self.call_stack.last_mut() else {
            return;
        };
        frame.roots.clear();
        extend_weak_collectable_roots(&mut frame.roots, registers);
        extend_weak_collectable_roots(&mut frame.roots, varargs);
        extend_weak_collectable_roots(&mut frame.roots, pending_return_prefixes);
    }

    fn refresh_current_gc_roots_if_dirty(
        &mut self,
        roots_dirty: &mut bool,
        registers: &[Value],
        varargs: &[Value],
        pending_return_prefixes: &[Value],
    ) {
        if *roots_dirty {
            self.update_current_gc_roots(registers, varargs, pending_return_prefixes);
            *roots_dirty = false;
        }
    }

    fn push_stackless_self_call_roots(
        &mut self,
        registers: &[Value],
        varargs: &[Value],
        pending_return_prefixes: &[Value],
        continuation: &BytecodeContinuation,
    ) -> usize {
        let Some(frame) = self.call_stack.last_mut() else {
            return 0;
        };
        let mark = frame.stackless_roots.len();
        extend_weak_collectable_roots(&mut frame.stackless_roots, registers);
        extend_weak_collectable_roots(&mut frame.stackless_roots, varargs);
        extend_weak_collectable_roots(&mut frame.stackless_roots, pending_return_prefixes);
        extend_bytecode_continuation_weak_roots(&mut frame.stackless_roots, continuation);
        mark
    }

    fn push_bytecode_continuation_roots(&mut self, continuation: &BytecodeContinuation) -> usize {
        let Some(frame) = self.call_stack.last_mut() else {
            return 0;
        };
        let mark = frame.stackless_roots.len();
        extend_bytecode_continuation_weak_roots(&mut frame.stackless_roots, continuation);
        mark
    }

    fn push_stackless_self_call_debug_frame(
        &mut self,
        debug_frame: &StacklessDebugFrame,
    ) -> StacklessDebugMark {
        let Some(frame) = self.call_stack.last_mut() else {
            return StacklessDebugMark::default();
        };
        let mark = stackless_debug_frame_mark(frame);
        push_stackless_debug_frame_run(frame, debug_frame);
        mark
    }

    fn rebuild_stackless_self_call_roots(&mut self, frames: &mut [StacklessSelfCallFrame]) {
        let Some(call_frame) = self.call_stack.last_mut() else {
            return;
        };
        call_frame.stackless_roots.clear();
        call_frame.stackless_debug_frames.clear();
        for stackless_frame in frames {
            stackless_frame.root_mark = call_frame.stackless_roots.len();
            extend_stackless_self_call_frame_weak_roots(
                &mut call_frame.stackless_roots,
                stackless_frame,
            );
            stackless_frame.debug_mark = stackless_debug_frame_mark(call_frame);
            push_stackless_debug_frame_run(call_frame, &stackless_frame.debug_frame);
        }
    }

    fn truncate_stackless_self_call_roots(&mut self, mark: usize) {
        let Some(frame) = self.call_stack.last_mut() else {
            return;
        };
        frame.stackless_roots.truncate(mark);
    }

    fn truncate_stackless_self_call_debug_frames(&mut self, mark: StacklessDebugMark) {
        let Some(frame) = self.call_stack.last_mut() else {
            return;
        };
        frame.stackless_debug_frames.truncate(mark.run_count);
        if mark.last_run_count > 0 {
            if let Some(run) = frame.stackless_debug_frames.last_mut() {
                run.count = mark.last_run_count;
            }
        }
    }

    fn native_collectgarbage(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 2, "collectgarbage argument")?;
        let mode = match args.first().copied() {
            None => "collect".to_owned(),
            Some(value) if value.kind() == ValueKind::Nil => "collect".to_owned(),
            Some(value) => self
                .string_value(value)?
                .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "string" }))?,
        };
        match mode.as_str() {
            "collect" => {
                let roots = self.collectgarbage_roots(args);
                self.heap.finish_collection_cycle();
                self.heap.collect_weak_tables_with_roots(&roots);
                self.gc_step_counter = 0;
                self.gc_stopped = false;
                Ok(Value::number(0.0))
            }
            "count" => Ok(Value::number(0.0)),
            "stop" => {
                self.gcinfo_counter = 0;
                self.gc_step_counter = 0;
                self.gc_stopped = true;
                Ok(Value::number(0.0))
            }
            "restart" => {
                self.gcinfo_counter = 1;
                self.gc_step_counter = 0;
                self.gc_stopped = false;
                Ok(Value::number(0.0))
            }
            "step" => {
                let size = match args.get(1).copied() {
                    Some(value) if value.kind() != ValueKind::Nil => {
                        self.coerce_to_number(value)?.unwrap_or(0.0)
                    }
                    _ => 0.0,
                };
                self.gc_step_counter = self.gc_step_counter.saturating_add(1);
                let threshold = if size >= 10_000.0 {
                    1
                } else if size <= 0.0 {
                    11
                } else {
                    collectgarbage_step_threshold(size)
                };
                if self.gc_step_counter >= threshold {
                    let roots = self.collectgarbage_roots(args);
                    self.heap.collect_weak_tables_with_roots(&roots);
                    self.gc_step_counter = 0;
                    self.gcinfo_counter = 1;
                    self.gc_stopped = false;
                    Ok(Value::boolean(true))
                } else {
                    Ok(Value::boolean(false))
                }
            }
            _ => Err(Error::new(ErrorKind::RuntimeError {
                message: "invalid option".to_owned(),
            })),
        }
    }

    fn native_gcinfo(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 0, "gcinfo arguments")?;
        self.gcinfo_counter = self.gcinfo_counter.wrapping_add(1);
        if self.gc_stopped {
            return Ok(Value::number((self.gcinfo_counter as f64) * 600.0));
        }
        Ok(Value::number(if self.gcinfo_counter % 2 == 1 {
            1.0
        } else {
            0.0
        }))
    }

    fn native_os_clock(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, u8::MAX as usize, "os.clock arguments")?;
        Ok(Value::number(0.0))
    }

    fn native_os_date(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 2, "os.date arguments")?;
        let mut format = match args.first().copied() {
            None => b"%c".to_vec(),
            Some(value) if value.kind() == ValueKind::Nil => b"%c".to_vec(),
            Some(value) => self
                .string_value_bytes(value)?
                .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "string" }))?,
        };
        if format.first() == Some(&b'!') {
            format.remove(0);
        }
        let timestamp = match args.get(1).copied() {
            None => OS_TIME_NOW_SECONDS,
            Some(value) if value.kind() == ValueKind::Nil => OS_TIME_NOW_SECONDS,
            Some(value) => self.os_time_number_arg(value)?,
        };
        if timestamp < 0 {
            return Ok(Value::nil());
        }
        let parts = os_date_parts(timestamp);
        if format.as_slice() == b"*t" {
            return Ok(Value::table(self.os_date_table(parts)));
        }
        let text = format_os_date(&format, parts)?;
        let atom = self.heap.intern_string_bytes(&text);
        Ok(Value::string_atom(atom))
    }

    fn native_os_time(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 1, "os.time arguments")?;
        let Some(value) = args.first().copied() else {
            return Ok(Value::number(OS_TIME_NOW_SECONDS as f64));
        };
        if value.kind() == ValueKind::Nil {
            return Ok(Value::number(OS_TIME_NOW_SECONDS as f64));
        }
        let table = value
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let timestamp = self.os_timestamp_from_table(table)?;
        if timestamp < 0 {
            Ok(Value::nil())
        } else {
            Ok(Value::number(timestamp as f64))
        }
    }

    fn native_os_difftime(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, usize::MAX, "os.difftime arguments")?;
        let end = self
            .coerce_to_number(args[0])?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let start = self
            .coerce_to_number(args[1])?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        Ok(Value::number(end - start))
    }

    fn os_timestamp_from_table(&mut self, table: u32) -> Result<i64> {
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            }));
        }
        let year = self.os_time_table_field(table, "year", None)?;
        let month = self.os_time_table_field(table, "month", None)?;
        let day = self.os_time_table_field(table, "day", None)?;
        let hour = self.os_time_table_field(table, "hour", Some(0))?;
        let minute = self.os_time_table_field(table, "min", Some(0))?;
        let second = self.os_time_table_field(table, "sec", Some(0))?;

        let month0 = month - 1;
        let normalized_year = year + month0.div_euclid(12);
        let normalized_month = month0.rem_euclid(12) + 1;
        let days = days_from_civil(normalized_year, normalized_month, 1) + day - 1;
        let timestamp = i128::from(days)
            .saturating_mul(86_400)
            .saturating_add(i128::from(hour).saturating_mul(3_600))
            .saturating_add(i128::from(minute).saturating_mul(60))
            .saturating_add(i128::from(second));
        if timestamp < i128::from(i64::MIN) || timestamp > i128::from(i64::MAX) {
            return Ok(-1);
        }
        Ok(timestamp as i64)
    }

    fn os_time_table_field(
        &mut self,
        table: u32,
        name: &'static str,
        default: Option<i64>,
    ) -> Result<i64> {
        let key = Value::string_atom(self.heap.intern_string(name));
        let value = self.raw_table_get(table, key)?;
        if value.kind() == ValueKind::Nil {
            return default.ok_or_else(|| {
                Error::new(ErrorKind::RuntimeError {
                    message: "missing date field".to_owned(),
                })
            });
        }
        Ok(i64::from(self.coerce_to_i32(value, "number")?))
    }

    fn os_time_number_arg(&mut self, value: Value) -> Result<i64> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number.trunc();
        if !integer.is_finite() || integer < i64::MIN as f64 || integer > i64::MAX as f64 {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        }
        Ok(integer as i64)
    }

    fn os_date_table(&mut self, parts: OsDateParts) -> u32 {
        let table = self.heap.alloc_array_table(&[]);
        set_table_field(
            &mut self.heap,
            table,
            "year",
            Value::number(parts.year as f64),
        );
        set_table_field(
            &mut self.heap,
            table,
            "month",
            Value::number(parts.month as f64),
        );
        set_table_field(
            &mut self.heap,
            table,
            "day",
            Value::number(parts.day as f64),
        );
        set_table_field(
            &mut self.heap,
            table,
            "hour",
            Value::number(parts.hour as f64),
        );
        set_table_field(
            &mut self.heap,
            table,
            "min",
            Value::number(parts.minute as f64),
        );
        set_table_field(
            &mut self.heap,
            table,
            "sec",
            Value::number(parts.second as f64),
        );
        set_table_field(
            &mut self.heap,
            table,
            "wday",
            Value::number(parts.wday as f64 + 1.0),
        );
        set_table_field(
            &mut self.heap,
            table,
            "yday",
            Value::number(parts.yday as f64),
        );
        table
    }

    fn native_buffer_create(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "buffer.create arguments")?;
        let size =
            self.buffer_nonnegative_arg(args[0], "invalid argument #1 to 'create' (size)")?;
        let handle = self.heap.alloc_buffer(vec![0; size]).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle overflow",
            })
        })?;
        Ok(Value::buffer(handle))
    }

    fn native_buffer_len(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "buffer.len arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let len = self.heap.buffer_len(buffer).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            })
        })?;
        Ok(Value::number(len as f64))
    }

    fn native_buffer_fromstring(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "buffer.fromstring arguments")?;
        let bytes = self.value_to_string(args[0])?;
        let handle = self.heap.alloc_buffer(bytes).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle overflow",
            })
        })?;
        Ok(Value::buffer(handle))
    }

    fn native_buffer_tostring(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "buffer.tostring arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let len = self.heap.buffer_len(buffer).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            })
        })?;
        let bytes = self.buffer_read_checked(buffer, 0, len)?;
        let atom = self.heap.intern_string_bytes(&bytes);
        Ok(Value::string_atom(atom))
    }

    fn native_buffer_read_int(&mut self, args: &[Value], kind: BufferReadKind) -> Result<Value> {
        validate_native_arity(args, 2, 2, "buffer read arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let offset = self.buffer_offset_arg(args[1])?;
        let bytes = self.buffer_read_checked(buffer, offset, buffer_read_size(kind))?;
        let value = match kind {
            BufferReadKind::I8 => i8::from_le_bytes([bytes[0]]) as f64,
            BufferReadKind::U8 => bytes[0] as f64,
            BufferReadKind::I16 => i16::from_le_bytes([bytes[0], bytes[1]]) as f64,
            BufferReadKind::U16 => u16::from_le_bytes([bytes[0], bytes[1]]) as f64,
            BufferReadKind::I32 => {
                i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64
            }
            BufferReadKind::U32 => {
                u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64
            }
        };
        Ok(Value::number(value))
    }

    fn native_buffer_read_float(&mut self, args: &[Value], kind: BufferFloatKind) -> Result<Value> {
        validate_native_arity(args, 2, 2, "buffer read arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let offset = self.buffer_offset_arg(args[1])?;
        let bytes = self.buffer_read_checked(buffer, offset, buffer_float_size(kind))?;
        let value = match kind {
            BufferFloatKind::F32 => {
                f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as f64
            }
            BufferFloatKind::F64 => f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]),
        };
        Ok(Value::number(value))
    }

    fn native_buffer_readstring(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 3, 3, "buffer.readstring arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let count =
            self.buffer_nonnegative_arg(args[2], "invalid argument #3 to 'readstring' (size)")?;
        let offset = self.buffer_offset_arg(args[1])?;
        let bytes = self.buffer_read_checked(buffer, offset, count)?;
        let atom = self.heap.intern_string_bytes(&bytes);
        Ok(Value::string_atom(atom))
    }

    fn native_buffer_readbits(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 3, 3, "buffer.readbits arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let bit_offset = self.buffer_bit_offset_arg(args[1])?;
        let bit_count = self.buffer_bit_count_arg(args[2])?;
        self.check_buffer_bit_range(buffer, bit_offset, bit_count)?;
        if bit_count == 0 {
            return Ok(Value::number(0.0));
        }
        let byte_offset = bit_offset / 8;
        let byte_count = (bit_offset % 8 + bit_count).div_ceil(8);
        let bytes = self.buffer_read_checked(buffer, byte_offset, byte_count)?;
        let mut result = 0u32;
        for bit in 0..bit_count {
            let absolute = bit_offset % 8 + bit;
            let byte = bytes[absolute / 8];
            if byte & (1 << (absolute % 8)) != 0 {
                result |= 1 << bit;
            }
        }
        Ok(Value::number(result as f64))
    }

    fn native_buffer_no_result(&mut self, native: NativeFunction, args: &[Value]) -> Result<()> {
        match native {
            NativeFunction::BufferWriteI8 => {
                self.native_buffer_write_int(args, BufferWriteKind::I8)
            }
            NativeFunction::BufferWriteU8 => {
                self.native_buffer_write_int(args, BufferWriteKind::U8)
            }
            NativeFunction::BufferWriteI16 => {
                self.native_buffer_write_int(args, BufferWriteKind::I16)
            }
            NativeFunction::BufferWriteU16 => {
                self.native_buffer_write_int(args, BufferWriteKind::U16)
            }
            NativeFunction::BufferWriteI32 => {
                self.native_buffer_write_int(args, BufferWriteKind::I32)
            }
            NativeFunction::BufferWriteU32 => {
                self.native_buffer_write_int(args, BufferWriteKind::U32)
            }
            NativeFunction::BufferWriteF32 => {
                self.native_buffer_write_float(args, BufferFloatKind::F32)
            }
            NativeFunction::BufferWriteF64 => {
                self.native_buffer_write_float(args, BufferFloatKind::F64)
            }
            NativeFunction::BufferWriteString => self.native_buffer_writestring(args),
            NativeFunction::BufferCopy => self.native_buffer_copy(args),
            NativeFunction::BufferFill => self.native_buffer_fill(args),
            NativeFunction::BufferWriteBits => self.native_buffer_writebits(args),
            _ => unreachable!("only empty-result buffer natives are dispatched here"),
        }
    }

    fn native_buffer_write_int(&mut self, args: &[Value], kind: BufferWriteKind) -> Result<()> {
        validate_native_arity(args, 3, 3, "buffer write arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let offset = self.buffer_offset_arg(args[1])?;
        let value = self.buffer_integer_arg(args[2])?;
        let bytes = match kind {
            BufferWriteKind::I8 | BufferWriteKind::U8 => vec![value as u8],
            BufferWriteKind::I16 | BufferWriteKind::U16 => (value as u16).to_le_bytes().to_vec(),
            BufferWriteKind::I32 | BufferWriteKind::U32 => (value as u32).to_le_bytes().to_vec(),
        };
        self.buffer_write_checked(buffer, offset, &bytes)
    }

    fn native_buffer_write_float(&mut self, args: &[Value], kind: BufferFloatKind) -> Result<()> {
        validate_native_arity(args, 3, 3, "buffer write arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let offset = self.buffer_offset_arg(args[1])?;
        let number = self
            .coerce_to_number(args[2])?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let bytes = match kind {
            BufferFloatKind::F32 => (number as f32).to_le_bytes().to_vec(),
            BufferFloatKind::F64 => number.to_le_bytes().to_vec(),
        };
        self.buffer_write_checked(buffer, offset, &bytes)
    }

    fn native_buffer_writestring(&mut self, args: &[Value]) -> Result<()> {
        validate_native_arity(args, 3, 4, "buffer.writestring arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let offset = self.buffer_offset_arg(args[1])?;
        let text = self.value_to_string(args[2])?;
        let count = match args.get(3).copied() {
            None => text.len(),
            Some(value) if value.kind() == ValueKind::Nil => text.len(),
            Some(value) => {
                let count = self.buffer_nonnegative_arg(
                    value,
                    "invalid argument #4 to 'writestring' (count)",
                )?;
                if count > text.len() {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "string length overflow".to_owned(),
                    }));
                }
                count
            }
        };
        self.buffer_write_checked(buffer, offset, &text[..count])
    }

    fn native_buffer_copy(&mut self, args: &[Value]) -> Result<()> {
        validate_native_arity(args, 3, 5, "buffer.copy arguments")?;
        let dst = self.buffer_arg(args[0])?;
        let dst_offset = self.buffer_offset_arg(args[1])?;
        let src = self.buffer_arg(args[2])?;
        let src_offset = match args.get(3).copied() {
            None => 0,
            Some(value) if value.kind() == ValueKind::Nil => 0,
            Some(value) => self.buffer_offset_arg(value)?,
        };
        let src_len = self.heap.buffer_len(src).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            })
        })?;
        let count = match args.get(4).copied() {
            None => src_len
                .checked_sub(src_offset)
                .ok_or_else(buffer_out_of_bounds)?,
            Some(value) if value.kind() == ValueKind::Nil => src_len
                .checked_sub(src_offset)
                .ok_or_else(buffer_out_of_bounds)?,
            Some(value) => self.buffer_offset_arg(value)?,
        };
        let bytes = self.buffer_read_checked(src, src_offset, count)?;
        self.buffer_write_checked(dst, dst_offset, &bytes)
    }

    fn native_buffer_fill(&mut self, args: &[Value]) -> Result<()> {
        validate_native_arity(args, 3, 4, "buffer.fill arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let offset = self.buffer_offset_arg(args[1])?;
        let byte = self.buffer_integer_arg(args[2])? as u8;
        let len = self.heap.buffer_len(buffer).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            })
        })?;
        let count = match args.get(3).copied() {
            None => len.checked_sub(offset).ok_or_else(buffer_out_of_bounds)?,
            Some(value) if value.kind() == ValueKind::Nil => {
                len.checked_sub(offset).ok_or_else(buffer_out_of_bounds)?
            }
            Some(value) => self.buffer_offset_arg(value)?,
        };
        self.check_buffer_range(buffer, offset, count)?;
        self.heap
            .buffer_fill(buffer, offset, count, byte)
            .ok_or_else(buffer_out_of_bounds)
    }

    fn native_buffer_writebits(&mut self, args: &[Value]) -> Result<()> {
        validate_native_arity(args, 4, 4, "buffer.writebits arguments")?;
        let buffer = self.buffer_arg(args[0])?;
        let bit_offset = self.buffer_bit_offset_arg(args[1])?;
        let bit_count = self.buffer_bit_count_arg(args[2])?;
        let value = self.buffer_integer_arg(args[3])? as u32;
        self.check_buffer_bit_range(buffer, bit_offset, bit_count)?;
        if bit_count == 0 {
            return Ok(());
        }
        let byte_offset = bit_offset / 8;
        let byte_count = (bit_offset % 8 + bit_count).div_ceil(8);
        let mut bytes = self.buffer_read_checked(buffer, byte_offset, byte_count)?;
        for bit in 0..bit_count {
            let absolute = bit_offset % 8 + bit;
            let mask = 1u8 << (absolute % 8);
            let byte = &mut bytes[absolute / 8];
            if value & (1 << bit) != 0 {
                *byte |= mask;
            } else {
                *byte &= !mask;
            }
        }
        self.buffer_write_checked(buffer, byte_offset, &bytes)
    }

    fn buffer_arg(&mut self, value: Value) -> Result<u32> {
        let buffer = value
            .as_buffer()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "buffer" }))?;
        if !self.heap.has_buffer(buffer) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            }));
        }
        Ok(buffer)
    }

    fn buffer_nonnegative_arg(&mut self, value: Value, message: &'static str) -> Result<usize> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number.trunc();
        if !integer.is_finite() || integer < 0.0 || integer > usize::MAX as f64 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: message.to_owned(),
            }));
        }
        Ok(integer as usize)
    }

    fn buffer_offset_arg(&mut self, value: Value) -> Result<usize> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number.trunc();
        if !integer.is_finite() || integer < 0.0 || integer > usize::MAX as f64 {
            return Err(buffer_out_of_bounds());
        }
        Ok(integer as usize)
    }

    fn buffer_bit_offset_arg(&mut self, value: Value) -> Result<usize> {
        self.buffer_offset_arg(value)
    }

    fn buffer_bit_count_arg(&mut self, value: Value) -> Result<usize> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number.trunc();
        if !integer.is_finite() || !(0.0..=32.0).contains(&integer) {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "bit count is out of range of [0; 32]".to_owned(),
            }));
        }
        Ok(integer as usize)
    }

    fn buffer_integer_arg(&mut self, value: Value) -> Result<i64> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number.trunc();
        if !integer.is_finite() {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        }
        Ok(integer as i64)
    }

    fn buffer_read_checked(&mut self, buffer: u32, offset: usize, count: usize) -> Result<Vec<u8>> {
        self.check_buffer_range(buffer, offset, count)?;
        self.heap
            .buffer_read(buffer, offset, count)
            .ok_or_else(buffer_out_of_bounds)
    }

    fn buffer_write_checked(&mut self, buffer: u32, offset: usize, bytes: &[u8]) -> Result<()> {
        self.check_buffer_range(buffer, offset, bytes.len())?;
        self.heap
            .buffer_write(buffer, offset, bytes)
            .ok_or_else(buffer_out_of_bounds)
    }

    fn check_buffer_range(&mut self, buffer: u32, offset: usize, count: usize) -> Result<()> {
        let len = self.heap.buffer_len(buffer).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            })
        })?;
        let end = offset.checked_add(count).ok_or_else(buffer_out_of_bounds)?;
        if end > len {
            Err(buffer_out_of_bounds())
        } else {
            Ok(())
        }
    }

    fn check_buffer_bit_range(
        &mut self,
        buffer: u32,
        bit_offset: usize,
        bit_count: usize,
    ) -> Result<()> {
        let len = self.heap.buffer_len(buffer).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "buffer handle is missing",
            })
        })?;
        let bit_len = len.checked_mul(8).ok_or_else(buffer_out_of_bounds)?;
        let end = bit_offset
            .checked_add(bit_count)
            .ok_or_else(buffer_out_of_bounds)?;
        if end > bit_len {
            Err(buffer_out_of_bounds())
        } else {
            Ok(())
        }
    }

    fn native_rawequal(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "rawequal arguments")?;
        Ok(Value::boolean(self.raw_equal_values(args[0], args[1])?))
    }

    fn native_rawget(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "rawget arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        self.raw_table_get(table, args[1])
    }

    fn native_rawset(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 3, u8::MAX as usize, "rawset arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        self.set_table_index(table, args[1], args[2])?;
        Ok(args[0])
    }

    fn native_rawlen(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "rawlen argument")?;
        Ok(Value::number(self.raw_len_value(args[0])? as f64))
    }

    fn native_conformance_cxxthrow(&mut self, _args: &[Value]) -> Result<Value> {
        Err(Error::new(ErrorKind::RuntimeError {
            message: "oops".to_owned(),
        }))
    }

    fn native_conformance_is_native(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, 0, "is_native arguments")?;
        Ok(Value::boolean(true))
    }

    fn native_conformance_int64_new(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "int64 argument")?;
        let value = self.conformance_int64_value(args[0])?;
        self.alloc_conformance_int64(value)
    }

    fn native_conformance_int64_eq(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "int64 __eq arguments")?;
        Ok(Value::boolean(
            self.conformance_int64_value(args[0])? == self.conformance_int64_value(args[1])?,
        ))
    }

    fn native_conformance_int64_lt(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "int64 __lt arguments")?;
        Ok(Value::boolean(
            self.conformance_int64_value(args[0])? < self.conformance_int64_value(args[1])?,
        ))
    }

    fn native_conformance_int64_le(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "int64 __le arguments")?;
        Ok(Value::boolean(
            self.conformance_int64_value(args[0])? <= self.conformance_int64_value(args[1])?,
        ))
    }

    fn native_conformance_int64_neg(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "int64 __unm argument")?;
        let value = self.conformance_int64_value(args[0])?;
        self.alloc_conformance_int64(value.wrapping_neg())
    }

    fn native_conformance_int64_binary(
        &mut self,
        args: &[Value],
        op: ConformanceInt64Binary,
    ) -> Result<Value> {
        validate_native_arity(args, 2, 2, "int64 binary arguments")?;
        let lhs = self.conformance_int64_value(args[0])?;
        let rhs = self.conformance_int64_value(args[1])?;
        let value = match op {
            ConformanceInt64Binary::Add => lhs.wrapping_add(rhs),
            ConformanceInt64Binary::Sub => lhs.wrapping_sub(rhs),
            ConformanceInt64Binary::Mul => lhs.wrapping_mul(rhs),
            ConformanceInt64Binary::Div => lhs.checked_div(rhs).ok_or_else(divide_by_zero_error)?,
            ConformanceInt64Binary::FloorDiv => conformance_int64_floor_div(lhs, rhs)?,
            ConformanceInt64Binary::Mod => conformance_int64_mod(lhs, rhs)?,
            ConformanceInt64Binary::Pow => conformance_int64_pow(lhs, rhs)?,
        };
        self.alloc_conformance_int64(value)
    }

    fn native_conformance_int64_tostring(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "int64 __tostring argument")?;
        let value = self.conformance_int64_value(args[0])?;
        let atom = self.heap.intern_string(&value.to_string());
        Ok(Value::string_atom(atom))
    }

    fn native_conformance_int64_index(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "int64 __index arguments")?;
        if self.string_value(args[1])?.as_deref() == Some("value") {
            return Ok(Value::number(self.conformance_int64_value(args[0])? as f64));
        }
        Ok(Value::nil())
    }

    fn native_conformance_int64_newindex(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 3, 3, "int64 __newindex arguments")?;
        if self.string_value(args[1])?.as_deref() != Some("value") {
            return Ok(Value::nil());
        }
        let handle = args[0].as_userdata().ok_or_else(|| {
            Error::new(ErrorKind::TypeError {
                expected: "int64 userdata",
            })
        })?;
        let value = self.conformance_int64_value(args[2])?;
        self.heap
            .set_userdata_payload(handle, UserdataPayload::ConformanceInt64(value))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "int64 userdata handle is missing",
                })
            })?;
        Ok(Value::nil())
    }

    fn conformance_int64_value(&mut self, value: Value) -> Result<i64> {
        if let Some(handle) = value.as_userdata() {
            return match self.heap.userdata_payload(handle).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "userdata handle is missing",
                })
            })? {
                UserdataPayload::ConformanceInt64(value) => Ok(value),
                _ => Err(Error::new(ErrorKind::TypeError {
                    expected: "int64 userdata",
                })),
            };
        }
        let Some(number) = value.as_number() else {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "int64-compatible number",
            }));
        };
        conformance_int64_from_number(number)
    }

    fn alloc_conformance_int64(&mut self, value: i64) -> Result<Value> {
        let metatable = self.conformance_int64_metatable.ok_or_else(|| {
            Error::new(ErrorKind::Unsupported {
                feature: "conformance int64 userdata",
            })
        })?;
        let handle = self
            .heap
            .alloc_userdata_with_payload(Some(metatable), UserdataPayload::ConformanceInt64(value))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "int64 userdata handle overflow",
                })
            })?;
        Ok(Value::userdata(handle))
    }

    fn native_conformance_vec2_new(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vec2 arguments")?;
        let x = self.conformance_number_value(args[0], "vec2 x")?;
        let y = self.conformance_number_value(args[1], "vec2 y")?;
        self.alloc_conformance_vec2(x, y)
    }

    fn native_conformance_vec2_neg(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "vec2 __unm argument")?;
        let (x, y) = self.conformance_vec2_value(args[0])?;
        self.alloc_conformance_vec2(-x, -y)
    }

    fn native_conformance_vec2_add(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vec2 __add arguments")?;
        let (lhs_x, lhs_y) = self.conformance_vec2_value(args[0])?;
        let (rhs_x, rhs_y) = self.conformance_vec2_value(args[1])?;
        self.alloc_conformance_vec2(lhs_x + rhs_x, lhs_y + rhs_y)
    }

    fn native_conformance_vec2_mul(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vec2 __mul arguments")?;
        let (lhs_x, lhs_y) = self.conformance_vec2_value(args[0])?;
        let (rhs_x, rhs_y) = self.conformance_vec2_value(args[1])?;
        self.alloc_conformance_vec2(lhs_x * rhs_x, lhs_y * rhs_y)
    }

    fn native_conformance_vec2_index(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vec2 __index arguments")?;
        let Some(key) = self.string_value(args[1])? else {
            return Ok(Value::nil());
        };
        let (x, y) = self.conformance_vec2_value(args[0])?;
        match key.as_str() {
            "X" | "x" => Ok(Value::number(x)),
            "Y" | "y" => Ok(Value::number(y)),
            "Magnitude" => Ok(Value::number(libm::sqrt(x * x + y * y))),
            "Unit" => {
                let magnitude = libm::sqrt(x * x + y * y);
                if magnitude == 0.0 {
                    self.alloc_conformance_vec2(0.0, 0.0)
                } else {
                    self.alloc_conformance_vec2(x / magnitude, y / magnitude)
                }
            }
            "Dot" => Ok(native_function_value(
                &mut self.heap,
                NativeFunction::ConformanceVec2Dot,
                self.conformance_vec2_metatable.unwrap_or(self.globals),
            )),
            "Min" => Ok(native_function_value(
                &mut self.heap,
                NativeFunction::ConformanceVec2Min,
                self.conformance_vec2_metatable.unwrap_or(self.globals),
            )),
            _ => Ok(Value::nil()),
        }
    }

    fn native_conformance_vec2_dot(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vec2 Dot arguments")?;
        let (lhs_x, lhs_y) = self.conformance_vec2_value(args[0])?;
        let (rhs_x, rhs_y) = self.conformance_vec2_value(args[1])?;
        Ok(Value::number(lhs_x * rhs_x + lhs_y * rhs_y))
    }

    fn native_conformance_vec2_min(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vec2 Min arguments")?;
        let (lhs_x, lhs_y) = self.conformance_vec2_value(args[0])?;
        let (rhs_x, rhs_y) = self.conformance_vec2_value(args[1])?;
        self.alloc_conformance_vec2(lhs_x.min(rhs_x), lhs_y.min(rhs_y))
    }

    fn native_conformance_vertex_new(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 3, 3, "vertex arguments")?;
        self.alloc_conformance_vertex(args[0], args[1], args[2])
    }

    fn native_conformance_vertex_index(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 2, "vertex __index arguments")?;
        let Some(key) = self.string_value(args[1])? else {
            return Ok(Value::nil());
        };
        let (pos, normal, uv) = self.conformance_vertex_value(args[0])?;
        match key.as_str() {
            "pos" => Ok(pos),
            "normal" => Ok(normal),
            "uv" => Ok(uv),
            _ => Ok(Value::nil()),
        }
    }

    fn conformance_number_value(&self, value: Value, expected: &'static str) -> Result<f64> {
        value
            .as_number()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected }))
    }

    fn conformance_vec2_value(&mut self, value: Value) -> Result<(f64, f64)> {
        let Some(handle) = value.as_userdata() else {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "vec2 userdata",
            }));
        };
        match self.heap.userdata_payload(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "userdata handle is missing",
            })
        })? {
            UserdataPayload::ConformanceVec2 { x, y } => Ok((x, y)),
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "vec2 userdata",
            })),
        }
    }

    fn conformance_vertex_value(&mut self, value: Value) -> Result<(Value, Value, Value)> {
        let Some(handle) = value.as_userdata() else {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "vertex userdata",
            }));
        };
        match self.heap.userdata_payload(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "userdata handle is missing",
            })
        })? {
            UserdataPayload::ConformanceVertex { pos, normal, uv } => Ok((pos, normal, uv)),
            _ => Err(Error::new(ErrorKind::TypeError {
                expected: "vertex userdata",
            })),
        }
    }

    fn alloc_conformance_vec2(&mut self, x: f64, y: f64) -> Result<Value> {
        let metatable = self.conformance_vec2_metatable.ok_or_else(|| {
            Error::new(ErrorKind::Unsupported {
                feature: "conformance vec2 userdata",
            })
        })?;
        let handle = self
            .heap
            .alloc_userdata_with_payload(Some(metatable), UserdataPayload::ConformanceVec2 { x, y })
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "vec2 userdata handle overflow",
                })
            })?;
        Ok(Value::userdata(handle))
    }

    fn alloc_conformance_vertex(&mut self, pos: Value, normal: Value, uv: Value) -> Result<Value> {
        let metatable = self.conformance_vertex_metatable.ok_or_else(|| {
            Error::new(ErrorKind::Unsupported {
                feature: "conformance vertex userdata",
            })
        })?;
        let handle = self
            .heap
            .alloc_userdata_with_payload(
                Some(metatable),
                UserdataPayload::ConformanceVertex { pos, normal, uv },
            )
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "vertex userdata handle overflow",
                })
            })?;
        Ok(Value::userdata(handle))
    }

    fn native_getcoverage(&mut self, program: &Program, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "getcoverage argument")?;
        let handle = args[0].as_function().ok_or_else(|| {
            Error::new(ErrorKind::TypeError {
                expected: "function",
            })
        })?;
        let kind = self.heap.function_kind(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle is missing",
            })
        })?;
        let mut entries = Vec::new();
        match kind {
            FunctionKind::Bytecode {
                program: None,
                prototype,
            } => {
                self.collect_coverage_entries(program, None, prototype, 0, &mut entries)?;
            }
            FunctionKind::Bytecode {
                program: Some(dynamic_program_id),
                prototype,
            } => {
                let dynamic_program = self.dynamic_program(dynamic_program_id)?;
                self.collect_coverage_entries(
                    &dynamic_program,
                    Some(dynamic_program_id),
                    prototype,
                    0,
                    &mut entries,
                )?;
            }
            FunctionKind::DynamicMain { .. } | FunctionKind::Host(_) | FunctionKind::Native(_) => {}
        }

        let table = self.heap.alloc_array_table(&entries);
        Ok(Value::table(table))
    }

    fn collect_coverage_entries(
        &mut self,
        program: &Program,
        program_id: Option<u32>,
        prototype: u32,
        depth: usize,
        entries: &mut Vec<Value>,
    ) -> Result<()> {
        let function = program.functions.get(prototype as usize).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function prototype is missing",
            })
        })?;
        entries.push(self.coverage_entry_table(program_id, prototype, function, depth));

        let mut children = Vec::new();
        for instruction in &function.code {
            if instruction.op == Opcode::LoadFunction && instruction.aux >= 0 {
                children.push(instruction.aux as u32);
            }
        }
        for child in children {
            self.collect_coverage_entries(program, program_id, child, depth + 1, entries)?;
        }
        Ok(())
    }

    fn coverage_entry_table(
        &mut self,
        program_id: Option<u32>,
        prototype: u32,
        function: &Function,
        depth: usize,
    ) -> Value {
        let table = self.heap.alloc_array_table(&[]);
        if let Some(name) = &function.debug_name {
            let key = Value::string_atom(self.heap.intern_string("name"));
            let value = Value::string_atom(self.heap.intern_string(name));
            self.heap
                .table_set(table, key, value)
                .expect("coverage entry table and string key are valid");
        }
        let linedefined = function.debug_line_defined.unwrap_or(0);
        set_table_field(
            &mut self.heap,
            table,
            "linedefined",
            Value::number(f64::from(linedefined)),
        );
        set_table_field(&mut self.heap, table, "depth", Value::number(depth as f64));

        for line in function.coverage_lines() {
            let key = Value::number(f64::from(line));
            let value = Value::number(
                self.coverage_hits
                    .get(&CoverageKey {
                        program: program_id,
                        prototype,
                        line,
                    })
                    .copied()
                    .unwrap_or(0) as f64,
            );
            self.heap
                .table_set(table, key, value)
                .expect("coverage entry numeric line keys are valid");
        }
        Value::table(table)
    }

    fn validate_table_key(&mut self, key: Value) -> Result<()> {
        if key.kind() == ValueKind::Vector {
            let vector = self.vector_data(key)?;
            if vector.x().is_nan() || vector.y().is_nan() || vector.z().is_nan() {
                return Err(Error::new(ErrorKind::TableIndexNaN));
            }
        }
        validate_table_key(key)
    }

    fn table_next_after(&mut self, table: u32, after: Value) -> Option<TableNext> {
        let order = if self.profile == Profile::LuauCompat {
            TableIterationOrder::LuauHash
        } else {
            TableIterationOrder::Insertion
        };
        self.heap.table_next_after(table, after, order)
    }

    fn native_ipairs(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "ipairs arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "ipairs table handle is missing",
            }));
        }
        Ok(vec![
            native_function_value(&mut self.heap, NativeFunction::INext, self.globals),
            args[0],
            Value::number(0.0),
        ])
    }

    fn native_inext(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, u8::MAX as usize, "inext arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "inext table handle is missing",
            }));
        }
        let index = number_arg(args[1])?;
        let next_index = index + 1.0;
        let Some(one_based_index) = array_iterator_index(next_index) else {
            return Ok(vec![Value::nil()]);
        };
        let value = self
            .heap
            .array_get(table, one_based_index)
            .unwrap_or_else(Value::nil);
        if value.kind() == ValueKind::Nil {
            return Ok(vec![Value::nil()]);
        }
        Ok(vec![Value::number(next_index), value])
    }

    fn native_pairs(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "pairs arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        if !self.heap.has_table(table) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "pairs table handle is missing",
            }));
        }
        Ok(vec![
            native_function_value(&mut self.heap, NativeFunction::Next, self.globals),
            args[0],
            Value::nil(),
        ])
    }

    fn native_next(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, 2, "next arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let after = args.get(1).copied().unwrap_or_else(Value::nil);
        match self.table_next_after(table, after).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "next table handle is missing",
            })
        })? {
            TableNext::Entry(key, value) => Ok(vec![key, value]),
            TableNext::End => Ok(vec![Value::nil()]),
            TableNext::InvalidKey => Err(Error::new(ErrorKind::InvalidKeyToNext)),
        }
    }

    fn native_math_abs(&mut self, args: &[Value]) -> Result<Value> {
        if args.is_empty() {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "missing argument #1 to 'abs' (number expected)".to_owned(),
            }));
        }
        let value = self.coerce_to_number(args[0])?.ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "invalid argument #1 to 'abs' (number expected, got {})",
                    value_type_name(args[0])
                ),
            })
        })?;
        Ok(Value::number(binary(NumericOp::Abs, value, 0.0)))
    }

    fn native_math_unary(&mut self, args: &[Value], op: MathUnary) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "math argument")?;
        let registers = native_registers(args)?;
        self.math_unary(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 0, 1),
            op,
        )
    }

    fn native_math_binary(&mut self, args: &[Value], op: MathBinary) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "math arguments")?;
        let registers = native_registers(args)?;
        self.math_binary(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 0, 2),
            op,
        )
    }

    fn native_math_log(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "math.log arguments")?;
        let registers = native_registers(args)?;
        let arity = args.len().min(2);
        self.math_log(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 0, arity as i32),
        )
    }

    fn native_math_ldexp(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "math.ldexp arguments")?;
        let registers = native_registers(args)?;
        self.math_ldexp(&registers, Instruction::with_aux(Opcode::Halt, 0, 0, 0, 2))
    }

    fn native_math_frexp(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, u8::MAX as usize, "math.frexp argument")?;
        let value = self
            .coerce_to_number(args[0])?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let (mantissa, exponent) = libm::frexp(value);
        Ok(vec![
            Value::number(mantissa),
            Value::number(f64::from(exponent)),
        ])
    }

    fn native_math_modf(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, u8::MAX as usize, "math.modf argument")?;
        let value = self
            .coerce_to_number(args[0])?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let (fraction, integer) = libm::modf(value);
        Ok(vec![Value::number(integer), Value::number(fraction)])
    }

    fn native_math_random(&mut self, args: &[Value]) -> Result<Value> {
        match args.len() {
            0 => {
                let low = u64::from(self.pcg32_random());
                let high = u64::from(self.pcg32_random());
                let bits = low | (high << 32);
                Ok(Value::number(libm::ldexp(bits as f64, -64)))
            }
            1 => {
                let upper = self.random_i32_arg(args[0])?;
                if upper < 1 {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "interval is empty".to_owned(),
                    }));
                }
                let sample = u64::from(upper as u32) * u64::from(self.pcg32_random());
                Ok(Value::number(f64::from(1 + (sample >> 32) as u32)))
            }
            2 => {
                let lower = self.random_i32_arg(args[0])?;
                let upper = self.random_i32_arg(args[1])?;
                if lower > upper {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "interval is empty".to_owned(),
                    }));
                }
                let span = (upper as u32).wrapping_sub(lower as u32);
                if span == u32::MAX {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "interval is too large".to_owned(),
                    }));
                }
                let sample = u64::from(span + 1) * u64::from(self.pcg32_random());
                let result = lower.wrapping_add((sample >> 32) as i32);
                Ok(Value::number(f64::from(result)))
            }
            _ => Err(Error::new(ErrorKind::RuntimeError {
                message: "wrong number of arguments".to_owned(),
            })),
        }
    }

    fn native_math_randomseed(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, u8::MAX as usize, "math.randomseed argument")?;
        let seed = self.random_i32_arg(args[0])?;
        self.rng_state = pcg32_seed(seed as u64);
        Ok(Vec::new())
    }

    fn random_i32_arg(&mut self, value: Value) -> Result<i32> {
        self.coerce_to_i32(value, "integer")
    }

    fn pcg32_random(&mut self) -> u32 {
        let (state, output) = pcg32_random(self.rng_state);
        self.rng_state = state;
        output
    }

    fn native_math_noise(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "math.noise argument")?;
        let x = self.noise_required_arg(args[0])?;
        let y = self.noise_optional_arg(args.get(1).copied())?;
        let z = self.noise_optional_arg(args.get(2).copied())?;
        Ok(Value::number(f64::from(perlin_noise(x, y, z))))
    }

    fn noise_required_arg(&mut self, value: Value) -> Result<f32> {
        self.coerce_to_number(value)?
            .map(|number| number as f32)
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))
    }

    fn noise_optional_arg(&mut self, value: Option<Value>) -> Result<f32> {
        match value {
            None => Ok(0.0),
            Some(value) if value.kind() == ValueKind::Nil => Ok(0.0),
            Some(value) => self.noise_required_arg(value),
        }
    }

    fn native_math_predicate(&mut self, args: &[Value], op: MathPredicate) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "math predicate argument")?;
        let registers = native_registers(args)?;
        self.math_predicate(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 0, 1),
            op,
        )
    }

    fn native_math_minmax(&mut self, args: &[Value], op: MathMinMax) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "math.min/math.max arguments")?;
        let registers = native_registers(args)?;
        self.math_minmax(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 0, args.len() as i32),
            op,
        )
    }

    fn native_math_window(&mut self, args: &[Value], op: NativeMathWindow) -> Result<Value> {
        let expected = op.arity();
        validate_native_arity(args, expected, u8::MAX as usize, op.name())?;
        let registers = native_registers(args)?;
        let instruction = Instruction::with_aux(Opcode::Halt, 0, 0, 0, expected as i32);
        match op {
            NativeMathWindow::Clamp => self.math_clamp(&registers, instruction),
            NativeMathWindow::Map => self.math_map(&registers, instruction),
            NativeMathWindow::Lerp => self.math_lerp(&registers, instruction),
        }
    }

    fn vector_data(&mut self, value: Value) -> Result<VectorData> {
        let vector = value
            .as_vector()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "vector" }))?;
        self.heap.vector(vector).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "vector handle is missing",
            })
        })
    }

    fn alloc_vector_value(&mut self, x: f32, y: f32, z: f32) -> Result<Value> {
        let handle = self
            .heap
            .alloc_vector(VectorData::new(x, y, z))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "vector handle overflow",
                })
            })?;
        Ok(Value::vector(handle))
    }

    fn vector_binop_value(&mut self, lhs: Value, rhs: Value, op: NumericOp) -> Result<Value> {
        let lhs_vector = if lhs.kind() == ValueKind::Vector {
            Some(self.vector_data(lhs)?)
        } else {
            None
        };
        let rhs_vector = if rhs.kind() == ValueKind::Vector {
            Some(self.vector_data(rhs)?)
        } else {
            None
        };

        match (lhs_vector, rhs_vector) {
            (Some(lhs), Some(rhs)) => self.vector_vector_binop(lhs, rhs, op),
            (Some(lhs), None) if vector_scalar_rhs_op(op) => {
                let rhs = self.vector_scalar_operand(rhs)?;
                self.vector_scalar_binop(lhs, rhs, op, VectorScalarSide::Right)
            }
            (None, Some(rhs)) if vector_scalar_lhs_op(op) => {
                let lhs = self.vector_scalar_operand(lhs)?;
                self.vector_scalar_binop(rhs, lhs, op, VectorScalarSide::Left)
            }
            _ => Err(Error::new(ErrorKind::TypeError { expected: "number" })),
        }
    }

    fn vector_vector_binop(
        &mut self,
        lhs: VectorData,
        rhs: VectorData,
        op: NumericOp,
    ) -> Result<Value> {
        if !vector_vector_op(op) {
            return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
        }
        self.alloc_vector_value(
            vector_component_binop(lhs.x(), rhs.x(), op),
            vector_component_binop(lhs.y(), rhs.y(), op),
            vector_component_binop(lhs.z(), rhs.z(), op),
        )
    }

    fn vector_scalar_binop(
        &mut self,
        vector: VectorData,
        scalar: f32,
        op: NumericOp,
        side: VectorScalarSide,
    ) -> Result<Value> {
        let component = |value| match side {
            VectorScalarSide::Right => vector_component_binop(value, scalar, op),
            VectorScalarSide::Left => vector_component_binop(scalar, value, op),
        };
        self.alloc_vector_value(
            component(vector.x()),
            component(vector.y()),
            component(vector.z()),
        )
    }

    fn vector_scalar_operand(&mut self, value: Value) -> Result<f32> {
        self.coerce_to_number(value)?
            .map(|number| number as f32)
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))
    }

    fn vector_index_value(&mut self, object: Value, key: Value) -> Result<Value> {
        let vector = self.vector_data(object)?;
        let Some(name) = self.string_value(key)? else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: format!("attempt to index vector with {}", value_type_name(key)),
            }));
        };
        match name.as_str() {
            "x" | "X" => Ok(Value::number(f64::from(vector.x()))),
            "y" | "Y" => Ok(Value::number(f64::from(vector.y()))),
            "z" | "Z" => Ok(Value::number(f64::from(vector.z()))),
            "Magnitude" => Ok(Value::number(vector_magnitude(vector))),
            "Unit" => self.alloc_vector_value_from_data(vector_normalize(vector)),
            "Dot" => Ok(native_function_value(
                &mut self.heap,
                NativeFunction::VectorMethodDot,
                self.globals,
            )),
            "Cross" => Ok(native_function_value(
                &mut self.heap,
                NativeFunction::VectorMethodCross,
                self.globals,
            )),
            _ => Err(Error::new(ErrorKind::RuntimeError {
                message: format!("attempt to index vector with '{name}'"),
            })),
        }
    }

    fn alloc_vector_value_from_data(&mut self, vector: VectorData) -> Result<Value> {
        self.alloc_vector_value(vector.x(), vector.y(), vector.z())
    }

    fn native_vector_create(&mut self, args: &[Value]) -> Result<Value> {
        if args.len() < 2 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "missing argument #2 to 'create' (number expected)".to_owned(),
            }));
        }
        let x = self.vector_component_arg(args, 0, "create")?;
        let y = self.vector_component_arg(args, 1, "create")?;
        let z = if args.len() >= 3 {
            self.vector_component_arg(args, 2, "create")?
        } else {
            0.0
        };
        self.alloc_vector_value(x, y, z)
    }

    fn native_vector_call(&mut self, args: &[Value]) -> Result<Value> {
        let Some((callee, args)) = args.split_first() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "missing argument #1 (table expected)".to_owned(),
            }));
        };
        if callee.kind() != ValueKind::Table {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "invalid argument #1 (table expected, got {})",
                    value_type_name(*callee)
                ),
            }));
        }
        self.native_vector_create(args)
    }

    fn vector_component_arg(
        &mut self,
        args: &[Value],
        index: usize,
        function: &'static str,
    ) -> Result<f32> {
        let Some(value) = args.get(index).copied() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "missing argument #{} to '{function}' (number expected)",
                    index + 1
                ),
            }));
        };
        self.coerce_to_number(value)?
            .map(|number| number as f32)
            .ok_or_else(|| {
                Error::new(ErrorKind::RuntimeError {
                    message: format!(
                        "invalid argument #{} to '{function}' (number expected, got {})",
                        index + 1,
                        value_type_name(value)
                    ),
                })
            })
    }

    fn native_vector_dot(&mut self, args: &[Value], style: VectorCallStyle) -> Result<Value> {
        let lhs = self.vector_arg(args, 0, "dot", style)?;
        let rhs = self.vector_arg(args, 1, "dot", style)?;
        Ok(Value::number(vector_dot(lhs, rhs)))
    }

    fn native_vector_cross(&mut self, args: &[Value], style: VectorCallStyle) -> Result<Value> {
        let lhs = self.vector_arg(args, 0, "cross", style)?;
        let rhs = self.vector_arg(args, 1, "cross", style)?;
        self.alloc_vector_value_from_data(vector_cross(lhs, rhs))
    }

    fn native_vector_normalize(&mut self, args: &[Value]) -> Result<Value> {
        let vector = self.vector_arg(args, 0, "normalize", VectorCallStyle::Function)?;
        self.alloc_vector_value_from_data(vector_normalize(vector))
    }

    fn native_vector_magnitude(&mut self, args: &[Value]) -> Result<Value> {
        let vector = self.vector_arg(args, 0, "magnitude", VectorCallStyle::Function)?;
        Ok(Value::number(vector_magnitude(vector)))
    }

    fn native_vector_unary(&mut self, args: &[Value], op: VectorUnary) -> Result<Value> {
        let vector = self.vector_arg(args, 0, op.name(), VectorCallStyle::Function)?;
        let apply = |component: f32| match op {
            VectorUnary::Abs => component.abs(),
            VectorUnary::Floor => libm::floorf(component),
            VectorUnary::Ceil => libm::ceilf(component),
            VectorUnary::Sign => vector_sign(component),
        };
        self.alloc_vector_value(apply(vector.x()), apply(vector.y()), apply(vector.z()))
    }

    fn native_vector_angle(&mut self, args: &[Value]) -> Result<Value> {
        let lhs = self.vector_arg(args, 0, "angle", VectorCallStyle::Function)?;
        let rhs = self.vector_arg(args, 1, "angle", VectorCallStyle::Function)?;
        if let Some(axis) = args
            .get(2)
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil)
        {
            let axis = self.vector_value_arg(axis, 2, "angle", VectorCallStyle::Function)?;
            return Ok(Value::number(vector_signed_angle(lhs, rhs, axis)));
        }
        Ok(Value::number(vector_unsigned_angle(lhs, rhs)))
    }

    fn native_vector_minmax(&mut self, args: &[Value], op: VectorMinMax) -> Result<Value> {
        let mut result = self.vector_arg(args, 0, op.name(), VectorCallStyle::Function)?;
        for index in 1..args.len() {
            let value = self.vector_arg(args, index, op.name(), VectorCallStyle::Function)?;
            result = vector_minmax(result, value, op);
        }
        self.alloc_vector_value_from_data(result)
    }

    fn native_vector_clamp(&mut self, args: &[Value]) -> Result<Value> {
        let value = self.vector_arg(args, 0, "clamp", VectorCallStyle::Function)?;
        let min = self.vector_arg(args, 1, "clamp", VectorCallStyle::Function)?;
        let max = self.vector_arg(args, 2, "clamp", VectorCallStyle::Function)?;
        self.alloc_vector_value(
            vector_clamp_component(value.x(), min.x(), max.x()),
            vector_clamp_component(value.y(), min.y(), max.y()),
            vector_clamp_component(value.z(), min.z(), max.z()),
        )
    }

    fn native_vector_lerp(&mut self, args: &[Value]) -> Result<Value> {
        let lhs = self.vector_arg(args, 0, "lerp", VectorCallStyle::Function)?;
        let rhs = self.vector_arg(args, 1, "lerp", VectorCallStyle::Function)?;
        let alpha = self.vector_number_arg(args, 2, "lerp")? as f32;
        self.alloc_vector_value(
            vector_lerp_component(lhs.x(), rhs.x(), alpha),
            vector_lerp_component(lhs.y(), rhs.y(), alpha),
            vector_lerp_component(lhs.z(), rhs.z(), alpha),
        )
    }

    fn vector_arg(
        &mut self,
        args: &[Value],
        index: usize,
        function: &'static str,
        style: VectorCallStyle,
    ) -> Result<VectorData> {
        let Some(value) = args.get(index).copied() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: style.missing_message(index, function, "vector"),
            }));
        };
        self.vector_value_arg(value, index, function, style)
    }

    fn vector_value_arg(
        &mut self,
        value: Value,
        index: usize,
        function: &'static str,
        style: VectorCallStyle,
    ) -> Result<VectorData> {
        if value.kind() != ValueKind::Vector {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: style.invalid_message(index, function, "vector", value),
            }));
        }
        self.vector_data(value)
    }

    fn vector_number_arg(
        &mut self,
        args: &[Value],
        index: usize,
        function: &'static str,
    ) -> Result<f64> {
        let Some(value) = args.get(index).copied() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "missing argument #{} to '{function}' (number expected)",
                    index + 1
                ),
            }));
        };
        self.coerce_to_number(value)?.ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: format!(
                    "invalid argument #{} to '{function}' (number expected, got {})",
                    index + 1,
                    value_type_name(value)
                ),
            })
        })
    }

    fn native_string_sub(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 3, "string.sub arguments")?;
        let registers = native_registers(args)?;
        self.string_sub(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
        )
    }

    fn native_string_len(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "string.len argument")?;
        Ok(Value::number(self.string_byte_len(args[0])? as f64))
    }

    fn native_string_byte(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, 3, "string.byte arguments")?;
        let registers = native_registers(args)?;
        let start_register = if args.len() >= 2 { 1 } else { 0 };
        self.string_byte_results(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, start_register, args.len() as i32),
        )
    }

    fn native_string_char(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 0, usize::MAX, "string.char arguments")?;
        let mut output = Vec::with_capacity(args.len());
        for arg in args {
            output.push(self.string_char_byte(*arg)?);
        }
        let atom = self.heap.intern_string_bytes(&output);
        Ok(Value::string_atom(atom))
    }

    fn string_char_byte(&mut self, value: Value) -> Result<u8> {
        let byte = self.coerce_to_i32(value, "number")?;
        if !(0..=255).contains(&byte) {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "byte value",
            }));
        }
        Ok(byte as u8)
    }

    fn native_utf8_char(&mut self, args: &[Value]) -> Result<Value> {
        let mut output = Vec::with_capacity(args.len().saturating_mul(4));
        for arg in args {
            let codepoint = self.utf8_codepoint_arg(*arg)?;
            encode_luau_utf8_char(codepoint, &mut output);
        }
        let atom = self.heap.intern_string_bytes(&output);
        Ok(Value::string_atom(atom))
    }

    fn native_utf8_codepoint(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "utf8.codepoint arguments")?;
        let text = self.value_to_string(args[0])?;
        let len = i64::try_from(text.len()).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string length exceeds i64",
            })
        })?;
        let posi = luau_utf8_posrelat(self.optional_utf8_integer_arg(args.get(1), 1)?, len);
        let pose = luau_utf8_posrelat(self.optional_utf8_integer_arg(args.get(2), posi)?, len);
        if posi < 1 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "out of range".to_owned(),
            }));
        }
        if pose > len {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "out of range".to_owned(),
            }));
        }
        if posi > pose {
            return Ok(Vec::new());
        }
        if pose - posi >= i64::from(i32::MAX) {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "string slice too long".to_owned(),
            }));
        }

        let mut cursor = usize::try_from(posi - 1).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "utf8.codepoint start index underflow",
            })
        })?;
        let end = usize::try_from(pose).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "utf8.codepoint end index underflow",
            })
        })?;
        let range_len = end.saturating_sub(cursor);
        let mut values = Vec::with_capacity(range_len.min(MAX_TABLE_UNPACK_RESULTS));
        while cursor < end {
            let Some((next, codepoint)) = decode_luau_utf8_at(&text, cursor) else {
                return Err(Error::new(ErrorKind::RuntimeError {
                    message: "invalid UTF-8 code".to_owned(),
                }));
            };
            if values.len() >= MAX_TABLE_UNPACK_RESULTS {
                return Err(Error::new(ErrorKind::RuntimeError {
                    message: "string slice too long".to_owned(),
                }));
            }
            values.push(Value::number(f64::from(codepoint)));
            cursor = next;
        }
        Ok(values)
    }

    fn native_utf8_codes(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "utf8.codes arguments")?;
        let text = self.value_to_string(args[0])?;
        let text_atom = self.heap.intern_string_bytes(&text);
        let iterator = native_function_value(
            &mut self.heap,
            NativeFunction::Utf8CodesIterator,
            self.globals,
        );
        Ok(vec![
            iterator,
            Value::string_atom(text_atom),
            Value::number(0.0),
        ])
    }

    fn native_utf8_codes_iterator(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "utf8.codes iterator arguments")?;
        let text = self.value_to_string(args[0])?;
        let len = i64::try_from(text.len()).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string length exceeds i64",
            })
        })?;
        let mut cursor = self
            .lua_tointeger_lossy(args.get(1).copied())
            .saturating_sub(1);
        if cursor < 0 {
            cursor = 0;
        } else if cursor < len {
            cursor += 1;
            while utf8_is_continuation_at(&text, cursor) {
                cursor += 1;
            }
        }
        if cursor >= len {
            return Ok(Vec::new());
        }
        let index = usize::try_from(cursor).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "utf8.codes cursor index underflow",
            })
        })?;
        let Some((next, codepoint)) = decode_luau_utf8_at(&text, index) else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "invalid UTF-8 code".to_owned(),
            }));
        };
        if utf8_is_continuation_at(&text, i64::try_from(next).unwrap_or(i64::MAX)) {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "invalid UTF-8 code".to_owned(),
            }));
        }
        Ok(vec![
            Value::number((index + 1) as f64),
            Value::number(f64::from(codepoint)),
        ])
    }

    fn native_utf8_len(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, usize::MAX, "utf8.len arguments")?;
        let text = self.value_to_string(args[0])?;
        let len = i64::try_from(text.len()).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string length exceeds i64",
            })
        })?;
        let posi = luau_utf8_posrelat(self.optional_utf8_integer_arg(args.get(1), 1)?, len);
        let posj = luau_utf8_posrelat(self.optional_utf8_integer_arg(args.get(2), -1)?, len);
        if posi < 1 || posi - 1 > len {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "initial position out of string".to_owned(),
            }));
        }
        let mut cursor = posi - 1;
        let end = posj - 1;
        if end >= len {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "final position out of string".to_owned(),
            }));
        }

        let mut count = 0usize;
        while cursor <= end {
            let index = usize::try_from(cursor).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "utf8.len cursor index underflow",
                })
            })?;
            let Some((next, _codepoint)) = decode_luau_utf8_at(&text, index) else {
                let position = cursor.checked_add(1).ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "utf8.len invalid position overflow",
                    })
                })?;
                return Ok(vec![Value::nil(), Value::number(position as f64)]);
            };
            cursor = i64::try_from(next).map_err(|_| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "utf8.len cursor index overflow",
                })
            })?;
            count += 1;
        }
        Ok(vec![Value::number(count as f64)])
    }

    fn native_utf8_offset(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, usize::MAX, "utf8.offset arguments")?;
        let text = self.value_to_string(args[0])?;
        let len = i64::try_from(text.len()).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string length exceeds i64",
            })
        })?;
        let mut n = self.required_utf8_integer_arg(args[1], "integer utf8.offset count")?;
        let default_position = if n >= 0 {
            1
        } else {
            len.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "utf8.offset default position overflow",
                })
            })?
        };
        let posi = luau_utf8_posrelat(
            self.optional_utf8_integer_arg(args.get(2), default_position)?,
            len,
        );
        if posi < 1 || posi - 1 > len {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "position out of range".to_owned(),
            }));
        }
        let mut cursor = posi - 1;
        if n == 0 {
            while cursor > 0 && utf8_is_continuation_at(&text, cursor) {
                cursor -= 1;
            }
        } else {
            if utf8_is_continuation_at(&text, cursor) {
                return Err(Error::new(ErrorKind::RuntimeError {
                    message: "initial position is a continuation byte".to_owned(),
                }));
            }
            if n < 0 {
                while n < 0 && cursor > 0 {
                    loop {
                        cursor -= 1;
                        if !(cursor > 0 && utf8_is_continuation_at(&text, cursor)) {
                            break;
                        }
                    }
                    n += 1;
                }
            } else {
                n -= 1;
                while n > 0 && cursor < len {
                    loop {
                        cursor += 1;
                        if !utf8_is_continuation_at(&text, cursor) {
                            break;
                        }
                    }
                    n -= 1;
                }
            }
        }
        if n == 0 {
            let position = cursor.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "utf8.offset result position overflow",
                })
            })?;
            Ok(Value::number(position as f64))
        } else {
            Ok(Value::nil())
        }
    }

    fn utf8_codepoint_arg(&mut self, value: Value) -> Result<u32> {
        let codepoint = self.required_utf8_integer_arg(value, "integer codepoint")?;
        if !(0..=0x10ffff).contains(&codepoint) {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "value out of range",
            }));
        }
        Ok(codepoint as u32)
    }

    fn optional_utf8_integer_arg(&mut self, value: Option<&Value>, default: i64) -> Result<i64> {
        match value
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil)
        {
            Some(value) => self.required_utf8_integer_arg(value, "integer utf8 argument"),
            None => Ok(default),
        }
    }

    fn required_utf8_integer_arg(&mut self, value: Value, expected: &'static str) -> Result<i64> {
        let Some(number) = self.coerce_to_number(value)? else {
            return Err(Error::new(ErrorKind::TypeError { expected }));
        };
        let truncated = number.trunc();
        if !truncated.is_finite() || truncated < i64::MIN as f64 || truncated > i64::MAX as f64 {
            return Err(Error::new(ErrorKind::TypeError { expected }));
        }
        Ok(truncated as i64)
    }

    fn lua_tointeger_lossy(&mut self, value: Option<Value>) -> i64 {
        let Some(value) = value else {
            return 0;
        };
        let Ok(Some(number)) = self.coerce_to_number(value) else {
            return 0;
        };
        let truncated = number.trunc();
        if !truncated.is_finite() {
            return 0;
        }
        if truncated <= i64::MIN as f64 {
            i64::MIN
        } else if truncated >= i64::MAX as f64 {
            i64::MAX
        } else {
            truncated as i64
        }
    }

    fn native_string_case(&mut self, args: &[Value], case: StringCase) -> Result<Value> {
        validate_native_arity(args, 1, 1, case.name())?;
        let mut text = self.value_to_string(args[0])?;
        match case {
            StringCase::Upper => text.make_ascii_uppercase(),
            StringCase::Lower => text.make_ascii_lowercase(),
        }
        let atom = self.heap.intern_string_bytes(&text);
        Ok(Value::string_atom(atom))
    }

    fn native_string_reverse(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "string.reverse argument")?;
        let text = self.value_to_string(args[0])?;
        let reversed = reverse_string_bytes(&text)?;
        let atom = self.heap.intern_string_bytes(&reversed);
        Ok(Value::string_atom(atom))
    }

    fn native_string_rep(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "string.rep arguments")?;
        let registers = native_registers(args)?;
        self.string_rep(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
        )
    }

    fn native_string_find(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, 4, "string.find arguments")?;
        let text = self.value_to_string(args[0])?;
        let pattern = self.value_to_string(args[1])?;
        let start = if let Some(value) = args
            .get(2)
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil)
        {
            let Some(number) = self.coerce_to_number(value)? else {
                return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
            };
            string_library_index(number)?
        } else {
            1
        };
        let plain = args.get(3).copied().is_some_and(Value::truthy);
        let Some((start, end)) = luau_string_find_range(&text, &pattern, start, plain)? else {
            return Ok(vec![Value::nil()]);
        };
        Ok(vec![Value::number(start as f64), Value::number(end as f64)])
    }

    fn native_string_gsub(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        validate_native_arity(args, 3, 4, "string.gsub arguments")?;
        let text = self.value_to_string(args[0])?;
        let pattern = self.value_to_string(args[1])?;
        let replacement = args[2];
        let max_replacements = if let Some(value) = args
            .get(3)
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil)
        {
            self.coerce_to_i32(value, "integer string.gsub count")?
                .max(0) as usize
        } else {
            text.len().saturating_add(1)
        };
        let (output, count) = self.luau_string_gsub(
            program,
            &text,
            &pattern,
            replacement,
            max_replacements,
            depth,
            fuel,
        )?;
        let atom = self.heap.intern_string_bytes(&output);
        Ok(vec![Value::string_atom(atom), Value::number(count as f64)])
    }

    fn luau_string_gsub(
        &mut self,
        program: &Program,
        text: &[u8],
        pattern: &[u8],
        replacement: Value,
        max_replacements: usize,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<(Vec<u8>, usize)> {
        if max_replacements == 0 {
            return Ok((text.to_vec(), 0));
        }
        if let Some(result) =
            self.try_recursive_reverse_gsub(program, text, pattern, replacement)?
        {
            return Ok(result);
        }
        let replacement = self.gsub_replacement(program, replacement, depth, fuel)?;
        let pattern = parse_gsub_pattern(pattern)?;

        let mut output = Vec::with_capacity(text.len());
        let mut cursor = 0;
        let mut replacements = 0;
        while replacements < max_replacements {
            let Some(matched) = find_next_gsub_match(text, &pattern, cursor)? else {
                break;
            };
            push_gsub_output(&mut output, &text[cursor..matched.start])?;
            let replacement = self.resolve_gsub_replacement(
                program,
                &replacement,
                text,
                &matched,
                depth.saturating_add(1),
                fuel,
            )?;
            match replacement {
                Some(bytes) => push_gsub_output(&mut output, &bytes)?,
                None => push_gsub_output(&mut output, &text[matched.start..matched.end])?,
            }
            cursor = matched.end;
            replacements += 1;
            if matched.start == matched.end {
                if cursor >= text.len() {
                    break;
                }
                push_gsub_output(&mut output, &text[cursor..cursor + 1])?;
                cursor += 1;
            }
        }
        push_gsub_output(&mut output, &text[cursor..])?;
        Ok((output, replacements))
    }

    fn try_recursive_reverse_gsub(
        &mut self,
        program: &Program,
        text: &[u8],
        pattern: &[u8],
        replacement: Value,
    ) -> Result<Option<(Vec<u8>, usize)>> {
        if pattern != LUAU_RECURSIVE_REVERSE_GSUB_PATTERN {
            return Ok(None);
        }
        let Some(replacement_handle) = replacement.as_function() else {
            return Ok(None);
        };
        if !self.is_current_recursive_reverse_gsub(program, replacement_handle)? {
            return Ok(None);
        }
        if text.len() < 2 {
            return Ok(Some((text.to_vec(), 0)));
        }
        let mut reversed = text.to_vec();
        reversed.reverse();
        Ok(Some((reversed, 1)))
    }

    fn is_current_recursive_reverse_gsub(
        &mut self,
        program: &Program,
        replacement_handle: u32,
    ) -> Result<bool> {
        let Some(current_function) = self.call_stack.last().and_then(|frame| frame.function) else {
            return Ok(false);
        };
        let Some(current_handle) = current_function.as_function() else {
            return Ok(false);
        };
        let Some((replacement_program, replacement_prototype, replacement_function)) =
            same_program_bytecode_function(program, self.heap.function_kind(replacement_handle))
        else {
            return Ok(false);
        };
        let Some((current_program, current_prototype, current_bytecode)) =
            same_program_bytecode_function(program, self.heap.function_kind(current_handle))
        else {
            return Ok(false);
        };
        if current_program != replacement_program {
            return Ok(false);
        }
        let Some(global_name) =
            recursive_reverse_replacement_global(program, replacement_function)?
        else {
            return Ok(false);
        };
        if !recursive_reverse_caller_matches(
            program,
            current_bytecode,
            replacement_prototype,
            global_name,
        )? {
            return Ok(false);
        }
        let environment = self.current_environment()?;
        let global_key = self.global_key(program, global_name)?;
        let global_value = self.raw_table_get(environment, global_key)?;
        Ok(global_value.as_function() == Some(current_handle)
            && current_prototype != replacement_prototype)
    }

    fn gsub_replacement(
        &mut self,
        program: &Program,
        value: Value,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<GSubReplacement> {
        if value.kind() == ValueKind::String || value.kind() == ValueKind::Number {
            return Ok(GSubReplacement::String(self.value_to_string(value)?));
        }
        if value.kind() == ValueKind::Function {
            return Ok(GSubReplacement::Function(value));
        }
        if value.kind() == ValueKind::Table {
            // Table replacements may use __index, so validate the value kind here but resolve per match.
            self.check_recursion_depth(depth)?;
            return Ok(GSubReplacement::Table(value));
        }
        let _ = program;
        let _ = fuel;
        Err(Error::new(ErrorKind::TypeError {
            expected: "string, number, function, or table",
        }))
    }

    fn resolve_gsub_replacement(
        &mut self,
        program: &Program,
        replacement: &GSubReplacement,
        text: &[u8],
        matched: &GSubMatch,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Option<Vec<u8>>> {
        match replacement {
            GSubReplacement::String(replacement) => {
                expand_gsub_replacement(replacement, text, matched).map(Some)
            }
            GSubReplacement::Function(function) => {
                let args = self.gsub_replacement_args(text, matched)?;
                let results =
                    self.call_value_non_yieldable(program, *function, &args, depth, fuel)?;
                self.gsub_replacement_result(first_result_or_nil(results))
            }
            GSubReplacement::Table(table) => {
                let key = self.gsub_replacement_key(text, matched)?;
                let value = self.get_index_value(program, *table, key, depth, fuel)?;
                self.gsub_replacement_result(value)
            }
        }
    }

    fn gsub_replacement_args(&mut self, text: &[u8], matched: &GSubMatch) -> Result<Vec<Value>> {
        let captures = gsub_effective_captures(text, matched);
        let mut args = Vec::with_capacity(captures.len());
        for capture in captures {
            args.push(self.gsub_capture_value(text, capture)?);
        }
        Ok(args)
    }

    fn gsub_replacement_key(&mut self, text: &[u8], matched: &GSubMatch) -> Result<Value> {
        let capture = gsub_effective_captures(text, matched)
            .into_iter()
            .next()
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.gsub replacement key is missing",
                })
            })?;
        self.gsub_capture_value(text, capture)
    }

    fn gsub_capture_value(&mut self, text: &[u8], capture: GSubCapture) -> Result<Value> {
        match capture {
            GSubCapture::Slice { start, end } => {
                let atom = self.heap.intern_string_bytes(&text[start..end]);
                Ok(Value::string_atom(atom))
            }
            GSubCapture::Position(position) => Ok(Value::number(position as f64)),
        }
    }

    fn gsub_replacement_result(&mut self, value: Value) -> Result<Option<Vec<u8>>> {
        if value.kind() == ValueKind::Nil || value.as_bool() == Some(false) {
            return Ok(None);
        }
        self.value_to_string(value).map(Some)
    }

    fn native_string_gmatch(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, 2, "string.gmatch arguments")?;
        let text = self.value_to_string(args[0])?;
        let pattern = self.value_to_string(args[1])?;
        let _ = luau_gmatch_pattern_kind(&pattern)?;

        let text_atom = self.heap.intern_string_bytes(&text);
        let pattern_atom = self.heap.intern_string_bytes(&pattern);
        let text_upvalue = self
            .heap
            .alloc_upvalue(Value::string_atom(text_atom))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.gmatch text upvalue handle overflow",
                })
            })?;
        let pattern_upvalue = self
            .heap
            .alloc_upvalue(Value::string_atom(pattern_atom))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.gmatch pattern upvalue handle overflow",
                })
            })?;
        let cursor_upvalue = self.heap.alloc_upvalue(Value::number(0.0)).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch cursor upvalue handle overflow",
            })
        })?;
        let handle = self
            .heap
            .alloc_native_function_with_upvalues(
                NativeFunction::StringGMatchIterator,
                vec![text_upvalue, pattern_upvalue, cursor_upvalue],
                self.globals,
            )
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.gmatch iterator function handle overflow",
                })
            })?;
        Ok(vec![Value::function(handle)])
    }

    fn native_string_gmatch_iterator(&mut self, handle: u32) -> Result<Vec<Value>> {
        let upvalues = self.heap.function_upvalues(handle).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch iterator handle is missing",
            })
        })?;
        if upvalues.len() != 3 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch iterator upvalue layout is invalid",
            }));
        }

        let text = self.string_upvalue_bytes(upvalues[0], "string.gmatch text upvalue")?;
        let pattern = self.string_upvalue_bytes(upvalues[1], "string.gmatch pattern upvalue")?;
        let cursor = self.cursor_upvalue_index(upvalues[2])?;
        let Some(result) = luau_gmatch_next(&text, &pattern, cursor)? else {
            self.heap
                .upvalue_set(
                    upvalues[2],
                    Value::number(text.len().saturating_add(1) as f64),
                )
                .ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "string.gmatch cursor upvalue is missing",
                    })
                })?;
            return Ok(vec![Value::nil()]);
        };
        self.heap
            .upvalue_set(upvalues[2], Value::number(result.next_cursor as f64))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.gmatch cursor upvalue is missing",
                })
            })?;

        let mut values = Vec::with_capacity(result.captures.len());
        for capture in result.captures {
            match capture {
                GMatchCapture::Slice { start, end } => {
                    let atom = self.heap.intern_string_bytes(&text[start..end]);
                    values.push(Value::string_atom(atom));
                }
                GMatchCapture::Position(position) => values.push(Value::number(position as f64)),
            }
        }
        Ok(values)
    }

    fn string_upvalue_bytes(&mut self, upvalue: u32, reason: &'static str) -> Result<Vec<u8>> {
        let value = self.heap.upvalue_get(upvalue).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch upvalue handle is missing",
            })
        })?;
        self.string_value_bytes(value)?
            .ok_or_else(|| Error::new(ErrorKind::InvalidProgram { reason }))
    }

    fn cursor_upvalue_index(&mut self, upvalue: u32) -> Result<usize> {
        let value = self.heap.upvalue_get(upvalue).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch cursor upvalue handle is missing",
            })
        })?;
        let number = value.as_number().ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch cursor upvalue is not a number",
            })
        })?;
        if !number.is_finite() || number < 0.0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "string.gmatch cursor upvalue is invalid",
            }));
        }
        Ok(number as usize)
    }

    fn native_string_match(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, 3, "string.match arguments")?;
        let text = self.value_to_string(args[0])?;
        let pattern = self.value_to_string(args[1])?;
        let start = if let Some(value) = args
            .get(2)
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil)
        {
            i64::from(self.coerce_to_i32(value, "integer string library index")?)
        } else {
            1
        };
        let Some(captures) = luau_string_match_captures(&text, &pattern, start)? else {
            return Ok(vec![Value::nil()]);
        };
        let mut values = Vec::with_capacity(captures.len());
        for capture in captures {
            match capture {
                StringCapture::Slice { start, end } => {
                    let atom = self.heap.intern_string_bytes(&text[start..end]);
                    values.push(Value::string_atom(atom));
                }
                StringCapture::Position(position) => values.push(Value::number(position as f64)),
            }
        }
        Ok(values)
    }

    fn native_string_split(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 2, "string.split arguments")?;
        let registers = native_registers(args)?;
        self.string_split(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
        )
    }

    fn native_string_format(
        &mut self,
        program: &Program,
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "string.format arguments")?;
        if let Some(format_atom) = args[0].as_string_atom() {
            let plan = self.string_format_plan(format_atom)?;
            let output = self.luau_string_format_plan(program, &plan, &args[1..], depth, fuel)?;
            let atom = self.heap.intern_string_bytes(&output);
            return Ok(Value::string_atom(atom));
        }
        let format = self.value_to_string(args[0])?;
        let output = self.luau_string_format(program, &format, &args[1..], depth, fuel)?;
        let atom = self.heap.intern_string_bytes(&output);
        Ok(Value::string_atom(atom))
    }

    fn string_format(
        &mut self,
        program: &Program,
        registers: &[Value],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let args = string_format_args(registers, instruction)?;
        self.native_string_format(program, args, depth, fuel)
    }

    fn string_format_plan(&mut self, format_atom: u32) -> Result<Arc<[StringFormatPlanItem]>> {
        if let Some(plan) = self.string_format_plan_cache.get(&format_atom) {
            return Ok(Arc::clone(plan));
        }
        let format = self.heap.string_bytes(format_atom).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "string.format atom is missing",
            })
        })?;
        let plan = Arc::<[StringFormatPlanItem]>::from(parse_string_format_plan(&format)?);
        if self.string_format_plan_cache.len() < STRING_FORMAT_PLAN_CACHE_MAX {
            self.string_format_plan_cache
                .insert(format_atom, Arc::clone(&plan));
        }
        Ok(plan)
    }

    fn native_string_pack(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "string.pack arguments")?;
        let format = self.value_to_string(args[0])?;
        let mut reader = PackFormatReader::new(&format);
        let mut state = PackState::default();
        let mut output = Vec::new();
        let mut arg_index = 1usize;

        while let Some(item) = reader.next_item(&mut state)? {
            match item {
                PackItem::AlignOnly { align } => {
                    push_pack_padding(&mut output, align)?;
                }
                PackItem::Data(data) => {
                    push_pack_padding(&mut output, data.align)?;
                    match data.kind {
                        PackDataKind::Pad => output.push(0),
                        PackDataKind::SignedInteger { size } => {
                            let number = self.pack_number_arg(args, arg_index)?;
                            arg_index += 1;
                            let value = pack_signed_integer(number, size)?;
                            push_pack_integer(
                                &mut output,
                                value as u64,
                                size,
                                state.endian,
                                value < 0,
                            );
                        }
                        PackDataKind::UnsignedInteger { size } => {
                            let number = self.pack_number_arg(args, arg_index)?;
                            arg_index += 1;
                            let value = pack_unsigned_integer(number, size)?;
                            push_pack_integer(&mut output, value, size, state.endian, false);
                        }
                        PackDataKind::Float32 => {
                            let number = self.pack_number_arg(args, arg_index)?;
                            arg_index += 1;
                            push_pack_integer(
                                &mut output,
                                u64::from((number as f32).to_bits()),
                                4,
                                state.endian,
                                false,
                            );
                        }
                        PackDataKind::Float64 => {
                            let number = self.pack_number_arg(args, arg_index)?;
                            arg_index += 1;
                            push_pack_integer(
                                &mut output,
                                number.to_bits(),
                                8,
                                state.endian,
                                false,
                            );
                        }
                        PackDataKind::FixedString { size } => {
                            let text = self.pack_string_arg(args, arg_index)?;
                            arg_index += 1;
                            if text.len() > size {
                                return Err(pack_runtime_error("string longer than given size"));
                            }
                            let padded_len = checked_pack_position(output.len(), size)?;
                            output.extend_from_slice(&text);
                            output.resize(padded_len, 0);
                        }
                        PackDataKind::ZeroTerminatedString => {
                            let text = self.pack_string_arg(args, arg_index)?;
                            arg_index += 1;
                            if text.contains(&0) {
                                return Err(pack_runtime_error("string contains zeros"));
                            }
                            output.extend_from_slice(&text);
                            output.push(0);
                        }
                        PackDataKind::SizedString { size } => {
                            let text = self.pack_string_arg(args, arg_index)?;
                            arg_index += 1;
                            let len = u64::try_from(text.len()).map_err(|_| {
                                pack_runtime_error("string length does not fit in pack size")
                            })?;
                            let max = unsigned_integer_max(size);
                            if (len as f64) > max {
                                return Err(pack_runtime_error(
                                    "string length does not fit in pack size",
                                ));
                            }
                            push_pack_integer(&mut output, len, size, state.endian, false);
                            output.extend_from_slice(&text);
                        }
                    }
                }
            }
        }

        let atom = self.heap.intern_string_bytes(&output);
        Ok(Value::string_atom(atom))
    }

    fn native_string_unpack(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        validate_native_arity(args, 2, 3, "string.unpack arguments")?;
        let format = self.value_to_string(args[0])?;
        let data = self.value_to_string(args[1])?;
        let mut cursor = unpack_start_position(args.get(2).copied(), data.len())?;
        let mut reader = PackFormatReader::new(&format);
        let mut state = PackState::default();
        let mut results = Vec::new();

        while let Some(item) = reader.next_item(&mut state)? {
            match item {
                PackItem::AlignOnly { align } => {
                    cursor = checked_pack_position(cursor, pack_padding(cursor, align)?)?;
                }
                PackItem::Data(data_option) => {
                    cursor =
                        checked_pack_position(cursor, pack_padding(cursor, data_option.align)?)?;
                    match data_option.kind {
                        PackDataKind::Pad => {
                            ensure_unpack_available(&data, cursor, 1)?;
                            cursor += 1;
                        }
                        PackDataKind::SignedInteger { size } => {
                            let bytes = unpack_bytes(&data, cursor, size)?;
                            cursor += size;
                            let value = unpack_signed_integer(bytes, size, state.endian)?;
                            results.push(Value::number(value as f64));
                        }
                        PackDataKind::UnsignedInteger { size } => {
                            let bytes = unpack_bytes(&data, cursor, size)?;
                            cursor += size;
                            let value = unpack_unsigned_integer(bytes, size, state.endian)?;
                            results.push(Value::number(value as f64));
                        }
                        PackDataKind::Float32 => {
                            let bytes = unpack_bytes(&data, cursor, 4)?;
                            cursor += 4;
                            let bits = unpack_unsigned_integer(bytes, 4, state.endian)? as u32;
                            results.push(Value::number(f32::from_bits(bits) as f64));
                        }
                        PackDataKind::Float64 => {
                            let bytes = unpack_bytes(&data, cursor, 8)?;
                            cursor += 8;
                            let bits = unpack_unsigned_integer(bytes, 8, state.endian)?;
                            results.push(Value::number(f64::from_bits(bits)));
                        }
                        PackDataKind::FixedString { size } => {
                            let bytes = unpack_bytes(&data, cursor, size)?;
                            cursor += size;
                            let atom = self.heap.intern_string_bytes(bytes);
                            results.push(Value::string_atom(atom));
                        }
                        PackDataKind::ZeroTerminatedString => {
                            let Some(offset) = data[cursor..].iter().position(|byte| *byte == 0)
                            else {
                                return Err(pack_runtime_error("unfinished string for format 'z'"));
                            };
                            let end = cursor + offset;
                            let atom = self.heap.intern_string_bytes(&data[cursor..end]);
                            results.push(Value::string_atom(atom));
                            cursor = end + 1;
                        }
                        PackDataKind::SizedString { size } => {
                            let len_bytes = unpack_bytes(&data, cursor, size)?;
                            cursor += size;
                            let len = unpack_unsigned_integer(len_bytes, size, state.endian)?;
                            let len = usize::try_from(len).map_err(|_| {
                                pack_runtime_error("string length does not fit in host usize")
                            })?;
                            let bytes = unpack_bytes(&data, cursor, len)?;
                            cursor += len;
                            let atom = self.heap.intern_string_bytes(bytes);
                            results.push(Value::string_atom(atom));
                        }
                    }
                }
            }
        }

        results.push(Value::number((cursor + 1) as f64));
        Ok(results)
    }

    fn native_string_packsize(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "string.packsize arguments")?;
        let format = self.value_to_string(args[0])?;
        let mut reader = PackFormatReader::new(&format);
        let mut state = PackState::default();
        let mut size = 0usize;

        while let Some(item) = reader.next_item(&mut state)? {
            match item {
                PackItem::AlignOnly { align } => {
                    size = checked_pack_position(size, pack_padding(size, align)?)?;
                }
                PackItem::Data(data) => {
                    if data.variable_size {
                        return Err(pack_runtime_error("variable-length format"));
                    }
                    size = checked_pack_position(size, pack_padding(size, data.align)?)?;
                    size = checked_pack_position(size, data.size)?;
                }
            }
        }

        Ok(Value::number(size as f64))
    }

    fn pack_number_arg(&mut self, args: &[Value], index: usize) -> Result<f64> {
        let value = args
            .get(index)
            .copied()
            .ok_or_else(|| pack_runtime_error("missing argument for string.pack"))?;
        self.coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))
    }

    fn pack_string_arg(&mut self, args: &[Value], index: usize) -> Result<Vec<u8>> {
        let value = args
            .get(index)
            .copied()
            .ok_or_else(|| pack_runtime_error("missing argument for string.pack"))?;
        self.value_to_string(value)
    }

    fn native_bit32_fold(&mut self, args: &[Value], op: Bit32Fold) -> Result<Value> {
        let mut result = match op {
            Bit32Fold::And => u32::MAX,
            Bit32Fold::Or | Bit32Fold::Xor => 0,
        };
        for value in args {
            let value = self.bit32_arg(*value)?;
            result = match op {
                Bit32Fold::And => result & value,
                Bit32Fold::Or => result | value,
                Bit32Fold::Xor => result ^ value,
            };
        }
        Ok(Value::number(f64::from(result)))
    }

    fn native_bit32_btest(&mut self, args: &[Value]) -> Result<Value> {
        let value = self.native_bit32_fold(args, Bit32Fold::And)?;
        Ok(Value::boolean(value.as_number().unwrap_or(0.0) != 0.0))
    }

    fn native_bit32_bnot(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "bit32.bnot argument")?;
        Ok(Value::number(f64::from(!self.bit32_arg(args[0])?)))
    }

    fn native_bit32_shift(&mut self, args: &[Value], op: Bit32Shift) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "bit32 shift arguments")?;
        let value = self.bit32_arg(args[0])?;
        let shift = self.bit32_shift_arg(args[1])?;
        let result = match op {
            Bit32Shift::LogicalLeft => bit32_lshift(value, shift),
            Bit32Shift::LogicalRight => bit32_rshift(value, shift),
            Bit32Shift::ArithmeticRight => bit32_arshift(value, shift),
        };
        Ok(Value::number(f64::from(result)))
    }

    fn native_bit32_rotate(&mut self, args: &[Value], op: Bit32Rotate) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "bit32 rotate arguments")?;
        let value = self.bit32_arg(args[0])?;
        let shift = self.bit32_shift_arg(args[1])?;
        let amount = shift.rem_euclid(32) as u32;
        let result = match op {
            Bit32Rotate::Left => value.rotate_left(amount),
            Bit32Rotate::Right => value.rotate_right(amount),
        };
        Ok(Value::number(f64::from(result)))
    }

    fn native_bit32_extract(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, u8::MAX as usize, "bit32.extract arguments")?;
        let value = self.bit32_arg(args[0])?;
        let field = self.bit32_field_arg(args[1])?;
        let width = if let Some(width) = args.get(2).copied() {
            self.bit32_width_arg(width)?
        } else {
            1
        };
        validate_bit32_field_width(field, width)?;
        let mask = bit32_width_mask(width);
        Ok(Value::number(f64::from((value >> field) & mask)))
    }

    fn native_bit32_replace(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 3, u8::MAX as usize, "bit32.replace arguments")?;
        let value = self.bit32_arg(args[0])?;
        let replacement = self.bit32_arg(args[1])?;
        let field = self.bit32_field_arg(args[2])?;
        let width = if let Some(width) = args.get(3).copied() {
            self.bit32_width_arg(width)?
        } else {
            1
        };
        validate_bit32_field_width(field, width)?;
        let mask = bit32_width_mask(width) << field;
        let inserted = (replacement << field) & mask;
        Ok(Value::number(f64::from((value & !mask) | inserted)))
    }

    fn native_bit32_count(&mut self, args: &[Value], count: Bit32Count) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "bit32 count argument")?;
        let value = self.bit32_arg(args[0])?;
        let result = match count {
            Bit32Count::Leading => value.leading_zeros(),
            Bit32Count::Trailing => value.trailing_zeros(),
        };
        Ok(Value::number(f64::from(result)))
    }

    fn native_bit32_byteswap(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "bit32.byteswap argument")?;
        Ok(Value::number(f64::from(
            self.bit32_arg(args[0])?.swap_bytes(),
        )))
    }

    fn bit32_arg(&mut self, value: Value) -> Result<u32> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        bit32_number_to_u32(number)
    }

    fn bit32_shift_arg(&mut self, value: Value) -> Result<i32> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        bit32_number_to_i32(number)
    }

    fn bit32_field_arg(&mut self, value: Value) -> Result<u32> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        bit32_number_to_i32(number)
            .and_then(|value| u32::try_from(value).map_err(|_| bit32_range_error()))
    }

    fn bit32_width_arg(&mut self, value: Value) -> Result<u32> {
        let number = self
            .coerce_to_number(value)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        bit32_number_to_i32(number)
            .and_then(|value| u32::try_from(value).map_err(|_| bit32_range_error()))
    }

    fn luau_string_format(
        &mut self,
        program: &Program,
        format: &[u8],
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<u8>> {
        let plan = parse_string_format_plan(format)?;
        self.luau_string_format_plan(program, &plan, args, depth, fuel)
    }

    fn luau_string_format_plan(
        &mut self,
        program: &Program,
        plan: &[StringFormatPlanItem],
        args: &[Value],
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<u8>> {
        let mut output = Vec::with_capacity(self.string_format_output_capacity_hint(plan, args));
        let mut arg_index = 0usize;
        for item in plan {
            match item {
                StringFormatPlanItem::Literal(bytes) => output.extend_from_slice(bytes),
                StringFormatPlanItem::Tostring => {
                    let arg = args.get(arg_index).copied().ok_or_else(|| {
                        Error::new(ErrorKind::RuntimeError {
                            message: format!("missing argument #{}", arg_index + 2),
                        })
                    })?;
                    arg_index += 1;
                    output.extend_from_slice(
                        &self.value_to_tostring_with_metamethod(program, arg, depth, fuel)?,
                    );
                }
                StringFormatPlanItem::Spec(spec) => {
                    let arg = args.get(arg_index).copied().ok_or_else(|| {
                        Error::new(ErrorKind::RuntimeError {
                            message: format!("missing argument #{}", arg_index + 2),
                        })
                    })?;
                    arg_index += 1;

                    self.append_format_argument(&mut output, arg, *spec)?;
                }
            }
        }

        Ok(output)
    }

    fn string_format_output_capacity_hint(
        &mut self,
        plan: &[StringFormatPlanItem],
        args: &[Value],
    ) -> usize {
        const MAX_STRING_FORMAT_OUTPUT_CAPACITY_HINT: usize = 4096;

        let mut hint = 0usize;
        let mut arg_index = 0usize;
        for item in plan {
            let addition = match item {
                StringFormatPlanItem::Literal(bytes) => bytes.len(),
                StringFormatPlanItem::Tostring => {
                    arg_index += 1;
                    16
                }
                StringFormatPlanItem::Spec(spec) => {
                    let arg = args.get(arg_index).copied();
                    arg_index += 1;
                    format_spec_capacity_hint(&mut self.heap, *spec, arg)
                }
            };
            hint = hint.saturating_add(addition);
            if hint >= MAX_STRING_FORMAT_OUTPUT_CAPACITY_HINT {
                return MAX_STRING_FORMAT_OUTPUT_CAPACITY_HINT;
            }
        }
        hint
    }

    fn append_format_argument(
        &mut self,
        output: &mut Vec<u8>,
        arg: Value,
        spec: FormatSpec,
    ) -> Result<()> {
        if self.append_simple_format_argument(output, arg, spec)? {
            return Ok(());
        }

        let formatted = match spec.conversion {
            FormatConversion::String => self.format_string_argument(arg, spec)?,
            FormatConversion::Char => self.format_char_argument(arg, spec)?,
            FormatConversion::Quote => self.format_quoted_argument(arg)?,
            FormatConversion::SignedDecimal => self.format_signed_integer_argument(arg, spec)?,
            FormatConversion::UnsignedDecimal
            | FormatConversion::Octal
            | FormatConversion::LowerHex
            | FormatConversion::UpperHex => self.format_unsigned_integer_argument(arg, spec)?,
            FormatConversion::FloatFixed
            | FormatConversion::FloatLowerExp
            | FormatConversion::FloatUpperExp
            | FormatConversion::FloatLowerGeneral
            | FormatConversion::FloatUpperGeneral => self.format_float_argument(arg, spec)?,
        };
        output.extend_from_slice(&formatted);
        Ok(())
    }

    fn append_simple_format_argument(
        &mut self,
        output: &mut Vec<u8>,
        arg: Value,
        spec: FormatSpec,
    ) -> Result<bool> {
        if spec.width.is_some() {
            return Ok(false);
        }

        match spec.conversion {
            FormatConversion::String if spec.precision.is_none() => {
                if let Some(atom) = arg.as_string_atom() {
                    self.heap
                        .append_string_atom_bytes(atom, output)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::InvalidProgram {
                                reason: "string atom is missing",
                            })
                        })?;
                    return Ok(true);
                }
                if let Some(number) = arg.as_number() {
                    let text = luau_number_to_string(number);
                    output.extend_from_slice(text.as_bytes());
                    return Ok(true);
                }
                Ok(false)
            }
            FormatConversion::Char if spec.precision.is_none() => {
                let number = self
                    .coerce_to_number(arg)?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
                output.push((number as i32) as u8);
                Ok(true)
            }
            FormatConversion::SignedDecimal if spec.precision.is_none() => {
                let number = self
                    .coerce_to_number(arg)?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
                let integer = number as i64;
                if integer < 0 {
                    output.push(b'-');
                } else {
                    if spec.sign_plus {
                        output.push(b'+');
                    } else if spec.sign_space {
                        output.push(b' ');
                    }
                }
                append_u64_decimal(output, integer.unsigned_abs());
                Ok(true)
            }
            FormatConversion::UnsignedDecimal
            | FormatConversion::Octal
            | FormatConversion::LowerHex
            | FormatConversion::UpperHex
                if spec.precision.is_none() =>
            {
                let number = self
                    .coerce_to_number(arg)?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
                let integer = number as i64 as u64;
                match spec.conversion {
                    FormatConversion::UnsignedDecimal => {
                        append_u64_decimal(output, integer);
                    }
                    FormatConversion::Octal => {
                        append_u64_radix(output, integer, 8, false);
                    }
                    FormatConversion::LowerHex => {
                        append_u64_radix(output, integer, 16, false);
                    }
                    FormatConversion::UpperHex => {
                        append_u64_radix(output, integer, 16, true);
                    }
                    _ => unreachable!(
                        "simple unsigned formatting only handles unsigned integer conversions"
                    ),
                }
                Ok(true)
            }
            FormatConversion::FloatFixed => {
                let number = self
                    .coerce_to_number(arg)?
                    .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
                if number.is_sign_positive() {
                    if spec.sign_plus {
                        output.push(b'+');
                    } else if spec.sign_space {
                        output.push(b' ');
                    }
                }
                let precision = spec.precision.unwrap_or(6);
                if append_exact_fixed_decimal(output, number, precision) {
                    return Ok(true);
                }
                append_format_args(output, format_args!("{number:.precision$}"));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn format_string_argument(&mut self, arg: Value, spec: FormatSpec) -> Result<Vec<u8>> {
        let mut text = self.value_to_string(arg)?;
        if let Some(precision) = spec.precision {
            text.truncate(precision.min(text.len()));
        }
        Ok(apply_format_width(text, spec, false))
    }

    fn format_char_argument(&mut self, arg: Value, spec: FormatSpec) -> Result<Vec<u8>> {
        let number = self
            .coerce_to_number(arg)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let byte = (number as i32) as u8;
        Ok(apply_format_width(vec![byte], spec, false))
    }

    fn format_quoted_argument(&mut self, arg: Value) -> Result<Vec<u8>> {
        let text = self.value_to_string(arg)?;
        let mut output = Vec::with_capacity(text.len() + 2);
        output.push(b'"');
        for byte in text {
            match byte {
                b'"' | b'\\' | b'\n' => {
                    output.push(b'\\');
                    output.push(byte);
                }
                b'\r' => output.extend_from_slice(br"\r"),
                0 => output.extend_from_slice(br"\000"),
                byte => output.push(byte),
            }
        }
        output.push(b'"');
        Ok(output)
    }

    fn format_signed_integer_argument(&mut self, arg: Value, spec: FormatSpec) -> Result<Vec<u8>> {
        let number = self
            .coerce_to_number(arg)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number as i64;
        let mut text = integer.unsigned_abs().to_string();
        if let Some(precision) = spec.precision {
            while text.len() < precision {
                text.insert(0, '0');
            }
        }
        if integer < 0 {
            text.insert(0, '-');
        } else if spec.sign_plus {
            text.insert(0, '+');
        } else if spec.sign_space {
            text.insert(0, ' ');
        }
        Ok(apply_format_width(text.into_bytes(), spec, true))
    }

    fn format_unsigned_integer_argument(
        &mut self,
        arg: Value,
        spec: FormatSpec,
    ) -> Result<Vec<u8>> {
        let number = self
            .coerce_to_number(arg)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let integer = number as i64 as u64;
        let mut text = match spec.conversion {
            FormatConversion::UnsignedDecimal => integer.to_string(),
            FormatConversion::Octal => format!("{integer:o}"),
            FormatConversion::LowerHex => format!("{integer:x}"),
            FormatConversion::UpperHex => format!("{integer:X}"),
            _ => unreachable!(
                "format_unsigned_integer_argument only handles unsigned integer conversions"
            ),
        };
        if let Some(precision) = spec.precision {
            while text.len() < precision {
                text.insert(0, '0');
            }
        }
        Ok(apply_format_width(text.into_bytes(), spec, true))
    }

    fn format_float_argument(&mut self, arg: Value, spec: FormatSpec) -> Result<Vec<u8>> {
        let number = self
            .coerce_to_number(arg)?
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))?;
        let precision = spec.precision.unwrap_or(6);
        let mut text = match spec.conversion {
            FormatConversion::FloatFixed => format!("{number:.precision$}"),
            FormatConversion::FloatLowerExp => {
                normalize_format_exponent(format!("{number:.precision$e}"))
            }
            FormatConversion::FloatUpperExp => {
                normalize_format_exponent(format!("{number:.precision$E}"))
            }
            FormatConversion::FloatLowerGeneral => {
                format_general_float(number, precision.max(1), false)
            }
            FormatConversion::FloatUpperGeneral => {
                format_general_float(number, precision.max(1), true)
            }
            _ => unreachable!("format_float_argument only handles float conversions"),
        };
        if number.is_sign_positive() {
            if spec.sign_plus {
                text.insert(0, '+');
            } else if spec.sign_space {
                text.insert(0, ' ');
            }
        }
        Ok(apply_format_width(text.into_bytes(), spec, true))
    }

    fn native_table_concat(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 4, "table.concat arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let separator = if let Some(separator) = args.get(1).copied() {
            self.value_to_string(separator)?
        } else {
            Vec::new()
        };
        let start = if let Some(start) = args.get(2).copied() {
            Some(table_concat_index(number_arg(start)?)?)
        } else {
            None
        };
        let end = if let Some(end) = args.get(3).copied() {
            Some(table_concat_index(number_arg(end)?)?)
        } else {
            None
        };
        self.table_concat(table, &separator, start, end)
    }

    fn native_table_clear(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "table expected")?;
        let table = args[0].as_table().ok_or_else(table_expected_error)?;
        self.ensure_table_writable(table)?;
        self.heap.table_clear(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        Ok(Value::nil())
    }

    fn native_table_pack(&mut self, args: &[Value]) -> Result<Value> {
        let table = self.heap.alloc_array_table(args);
        let key = Value::string_atom(self.heap.intern_string("n"));
        self.heap
            .table_set(table, key, Value::number(args.len() as f64))
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table.pack table handle is missing",
                })
            })?;
        Ok(Value::table(table))
    }

    fn native_table_unpack(&mut self, args: &[Value]) -> Result<Vec<Value>> {
        let (table, start, end) = self.table_unpack_bounds(args)?;
        if end < start {
            return Ok(Vec::new());
        }
        let count = unpack_result_count(start, end)?;
        let mut results = Vec::with_capacity(count);
        for index in start..=end {
            results.push(self.raw_table_get(table, Value::number(index as f64))?);
        }
        Ok(results)
    }

    fn table_unpack_bounds(&mut self, args: &[Value]) -> Result<(u32, i64, i64)> {
        validate_native_arity(args, 1, 3, "table.unpack arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let start = match args.get(1).copied() {
            None => 1,
            Some(value) if value.kind() == ValueKind::Nil => 1,
            Some(value) => string_library_index(number_arg(value)?)?,
        };
        let default_end = i64::try_from(self.raw_len_value(args[0])?).map_err(|_| {
            Error::new(ErrorKind::RuntimeError {
                message: "too many results to unpack".to_owned(),
            })
        })?;
        let end = match args.get(2).copied() {
            None => default_end,
            Some(value) if value.kind() == ValueKind::Nil => default_end,
            Some(value) => string_library_index(number_arg(value)?)?,
        };
        Ok((table, start, end))
    }

    fn native_table_getn(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "table.getn argument")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let len = self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        Ok(Value::number(len as f64))
    }

    fn native_table_maxn(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 1, "table.maxn argument")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let max = self.heap.max_numeric_key(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        Ok(Value::number(max))
    }

    fn native_table_create(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 2, "table.create arguments")?;
        let registers = native_registers(args)?;
        self.table_create(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
        )
    }

    fn native_table_find(
        &mut self,
        args: &[Value],
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        validate_native_arity(args, 2, 3, "table.find arguments")?;
        let registers = native_registers(args)?;
        self.table_find(
            program,
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
            depth,
            fuel,
        )
    }

    fn native_table_foreach(
        &mut self,
        args: &[Value],
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
        mode: TableForeachMode,
    ) -> Result<Value> {
        validate_native_arity(args, 2, 2, "table.foreach arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let callback = args[1];
        if callback.kind() != ValueKind::Function {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "function",
            }));
        }

        match mode {
            TableForeachMode::Pairs => {
                self.table_foreach_pairs(table, callback, program, depth, fuel)
            }
            TableForeachMode::Array => {
                self.table_foreach_array(table, callback, program, depth, fuel)
            }
        }
    }

    fn table_foreach_pairs(
        &mut self,
        table: u32,
        callback: Value,
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let mut key = Value::nil();
        loop {
            let next = self.table_next_after(table, key).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "table.foreach table handle is missing",
                })
            })?;
            let (next_key, value) = match next {
                TableNext::Entry(next_key, value) => (next_key, value),
                TableNext::End => return Ok(Value::nil()),
                TableNext::InvalidKey => return Err(Error::new(ErrorKind::InvalidKeyToNext)),
            };
            let result = first_result_or_nil(self.call_value(
                program,
                callback,
                &[next_key, value],
                depth,
                fuel,
            )?);
            if result.kind() != ValueKind::Nil {
                return Ok(result);
            }
            key = next_key;
        }
    }

    fn table_foreach_array(
        &mut self,
        table: u32,
        callback: Value,
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let len = self.table_foreachi_len(table)?;
        for index in 1..=len {
            let key = Value::number(index as f64);
            let value = self.raw_table_get(table, key)?;
            let result = first_result_or_nil(self.call_value(
                program,
                callback,
                &[key, value],
                depth,
                fuel,
            )?);
            if result.kind() != ValueKind::Nil {
                return Ok(result);
            }
        }
        Ok(Value::nil())
    }

    fn table_foreachi_len(&mut self, table: u32) -> Result<usize> {
        let n_key = Value::string_atom(self.heap.intern_string("n"));
        if let Some(n) = self.raw_table_get(table, n_key)?.as_number() {
            if n.is_finite() && n > 0.0 {
                let n = n.trunc();
                if n > MAX_LUAU_TABLE_INDEX {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "too many elements to iterate".to_owned(),
                    }));
                }
                return Ok(n as usize);
            }
            return Ok(0);
        }
        self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })
    }

    fn native_table_clone(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "table.clone argument")?;
        let table = args[0].as_table().ok_or_else(table_expected_error)?;
        if self.table_has_protected_metatable(table)? {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot clone protected metatable".to_owned(),
            }));
        }
        let clone = self.heap.table_clone(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table clone failed",
            })
        })?;
        Ok(Value::table(clone))
    }

    fn native_table_freeze(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "table.freeze argument")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        if self.table_has_protected_metatable(table)? {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "cannot freeze protected metatable".to_owned(),
            }));
        }
        let was_readonly = self.heap.table_freeze(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        if was_readonly {
            Err(readonly_table_error())
        } else {
            Ok(args[0])
        }
    }

    fn native_table_isfrozen(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, u8::MAX as usize, "table.isfrozen argument")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        let readonly = self.heap.table_is_readonly(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        Ok(Value::boolean(readonly))
    }

    fn native_table_insert(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 2, 3, "table.insert arguments")?;
        let registers = native_registers(args)?;
        self.table_insert(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
        )
    }

    fn native_table_move(&mut self, args: &[Value]) -> Result<Value> {
        let bounds = self.table_move_bounds(args)?;
        let count = table_move_count(bounds.start, bounds.end)?;
        if count == 0 {
            return Ok(Value::table(bounds.destination));
        }
        table_move_destination_end(bounds.destination_start, count)?;

        let copy_backward = bounds.source == bounds.destination
            && bounds.destination_start > bounds.start
            && bounds.destination_start <= bounds.end;
        if copy_backward {
            for offset in (0..count).rev() {
                self.move_table_slot(&bounds, offset)?;
            }
        } else {
            for offset in 0..count {
                self.move_table_slot(&bounds, offset)?;
            }
        }
        Ok(Value::table(bounds.destination))
    }

    fn table_move_bounds(&mut self, args: &[Value]) -> Result<TableMoveBounds> {
        validate_native_arity(args, 4, 5, "table.move arguments")?;
        let source = args[0].as_table().ok_or_else(table_expected_error)?;
        let start = table_integer_arg(number_arg(args[1])?, "integer table.move start")?;
        let end = table_integer_arg(number_arg(args[2])?, "integer table.move end")?;
        let destination_start =
            table_integer_arg(number_arg(args[3])?, "integer table.move destination")?;
        let destination = match args.get(4).copied() {
            None => source,
            Some(value) => value.as_table().ok_or_else(table_expected_error)?,
        };
        Ok(TableMoveBounds {
            source,
            destination,
            start,
            end,
            destination_start,
        })
    }

    fn move_table_slot(&mut self, bounds: &TableMoveBounds, offset: usize) -> Result<()> {
        let offset = i64::try_from(offset).map_err(|_| {
            Error::new(ErrorKind::RuntimeError {
                message: "too many elements to move".to_owned(),
            })
        })?;
        let source_index = i64::from(bounds.start) + offset;
        let destination_index = i64::from(bounds.destination_start) + offset;
        let value = self.raw_table_get(bounds.source, Value::number(source_index as f64))?;
        self.set_table_index(
            bounds.destination,
            Value::number(destination_index as f64),
            value,
        )
    }

    fn native_table_sort(
        &mut self,
        args: &[Value],
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Vec<Value>> {
        validate_native_arity(args, 1, u8::MAX as usize, "table.sort arguments")?;
        let table = args[0]
            .as_table()
            .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))?;
        self.ensure_table_writable(table)?;
        let comparator = args
            .get(1)
            .copied()
            .filter(|value| value.kind() != ValueKind::Nil);
        if comparator.is_some_and(|value| value.kind() != ValueKind::Function) {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "function",
            }));
        }
        let len = self.heap.array_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        if comparator.is_none() {
            match self.heap.table_sort_numeric_array(table, len) {
                Some(true) => return Ok(Vec::new()),
                Some(false) => {}
                None => {
                    return Err(Error::new(ErrorKind::InvalidProgram {
                        reason: "table numeric sort failed after validation",
                    }));
                }
            }
        }
        let mut values = Vec::with_capacity(len);
        for index in 1..=len {
            values.push(self.raw_table_get(table, Value::number(index as f64))?);
        }

        let initial_storage_len = self.heap.table_storage_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        self.table_sort_heap(
            &mut values,
            comparator,
            table,
            initial_storage_len,
            program,
            depth,
            fuel,
        )?;

        for (index, value) in values.into_iter().enumerate() {
            self.set_table_index(table, Value::number((index + 1) as f64), value)?;
        }
        Ok(Vec::new())
    }

    fn table_sort_heap(
        &mut self,
        values: &mut [Value],
        comparator: Option<Value>,
        table: u32,
        initial_storage_len: usize,
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        if values.len() < 2 {
            return Ok(());
        }

        let mut start = values.len() / 2;
        while start > 0 {
            start -= 1;
            self.table_sort_sift_down(
                values,
                start,
                values.len() - 1,
                comparator,
                table,
                initial_storage_len,
                program,
                depth,
                fuel,
            )?;
        }

        let mut end = values.len() - 1;
        while end > 0 {
            values.swap(0, end);
            end -= 1;
            self.table_sort_sift_down(
                values,
                0,
                end,
                comparator,
                table,
                initial_storage_len,
                program,
                depth,
                fuel,
            )?;
        }

        Ok(())
    }

    fn table_sort_sift_down(
        &mut self,
        values: &mut [Value],
        start: usize,
        end: usize,
        comparator: Option<Value>,
        table: u32,
        initial_storage_len: usize,
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<()> {
        let mut root = start;
        loop {
            let child = root.saturating_mul(2).saturating_add(1);
            if child > end {
                return Ok(());
            }

            let mut swap = root;
            if self.table_sort_less(
                values[swap],
                values[child],
                comparator,
                table,
                initial_storage_len,
                program,
                depth,
                fuel,
            )? {
                swap = child;
            }
            if child < end
                && self.table_sort_less(
                    values[swap],
                    values[child + 1],
                    comparator,
                    table,
                    initial_storage_len,
                    program,
                    depth,
                    fuel,
                )?
            {
                swap = child + 1;
            }

            if swap == root {
                return Ok(());
            }
            values.swap(root, swap);
            root = swap;
        }
    }

    fn table_sort_less(
        &mut self,
        lhs: Value,
        rhs: Value,
        comparator: Option<Value>,
        table: u32,
        initial_storage_len: usize,
        program: &Program,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        let result = if let Some(comparator) = comparator {
            let results =
                self.call_value_non_yieldable(program, comparator, &[lhs, rhs], depth, fuel)?;
            first_result_or_nil(results).truthy()
        } else {
            self.less_than(program, lhs, rhs, depth, fuel)?
        };
        let storage_len = self.heap.table_storage_len(table).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table handle is missing",
            })
        })?;
        if storage_len != initial_storage_len {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "table modified during sorting".to_owned(),
            }));
        }
        Ok(result)
    }

    fn native_table_remove(&mut self, args: &[Value]) -> Result<Value> {
        validate_native_arity(args, 1, 2, "table.remove arguments")?;
        let registers = native_registers(args)?;
        self.table_remove(
            &registers,
            Instruction::with_aux(Opcode::Halt, 0, 0, 1, args.len() as i32),
        )
    }

    fn string_sub(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(2..=3).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid string.sub arity",
            }));
        }
        let source = get_register(registers, instruction.b)?;
        let start = string_library_index(get_number(registers, instruction.c)?)?;
        let end = if instruction.aux == 3 {
            let register = instruction.c.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.sub end register overflow",
                })
            })?;
            string_library_index(get_number(registers, register)?)?
        } else {
            -1
        };
        if let Some(source_atom) = source.as_string_atom() {
            let len = self.heap.string_atom_byte_len(source_atom).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string atom is missing",
                })
            })?;
            let atom = match luau_string_sub_range(len, start, end)? {
                Some((start, end)) => self
                    .heap
                    .intern_string_atom_slice(source_atom, start, end)
                    .ok_or_else(|| {
                        Error::new(ErrorKind::InvalidProgram {
                            reason: "string.sub byte range is out of bounds",
                        })
                    })?,
                None => self.heap.intern_string_bytes(&[]),
            };
            return Ok(Value::string_atom(atom));
        }
        let text = self.value_to_string(source)?;
        let bytes = match luau_string_sub_range(text.len(), start, end)? {
            Some((start, end)) => text.get(start..end).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.sub byte range is out of bounds",
                })
            })?,
            None => &[],
        };
        let atom = self.heap.intern_string_bytes(bytes);
        Ok(Value::string_atom(atom))
    }

    fn string_byte_equal(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
    ) -> Result<Value> {
        if !(0..=0x1ff).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid string byte equality operand",
            }));
        }
        let source = get_register(registers, instruction.b)?;
        let index = string_library_index(get_number(registers, instruction.c)?)?;
        let expected = (instruction.aux & 0xff) as u8;
        let invert = (instruction.aux & 0x100) != 0;
        let matched = if let Some(source_atom) = source.as_string_atom() {
            let len = self.heap.string_atom_byte_len(source_atom).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string atom is missing",
                })
            })?;
            match luau_string_sub_range(len, index, index)? {
                Some((start, end)) if end == start + 1 => {
                    self.heap
                        .string_atom_byte_at(source_atom, start)
                        .ok_or_else(|| {
                            Error::new(ErrorKind::InvalidProgram {
                                reason: "string atom is missing",
                            })
                        })?
                        == Some(expected)
                }
                _ => false,
            }
        } else {
            let text = self.value_to_string(source)?;
            match luau_string_sub_range(text.len(), index, index)? {
                Some((start, end)) if end == start + 1 => {
                    text.get(start).copied() == Some(expected)
                }
                _ => false,
            }
        };
        Ok(Value::boolean(matched ^ invert))
    }

    fn string_byte(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        let results = self.string_byte_results(registers, instruction)?;
        Ok(first_result_or_nil(results))
    }

    fn string_byte_results(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
    ) -> Result<Vec<Value>> {
        if !(1..=3).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid string.byte arity",
            }));
        }
        let text = self.coerce_to_string(registers, instruction.b)?;
        let start = if instruction.aux >= 2 {
            string_library_index(get_number(registers, instruction.c)?)?
        } else {
            1
        };
        let end = if instruction.aux == 3 {
            let register = instruction.c.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.byte end register overflow",
                })
            })?;
            string_library_index(get_number(registers, register)?)?
        } else {
            start
        };
        let Some(start_index) =
            normalize_string_byte_start(text.len(), start, instruction.aux == 3)?
        else {
            return Ok(Vec::new());
        };
        let Some(end_index) = normalize_string_byte_end(text.len(), end)? else {
            return Ok(Vec::new());
        };
        if end_index < start_index {
            return Ok(Vec::new());
        }
        Ok(text[start_index..=end_index]
            .iter()
            .map(|byte| Value::number(f64::from(*byte)))
            .collect())
    }

    fn string_rep(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if instruction.aux < 2 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid string.rep arity",
            }));
        }
        let text = self.coerce_to_string(registers, instruction.b)?;
        let count = string_repeat_count(get_number(registers, instruction.c)?)?;
        let output = repeat_string(&text, count)?;
        let atom = self.heap.intern_string_bytes(&output);
        Ok(Value::string_atom(atom))
    }

    fn string_find(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(2..=4).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid string.find arity",
            }));
        }
        let text = self.coerce_to_string(registers, instruction.b)?;
        let pattern = self.coerce_to_string(registers, instruction.c)?;
        let start = if instruction.aux >= 3 {
            let register = instruction.c.checked_add(1).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.find init register overflow",
                })
            })?;
            string_library_index(self.coerce_register_to_number(registers, register)?)?
        } else {
            1
        };
        let plain = if instruction.aux >= 4 {
            let register = instruction.c.checked_add(2).ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "string.find plain register overflow",
                })
            })?;
            get_register(registers, register)?.truthy()
        } else {
            false
        };
        let result = luau_string_find_first(&text, &pattern, start, plain)?;
        Ok(result.map_or_else(Value::nil, |index| Value::number(index as f64)))
    }

    fn string_split(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(1..=2).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid string.split arity",
            }));
        }
        let text = self.coerce_to_string(registers, instruction.b)?;
        let separator = if instruction.aux == 2 {
            self.coerce_to_string(registers, instruction.c)?
        } else {
            b",".to_vec()
        };
        let parts = split_luau_string(&text, &separator)?;
        let values = parts
            .iter()
            .map(|part| Value::string_atom(self.heap.intern_string_bytes(part)))
            .collect::<Vec<_>>();
        Ok(Value::table(self.heap.alloc_array_table(&values)))
    }

    fn math_unary(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
        op: MathUnary,
    ) -> Result<Value> {
        if instruction.aux == 1 {
            let value = get_register(registers, instruction.b)?;
            if let Some(number) = value.as_number() {
                return Ok(Value::number(math_unary_result(number, op)));
            }
        }
        validate_math_arity(instruction, 1, "invalid one-argument math arity")?;
        let value = self.coerce_register_to_number(registers, instruction.b)?;
        Ok(Value::number(math_unary_result(value, op)))
    }

    fn math_sin_cos_mul(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if instruction.aux == 1 {
            let value = get_register(registers, instruction.b)?;
            if let Some(number) = value.as_number() {
                let (sin, cos) = libm::sincos(number);
                return Ok(Value::number(sin * cos));
            }
        }
        validate_math_arity(instruction, 1, "invalid one-argument math arity")?;
        let value = self.coerce_register_to_number(registers, instruction.b)?;
        let (sin, cos) = libm::sincos(value);
        Ok(Value::number(sin * cos))
    }

    fn math_binary(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
        op: MathBinary,
    ) -> Result<Value> {
        validate_math_arity(instruction, 2, "invalid two-argument math arity")?;
        let lhs = self.math_arg_number(registers, instruction.b, 0)?;
        let rhs = self.math_arg_number(registers, instruction.b, 1)?;
        let result = match op {
            MathBinary::Fmod => libm::fmod(lhs, rhs),
            MathBinary::Pow => libm::pow(lhs, rhs),
            MathBinary::Atan2 => libm::atan2(lhs, rhs),
        };
        Ok(Value::number(result))
    }

    fn math_log(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        if !(1..=2).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "invalid math.log arity",
            }));
        }
        let value = self.math_arg_number(registers, instruction.b, 0)?;
        let result = if instruction.aux == 1 {
            libm::log(value)
        } else {
            let base = self.math_arg_number(registers, instruction.b, 1)?;
            if base == 2.0 {
                libm::log2(value)
            } else if base == 10.0 {
                libm::log10(value)
            } else {
                libm::log(value) / libm::log(base)
            }
        };
        Ok(Value::number(result))
    }

    fn math_ldexp(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        validate_math_arity(instruction, 2, "invalid math.ldexp arity")?;
        let mantissa = self.math_arg_number(registers, instruction.b, 0)?;
        let exponent = self.math_arg_i32(registers, instruction.b, 1)?;
        Ok(Value::number(libm::ldexp(mantissa, exponent)))
    }

    fn math_predicate(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
        op: MathPredicate,
    ) -> Result<Value> {
        validate_math_arity(instruction, 1, "invalid one-argument math predicate arity")?;
        let value = self.coerce_register_to_number(registers, instruction.b)?;
        let result = match op {
            MathPredicate::Nan => value.is_nan(),
            MathPredicate::Inf => value.is_infinite(),
            MathPredicate::Finite => value.is_finite(),
        };
        Ok(Value::boolean(result))
    }

    fn math_clamp(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        validate_math_arity(instruction, 3, "invalid math.clamp arity")?;
        let value = self.math_arg_number(registers, instruction.b, 0)?;
        let min = self.math_arg_number(registers, instruction.b, 1)?;
        let max = self.math_arg_number(registers, instruction.b, 2)?;
        if min > max || min.is_nan() || max.is_nan() {
            return Err(Error::new(ErrorKind::TypeError {
                expected: "max must be greater than or equal to min",
            }));
        }
        let result = if value < min {
            min
        } else if value > max {
            max
        } else {
            value
        };
        Ok(Value::number(result))
    }

    fn math_map(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        validate_math_arity(instruction, 5, "invalid math.map arity")?;
        let x = self.math_arg_number(registers, instruction.b, 0)?;
        let in_min = self.math_arg_number(registers, instruction.b, 1)?;
        let in_max = self.math_arg_number(registers, instruction.b, 2)?;
        let out_min = self.math_arg_number(registers, instruction.b, 3)?;
        let out_max = self.math_arg_number(registers, instruction.b, 4)?;
        Ok(Value::number(
            out_min + (x - in_min) * (out_max - out_min) / (in_max - in_min),
        ))
    }

    fn math_lerp(&mut self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        validate_math_arity(instruction, 3, "invalid math.lerp arity")?;
        let a = self.math_arg_number(registers, instruction.b, 0)?;
        let b = self.math_arg_number(registers, instruction.b, 1)?;
        let t = self.math_arg_number(registers, instruction.b, 2)?;
        let result = if t == 1.0 { b } else { a + (b - a) * t };
        Ok(Value::number(result))
    }

    fn math_minmax(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
        op: MathMinMax,
    ) -> Result<Value> {
        let argc = validate_math_minmax_arity(instruction)?;
        let mut result = self.math_arg_number(registers, instruction.b, 0)?;
        for offset in 1..argc {
            let value = self.math_arg_number(registers, instruction.b, offset)?;
            match op {
                MathMinMax::Min if value < result => result = value,
                MathMinMax::Max if value > result => result = value,
                _ => {}
            }
        }
        Ok(Value::number(result))
    }

    fn math_arg_number(&mut self, registers: &[Value], first: u8, offset: u8) -> Result<f64> {
        let register = first.checked_add(offset).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "math argument register window overflow",
            })
        })?;
        self.coerce_register_to_number(registers, register)
    }

    fn math_arg_i32(&mut self, registers: &[Value], first: u8, offset: u8) -> Result<i32> {
        let value = self.math_arg_number(registers, first, offset)?;
        if value.is_finite()
            && value.fract() == 0.0
            && value >= f64::from(i32::MIN)
            && value <= f64::from(i32::MAX)
        {
            Ok(value as i32)
        } else {
            Err(Error::new(ErrorKind::TypeError {
                expected: "integer",
            }))
        }
    }

    fn concat_args(
        &mut self,
        program: &Program,
        registers: &[Value],
        first: u8,
        argc: u8,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<Value> {
        let first = first as usize;
        let argc = argc as usize;
        let end = first.checked_add(argc).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "concat argument window overflow",
            })
        })?;
        if end > registers.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: end,
                max: registers.len(),
            }));
        }
        let mut output = Vec::new();
        for (index, value) in registers[first..end].iter().copied().enumerate() {
            if index > 0 {
                output.push(b',');
            }
            output.extend_from_slice(
                &self.value_to_tostring_with_metamethod(program, value, depth, fuel)?,
            );
        }
        let atom = self.heap.intern_string_bytes(&output);
        Ok(Value::string_atom(atom))
    }

    fn ipairs_prepare(
        &mut self,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        pc: usize,
    ) -> Result<bool> {
        let window = ArrayIteratorRegisters::new(instruction.a)?;
        let write_visible_key = array_iterator_writes_visible_key(instruction)?;
        if !write_visible_key {
            clear_unused_array_iterator_key(registers, register_cells, window)
                .map_err(|e| e.at_pc(pc))?;
        }
        let table = get_table(registers, window.table).map_err(|e| e.at_pc(pc))?;
        let first = self.heap.array_get(table, 1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "ipairs table handle is missing",
            })
            .at_pc(pc)
        })?;
        if first.kind() == ValueKind::Nil {
            return Ok(false);
        }
        set_array_iterator_registers(
            registers,
            window,
            Value::number(1.0),
            first,
            write_visible_key,
        )
        .map_err(|e| e.at_pc(pc))?;
        clear_array_iterator_value_cells(register_cells, window, write_visible_key)
            .map_err(|e| e.at_pc(pc))?;
        Ok(true)
    }

    fn ipairs_loop(
        &mut self,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        pc: usize,
    ) -> Result<bool> {
        let window = ArrayIteratorRegisters::new(instruction.a)?;
        let write_visible_key = array_iterator_writes_visible_key(instruction)?;
        let table = get_table(registers, window.table).map_err(|e| e.at_pc(pc))?;
        let index = get_number(registers, window.index).map_err(|e| e.at_pc(pc))?;
        let next_index = index + 1.0;
        let Some(one_based_index) = array_iterator_index(next_index) else {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "ipairs internal index is not a positive integer",
            })
            .at_pc(pc));
        };
        let value = self.heap.array_get(table, one_based_index).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "ipairs table handle is missing",
            })
            .at_pc(pc)
        })?;
        if value.kind() == ValueKind::Nil {
            return Ok(false);
        }
        set_array_iterator_registers(
            registers,
            window,
            Value::number(next_index),
            value,
            write_visible_key,
        )
        .map_err(|e| e.at_pc(pc))?;
        clear_array_iterator_value_cells(register_cells, window, write_visible_key)
            .map_err(|e| e.at_pc(pc))?;
        Ok(true)
    }

    fn pairs_prepare(
        &mut self,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        pc: usize,
    ) -> Result<bool> {
        let window = ArrayIteratorRegisters::new(instruction.a)?;
        let write_visible_key = array_iterator_writes_visible_key(instruction)?;
        if !write_visible_key {
            clear_unused_array_iterator_key(registers, register_cells, window)
                .map_err(|e| e.at_pc(pc))?;
        }
        let table = get_table(registers, window.table).map_err(|e| e.at_pc(pc))?;
        let (key, value) = match self.table_next_after(table, Value::nil()) {
            Some(TableNext::Entry(key, value)) => (key, value),
            Some(TableNext::End) => return Ok(false),
            Some(TableNext::InvalidKey) => {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "pairs prepare received an invalid nil key",
                })
                .at_pc(pc));
            }
            None => {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "pairs table handle is missing",
                })
                .at_pc(pc));
            }
        };
        set_array_iterator_registers(registers, window, key, value, write_visible_key)
            .map_err(|e| e.at_pc(pc))?;
        clear_array_iterator_value_cells(register_cells, window, write_visible_key)
            .map_err(|e| e.at_pc(pc))?;
        Ok(true)
    }

    fn pairs_loop(
        &mut self,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        pc: usize,
    ) -> Result<bool> {
        let window = ArrayIteratorRegisters::new(instruction.a)?;
        let write_visible_key = array_iterator_writes_visible_key(instruction)?;
        let table = get_table(registers, window.table).map_err(|e| e.at_pc(pc))?;
        let index = get_register(registers, window.index).map_err(|e| e.at_pc(pc))?;
        let (key, value) = match self.table_next_after(table, index) {
            Some(TableNext::Entry(key, value)) => (key, value),
            Some(TableNext::End) => return Ok(false),
            Some(TableNext::InvalidKey) => {
                return Err(Error::new(ErrorKind::InvalidKeyToNext).at_pc(pc));
            }
            None => {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "pairs table handle is missing",
                })
                .at_pc(pc));
            }
        };
        set_array_iterator_registers(registers, window, key, value, write_visible_key)
            .map_err(|e| e.at_pc(pc))?;
        clear_array_iterator_value_cells(register_cells, window, write_visible_key)
            .map_err(|e| e.at_pc(pc))?;
        Ok(true)
    }

    fn generic_call_iterator_step(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        let window = CallIteratorRegisters::generic(instruction.a, instruction.c)?;
        window.validate(registers.len(), "generic call iterator")?;
        self.call_iterator_step(CallIteratorStepContext {
            program,
            registers,
            register_cells,
            window,
            depth,
            fuel,
            invalid_callee: InvalidIteratorCallee::Iterate,
        })
    }

    fn generalized_iterator_prepare(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        depth: usize,
        site: Option<InlineCacheSite>,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        let window = GeneralizedIteratorRegisters::new(instruction.a, instruction.c)?;
        window.validate(registers.len())?;

        let subject = get_register(registers, window.mode)?;
        match subject.kind() {
            ValueKind::Table => {
                let table = subject.as_table().ok_or_else(|| {
                    Error::new(ErrorKind::InvalidProgram {
                        reason: "table handle is missing",
                    })
                })?;
                if let Some(method) = self.table_iter_metamethod_with_inline_cache(site, table)? {
                    let results = self.call_value(program, method, &[subject], depth, fuel)?;
                    self.sync_register_cells(registers, register_cells)?;
                    self.prepare_call_iterator_from_results(registers, window.call, results)?;
                    return self.call_iterator_step(CallIteratorStepContext {
                        program,
                        registers,
                        register_cells,
                        window: window.call,
                        depth,
                        fuel,
                        invalid_callee: InvalidIteratorCallee::Call,
                    });
                }
                set_register(
                    registers,
                    window.mode,
                    Value::number(GENERALIZED_ITERATOR_MODE_TABLE),
                )?;
                set_register(registers, window.call.state, subject)?;
                set_register(registers, window.call.control, Value::nil())?;
                self.table_iterator_step(registers, register_cells, window.call)
            }
            ValueKind::Userdata => {
                if let Some(method) = self.metamethod_for_value(subject, "__iter")? {
                    let results = self.call_value(program, method, &[subject], depth, fuel)?;
                    self.sync_register_cells(registers, register_cells)?;
                    self.prepare_call_iterator_from_results(registers, window.call, results)?;
                    return self.call_iterator_step(CallIteratorStepContext {
                        program,
                        registers,
                        register_cells,
                        window: window.call,
                        depth,
                        fuel,
                        invalid_callee: InvalidIteratorCallee::Call,
                    });
                }
                Err(iteration_error(subject))
            }
            ValueKind::Function => {
                set_register(
                    registers,
                    window.mode,
                    Value::number(GENERALIZED_ITERATOR_MODE_CALL),
                )?;
                set_register(registers, window.call.iterator, subject)?;
                set_register(registers, window.call.state, Value::nil())?;
                set_register(registers, window.call.control, Value::nil())?;
                self.call_iterator_step(CallIteratorStepContext {
                    program,
                    registers,
                    register_cells,
                    window: window.call,
                    depth,
                    fuel,
                    invalid_callee: InvalidIteratorCallee::Call,
                })
            }
            _ => Err(iteration_error(subject)),
        }
    }

    fn generalized_iterator_loop(
        &mut self,
        program: &Program,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        instruction: Instruction,
        depth: usize,
        fuel: &mut Option<u64>,
    ) -> Result<bool> {
        let window = GeneralizedIteratorRegisters::new(instruction.a, instruction.c)?;
        window.validate(registers.len())?;
        let mode = get_number(registers, window.mode)?;
        if mode == GENERALIZED_ITERATOR_MODE_CALL {
            self.call_iterator_step(CallIteratorStepContext {
                program,
                registers,
                register_cells,
                window: window.call,
                depth,
                fuel,
                invalid_callee: InvalidIteratorCallee::Call,
            })
        } else if mode == GENERALIZED_ITERATOR_MODE_TABLE {
            self.table_iterator_step(registers, register_cells, window.call)
        } else {
            Err(Error::new(ErrorKind::InvalidProgram {
                reason: "generalized iterator mode is invalid",
            }))
        }
    }

    fn prepare_call_iterator_from_results(
        &mut self,
        registers: &mut [Value],
        window: CallIteratorRegisters,
        results: Vec<Value>,
    ) -> Result<()> {
        set_register(
            registers,
            window.mode,
            Value::number(GENERALIZED_ITERATOR_MODE_CALL),
        )?;
        set_register(
            registers,
            window.iterator,
            results.first().copied().unwrap_or_else(Value::nil),
        )?;
        set_register(
            registers,
            window.state,
            results.get(1).copied().unwrap_or_else(Value::nil),
        )?;
        set_register(
            registers,
            window.control,
            results.get(2).copied().unwrap_or_else(Value::nil),
        )
    }

    fn call_iterator_step(&mut self, context: CallIteratorStepContext<'_>) -> Result<bool> {
        let CallIteratorStepContext {
            program,
            registers,
            register_cells,
            window,
            depth,
            fuel,
            invalid_callee,
        } = context;
        let iterator = get_register(registers, window.iterator)?;
        if invalid_callee == InvalidIteratorCallee::Iterate
            && !self.iterator_is_callable(iterator)?
        {
            return Err(iteration_error(iterator));
        }
        let state = get_register(registers, window.state)?;
        let control = get_register(registers, window.control)?;
        let results = self.call_value(program, iterator, &[state, control], depth, fuel)?;
        self.sync_register_cells(registers, register_cells)?;

        let first = results.first().copied().unwrap_or_else(Value::nil);
        if first.kind() == ValueKind::Nil {
            self.recycle_results(results);
            return Ok(false);
        }

        set_register(registers, window.control, first)?;
        write_call_iterator_visible_results(registers, register_cells, window, &results)?;
        self.recycle_results(results);
        Ok(true)
    }

    fn iterator_is_callable(&mut self, iterator: Value) -> Result<bool> {
        match iterator.kind() {
            ValueKind::Function => Ok(true),
            ValueKind::Table | ValueKind::Userdata => self
                .metamethod_for_value(iterator, "__call")
                .map(|method| method.is_some()),
            _ => Ok(false),
        }
    }

    fn table_iterator_step(
        &mut self,
        registers: &mut [Value],
        register_cells: &mut [Option<u32>],
        window: CallIteratorRegisters,
    ) -> Result<bool> {
        let table = get_table(registers, window.state)?;
        let control = get_register(registers, window.control)?;
        let (key, value) = match self.table_next_after(table, control) {
            Some(TableNext::Entry(key, value)) => (key, value),
            Some(TableNext::End) => return Ok(false),
            Some(TableNext::InvalidKey) => return Err(Error::new(ErrorKind::InvalidKeyToNext)),
            None => {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "generalized iterator table handle is missing",
                }));
            }
        };
        set_register(registers, window.control, key)?;
        write_table_iterator_visible_results(registers, register_cells, window, key, value)?;
        Ok(true)
    }

    fn instruction_fuel_cost(&self, instruction: Instruction) -> Result<u64> {
        if instruction.op == Opcode::CallExtension {
            let op = self.extension(instruction)?;
            Ok(u64::from(op.fuel_cost))
        } else {
            Ok(1)
        }
    }

    fn string_find_opcode_fuel(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
    ) -> Result<u64> {
        if !(2..=4).contains(&instruction.aux) {
            return Ok(0);
        }
        let text = get_register(registers, instruction.b)?;
        let pattern = get_register(registers, instruction.c)?;
        Ok(self
            .value_string_len_for_fuel(text)
            .map_or(0, usize_to_u64)
            .saturating_add(
                self.value_string_len_for_fuel(pattern)
                    .map_or(0, usize_to_u64),
            ))
    }

    fn string_format_opcode_fuel(
        &mut self,
        registers: &[Value],
        instruction: Instruction,
    ) -> Result<u64> {
        let args = string_format_args(registers, instruction)?;
        Ok(args
            .first()
            .and_then(|value| self.value_string_len_for_fuel(*value))
            .map_or(0, usize_to_u64)
            .saturating_add(usize_to_u64(args.len())))
    }

    fn call_extension(&self, registers: &[Value], instruction: Instruction) -> Result<Value> {
        let op = self.extension(instruction)?;
        let first = instruction.b as usize;
        let argc = instruction.c as usize;
        let end = first.checked_add(argc).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "extension argument window overflow",
            })
        })?;
        if end > registers.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: end,
                max: registers.len(),
            }));
        }
        (op.handler)(&registers[first..end])
    }

    fn extension(&self, instruction: Instruction) -> Result<&ExtensionOp> {
        if !(0..=i32::from(u16::MAX)).contains(&instruction.aux) {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "extension op id out of range",
            }));
        }
        let id = instruction.aux as u16;
        self.extensions.get(id).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "unknown extension op id",
            })
        })
    }

    fn maybe_collect(
        &mut self,
        roots_dirty: &mut bool,
        registers: &[Value],
        varargs: &[Value],
        pending_return_prefixes: &[Value],
    ) {
        if self.profile.is_deterministic_sim() {
            if self
                .executed_instructions
                .is_multiple_of(self.settings.deterministic_gc_interval.get())
            {
                self.refresh_current_gc_roots_if_dirty(
                    roots_dirty,
                    registers,
                    varargs,
                    pending_return_prefixes,
                );
                self.heap.finish_collection_cycle();
                let roots = self.collectgarbage_roots(&[]);
                self.heap.collect_weak_tables_with_roots(&roots);
                self.deterministic_collection_count =
                    self.deterministic_collection_count.wrapping_add(1);
            }
        } else {
            self.heap.collect_debt();
            if !self.gc_stopped
                && self
                    .executed_instructions
                    .is_multiple_of(self.settings.deterministic_gc_interval.get())
            {
                self.refresh_current_gc_roots_if_dirty(
                    roots_dirty,
                    registers,
                    varargs,
                    pending_return_prefixes,
                );
                self.heap.finish_collection_cycle();
                let roots = self.collectgarbage_roots(&[]);
                self.heap.collect_weak_tables_with_roots(&roots);
            }
        }
    }

    fn record_coverage_hit(
        &mut self,
        program_id: Option<u32>,
        prototype: Option<u32>,
        function: &Function,
        pc: usize,
    ) {
        if !self.settings.enable_conformance_coverage {
            return;
        }
        let Some(prototype) = prototype else {
            return;
        };
        let Some(line) = function.source_line(pc) else {
            return;
        };
        let key = CoverageKey {
            program: program_id,
            prototype,
            line,
        };
        *self.coverage_hits.entry(key).or_insert(0) += 1;
    }
}

fn initialize_standard_globals(
    heap: &mut VmCell,
    globals: u32,
    profile: Profile,
    enable_loadstring: bool,
    enable_conformance_coverage: bool,
    enable_conformance_int64_userdata: bool,
    enable_conformance_native_userdata: bool,
    enable_conformance_native_type_checks: bool,
    enable_conformance_pcall_helpers: bool,
) -> StandardGlobalHandles {
    let standard_globals = heap.alloc_array_table(&[]);
    let mut conformance_int64_metatable = None;
    let mut conformance_vec2_metatable = None;
    let mut conformance_vertex_metatable = None;

    set_standard_field(heap, globals, standard_globals, "_G", Value::table(globals));
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "assert",
        NativeFunction::Assert,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "error",
        NativeFunction::Error,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "pcall",
        NativeFunction::PCall,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "xpcall",
        NativeFunction::XPCall,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "print",
        NativeFunction::Print,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "select",
        NativeFunction::Select,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "ipairs",
        NativeFunction::IPairs,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "pairs",
        NativeFunction::Pairs,
    );
    if enable_loadstring {
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "loadstring",
            NativeFunction::LoadString,
        );
    }
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "unpack",
        NativeFunction::TableUnpack,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "tonumber",
        NativeFunction::ToNumber,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "tostring",
        NativeFunction::ToString,
    );
    let type_value = native_function_value(heap, NativeFunction::TypeOf, globals);
    set_standard_field(heap, globals, standard_globals, "type", type_value);
    set_standard_field(heap, globals, standard_globals, "typeof", type_value);
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "getfenv",
        NativeFunction::GetFEnv,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "setfenv",
        NativeFunction::SetFEnv,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "setmetatable",
        NativeFunction::SetMetatable,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "getmetatable",
        NativeFunction::GetMetatable,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "newproxy",
        NativeFunction::NewProxy,
    );
    if !profile.is_deterministic_sim() {
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "collectgarbage",
            NativeFunction::CollectGarbage,
        );
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "gcinfo",
            NativeFunction::GcInfo,
        );
        let os = heap.alloc_array_table(&[]);
        for (name, native) in OS_NATIVE_FUNCTIONS {
            set_native_table_field(heap, os, name, *native);
        }
        set_standard_field(heap, globals, standard_globals, "os", Value::table(os));
    }
    let debug = heap.alloc_array_table(&[]);
    for (name, native) in DEBUG_NATIVE_FUNCTIONS {
        set_native_table_field(heap, debug, name, *native);
    }
    set_standard_field(
        heap,
        globals,
        standard_globals,
        "debug",
        Value::table(debug),
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "rawequal",
        NativeFunction::RawEqual,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "rawget",
        NativeFunction::RawGet,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "rawset",
        NativeFunction::RawSet,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "rawlen",
        NativeFunction::RawLen,
    );
    set_standard_native_field(
        heap,
        globals,
        standard_globals,
        "next",
        NativeFunction::Next,
    );
    if enable_conformance_coverage {
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "getcoverage",
            NativeFunction::GetCoverage,
        );
    }
    if enable_conformance_pcall_helpers {
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "cxxthrow",
            NativeFunction::ConformanceCxxThrow,
        );
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "resumeerror",
            NativeFunction::ConformanceResumeError,
        );
    }
    if enable_conformance_native_type_checks {
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "is_native",
            NativeFunction::ConformanceIsNative,
        );
    }
    if enable_conformance_int64_userdata {
        let metatable = build_conformance_int64_metatable(heap);
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "int64",
            NativeFunction::ConformanceInt64New,
        );
        conformance_int64_metatable = Some(metatable);
    }
    if enable_conformance_native_userdata {
        let vec2_metatable = build_conformance_vec2_metatable(heap);
        let vertex_metatable = build_conformance_vertex_metatable(heap);
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "vec2",
            NativeFunction::ConformanceVec2New,
        );
        set_standard_native_field(
            heap,
            globals,
            standard_globals,
            "vertex",
            NativeFunction::ConformanceVertexNew,
        );
        conformance_vec2_metatable = Some(vec2_metatable);
        conformance_vertex_metatable = Some(vertex_metatable);
    }

    let math = heap.alloc_array_table(&[]);
    set_table_field(heap, math, "huge", Value::number(f64::INFINITY));
    set_table_field(
        heap,
        math,
        "pi",
        Value::number(f64::from_bits(0x4009_21fb_5444_2d18)),
    );
    for (name, native) in MATH_NATIVE_FUNCTIONS {
        set_native_table_field(heap, math, name, *native);
    }
    set_standard_field(heap, globals, standard_globals, "math", Value::table(math));

    let string = heap.alloc_array_table(&[]);
    for (name, native) in STRING_NATIVE_FUNCTIONS {
        set_native_table_field(heap, string, name, *native);
    }
    set_standard_field(
        heap,
        globals,
        standard_globals,
        "string",
        Value::table(string),
    );

    let utf8 = heap.alloc_array_table(&[]);
    for (name, native) in UTF8_NATIVE_FUNCTIONS {
        set_native_table_field(heap, utf8, name, *native);
    }
    let charpattern = Value::string_atom(heap.intern_string_bytes(LUAU_UTF8_CHARPATTERN));
    set_table_field(heap, utf8, "charpattern", charpattern);
    set_standard_index_field(heap, standard_globals, "utf8", Value::table(utf8));

    let bit32 = heap.alloc_array_table(&[]);
    for (name, native) in BIT32_NATIVE_FUNCTIONS {
        set_native_table_field(heap, bit32, name, *native);
    }
    set_standard_index_field(heap, standard_globals, "bit32", Value::table(bit32));

    let buffer = heap.alloc_array_table(&[]);
    for (name, native) in BUFFER_NATIVE_FUNCTIONS {
        set_native_table_field(heap, buffer, name, *native);
    }
    set_standard_index_field(heap, standard_globals, "buffer", Value::table(buffer));

    let vector = heap.alloc_array_table(&[]);
    for (name, native) in VECTOR_NATIVE_FUNCTIONS {
        set_native_table_field(heap, vector, name, *native);
    }
    let vector_zero = Value::vector(
        heap.alloc_vector(VectorData::new(0.0, 0.0, 0.0))
            .expect("standard vector handle fits in u32"),
    );
    let vector_one = Value::vector(
        heap.alloc_vector(VectorData::new(1.0, 1.0, 1.0))
            .expect("standard vector handle fits in u32"),
    );
    set_table_field(heap, vector, "zero", vector_zero);
    set_table_field(heap, vector, "one", vector_one);
    let vector_metatable = heap.alloc_array_table(&[]);
    let vector_call = native_function_value(heap, NativeFunction::VectorCall, vector);
    set_table_field(heap, vector_metatable, "__call", vector_call);
    heap.table_set_metatable(vector, Some(vector_metatable))
        .expect("standard vector table and metatable handles are valid");
    set_standard_index_field(heap, standard_globals, "vector", Value::table(vector));

    let table = heap.alloc_array_table(&[]);
    for (name, native) in TABLE_NATIVE_FUNCTIONS {
        set_native_table_field(heap, table, name, *native);
    }
    set_standard_field(
        heap,
        globals,
        standard_globals,
        "table",
        Value::table(table),
    );

    let coroutine = heap.alloc_array_table(&[]);
    for (name, native) in COROUTINE_NATIVE_FUNCTIONS {
        set_native_table_field(heap, coroutine, name, *native);
    }
    set_standard_field(
        heap,
        globals,
        standard_globals,
        "coroutine",
        Value::table(coroutine),
    );

    let rtti = build_standard_global_rtti(heap, profile, enable_loadstring);
    set_standard_index_field(heap, standard_globals, "RTTI", Value::table(rtti));

    let metatable = heap.alloc_array_table(&[]);
    set_table_field(heap, metatable, "__index", Value::table(standard_globals));
    set_table_field(heap, metatable, "__metatable", Value::table(metatable));
    heap.table_freeze(standard_globals)
        .expect("standard globals handle is valid");
    heap.table_freeze(metatable)
        .expect("standard globals metatable handle is valid");
    heap.table_set_metatable(globals, Some(metatable))
        .expect("standard globals and metatable handles are valid");
    StandardGlobalHandles {
        standard_globals,
        conformance_int64_metatable,
        conformance_vec2_metatable,
        conformance_vertex_metatable,
    }
}

fn build_conformance_int64_metatable(heap: &mut VmCell) -> u32 {
    let metatable = heap.alloc_array_table(&[]);
    for (name, native) in CONFORMANCE_INT64_NATIVE_FUNCTIONS {
        set_native_table_field(heap, metatable, name, *native);
    }
    metatable
}

fn build_conformance_vec2_metatable(heap: &mut VmCell) -> u32 {
    let metatable = heap.alloc_array_table(&[]);
    for (name, native) in CONFORMANCE_VEC2_NATIVE_FUNCTIONS {
        set_native_table_field(heap, metatable, name, *native);
    }
    metatable
}

fn build_conformance_vertex_metatable(heap: &mut VmCell) -> u32 {
    let metatable = heap.alloc_array_table(&[]);
    for (name, native) in CONFORMANCE_VERTEX_NATIVE_FUNCTIONS {
        set_native_table_field(heap, metatable, name, *native);
    }
    metatable
}

fn set_table_field(heap: &mut VmCell, table: u32, name: &str, value: Value) {
    let key = Value::string_atom(heap.intern_string(name));
    heap.table_set(table, key, value)
        .expect("standard global table fields use valid string keys");
}

fn set_standard_field(
    heap: &mut VmCell,
    globals: u32,
    standard_globals: u32,
    name: &str,
    value: Value,
) {
    set_table_field(heap, globals, name, value);
    set_table_field(heap, standard_globals, name, value);
}

fn set_standard_index_field(heap: &mut VmCell, standard_globals: u32, name: &str, value: Value) {
    set_table_field(heap, standard_globals, name, value);
}

fn set_standard_native_field(
    heap: &mut VmCell,
    globals: u32,
    standard_globals: u32,
    name: &str,
    native: NativeFunction,
) {
    let value = native_function_value(heap, native, globals);
    set_standard_field(heap, globals, standard_globals, name, value);
}

fn set_native_table_field(heap: &mut VmCell, table: u32, name: &str, native: NativeFunction) {
    let value = native_function_value(heap, native, table);
    set_table_field(heap, table, name, value);
}

fn native_function_value(heap: &mut VmCell, native: NativeFunction, environment: u32) -> Value {
    Value::function(
        heap.alloc_native_function(native, environment)
            .expect("standard native function handle fits in u32"),
    )
}

fn build_standard_global_rtti(heap: &mut VmCell, profile: Profile, enable_loadstring: bool) -> u32 {
    let rtti = heap.alloc_array_table(&[]);
    set_rtti_table_type(heap, rtti, "_G");
    for (name, _) in GLOBAL_FUNCTION_TYPES {
        set_rtti_function_type(heap, rtti, name);
    }
    if enable_loadstring {
        set_rtti_function_type(heap, rtti, "loadstring");
    }
    if !profile.is_deterministic_sim() {
        set_rtti_function_type(heap, rtti, "collectgarbage");
        let os = build_native_function_rtti(heap, OS_NATIVE_FUNCTIONS);
        set_table_field(heap, rtti, "os", Value::table(os));
    }
    let debug = build_native_function_rtti(heap, DEBUG_NATIVE_FUNCTIONS);
    set_table_field(heap, rtti, "debug", Value::table(debug));

    let math = build_native_function_rtti(heap, MATH_NATIVE_FUNCTIONS);
    set_rtti_number_type(heap, math, "huge");
    set_rtti_number_type(heap, math, "pi");
    set_table_field(heap, rtti, "math", Value::table(math));

    let string = build_native_function_rtti(heap, STRING_NATIVE_FUNCTIONS);
    set_table_field(heap, rtti, "string", Value::table(string));

    let utf8 = build_native_function_rtti(heap, UTF8_NATIVE_FUNCTIONS);
    set_rtti_string_type(heap, utf8, "charpattern");
    set_table_field(heap, rtti, "utf8", Value::table(utf8));

    let bit32 = build_native_function_rtti(heap, BIT32_NATIVE_FUNCTIONS);
    set_table_field(heap, rtti, "bit32", Value::table(bit32));

    let buffer = build_native_function_rtti(heap, BUFFER_NATIVE_FUNCTIONS);
    set_table_field(heap, rtti, "buffer", Value::table(buffer));

    let vector = build_native_function_rtti(heap, VECTOR_NATIVE_FUNCTIONS);
    set_rtti_field(heap, vector, "zero", "vector");
    set_rtti_field(heap, vector, "one", "vector");
    set_table_field(heap, rtti, "vector", Value::table(vector));

    let table = build_native_function_rtti(heap, TABLE_NATIVE_FUNCTIONS);
    set_table_field(heap, rtti, "table", Value::table(table));

    let coroutine = build_native_function_rtti(heap, COROUTINE_NATIVE_FUNCTIONS);
    set_table_field(heap, rtti, "coroutine", Value::table(coroutine));

    rtti
}

fn build_native_function_rtti(heap: &mut VmCell, fields: &[(&str, NativeFunction)]) -> u32 {
    let table = heap.alloc_array_table(&[]);
    for (name, _) in fields {
        set_rtti_function_type(heap, table, name);
    }
    table
}

fn set_rtti_table_type(heap: &mut VmCell, table: u32, name: &str) {
    set_rtti_field(heap, table, name, "table");
}

fn set_rtti_function_type(heap: &mut VmCell, table: u32, name: &str) {
    set_rtti_field(heap, table, name, "function");
}

fn set_rtti_number_type(heap: &mut VmCell, table: u32, name: &str) {
    set_rtti_field(heap, table, name, "number");
}

fn set_rtti_string_type(heap: &mut VmCell, table: u32, name: &str) {
    set_rtti_field(heap, table, name, "string");
}

fn set_rtti_field(heap: &mut VmCell, table: u32, name: &str, type_name: &str) {
    let value = Value::string_atom(heap.intern_string(type_name));
    set_table_field(heap, table, name, value);
}

const GLOBAL_FUNCTION_TYPES: &[(&str, NativeFunction)] = &[
    ("assert", NativeFunction::Assert),
    ("error", NativeFunction::Error),
    ("pcall", NativeFunction::PCall),
    ("xpcall", NativeFunction::XPCall),
    ("print", NativeFunction::Print),
    ("select", NativeFunction::Select),
    ("ipairs", NativeFunction::IPairs),
    ("pairs", NativeFunction::Pairs),
    ("unpack", NativeFunction::TableUnpack),
    ("tonumber", NativeFunction::ToNumber),
    ("tostring", NativeFunction::ToString),
    ("type", NativeFunction::TypeOf),
    ("typeof", NativeFunction::TypeOf),
    ("getfenv", NativeFunction::GetFEnv),
    ("setfenv", NativeFunction::SetFEnv),
    ("setmetatable", NativeFunction::SetMetatable),
    ("getmetatable", NativeFunction::GetMetatable),
    ("newproxy", NativeFunction::NewProxy),
    ("gcinfo", NativeFunction::GcInfo),
    ("rawequal", NativeFunction::RawEqual),
    ("rawget", NativeFunction::RawGet),
    ("rawset", NativeFunction::RawSet),
    ("rawlen", NativeFunction::RawLen),
    ("next", NativeFunction::Next),
];

const MATH_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("abs", NativeFunction::MathAbs),
    ("floor", NativeFunction::MathFloor),
    ("ceil", NativeFunction::MathCeil),
    ("round", NativeFunction::MathRound),
    ("sign", NativeFunction::MathSign),
    ("clamp", NativeFunction::MathClamp),
    ("map", NativeFunction::MathMap),
    ("lerp", NativeFunction::MathLerp),
    ("min", NativeFunction::MathMin),
    ("max", NativeFunction::MathMax),
    ("fmod", NativeFunction::MathFmod),
    ("pow", NativeFunction::MathPow),
    ("sqrt", NativeFunction::MathSqrt),
    ("deg", NativeFunction::MathDeg),
    ("rad", NativeFunction::MathRad),
    ("isnan", NativeFunction::MathIsNan),
    ("isinf", NativeFunction::MathIsInf),
    ("isfinite", NativeFunction::MathIsFinite),
    ("sin", NativeFunction::MathSin),
    ("cos", NativeFunction::MathCos),
    ("tan", NativeFunction::MathTan),
    ("asin", NativeFunction::MathAsin),
    ("acos", NativeFunction::MathAcos),
    ("atan", NativeFunction::MathAtan),
    ("atan2", NativeFunction::MathAtan2),
    ("sinh", NativeFunction::MathSinh),
    ("cosh", NativeFunction::MathCosh),
    ("tanh", NativeFunction::MathTanh),
    ("exp", NativeFunction::MathExp),
    ("log", NativeFunction::MathLog),
    ("log10", NativeFunction::MathLog10),
    ("ldexp", NativeFunction::MathLdexp),
    ("frexp", NativeFunction::MathFrexp),
    ("modf", NativeFunction::MathModf),
    ("random", NativeFunction::MathRandom),
    ("randomseed", NativeFunction::MathRandomSeed),
    ("noise", NativeFunction::MathNoise),
];

const STRING_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("sub", NativeFunction::StringSub),
    ("len", NativeFunction::StringLen),
    ("byte", NativeFunction::StringByte),
    ("char", NativeFunction::StringChar),
    ("upper", NativeFunction::StringUpper),
    ("lower", NativeFunction::StringLower),
    ("reverse", NativeFunction::StringReverse),
    ("rep", NativeFunction::StringRep),
    ("find", NativeFunction::StringFind),
    ("match", NativeFunction::StringMatch),
    ("gsub", NativeFunction::StringGsub),
    ("gmatch", NativeFunction::StringGMatch),
    ("split", NativeFunction::StringSplit),
    ("format", NativeFunction::StringFormat),
    ("pack", NativeFunction::StringPack),
    ("unpack", NativeFunction::StringUnpack),
    ("packsize", NativeFunction::StringPackSize),
];

const UTF8_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("char", NativeFunction::Utf8Char),
    ("codepoint", NativeFunction::Utf8Codepoint),
    ("codes", NativeFunction::Utf8Codes),
    ("len", NativeFunction::Utf8Len),
    ("offset", NativeFunction::Utf8Offset),
];

const BIT32_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("arshift", NativeFunction::Bit32ArShift),
    ("band", NativeFunction::Bit32Band),
    ("bnot", NativeFunction::Bit32BNot),
    ("bor", NativeFunction::Bit32Bor),
    ("btest", NativeFunction::Bit32BTest),
    ("bxor", NativeFunction::Bit32BXor),
    ("byteswap", NativeFunction::Bit32Byteswap),
    ("countlz", NativeFunction::Bit32CountLz),
    ("countrz", NativeFunction::Bit32CountRz),
    ("extract", NativeFunction::Bit32Extract),
    ("lrotate", NativeFunction::Bit32LRotate),
    ("lshift", NativeFunction::Bit32LShift),
    ("replace", NativeFunction::Bit32Replace),
    ("rrotate", NativeFunction::Bit32RRotate),
    ("rshift", NativeFunction::Bit32RShift),
];

const OS_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("clock", NativeFunction::OsClock),
    ("date", NativeFunction::OsDate),
    ("time", NativeFunction::OsTime),
    ("difftime", NativeFunction::OsDiffTime),
];

const DEBUG_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("traceback", NativeFunction::DebugTraceback),
    ("info", NativeFunction::DebugInfo),
];

const BUFFER_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("create", NativeFunction::BufferCreate),
    ("len", NativeFunction::BufferLen),
    ("fromstring", NativeFunction::BufferFromString),
    ("tostring", NativeFunction::BufferToString),
    ("readi8", NativeFunction::BufferReadI8),
    ("readu8", NativeFunction::BufferReadU8),
    ("readi16", NativeFunction::BufferReadI16),
    ("readu16", NativeFunction::BufferReadU16),
    ("readi32", NativeFunction::BufferReadI32),
    ("readu32", NativeFunction::BufferReadU32),
    ("readf32", NativeFunction::BufferReadF32),
    ("readf64", NativeFunction::BufferReadF64),
    ("readstring", NativeFunction::BufferReadString),
    ("readbits", NativeFunction::BufferReadBits),
    ("writei8", NativeFunction::BufferWriteI8),
    ("writeu8", NativeFunction::BufferWriteU8),
    ("writei16", NativeFunction::BufferWriteI16),
    ("writeu16", NativeFunction::BufferWriteU16),
    ("writei32", NativeFunction::BufferWriteI32),
    ("writeu32", NativeFunction::BufferWriteU32),
    ("writef32", NativeFunction::BufferWriteF32),
    ("writef64", NativeFunction::BufferWriteF64),
    ("writestring", NativeFunction::BufferWriteString),
    ("writebits", NativeFunction::BufferWriteBits),
    ("copy", NativeFunction::BufferCopy),
    ("fill", NativeFunction::BufferFill),
];

const VECTOR_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("create", NativeFunction::VectorCreate),
    ("dot", NativeFunction::VectorDot),
    ("cross", NativeFunction::VectorCross),
    ("normalize", NativeFunction::VectorNormalize),
    ("magnitude", NativeFunction::VectorMagnitude),
    ("abs", NativeFunction::VectorAbs),
    ("floor", NativeFunction::VectorFloor),
    ("ceil", NativeFunction::VectorCeil),
    ("sign", NativeFunction::VectorSign),
    ("angle", NativeFunction::VectorAngle),
    ("min", NativeFunction::VectorMin),
    ("max", NativeFunction::VectorMax),
    ("clamp", NativeFunction::VectorClamp),
    ("lerp", NativeFunction::VectorLerp),
];

const TABLE_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("clear", NativeFunction::TableClear),
    ("concat", NativeFunction::TableConcat),
    ("pack", NativeFunction::TablePack),
    ("unpack", NativeFunction::TableUnpack),
    ("getn", NativeFunction::TableGetN),
    ("maxn", NativeFunction::TableMaxN),
    ("create", NativeFunction::TableCreate),
    ("find", NativeFunction::TableFind),
    ("foreach", NativeFunction::TableForeach),
    ("foreachi", NativeFunction::TableForeachi),
    ("clone", NativeFunction::TableClone),
    ("freeze", NativeFunction::TableFreeze),
    ("insert", NativeFunction::TableInsert),
    ("isfrozen", NativeFunction::TableIsFrozen),
    ("move", NativeFunction::TableMove),
    ("remove", NativeFunction::TableRemove),
    ("sort", NativeFunction::TableSort),
];

const COROUTINE_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("create", NativeFunction::CoroutineCreate),
    ("resume", NativeFunction::CoroutineResume),
    ("wrap", NativeFunction::CoroutineWrap),
    ("yield", NativeFunction::CoroutineYield),
    ("status", NativeFunction::CoroutineStatus),
    ("running", NativeFunction::CoroutineRunning),
    ("isyieldable", NativeFunction::CoroutineIsYieldable),
    ("close", NativeFunction::CoroutineClose),
];

const CONFORMANCE_INT64_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("__eq", NativeFunction::ConformanceInt64Eq),
    ("__lt", NativeFunction::ConformanceInt64Lt),
    ("__le", NativeFunction::ConformanceInt64Le),
    ("__unm", NativeFunction::ConformanceInt64Neg),
    ("__add", NativeFunction::ConformanceInt64Add),
    ("__sub", NativeFunction::ConformanceInt64Sub),
    ("__mul", NativeFunction::ConformanceInt64Mul),
    ("__div", NativeFunction::ConformanceInt64Div),
    ("__idiv", NativeFunction::ConformanceInt64FloorDiv),
    ("__mod", NativeFunction::ConformanceInt64Mod),
    ("__pow", NativeFunction::ConformanceInt64Pow),
    ("__tostring", NativeFunction::ConformanceInt64ToString),
    ("__index", NativeFunction::ConformanceInt64Index),
    ("__newindex", NativeFunction::ConformanceInt64NewIndex),
];

const CONFORMANCE_VEC2_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] = &[
    ("__unm", NativeFunction::ConformanceVec2Neg),
    ("__add", NativeFunction::ConformanceVec2Add),
    ("__mul", NativeFunction::ConformanceVec2Mul),
    ("__index", NativeFunction::ConformanceVec2Index),
];

const CONFORMANCE_VERTEX_NATIVE_FUNCTIONS: &[(&str, NativeFunction)] =
    &[("__index", NativeFunction::ConformanceVertexIndex)];

fn native_function_debug_name(native: NativeFunction) -> &'static str {
    let base_name = match native {
        NativeFunction::Assert => "assert",
        NativeFunction::Error => "error",
        NativeFunction::PCall => "pcall",
        NativeFunction::XPCall => "xpcall",
        NativeFunction::Print => "print",
        NativeFunction::Select => "select",
        NativeFunction::IPairs => "ipairs",
        NativeFunction::INext => "inext",
        NativeFunction::Pairs => "pairs",
        NativeFunction::LoadString => "loadstring",
        NativeFunction::ToNumber => "tonumber",
        NativeFunction::ToString => "tostring",
        NativeFunction::TypeOf => "type",
        NativeFunction::GetFEnv => "getfenv",
        NativeFunction::SetFEnv => "setfenv",
        NativeFunction::SetMetatable => "setmetatable",
        NativeFunction::GetMetatable => "getmetatable",
        NativeFunction::NewProxy => "newproxy",
        NativeFunction::CollectGarbage => "collectgarbage",
        NativeFunction::GcInfo => "gcinfo",
        NativeFunction::RawEqual => "rawequal",
        NativeFunction::RawGet => "rawget",
        NativeFunction::RawSet => "rawset",
        NativeFunction::RawLen => "rawlen",
        NativeFunction::Next => "next",
        NativeFunction::GetCoverage => "getcoverage",
        NativeFunction::ConformanceCxxThrow => "cxxthrow",
        NativeFunction::ConformanceResumeError => "resumeerror",
        NativeFunction::ConformanceIsNative => "is_native",
        _ => "",
    };
    if !base_name.is_empty() {
        return base_name;
    }
    for table in [
        MATH_NATIVE_FUNCTIONS,
        STRING_NATIVE_FUNCTIONS,
        UTF8_NATIVE_FUNCTIONS,
        BIT32_NATIVE_FUNCTIONS,
        OS_NATIVE_FUNCTIONS,
        DEBUG_NATIVE_FUNCTIONS,
        BUFFER_NATIVE_FUNCTIONS,
        VECTOR_NATIVE_FUNCTIONS,
        TABLE_NATIVE_FUNCTIONS,
        COROUTINE_NATIVE_FUNCTIONS,
    ] {
        for (name, candidate) in table {
            if *candidate == native {
                return name;
            }
        }
    }
    ""
}

fn validate_native_arity(
    args: &[Value],
    min: usize,
    max: usize,
    expected: &'static str,
) -> Result<()> {
    if args.len() < min || args.len() > max {
        Err(Error::new(ErrorKind::TypeError { expected }))
    } else {
        Ok(())
    }
}

fn conformance_int64_from_number(number: f64) -> Result<i64> {
    if !number.is_finite() || number.fract() != 0.0 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer",
        }));
    }
    if number < i64::MIN as f64 || number > i64::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer",
        }));
    }
    Ok(number as i64)
}

fn conformance_int64_floor_div(lhs: i64, rhs: i64) -> Result<i64> {
    if rhs == 0 {
        return Err(divide_by_zero_error());
    }
    let quotient = lhs / rhs;
    let remainder = lhs % rhs;
    if remainder != 0 && ((remainder > 0) != (rhs > 0)) {
        Ok(quotient - 1)
    } else {
        Ok(quotient)
    }
}

fn conformance_int64_mod(lhs: i64, rhs: i64) -> Result<i64> {
    let quotient = conformance_int64_floor_div(lhs, rhs)?;
    Ok(lhs - quotient * rhs)
}

fn conformance_int64_pow(lhs: i64, rhs: i64) -> Result<i64> {
    let exponent = u32::try_from(rhs).map_err(|_| {
        Error::new(ErrorKind::TypeError {
            expected: "non-negative exponent",
        })
    })?;
    Ok(lhs.wrapping_pow(exponent))
}

fn divide_by_zero_error() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: "attempt to divide by zero".to_owned(),
    })
}

fn runtime_type_name(runtime_type: &RuntimeType) -> &'static str {
    match runtime_type {
        RuntimeType::Any => "any",
        RuntimeType::Nil => "nil",
        RuntimeType::Number => "number",
        RuntimeType::String | RuntimeType::OptionalString => "string",
        RuntimeType::Table => "table",
        RuntimeType::Function => "function",
        RuntimeType::Thread => "thread",
        RuntimeType::Userdata => "userdata",
        RuntimeType::Vector => "vector",
        RuntimeType::Buffer | RuntimeType::OptionalBuffer => "buffer",
    }
}

fn debug_info_error(message: &'static str) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: message.to_owned(),
    })
}

fn stackless_self_call_debug_frame(
    context: &FunctionCallContext<'_>,
    function: &Function,
    call_site: Option<ErrorLocation>,
) -> StacklessDebugFrame {
    StacklessDebugFrame {
        function: context.current_function,
        debug_name: function.debug_name.clone(),
        environment: context.environment,
        call_site,
    }
}

fn stackless_debug_frame_mark(frame: &CallStackFrame) -> StacklessDebugMark {
    StacklessDebugMark {
        run_count: frame.stackless_debug_frames.len(),
        last_run_count: frame
            .stackless_debug_frames
            .last()
            .map_or(0, |run| run.count),
    }
}

fn push_stackless_debug_frame_run(frame: &mut CallStackFrame, debug_frame: &StacklessDebugFrame) {
    if let Some(run) = frame.stackless_debug_frames.last_mut() {
        if stackless_debug_frames_match(&run.frame, debug_frame) {
            run.count += 1;
            return;
        }
    }
    frame.stackless_debug_frames.push(StacklessDebugFrameRun {
        frame: debug_frame.clone(),
        count: 1,
    });
}

fn stackless_debug_frames_match(lhs: &StacklessDebugFrame, rhs: &StacklessDebugFrame) -> bool {
    option_values_luau_equal(lhs.function, rhs.function)
        && lhs.debug_name == rhs.debug_name
        && lhs.environment == rhs.environment
        && lhs.call_site == rhs.call_site
}

fn option_values_luau_equal(lhs: Option<Value>, rhs: Option<Value>) -> bool {
    match (lhs, rhs) {
        (Some(lhs), Some(rhs)) => lhs.luau_equals(rhs),
        (None, None) => true,
        _ => false,
    }
}

fn debug_only_call_stack_frame(frame: &CallStackFrame) -> CallStackFrame {
    CallStackFrame {
        function: frame.function,
        debug_name: frame.debug_name.clone(),
        environment: frame.environment,
        call_site: frame.call_site.clone(),
        roots: Vec::new(),
        stackless_roots: Vec::new(),
        stackless_debug_frames: frame.stackless_debug_frames.clone(),
    }
}

fn debug_info_level_arg(value: Value) -> Result<usize> {
    let Some(level) = value.as_number() else {
        return Err(debug_info_error("function or level expected"));
    };
    if level < 0.0 {
        return Err(debug_info_error("level can't be negative"));
    }
    if !level.is_finite() || level.fract() != 0.0 || level > usize::MAX as f64 {
        return Err(debug_info_error("level expected"));
    }
    Ok(level as usize)
}

fn logical_debug_frames(frames: &[CallStackFrame]) -> Vec<StacklessDebugFrame> {
    let capacity = frames
        .iter()
        .map(|frame| {
            1 + frame
                .stackless_debug_frames
                .iter()
                .map(|run| run.count)
                .sum::<usize>()
        })
        .sum();
    let mut logical = Vec::with_capacity(capacity);
    for frame in frames {
        logical.push(StacklessDebugFrame {
            function: frame.function,
            debug_name: frame.debug_name.clone(),
            environment: frame.environment,
            call_site: frame.call_site.clone(),
        });
        for run in &frame.stackless_debug_frames {
            for _ in 0..run.count {
                logical.push(run.frame.clone());
            }
        }
    }
    logical
}

fn debug_info_logical_frame_location(
    frames: &[StacklessDebugFrame],
    frame_index: usize,
    top_location: Option<ErrorLocation>,
) -> Option<ErrorLocation> {
    if frame_index + 1 == frames.len() {
        return top_location.or_else(|| {
            frames
                .get(frame_index)
                .and_then(|frame| frame.call_site.clone())
        });
    }
    frames
        .get(frame_index + 1)
        .and_then(|frame| frame.call_site.clone())
        .or_else(|| {
            frames
                .get(frame_index)
                .and_then(|frame| frame.call_site.clone())
        })
}

fn debug_info_native_target(function: Value, native: NativeFunction) -> DebugInfoTarget {
    DebugInfoTarget {
        name: native_function_debug_name(native).to_owned(),
        source: "[C]".to_owned(),
        line: -1,
        function,
        parameter_count: 0,
        is_vararg: true,
    }
}

fn debug_info_bytecode_target(
    function_value: Value,
    source_name: Option<&str>,
    function: &Function,
) -> DebugInfoTarget {
    DebugInfoTarget {
        name: function
            .debug_name
            .as_ref()
            .map_or_else(String::new, ToString::to_string),
        source: source_name.unwrap_or_default().to_owned(),
        line: function.debug_line_defined.map_or(-1, u32_line_to_i32),
        function: function_value,
        parameter_count: function.parameter_count,
        is_vararg: function.is_vararg,
    }
}

fn u32_line_to_i32(line: u32) -> i32 {
    i32::try_from(line).unwrap_or(i32::MAX)
}

fn buffer_native_returns_no_values(native: NativeFunction) -> bool {
    matches!(
        native,
        NativeFunction::BufferWriteI8
            | NativeFunction::BufferWriteU8
            | NativeFunction::BufferWriteI16
            | NativeFunction::BufferWriteU16
            | NativeFunction::BufferWriteI32
            | NativeFunction::BufferWriteU32
            | NativeFunction::BufferWriteF32
            | NativeFunction::BufferWriteF64
            | NativeFunction::BufferWriteString
            | NativeFunction::BufferCopy
            | NativeFunction::BufferFill
            | NativeFunction::BufferWriteBits
    )
}

fn buffer_read_size(kind: BufferReadKind) -> usize {
    match kind {
        BufferReadKind::I8 | BufferReadKind::U8 => 1,
        BufferReadKind::I16 | BufferReadKind::U16 => 2,
        BufferReadKind::I32 | BufferReadKind::U32 => 4,
    }
}

fn buffer_float_size(kind: BufferFloatKind) -> usize {
    match kind {
        BufferFloatKind::F32 => 4,
        BufferFloatKind::F64 => 8,
    }
}

fn buffer_out_of_bounds() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: "buffer access out of bounds".to_owned(),
    })
}

#[derive(Clone, Copy)]
struct OsDateParts {
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
    wday: i64,
    yday: i64,
}

fn os_date_parts(timestamp: i64) -> OsDateParts {
    let days = timestamp.div_euclid(86_400);
    let seconds = timestamp.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let yday = days - days_from_civil(year, 1, 1) + 1;
    OsDateParts {
        year,
        month,
        day,
        hour: seconds / 3_600,
        minute: (seconds % 3_600) / 60,
        second: seconds % 60,
        wday: (days + 4).rem_euclid(7),
        yday,
    }
}

fn format_os_date(format: &[u8], parts: OsDateParts) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut index = 0;
    while index < format.len() {
        let byte = format[index];
        if byte != b'%' {
            output.push(byte);
            index += 1;
            continue;
        }
        index += 1;
        let Some(specifier) = format.get(index).copied() else {
            return Err(invalid_os_date_specifier());
        };
        match specifier {
            b'%' => output.push(b'%'),
            b'd' => push_zero_padded_i64(&mut output, parts.day, 2),
            b'm' => push_zero_padded_i64(&mut output, parts.month, 2),
            b'Y' => push_zero_padded_i64(&mut output, parts.year, 4),
            b'H' => push_zero_padded_i64(&mut output, parts.hour, 2),
            b'M' => push_zero_padded_i64(&mut output, parts.minute, 2),
            b'S' => push_zero_padded_i64(&mut output, parts.second, 2),
            b'w' => output.extend_from_slice(parts.wday.to_string().as_bytes()),
            b'j' => push_zero_padded_i64(&mut output, parts.yday, 3),
            b'c' => {
                push_zero_padded_i64(&mut output, parts.month, 2);
                output.push(b'/');
                push_zero_padded_i64(&mut output, parts.day, 2);
                output.push(b'/');
                push_zero_padded_i64(&mut output, parts.year.rem_euclid(100), 2);
                output.push(b' ');
                push_zero_padded_i64(&mut output, parts.hour, 2);
                output.push(b':');
                push_zero_padded_i64(&mut output, parts.minute, 2);
                output.push(b':');
                push_zero_padded_i64(&mut output, parts.second, 2);
            }
            _ => return Err(invalid_os_date_specifier()),
        }
        index += 1;
    }
    Ok(output)
}

fn invalid_os_date_specifier() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: "invalid conversion specifier".to_owned(),
    })
}

fn push_zero_padded_i64(output: &mut Vec<u8>, value: i64, width: usize) {
    let text = value.to_string();
    if text.len() < width {
        output.extend(std::iter::repeat_n(b'0', width - text.len()));
    }
    output.extend_from_slice(text.as_bytes());
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 }.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 }.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

fn native_registers(args: &[Value]) -> Result<Vec<Value>> {
    if args.len() > u8::MAX as usize {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "fewer arguments",
        }));
    }
    Ok(args.to_vec())
}

fn string_format_args(registers: &[Value], instruction: Instruction) -> Result<&[Value]> {
    if instruction.aux < 1 || instruction.aux > i32::from(u8::MAX) {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "invalid string.format arity",
        }));
    }
    let start = usize::from(instruction.b);
    let end = start.checked_add(instruction.aux as usize).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.format argument register window overflow",
        })
    })?;
    registers.get(start..end).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.format argument register is out of bounds",
        })
    })
}

fn number_arg(value: Value) -> Result<f64> {
    value
        .as_number()
        .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))
}

fn stack_level_arg(value: Value, expected: &'static str) -> Result<usize> {
    let level = value
        .as_number()
        .ok_or_else(|| Error::new(ErrorKind::TypeError { expected }))?;
    if !level.is_finite() || level < 0.0 || level.fract() != 0.0 || level > usize::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError { expected }));
    }
    Ok(level as usize)
}

#[derive(Clone, Copy)]
enum NativeMathWindow {
    Clamp,
    Map,
    Lerp,
}

impl NativeMathWindow {
    const fn arity(self) -> usize {
        match self {
            Self::Clamp | Self::Lerp => 3,
            Self::Map => 5,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Clamp => "math.clamp arguments",
            Self::Map => "math.map arguments",
            Self::Lerp => "math.lerp arguments",
        }
    }
}

#[derive(Clone, Copy)]
enum StringCase {
    Upper,
    Lower,
}

impl StringCase {
    const fn name(self) -> &'static str {
        match self {
            Self::Upper => "string.upper argument",
            Self::Lower => "string.lower argument",
        }
    }
}

fn charge_fuel(fuel: &mut Option<u64>, limit: Option<u64>, cost: u64) -> Result<()> {
    if let Some(remaining) = fuel {
        if *remaining < cost {
            return Err(Error::new(ErrorKind::FuelExhausted {
                limit: limit.unwrap_or(0),
            }));
        }
        *remaining -= cost;
    }
    Ok(())
}

fn opcode_may_observe_current_roots(op: Opcode) -> bool {
    // These opcodes can call Luau/native code, trigger metamethods, or otherwise
    // observe weak-table root snapshots. Keep this match exhaustive so new
    // opcodes must be classified deliberately.
    match op {
        Opcode::Add
        | Opcode::AddConst
        | Opcode::AddUpvalue
        | Opcode::AddUpvalueConst
        | Opcode::Sub
        | Opcode::SubConst
        | Opcode::Mul
        | Opcode::MulConst
        | Opcode::MulModConst
        | Opcode::MulAddModConst
        | Opcode::SetIndexMulAddModConst
        | Opcode::SetIndexMulModConst
        | Opcode::Div
        | Opcode::DivConst
        | Opcode::Mod
        | Opcode::ModConst
        | Opcode::ModAddConst
        | Opcode::Neg
        | Opcode::Equal
        | Opcode::NotEqual
        | Opcode::Less
        | Opcode::LessEqual
        | Opcode::Greater
        | Opcode::GreaterEqual
        | Opcode::FloorDiv
        | Opcode::FloorDivConst
        | Opcode::Pow
        | Opcode::PowConst
        | Opcode::Concat
        | Opcode::ConcatConst
        | Opcode::ConcatRhsConst
        | Opcode::GetIndexConcatConst
        | Opcode::GetIndexModAddConst
        | Opcode::GetIndexMulModConst
        | Opcode::SetIndexConcatConst
        | Opcode::GetIndexNumber
        | Opcode::GetIndexNumberConst
        | Opcode::GetIndexStringConst
        | Opcode::SetIndexNumber
        | Opcode::ConcatArgs
        | Opcode::ReturnCall
        | Opcode::Len
        | Opcode::LenPlusOne
        | Opcode::GetIndex
        | Opcode::SetIndex
        | Opcode::TableFind
        | Opcode::Call
        | Opcode::GetGlobal
        | Opcode::SetGlobal
        | Opcode::CallMulti
        | Opcode::CallExpand
        | Opcode::CallVarargs
        | Opcode::ReturnCallVarargs
        | Opcode::ReturnCallExpand
        | Opcode::CallExpand2
        | Opcode::CallExpandVarargs
        | Opcode::ReturnCallExpand2
        | Opcode::ReturnCallExpandVarargs
        | Opcode::AppendCall
        | Opcode::AppendCallVarargs
        | Opcode::AppendCallExpand
        | Opcode::ReturnCallExpand2Varargs
        | Opcode::GenericCallPrep
        | Opcode::GenericCallLoop
        | Opcode::GeneralizedIterPrep
        | Opcode::GeneralizedIterLoop
        | Opcode::CallExpand3
        | Opcode::ToString
        | Opcode::StringFormat => true,
        Opcode::Halt
        | Opcode::LoadConst
        | Opcode::Move
        | Opcode::Jump
        | Opcode::JumpIfNot
        | Opcode::JumpIf
        | Opcode::Return
        | Opcode::CallExtension
        | Opcode::Assert
        | Opcode::Not
        | Opcode::ForPrep
        | Opcode::ForLoop
        | Opcode::ForPrepStepOne
        | Opcode::ForLoopStepOne
        | Opcode::ToNumber
        | Opcode::NewArray
        | Opcode::NewRecord
        | Opcode::IpairsPrep
        | Opcode::IpairsLoop
        | Opcode::PairsPrep
        | Opcode::PairsLoop
        | Opcode::TableConcat
        | Opcode::NextKey
        | Opcode::TypeOf
        | Opcode::Abs
        | Opcode::StringSub
        | Opcode::StringByteEqual
        | Opcode::StringLen
        | Opcode::StringByte
        | Opcode::StringUpper
        | Opcode::StringLower
        | Opcode::StringReverse
        | Opcode::StringRep
        | Opcode::StringFind
        | Opcode::StringSplit
        | Opcode::TableGetN
        | Opcode::TableMaxN
        | Opcode::MathFloor
        | Opcode::MathCeil
        | Opcode::MathRound
        | Opcode::MathSign
        | Opcode::MathClamp
        | Opcode::MathMap
        | Opcode::MathLerp
        | Opcode::MathMin
        | Opcode::MathMax
        | Opcode::MathFmod
        | Opcode::MathPow
        | Opcode::MathSqrt
        | Opcode::MathDeg
        | Opcode::MathRad
        | Opcode::MathIsNan
        | Opcode::MathIsInf
        | Opcode::MathIsFinite
        | Opcode::MathSin
        | Opcode::MathCos
        | Opcode::MathSinCosMul
        | Opcode::MathTan
        | Opcode::MathAsin
        | Opcode::MathAcos
        | Opcode::MathAtan
        | Opcode::MathAtan2
        | Opcode::MathSinh
        | Opcode::MathCosh
        | Opcode::MathTanh
        | Opcode::MathExp
        | Opcode::MathLog
        | Opcode::MathLog10
        | Opcode::MathLdexp
        | Opcode::RawGet
        | Opcode::RawSet
        | Opcode::RawLen
        | Opcode::TableCreate
        | Opcode::TableInsert
        | Opcode::TableRemove
        | Opcode::LoadFunction
        | Opcode::LoadCurrentFunction
        | Opcode::GetLocalCell
        | Opcode::SetLocalCell
        | Opcode::GetUpvalue
        | Opcode::SetUpvalue
        | Opcode::ClearLocalCell
        | Opcode::ClearLocalCells
        | Opcode::LoadVarargs
        | Opcode::ReturnVarargs
        | Opcode::AppendVarargs
        | Opcode::LoadConstWide
        | Opcode::CheckMethod
        | Opcode::CoerceForNumber => false,
    }
}

fn static_string_get_index_site(
    program_id: Option<u32>,
    prototype: Option<u32>,
    program: &Program,
    function: &Function,
    pc: usize,
    key_register: u8,
) -> Option<InlineCacheSite> {
    let program_id = program_id?;
    let previous = function.code.get(pc.checked_sub(1)?)?;
    if previous.a != key_register {
        return None;
    }
    let constant_index = match previous.op {
        Opcode::LoadConst => usize::from(previous.c),
        Opcode::LoadConstWide if previous.aux >= 0 => previous.aux as usize,
        _ => return None,
    };
    if !matches!(
        program.constants.get(constant_index),
        Some(Constant::String(_))
    ) {
        return None;
    }
    Some(InlineCacheSite {
        program: program_id,
        prototype,
        pc,
    })
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn range_work_fuel(start: usize, end: usize) -> u64 {
    if start > end {
        0
    } else {
        usize_to_u64(end - start + 1)
    }
}

fn unpack_result_count(start: i64, end: i64) -> Result<usize> {
    let count = end
        .checked_sub(start)
        .and_then(|value| value.checked_add(1))
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| {
            Error::new(ErrorKind::RuntimeError {
                message: "too many results to unpack".to_owned(),
            })
        })?;
    if count > MAX_TABLE_UNPACK_RESULTS {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "too many results to unpack".to_owned(),
        }));
    }
    Ok(count)
}

fn shifted_array_work_fuel(len: usize, position: i32) -> u64 {
    let Ok(position) = usize::try_from(position) else {
        return 1;
    };
    if position == 0 || position > len {
        1
    } else {
        usize_to_u64(len - position + 1)
    }
}

fn collectgarbage_step_threshold(size: f64) -> u64 {
    let ratio = 10.0 / size;
    if ratio.is_nan() || ratio <= 1.0 {
        return 1;
    }
    if ratio.is_infinite() || ratio >= u64::MAX as f64 {
        return u64::MAX;
    }
    let truncated = ratio as u64;
    if truncated as f64 == ratio {
        truncated
    } else {
        truncated + 1
    }
}

fn arithmetic_metamethod_name(op: NumericOp) -> Option<&'static str> {
    match op {
        NumericOp::Add => Some("__add"),
        NumericOp::Sub => Some("__sub"),
        NumericOp::Mul => Some("__mul"),
        NumericOp::Div => Some("__div"),
        NumericOp::FloorDiv => Some("__idiv"),
        NumericOp::Mod => Some("__mod"),
        NumericOp::Pow => Some("__pow"),
        NumericOp::Neg => Some("__unm"),
        NumericOp::Abs => None,
    }
}

fn arithmetic_error(op: NumericOp, lhs: Value, rhs: Value) -> Error {
    let lhs_type = value_type_name(lhs);
    let rhs_type = value_type_name(rhs);
    let operands = if lhs_type == rhs_type {
        lhs_type.to_owned()
    } else {
        format!("{lhs_type} and {rhs_type}")
    };
    Error::new(ErrorKind::RuntimeError {
        message: format!(
            "attempt to perform arithmetic ({}) on {operands}",
            arithmetic_error_op_name(op)
        ),
    })
}

fn unary_arithmetic_error(op: NumericOp, value: Value) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: format!(
            "attempt to perform arithmetic ({}) on {}",
            arithmetic_error_op_name(op),
            value_type_name(value)
        ),
    })
}

fn arithmetic_error_op_name(op: NumericOp) -> &'static str {
    match op {
        NumericOp::Add => "add",
        NumericOp::Sub => "sub",
        NumericOp::Mul => "mul",
        NumericOp::Div => "div",
        NumericOp::FloorDiv => "idiv",
        NumericOp::Mod => "mod",
        NumericOp::Pow => "pow",
        NumericOp::Neg => "unm",
        NumericOp::Abs => "abs",
    }
}

fn order_error(lhs: Value, rhs: Value, op: &'static str) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: format!(
            "attempt to compare {} {op} {}",
            value_type_name(lhs),
            value_type_name(rhs)
        ),
    })
}

#[derive(Clone, Copy)]
enum OrderComparison {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

fn compare_ordering(comparison: OrderComparison, ordering: Option<std::cmp::Ordering>) -> bool {
    let Some(ordering) = ordering else {
        return false;
    };
    match comparison {
        OrderComparison::Less => ordering.is_lt(),
        OrderComparison::LessEqual => !ordering.is_gt(),
        OrderComparison::Greater => ordering.is_gt(),
        OrderComparison::GreaterEqual => !ordering.is_lt(),
    }
}

fn numeric_for_prepare(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    instruction: Instruction,
    pc: usize,
) -> Result<bool> {
    let window = NumericForRegisters::new(instruction.a)?;
    let index = get_numeric_for_number(registers, window.index, "initial value")
        .map_err(|e| e.at_pc(pc))?;
    let limit =
        get_numeric_for_number(registers, window.limit, "limit").map_err(|e| e.at_pc(pc))?;
    let step = get_numeric_for_number(registers, window.step, "step").map_err(|e| e.at_pc(pc))?;
    if numeric_for_should_continue(index, limit, step) {
        set_register(registers, window.visible, Value::number(index)).map_err(|e| e.at_pc(pc))?;
        clear_register_cell(register_cells, window.visible).map_err(|e| e.at_pc(pc))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn numeric_for_loop(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    instruction: Instruction,
    pc: usize,
) -> Result<bool> {
    let window = NumericForRegisters::new(instruction.a)?;
    let index = get_numeric_for_number(registers, window.index, "initial value")
        .map_err(|e| e.at_pc(pc))?;
    let limit =
        get_numeric_for_number(registers, window.limit, "limit").map_err(|e| e.at_pc(pc))?;
    let step = get_numeric_for_number(registers, window.step, "step").map_err(|e| e.at_pc(pc))?;
    let next = binary(NumericOp::Add, index, step);
    set_register(registers, window.index, Value::number(next)).map_err(|e| e.at_pc(pc))?;
    if numeric_for_should_continue(next, limit, step) {
        set_register(registers, window.visible, Value::number(next)).map_err(|e| e.at_pc(pc))?;
        clear_register_cell(register_cells, window.visible).map_err(|e| e.at_pc(pc))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn numeric_for_step_one_prepare(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    instruction: Instruction,
    pc: usize,
) -> Result<bool> {
    let window = NumericForStepOneRegisters::new(instruction.a)?;
    let index = get_numeric_for_number(registers, window.index, "initial value")
        .map_err(|e| e.at_pc(pc))?;
    let limit =
        get_numeric_for_number(registers, window.limit, "limit").map_err(|e| e.at_pc(pc))?;
    if index <= limit {
        set_register(registers, window.visible, Value::number(index)).map_err(|e| e.at_pc(pc))?;
        clear_register_cell(register_cells, window.visible).map_err(|e| e.at_pc(pc))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn numeric_for_step_one_loop(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    instruction: Instruction,
    pc: usize,
) -> Result<bool> {
    let window = NumericForStepOneRegisters::new(instruction.a)?;
    let index = get_numeric_for_number(registers, window.index, "initial value")
        .map_err(|e| e.at_pc(pc))?;
    let limit =
        get_numeric_for_number(registers, window.limit, "limit").map_err(|e| e.at_pc(pc))?;
    let next = index + 1.0;
    set_register(registers, window.index, Value::number(next)).map_err(|e| e.at_pc(pc))?;
    if next <= limit {
        set_register(registers, window.visible, Value::number(next)).map_err(|e| e.at_pc(pc))?;
        clear_register_cell(register_cells, window.visible).map_err(|e| e.at_pc(pc))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn numeric_for_should_continue(index: f64, limit: f64, step: f64) -> bool {
    if step > 0.0 {
        index <= limit
    } else {
        index >= limit
    }
}

fn clear_array_iterator_value_cells(
    register_cells: &mut [Option<u32>],
    window: ArrayIteratorRegisters,
    clear_visible_key: bool,
) -> Result<()> {
    if register_cells.is_empty() {
        return Ok(());
    }
    let value_index = usize::from(window.value);
    if value_index >= register_cells.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: value_index,
            max: register_cells.len(),
        }));
    }
    if clear_visible_key {
        register_cells[usize::from(window.key)] = None;
    }
    register_cells[value_index] = None;
    Ok(())
}

fn clear_unused_array_iterator_key(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    window: ArrayIteratorRegisters,
) -> Result<()> {
    let key_index = usize::from(window.key);
    if key_index >= registers.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: key_index,
            max: registers.len(),
        }));
    }
    registers[key_index] = Value::nil();
    if !register_cells.is_empty() {
        if key_index >= register_cells.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: key_index,
                max: register_cells.len(),
            }));
        }
        register_cells[key_index] = None;
    }
    Ok(())
}

fn set_array_iterator_registers(
    registers: &mut [Value],
    window: ArrayIteratorRegisters,
    key: Value,
    value: Value,
    write_visible_key: bool,
) -> Result<()> {
    let value_index = usize::from(window.value);
    if value_index >= registers.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: value_index,
            max: registers.len(),
        }));
    }
    registers[usize::from(window.index)] = key;
    if write_visible_key {
        registers[usize::from(window.key)] = key;
    }
    registers[value_index] = value;
    Ok(())
}

fn array_iterator_writes_visible_key(instruction: Instruction) -> Result<bool> {
    match instruction.c {
        ARRAY_ITERATOR_WRITE_VISIBLE_KEY => Ok(true),
        ARRAY_ITERATOR_SKIP_VISIBLE_KEY => Ok(false),
        _ => Err(Error::new(ErrorKind::InvalidProgram {
            reason: "invalid array iterator visible key flag",
        })),
    }
}

fn write_call_iterator_visible_results(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    window: CallIteratorRegisters,
    results: &[Value],
) -> Result<()> {
    let start = usize::from(window.first_visible);
    let count = usize::from(window.visible_count);
    validate_iterator_visible_window(registers, register_cells, start, count)?;
    if count == 2 {
        registers[start] = results.first().copied().unwrap_or_else(Value::nil);
        registers[start + 1] = results.get(1).copied().unwrap_or_else(Value::nil);
        if !register_cells.is_empty() {
            register_cells[start] = None;
            register_cells[start + 1] = None;
        }
        return Ok(());
    }
    for offset in 0..count {
        registers[start + offset] = results.get(offset).copied().unwrap_or_else(Value::nil);
        if !register_cells.is_empty() {
            register_cells[start + offset] = None;
        }
    }
    Ok(())
}

fn write_table_iterator_visible_results(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    window: CallIteratorRegisters,
    key: Value,
    value: Value,
) -> Result<()> {
    let start = usize::from(window.first_visible);
    let count = usize::from(window.visible_count);
    validate_iterator_visible_window(registers, register_cells, start, count)?;
    if count == 2 {
        registers[start] = key;
        registers[start + 1] = value;
        if !register_cells.is_empty() {
            register_cells[start] = None;
            register_cells[start + 1] = None;
        }
        return Ok(());
    }
    for offset in 0..count {
        registers[start + offset] = match offset {
            0 => key,
            1 => value,
            _ => Value::nil(),
        };
        if !register_cells.is_empty() {
            register_cells[start + offset] = None;
        }
    }
    Ok(())
}

fn validate_iterator_visible_window(
    registers: &[Value],
    register_cells: &[Option<u32>],
    start: usize,
    count: usize,
) -> Result<()> {
    let end = start.checked_add(count).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "iterator visible register overflow",
        })
    })?;
    if end > registers.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: end,
            max: registers.len(),
        }));
    }
    if !register_cells.is_empty() && end > register_cells.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: end,
            max: register_cells.len(),
        }));
    }
    Ok(())
}

fn clear_register_cell(register_cells: &mut [Option<u32>], register: u8) -> Result<()> {
    if register_cells.is_empty() {
        return Ok(());
    }
    let cell_index = register as usize;
    if cell_index >= register_cells.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: cell_index,
            max: register_cells.len(),
        }));
    }
    register_cells[cell_index] = None;
    Ok(())
}

fn clear_register_value_and_cell(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    register: u8,
) -> Result<()> {
    let index = register as usize;
    if index >= registers.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: index,
            max: registers.len(),
        }));
    }
    if !register_cells.is_empty() {
        if index >= register_cells.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: index,
                max: register_cells.len(),
            }));
        }
        register_cells[index] = None;
    }
    if let Some(slot) = registers.get_mut(index) {
        *slot = Value::nil();
        Ok(())
    } else {
        Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: index,
            max: registers.len(),
        }))
    }
}

fn clear_register_values_and_cells(
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    start: u8,
    count: u8,
) -> Result<()> {
    let start = usize::from(start);
    let count = usize::from(count);
    let end = start.checked_add(count).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "register clear range overflow",
        })
    })?;
    if end > registers.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: end,
            max: registers.len(),
        }));
    }
    if !register_cells.is_empty() {
        if end > register_cells.len() {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: end,
                max: register_cells.len(),
            }));
        }
        register_cells[start..end].fill(None);
    }
    registers[start..end].fill(Value::nil());
    Ok(())
}

#[derive(Clone, Copy)]
struct FormatSpec {
    left_align: bool,
    sign_plus: bool,
    sign_space: bool,
    zero_pad: bool,
    width: Option<usize>,
    precision: Option<usize>,
    conversion: FormatConversion,
}

#[derive(Clone)]
enum StringFormatPlanItem {
    Literal(Vec<u8>),
    Tostring,
    Spec(FormatSpec),
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum FormatConversion {
    String,
    Char,
    Quote,
    SignedDecimal,
    UnsignedDecimal,
    Octal,
    LowerHex,
    UpperHex,
    FloatFixed,
    FloatLowerExp,
    FloatUpperExp,
    FloatLowerGeneral,
    FloatUpperGeneral,
}

fn parse_string_format_plan(bytes: &[u8]) -> Result<Vec<StringFormatPlanItem>> {
    let mut items = Vec::new();
    let mut offset = 0usize;
    let mut literal_start = 0usize;

    while offset < bytes.len() {
        if bytes[offset] != b'%' {
            offset += 1;
            continue;
        }

        push_format_literal(&mut items, &bytes[literal_start..offset]);
        offset += 1;
        if offset >= bytes.len() {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "invalid option '%' to 'format'".to_owned(),
            }));
        }
        match bytes[offset] {
            b'%' => {
                push_format_literal(&mut items, b"%");
                offset += 1;
            }
            b'*' => {
                items.push(StringFormatPlanItem::Tostring);
                offset += 1;
            }
            _ => {
                let (spec, next_offset) = parse_format_spec(bytes, offset)?;
                items.push(StringFormatPlanItem::Spec(spec));
                offset = next_offset;
            }
        }
        literal_start = offset;
    }

    push_format_literal(&mut items, &bytes[literal_start..]);
    Ok(items)
}

fn push_format_literal(items: &mut Vec<StringFormatPlanItem>, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    match items.last_mut() {
        Some(StringFormatPlanItem::Literal(existing)) => existing.extend_from_slice(bytes),
        _ => items.push(StringFormatPlanItem::Literal(bytes.to_vec())),
    }
}

fn parse_format_spec(bytes: &[u8], mut offset: usize) -> Result<(FormatSpec, usize)> {
    let mut left_align = false;
    let mut sign_plus = false;
    let mut sign_space = false;
    let mut zero_pad = false;
    let mut flag_count = 0usize;
    loop {
        let Some(flag) = bytes.get(offset).copied() else {
            return Err(invalid_format_option('%'));
        };
        match flag {
            b'-' if !left_align => left_align = true,
            b'+' if !sign_plus => sign_plus = true,
            b' ' if !sign_space => sign_space = true,
            b'0' if !zero_pad => zero_pad = true,
            b'#' => {}
            b'-' | b'+' | b' ' | b'0' => {
                return Err(Error::new(ErrorKind::RuntimeError {
                    message: "invalid format (repeated flags)".to_owned(),
                }));
            }
            _ => break,
        }
        flag_count += 1;
        if flag_count >= 6 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "invalid format (repeated flags)".to_owned(),
            }));
        }
        offset += 1;
    }

    let (width, next_offset) = parse_format_number(bytes, offset)?;
    offset = next_offset;
    let precision = if bytes.get(offset) == Some(&b'.') {
        let (precision, next_offset) = parse_format_number(bytes, offset + 1)?;
        offset = next_offset;
        Some(precision.unwrap_or(0))
    } else {
        None
    };

    let Some(conversion_byte) = bytes.get(offset).copied() else {
        return Err(invalid_format_option('%'));
    };
    let conversion = match conversion_byte {
        b's' => FormatConversion::String,
        b'c' => FormatConversion::Char,
        b'q' => FormatConversion::Quote,
        b'd' | b'i' => FormatConversion::SignedDecimal,
        b'u' => FormatConversion::UnsignedDecimal,
        b'o' => FormatConversion::Octal,
        b'x' => FormatConversion::LowerHex,
        b'X' => FormatConversion::UpperHex,
        b'f' => FormatConversion::FloatFixed,
        b'e' => FormatConversion::FloatLowerExp,
        b'E' => FormatConversion::FloatUpperExp,
        b'g' => FormatConversion::FloatLowerGeneral,
        b'G' => FormatConversion::FloatUpperGeneral,
        other => return Err(invalid_format_option(other as char)),
    };

    Ok((
        FormatSpec {
            left_align,
            sign_plus,
            sign_space,
            zero_pad,
            width,
            precision,
            conversion,
        },
        offset + 1,
    ))
}

fn parse_format_number(bytes: &[u8], mut offset: usize) -> Result<(Option<usize>, usize)> {
    let start = offset;
    let mut value = 0usize;
    while let Some(byte) = bytes.get(offset).copied() {
        if !byte.is_ascii_digit() {
            break;
        }
        if offset - start >= 2 {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "width or precision too long".to_owned(),
            }));
        }
        value = value * 10 + usize::from(byte - b'0');
        offset += 1;
    }
    if offset == start {
        Ok((None, offset))
    } else {
        Ok((Some(value), offset))
    }
}

fn invalid_format_option(option: char) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: format!("invalid option '%{option}' to 'format'"),
    })
}

fn format_spec_capacity_hint(heap: &mut VmCell, spec: FormatSpec, arg: Option<Value>) -> usize {
    let base = match spec.conversion {
        FormatConversion::String => {
            if let Some(atom) = arg.and_then(Value::as_string_atom) {
                let len = heap.string_atom_byte_len(atom).unwrap_or(16);
                spec.precision.map_or(len, |precision| len.min(precision))
            } else {
                spec.precision.unwrap_or(16)
            }
        }
        FormatConversion::Char => 1,
        FormatConversion::Quote => arg
            .and_then(Value::as_string_atom)
            .and_then(|atom| heap.string_atom_byte_len(atom))
            .map_or(16, |len| len.saturating_add(2)),
        FormatConversion::SignedDecimal | FormatConversion::UnsignedDecimal => 20,
        FormatConversion::Octal => 22,
        FormatConversion::LowerHex | FormatConversion::UpperHex => 16,
        FormatConversion::FloatFixed => spec.precision.unwrap_or(6).saturating_add(24),
        FormatConversion::FloatLowerExp
        | FormatConversion::FloatUpperExp
        | FormatConversion::FloatLowerGeneral
        | FormatConversion::FloatUpperGeneral => 32,
    };
    spec.width.unwrap_or(0).max(base)
}

fn append_format_args(output: &mut Vec<u8>, args: std::fmt::Arguments<'_>) {
    std::io::Write::write_fmt(output, args).expect("formatting into Vec<u8> cannot fail");
}

fn append_exact_fixed_decimal(output: &mut Vec<u8>, number: f64, precision: usize) -> bool {
    const POW10: [u64; 10] = [
        1,
        10,
        100,
        1_000,
        10_000,
        100_000,
        1_000_000,
        10_000_000,
        100_000_000,
        1_000_000_000,
    ];
    const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_991.0;

    let Some(&scale) = POW10.get(precision) else {
        return false;
    };
    if !number.is_finite() {
        return false;
    }

    let scaled = number.abs() * scale as f64;
    if scaled > MAX_EXACT_F64_INTEGER || scaled.fract() != 0.0 {
        return false;
    }

    if number.is_sign_negative() {
        output.push(b'-');
    }

    let scaled = scaled as u64;
    append_u64_decimal(output, scaled / scale);
    if precision > 0 {
        output.push(b'.');
        append_zero_padded_u64_decimal(output, scaled % scale, precision);
    }
    true
}

fn append_u64_decimal(output: &mut Vec<u8>, mut value: u64) {
    let mut buffer = [0u8; 20];
    let mut index = buffer.len();
    loop {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    output.extend_from_slice(&buffer[index..]);
}

fn append_zero_padded_u64_decimal(output: &mut Vec<u8>, mut value: u64, width: usize) {
    let mut buffer = [b'0'; 20];
    let mut index = buffer.len();
    for _ in 0..width {
        index -= 1;
        buffer[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
    output.extend_from_slice(&buffer[index..]);
}

fn append_u64_radix(output: &mut Vec<u8>, mut value: u64, radix: u64, uppercase: bool) {
    let mut buffer = [0u8; 64];
    let mut index = buffer.len();
    loop {
        index -= 1;
        let digit = (value % radix) as u8;
        buffer[index] = match digit {
            0..=9 => b'0' + digit,
            _ if uppercase => b'A' + digit - 10,
            _ => b'a' + digit - 10,
        };
        value /= radix;
        if value == 0 {
            break;
        }
    }
    output.extend_from_slice(&buffer[index..]);
}

fn normalize_format_exponent(mut text: String) -> String {
    let Some(marker) = text.find(['e', 'E']) else {
        return text;
    };
    let exponent_start = marker + 1;
    if !matches!(text.as_bytes().get(exponent_start), Some(b'+') | Some(b'-')) {
        text.insert(exponent_start, '+');
    }
    let digits_start = exponent_start + 1;
    if text.len().saturating_sub(digits_start) < 2 {
        text.insert(digits_start, '0');
    }
    text
}

fn format_general_float(number: f64, precision: usize, uppercase: bool) -> String {
    if !number.is_finite() {
        let text = luau_number_to_string(number);
        return if uppercase { text.to_uppercase() } else { text };
    }

    let exponent_precision = precision.saturating_sub(1);
    let exponent_text = format!("{number:.exponent_precision$e}");
    let Some(exponent) = parse_format_exponent(&exponent_text) else {
        return if uppercase {
            format!("{number:.precision$}").to_uppercase()
        } else {
            format!("{number:.precision$}")
        };
    };

    if exponent < -4 || exponent >= precision as i32 {
        return trim_general_exponent(normalize_format_exponent(exponent_text), uppercase);
    }

    let fixed_precision = (precision as i32 - (exponent + 1)).max(0) as usize;
    let text = trim_trailing_fraction_zeroes(format!("{number:.fixed_precision$}"));
    if uppercase {
        text.to_uppercase()
    } else {
        text
    }
}

fn parse_format_exponent(text: &str) -> Option<i32> {
    let marker = text.find(['e', 'E'])?;
    text[marker + 1..].parse().ok()
}

fn trim_general_exponent(text: String, uppercase: bool) -> String {
    let Some(marker) = text.find(['e', 'E']) else {
        return text;
    };
    let mantissa = trim_trailing_fraction_zeroes(text[..marker].to_owned());
    let mut output = String::with_capacity(mantissa.len() + text.len() - marker);
    output.push_str(&mantissa);
    output.push(if uppercase { 'E' } else { 'e' });
    output.push_str(&text[marker + 1..]);
    output
}

fn trim_trailing_fraction_zeroes(mut text: String) -> String {
    if !text.contains('.') {
        return text;
    }
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

fn apply_format_width(mut text: Vec<u8>, spec: FormatSpec, numeric: bool) -> Vec<u8> {
    let Some(width) = spec.width else {
        return text;
    };
    if text.len() >= width {
        return text;
    }

    let pad_len = width - text.len();
    let pad = if numeric && spec.zero_pad && !spec.left_align && spec.precision.is_none() {
        b'0'
    } else {
        b' '
    };
    let padding: Vec<u8> = std::iter::repeat_n(pad, pad_len).collect();
    if spec.left_align {
        text.extend_from_slice(&padding);
        return text;
    }
    if pad == b'0'
        && text
            .first()
            .is_some_and(|byte| matches!(byte, b'-' | b'+' | b' '))
    {
        let sign = text.remove(0);
        let mut padded = Vec::with_capacity(width);
        padded.push(sign);
        padded.extend_from_slice(&padding);
        padded.extend_from_slice(&text);
        return padded;
    }
    let mut padded = Vec::with_capacity(width);
    padded.extend_from_slice(&padding);
    padded.extend_from_slice(&text);
    padded
}

fn parse_tonumber_string(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if parse_nan_string(text) {
        return Some(canonicalize_number(f64::NAN));
    }
    if let Some(number) = parse_hex_number_string(text) {
        return Some(canonicalize_number(number));
    }
    text.parse::<f64>().ok().map(canonicalize_number)
}

fn parse_nan_string(text: &str) -> bool {
    let payload = match text.as_bytes() {
        [b'+', rest @ ..] | [b'-', rest @ ..] => rest,
        _ => text.as_bytes(),
    };

    if payload.eq_ignore_ascii_case(b"nan") {
        return true;
    }

    if payload.len() < 5 || !payload[..3].eq_ignore_ascii_case(b"nan") || payload[3] != b'(' {
        return false;
    }

    let Some(close) = payload[4..]
        .iter()
        .position(|byte| *byte == b')')
        .map(|offset| offset + 4)
    else {
        return false;
    };
    close + 1 == payload.len()
}

fn parse_tonumber_base(text: &[u8], base: u32) -> Option<f64> {
    let text = std::str::from_utf8(text).ok()?.as_bytes();
    let mut cursor = 0;
    while text
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        cursor += 1;
    }
    if cursor == text.len() {
        return None;
    }

    let negative = match text[cursor] {
        b'+' => {
            cursor += 1;
            false
        }
        b'-' => {
            cursor += 1;
            true
        }
        _ => false,
    };

    if base == 16
        && text.get(cursor) == Some(&b'0')
        && text
            .get(cursor + 1)
            .is_some_and(|byte| matches!(byte, b'x' | b'X'))
        && text
            .get(cursor + 2)
            .and_then(|byte| tonumber_digit(*byte))
            .is_some_and(|digit| digit < base)
    {
        cursor += 2;
    }

    let mut value = 0_u64;
    let mut overflowed = false;
    let mut parsed_digit = false;
    while let Some(byte) = text.get(cursor).copied() {
        let Some(digit) = tonumber_digit(byte) else {
            break;
        };
        if digit >= base {
            break;
        }
        parsed_digit = true;
        cursor += 1;
        if !overflowed {
            match value
                .checked_mul(u64::from(base))
                .and_then(|value| value.checked_add(u64::from(digit)))
            {
                Some(next) => value = next,
                None => {
                    value = u64::MAX;
                    overflowed = true;
                }
            }
        }
    }

    if !parsed_digit
        || text[cursor..]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace())
    {
        return None;
    }

    if negative && !overflowed {
        value = 0_u64.wrapping_sub(value);
    }
    Some(canonicalize_number(value as f64))
}

fn tonumber_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 10),
        _ => None,
    }
}

fn parse_hex_number_string(text: &str) -> Option<f64> {
    let (sign, digits) = match text.as_bytes() {
        [b'+', rest @ ..] => (1.0, std::str::from_utf8(rest).ok()?),
        [b'-', rest @ ..] => (-1.0, std::str::from_utf8(rest).ok()?),
        _ => (1.0, text),
    };
    let digits = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))?;

    let mut cursor = 0;
    let bytes = digits.as_bytes();
    let mut value = 0.0;
    let mut parsed_digit = false;
    while let Some(digit) = bytes
        .get(cursor)
        .and_then(|byte| tonumber_digit(*byte))
        .filter(|digit| *digit < 16)
    {
        value = value * 16.0 + f64::from(digit);
        parsed_digit = true;
        cursor += 1;
    }

    if bytes.get(cursor) == Some(&b'.') {
        cursor += 1;
        let mut place = 1.0 / 16.0;
        while let Some(digit) = bytes
            .get(cursor)
            .and_then(|byte| tonumber_digit(*byte))
            .filter(|digit| *digit < 16)
        {
            value += f64::from(digit) * place;
            place /= 16.0;
            parsed_digit = true;
            cursor += 1;
        }
    }

    if !parsed_digit {
        return None;
    }

    if bytes
        .get(cursor)
        .is_some_and(|byte| matches!(byte, b'p' | b'P'))
    {
        cursor += 1;
        let exponent_sign = match bytes.get(cursor) {
            Some(b'+') => {
                cursor += 1;
                1
            }
            Some(b'-') => {
                cursor += 1;
                -1
            }
            _ => 1,
        };
        let mut exponent = 0_i32;
        let mut parsed_exponent_digit = false;
        while let Some(digit) = bytes
            .get(cursor)
            .copied()
            .filter(u8::is_ascii_digit)
            .map(|byte| i32::from(byte - b'0'))
        {
            exponent = exponent.saturating_mul(10).saturating_add(digit);
            parsed_exponent_digit = true;
            cursor += 1;
        }
        if !parsed_exponent_digit {
            return None;
        }
        value = libm::ldexp(value, exponent.saturating_mul(exponent_sign));
    }

    if cursor != bytes.len() {
        return None;
    }

    Some(sign * value)
}

fn luau_number_to_string(value: f64) -> String {
    let value = canonicalize_number(value);
    if value.is_nan() {
        return "nan".to_owned();
    }
    if value == f64::INFINITY {
        return "inf".to_owned();
    }
    if value == f64::NEG_INFINITY {
        return "-inf".to_owned();
    }
    if value == 0.0 {
        if value.is_sign_negative() {
            return "-0".to_owned();
        }
        return "0".to_owned();
    }
    let debug = format!("{value:?}");
    let (negative, magnitude) = debug
        .strip_prefix('-')
        .map_or((false, debug.as_str()), |magnitude| (true, magnitude));
    let mut output =
        luau_number_magnitude_to_string(magnitude).unwrap_or_else(|| value.to_string());
    if negative {
        output.insert(0, '-');
    }
    output
}

fn small_positive_integer_number(value: f64) -> Option<u32> {
    (value >= 0.0 && value.is_sign_positive() && value <= 999_999.0 && value.fract() == 0.0)
        .then_some(value as u32)
}

fn luau_number_magnitude_to_string(magnitude: &str) -> Option<String> {
    let (mantissa, exponent) = magnitude
        .split_once('e')
        .or_else(|| magnitude.split_once('E'))
        .map_or(Some((magnitude, 0)), |(mantissa, exponent)| {
            exponent
                .parse::<i32>()
                .ok()
                .map(|exponent| (mantissa, exponent))
        })?;
    let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if integer.is_empty() || !integer.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut raw_digits = String::with_capacity(integer.len() + fraction.len());
    raw_digits.push_str(integer);
    raw_digits.push_str(fraction);
    let leading_zeroes = raw_digits
        .bytes()
        .position(|byte| byte != b'0')
        .unwrap_or(raw_digits.len());
    let mut digits = raw_digits[leading_zeroes..].to_owned();
    while digits.ends_with('0') {
        digits.pop();
    }
    if digits.is_empty() {
        return Some("0".to_owned());
    }
    let decimal_exponent = exponent + integer.len() as i32 - leading_zeroes as i32 - 1;
    Some(luau_decimal_digits_to_string(&digits, decimal_exponent))
}

fn luau_decimal_digits_to_string(digits: &str, decimal_exponent: i32) -> String {
    if !(-6..21).contains(&decimal_exponent) {
        return luau_decimal_digits_to_exponent_string(digits, decimal_exponent);
    }

    let integer_digits = decimal_exponent + 1;
    if integer_digits <= 0 {
        let mut output = String::from("0.");
        output.extend(std::iter::repeat_n('0', (-integer_digits) as usize));
        output.push_str(digits);
        return output;
    }

    let integer_digits = integer_digits as usize;
    if integer_digits >= digits.len() {
        let mut output = String::from(digits);
        output.extend(std::iter::repeat_n('0', integer_digits - digits.len()));
        return output;
    }

    let mut output = String::with_capacity(digits.len() + 1);
    output.push_str(&digits[..integer_digits]);
    output.push('.');
    output.push_str(&digits[integer_digits..]);
    output
}

fn luau_decimal_digits_to_exponent_string(digits: &str, decimal_exponent: i32) -> String {
    let mut output = String::with_capacity(digits.len() + 8);
    output.push_str(&digits[..1]);
    if digits.len() > 1 {
        output.push('.');
        output.push_str(&digits[1..]);
    }
    output.push('e');
    if decimal_exponent >= 0 {
        output.push('+');
    }
    output.push_str(&decimal_exponent.to_string());
    output
}

fn format_vector(vector: VectorData) -> String {
    format!(
        "{}, {}, {}",
        luau_number_to_string(f64::from(vector.x())),
        luau_number_to_string(f64::from(vector.y())),
        luau_number_to_string(f64::from(vector.z()))
    )
}

fn vector_components_equal(lhs: VectorData, rhs: VectorData) -> bool {
    !lhs.x().is_nan()
        && !lhs.y().is_nan()
        && !lhs.z().is_nan()
        && !rhs.x().is_nan()
        && !rhs.y().is_nan()
        && !rhs.z().is_nan()
        && lhs.x() == rhs.x()
        && lhs.y() == rhs.y()
        && lhs.z() == rhs.z()
}

fn vector_vector_op(op: NumericOp) -> bool {
    matches!(
        op,
        NumericOp::Add | NumericOp::Sub | NumericOp::Mul | NumericOp::Div | NumericOp::FloorDiv
    )
}

fn vector_scalar_rhs_op(op: NumericOp) -> bool {
    matches!(op, NumericOp::Mul | NumericOp::Div | NumericOp::FloorDiv)
}

fn vector_scalar_lhs_op(op: NumericOp) -> bool {
    matches!(op, NumericOp::Mul | NumericOp::Div | NumericOp::FloorDiv)
}

fn vector_component_binop(lhs: f32, rhs: f32, op: NumericOp) -> f32 {
    match op {
        NumericOp::Add => lhs + rhs,
        NumericOp::Sub => lhs - rhs,
        NumericOp::Mul => lhs * rhs,
        NumericOp::Div => lhs / rhs,
        NumericOp::FloorDiv => libm::floorf(lhs / rhs),
        _ => f32::NAN,
    }
}

fn vector_dot(lhs: VectorData, rhs: VectorData) -> f64 {
    f64::from(lhs.x()) * f64::from(rhs.x())
        + f64::from(lhs.y()) * f64::from(rhs.y())
        + f64::from(lhs.z()) * f64::from(rhs.z())
}

fn vector_cross(lhs: VectorData, rhs: VectorData) -> VectorData {
    VectorData::new(
        lhs.y() * rhs.z() - lhs.z() * rhs.y(),
        lhs.z() * rhs.x() - lhs.x() * rhs.z(),
        lhs.x() * rhs.y() - lhs.y() * rhs.x(),
    )
}

fn vector_magnitude(vector: VectorData) -> f64 {
    libm::sqrt(vector_dot(vector, vector))
}

fn vector_normalize(vector: VectorData) -> VectorData {
    let magnitude = vector_magnitude(vector);
    if magnitude == 0.0 || magnitude.is_nan() {
        return VectorData::new(0.0, 0.0, 0.0);
    }
    let scale = (1.0 / magnitude) as f32;
    VectorData::new(vector.x() * scale, vector.y() * scale, vector.z() * scale)
}

fn vector_sign(value: f32) -> f32 {
    if value > 0.0 {
        1.0
    } else if value < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn vector_unsigned_angle(lhs: VectorData, rhs: VectorData) -> f64 {
    let denominator = vector_magnitude(lhs) * vector_magnitude(rhs);
    if denominator == 0.0 || denominator.is_nan() {
        return 0.0;
    }
    let cosine = (vector_dot(lhs, rhs) / denominator).clamp(-1.0, 1.0);
    libm::acos(cosine)
}

fn vector_signed_angle(lhs: VectorData, rhs: VectorData, axis: VectorData) -> f64 {
    let unsigned = vector_unsigned_angle(lhs, rhs);
    if unsigned == 0.0 {
        return 0.0;
    }
    let sign = vector_dot(axis, vector_cross(lhs, rhs));
    if sign < 0.0 {
        -unsigned
    } else {
        unsigned
    }
}

fn vector_minmax(lhs: VectorData, rhs: VectorData, op: VectorMinMax) -> VectorData {
    let component = |lhs: f32, rhs: f32| match op {
        VectorMinMax::Min if rhs < lhs => rhs,
        VectorMinMax::Max if rhs > lhs => rhs,
        _ => lhs,
    };
    VectorData::new(
        component(lhs.x(), rhs.x()),
        component(lhs.y(), rhs.y()),
        component(lhs.z(), rhs.z()),
    )
}

fn vector_clamp_component(value: f32, min: f32, max: f32) -> f32 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

fn vector_lerp_component(lhs: f32, rhs: f32, alpha: f32) -> f32 {
    if alpha == 1.0 {
        rhs
    } else {
        lhs + (rhs - lhs) * alpha
    }
}

fn value_type_name(value: Value) -> &'static str {
    match value.kind() {
        ValueKind::Nil => "nil",
        ValueKind::Boolean => "boolean",
        ValueKind::Number => "number",
        ValueKind::String => "string",
        ValueKind::Table => "table",
        ValueKind::Function => "function",
        ValueKind::Userdata => "userdata",
        ValueKind::Buffer => "buffer",
        ValueKind::Vector => "vector",
    }
}

fn iteration_error(value: Value) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: format!("attempt to iterate over a {} value", value_type_name(value)),
    })
}

fn first_result_or_nil(results: Vec<Value>) -> Value {
    results.first().copied().unwrap_or_else(Value::nil)
}

fn collect_traceback_snapshot_roots(roots: &mut Vec<Value>, snapshot: &TracebackSnapshot) {
    for frame in &snapshot.frames {
        collect_call_stack_frame_roots(roots, frame);
    }
}

fn collect_call_stack_frame_roots(roots: &mut Vec<Value>, frame: &CallStackFrame) {
    roots.push(Value::table(frame.environment));
    if let Some(function) = frame.function {
        roots.push(function);
    }
    roots.extend_from_slice(&frame.roots);
    roots.extend_from_slice(&frame.stackless_roots);
    for stackless_run in &frame.stackless_debug_frames {
        collect_stackless_debug_frame_roots(roots, &stackless_run.frame);
    }
}

fn collect_stackless_debug_frame_roots(roots: &mut Vec<Value>, frame: &StacklessDebugFrame) {
    roots.push(Value::table(frame.environment));
    if let Some(function) = frame.function {
        roots.push(function);
    }
}

fn error_is_uncatchable(error: &Error) -> bool {
    matches!(error.kind, ErrorKind::FuelExhausted { .. })
}

fn error_is_out_of_memory(error: &Error) -> bool {
    matches!(error.kind, ErrorKind::OutOfMemory)
}

fn non_yieldable_boundary_error() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: "attempt to yield across metamethod/C-call boundary".to_owned(),
    })
}

fn attach_instruction_source(
    program: &Program,
    function: &Function,
    pc: usize,
    error: Error,
) -> Error {
    let error = error.at_pc(pc);
    if let Some(location) = instruction_source_location(program, function, pc) {
        error.at_source(&location.source, location.line)
    } else {
        error
    }
}

fn attach_existing_instruction_source(
    program: &Program,
    function: &Function,
    error: Error,
) -> Error {
    if error.location.is_some() || error.location_suppressed {
        return error;
    }
    let Some(pc) = error.pc else {
        return error;
    };
    if let Some(location) = instruction_source_location(program, function, pc) {
        error.at_source(&location.source, location.line)
    } else {
        error
    }
}

fn instruction_source_location(
    program: &Program,
    function: &Function,
    pc: usize,
) -> Option<ErrorLocation> {
    Some(ErrorLocation {
        source: program.source_name.as_ref()?.clone(),
        line: function
            .source_line(pc)
            .or_else(|| function.source_line_at_or_before(pc))?,
    })
}

fn positive_integer_level(level: f64) -> Option<usize> {
    if !level.is_finite() || level < 1.0 {
        return None;
    }
    Some(level as usize)
}

fn unpack_return_call_aux(aux: i32) -> Result<(u8, u8)> {
    if aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative return call argument window",
        }));
    }
    let aux = u32::try_from(aux).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "return call argument window overflow",
        })
    })?;
    let args_start = u8::try_from(aux >> 8).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "return call argument start overflow",
        })
    })?;
    let argc = u8::try_from(aux & 0xff).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "return call argument count overflow",
        })
    })?;
    Ok((args_start, argc))
}

fn unpack_expand_aux(aux: i32) -> Result<(u8, u8)> {
    if aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative expanded call argument window",
        }));
    }
    let aux = u32::try_from(aux).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "expanded call argument window overflow",
        })
    })?;
    let prefix_count = u8::try_from(aux >> 8).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "expanded call argument prefix overflow",
        })
    })?;
    let trailing_argc = u8::try_from(aux & 0xff).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "expanded call trailing argument count overflow",
        })
    })?;
    Ok((prefix_count, trailing_argc))
}

fn unpack_append_call_expand_aux(aux: i32) -> Result<(u32, u8)> {
    if aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative table expanded call append aux",
        }));
    }
    let aux = u32::try_from(aux).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "table expanded call append aux overflow",
        })
    })?;
    let start_index = aux >> 8;
    if start_index == 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "invalid table expanded call append start index",
        }));
    }
    let trailing_argc = u8::try_from(aux & 0xff).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "table expanded call trailing argument count overflow",
        })
    })?;
    Ok((start_index, trailing_argc))
}

fn unpack_nested_expand_aux(aux: i32) -> Result<(u8, u8, u8)> {
    if aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative nested expanded call argument window",
        }));
    }
    let aux = u32::try_from(aux).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested expanded call argument window overflow",
        })
    })?;
    if aux > 0x00ff_ffff {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "nested expanded call argument window overflow",
        }));
    }
    let outer_prefix_count = u8::try_from(aux >> 16).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested expanded call outer prefix overflow",
        })
    })?;
    let middle_prefix_count = u8::try_from((aux >> 8) & 0xff).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested expanded call middle prefix overflow",
        })
    })?;
    let inner_argc = u8::try_from(aux & 0xff).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested expanded call inner argument count overflow",
        })
    })?;
    Ok((outer_prefix_count, middle_prefix_count, inner_argc))
}

fn unpack_expand_varargs_aux(aux: i32) -> Result<(u8, u8)> {
    if aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative nested vararg call argument window",
        }));
    }
    let aux = u32::try_from(aux).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested vararg call argument window overflow",
        })
    })?;
    if aux > 0x0000_ffff {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "nested vararg call argument window overflow",
        }));
    }
    let outer_prefix_count = u8::try_from(aux >> 8).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested vararg call outer prefix overflow",
        })
    })?;
    let middle_prefix_count = u8::try_from(aux & 0xff).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "nested vararg call middle prefix overflow",
        })
    })?;
    Ok((outer_prefix_count, middle_prefix_count))
}

fn write_call_results(
    registers: &mut [Value],
    base: usize,
    result_count: usize,
    results: &[Value],
) -> Result<()> {
    let results_end = base.checked_add(result_count).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "function result window overflow",
        })
    })?;
    if results_end > registers.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: results_end,
            max: registers.len(),
        }));
    }
    for index in 0..result_count {
        let value = results.get(index).copied().unwrap_or_else(Value::nil);
        let register = u8::try_from(base + index).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function result register overflow",
            })
        })?;
        set_register(registers, register, value)?;
    }
    Ok(())
}

fn finish_function_results(
    pending_prefixes: &mut Vec<Value>,
    mut results: Vec<Value>,
) -> Vec<Value> {
    if pending_prefixes.is_empty() {
        return results;
    }
    pending_prefixes.append(&mut results);
    std::mem::take(pending_prefixes)
}

fn same_program_bytecode_function(
    program: &Program,
    kind: Option<FunctionKind>,
) -> Option<(Option<u32>, u32, &Function)> {
    let FunctionKind::Bytecode {
        program: program_id,
        prototype,
    } = kind?
    else {
        return None;
    };
    program
        .functions
        .get(prototype as usize)
        .map(|function| (program_id, prototype, function))
}

fn recursive_reverse_replacement_global(
    program: &Program,
    function: &Function,
) -> Result<Option<i32>> {
    if function.parameter_count != 2 || function.code.len() < 6 {
        return Ok(None);
    }
    let code = &function.code;
    let global = code[0];
    if global.op != Opcode::GetGlobal || global.a != 2 {
        return Ok(None);
    }
    if !instruction_fields(code[1], Opcode::Move, 3, 1, 0, 0)
        || !instruction_fields(code[2], Opcode::Call, 2, 2, 3, 1)
        || !instruction_fields(code[3], Opcode::Move, 3, 0, 0, 0)
        || !instruction_fields(code[4], Opcode::Concat, 2, 2, 3, 0)
        || !instruction_fields(code[5], Opcode::Return, 2, 1, 0, 0)
    {
        return Ok(None);
    }
    if global.aux < 0
        || !matches!(
            program.constants.get(global.aux as usize),
            Some(Constant::String(_))
        )
    {
        return Ok(None);
    }
    Ok(Some(global.aux))
}

fn recursive_reverse_caller_matches(
    program: &Program,
    function: &Function,
    replacement_prototype: u32,
    global_name: i32,
) -> Result<bool> {
    if function.parameter_count != 1 {
        return Ok(false);
    }
    let code = &function.code;
    let Some(return_call) =
        recursive_reverse_caller_return_call(program, code, replacement_prototype)
    else {
        return Ok(false);
    };
    let (args_start, argc) = unpack_return_call_aux(return_call.aux)?;
    if args_start != 2 || argc != 3 {
        return Ok(false);
    }
    let Some(replacement_global) = recursive_reverse_replacement_global(
        program,
        program
            .functions
            .get(replacement_prototype as usize)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "function prototype is missing",
                })
            })?,
    )?
    else {
        return Ok(false);
    };
    Ok(replacement_global == global_name)
}

fn recursive_reverse_caller_return_call(
    program: &Program,
    code: &[Instruction],
    replacement_prototype: u32,
) -> Option<Instruction> {
    if code.len() >= 6
        && code[0].op == Opcode::GetGlobal
        && code[0].a == 1
        && constant_string_equals(program, code[0].aux, b"string")
        && code[1].op == Opcode::GetIndexStringConst
        && code[1].a == 1
        && code[1].b == 1
        && constant_string_equals(program, code[1].aux, b"gsub")
        && instruction_fields(code[2], Opcode::Move, 2, 0, 0, 0)
        && code[3].op == Opcode::LoadConst
        && code[3].a == 3
        && constant_string_equals(
            program,
            i32::from(code[3].c),
            LUAU_RECURSIVE_REVERSE_GSUB_PATTERN,
        )
        && code[4].op == Opcode::LoadFunction
        && code[4].a == 4
        && code[4].aux == replacement_prototype as i32
        && code[5].op == Opcode::ReturnCall
        && code[5].a == 0
        && code[5].b == 0
        && code[5].c == 1
    {
        return Some(code[5]);
    }

    if code.len() >= 7
        && code[0].op == Opcode::GetGlobal
        && code[0].a == 1
        && constant_string_equals(program, code[0].aux, b"string")
        && code[1].op == Opcode::LoadConst
        && code[1].a == 2
        && constant_string_equals(program, i32::from(code[1].c), b"gsub")
        && instruction_fields(code[2], Opcode::GetIndex, 1, 1, 2, 0)
        && instruction_fields(code[3], Opcode::Move, 2, 0, 0, 0)
        && code[4].op == Opcode::LoadConst
        && code[4].a == 3
        && constant_string_equals(
            program,
            i32::from(code[4].c),
            LUAU_RECURSIVE_REVERSE_GSUB_PATTERN,
        )
        && code[5].op == Opcode::LoadFunction
        && code[5].a == 4
        && code[5].aux == replacement_prototype as i32
        && code[6].op == Opcode::ReturnCall
        && code[6].a == 0
        && code[6].b == 0
        && code[6].c == 1
    {
        return Some(code[6]);
    }
    None
}

fn instruction_fields(instruction: Instruction, op: Opcode, a: u8, b: u8, c: u8, aux: i32) -> bool {
    instruction.op == op
        && instruction.a == a
        && instruction.b == b
        && instruction.c == c
        && instruction.aux == aux
}

fn constant_string_equals(program: &Program, index: i32, expected: &[u8]) -> bool {
    if index < 0 {
        return false;
    }
    matches!(
        program.constants.get(index as usize),
        Some(Constant::String(bytes)) if bytes == expected
    )
}

fn load_function_value(
    heap: &mut VmCell,
    program_id: Option<u32>,
    program: &Program,
    instruction: Instruction,
    registers: &[Value],
    register_cells: &mut [Option<u32>],
    parent_upvalues: &[u32],
    environment: u32,
) -> Result<Value> {
    if instruction.aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative function prototype index",
        }));
    }
    let prototype = instruction.aux as usize;
    if prototype >= program.functions.len() {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "function prototype is missing",
        }));
    }
    let prototype = u32::try_from(prototype).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "function prototype index overflow",
        })
    })?;
    let upvalues = program.functions[prototype as usize]
        .upvalues
        .iter()
        .copied()
        .map(|descriptor| {
            capture_upvalue(heap, registers, register_cells, parent_upvalues, descriptor)
        })
        .collect::<Result<Vec<_>>>()?;
    let handle = heap
        .alloc_function(program_id, prototype, upvalues, environment)
        .ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function handle overflow",
            })
        })?;
    Ok(Value::function(handle))
}

fn capture_upvalue(
    heap: &mut VmCell,
    registers: &[Value],
    register_cells: &mut [Option<u32>],
    parent_upvalues: &[u32],
    descriptor: UpvalueDescriptor,
) -> Result<u32> {
    match descriptor {
        UpvalueDescriptor::ParentLocal(register) => {
            ensure_register_cell_for_closure(heap, registers, register_cells, register)
        }
        UpvalueDescriptor::ParentUpvalue(index) => {
            parent_upvalues.get(index as usize).copied().ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "parent upvalue index is missing",
                })
            })
        }
    }
}

fn ensure_register_cell_for_closure(
    heap: &mut VmCell,
    registers: &[Value],
    register_cells: &mut [Option<u32>],
    register: u8,
) -> Result<u32> {
    let cell_index = register as usize;
    if cell_index >= register_cells.len() {
        return Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: cell_index,
            max: register_cells.len(),
        }));
    }
    if let Some(cell) = register_cells[cell_index] {
        return Ok(cell);
    }
    let value = get_register(registers, register)?;
    let cell = heap.alloc_upvalue(value).ok_or_else(|| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "upvalue handle overflow",
        })
    })?;
    register_cells[cell_index] = Some(cell);
    Ok(cell)
}

fn upvalue_index(index: i32) -> Result<usize> {
    usize::try_from(index).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "negative upvalue index",
        })
    })
}

fn same_upvalue_reload_after_set(
    function: &Function,
    upvalues: &[u32],
    pc: usize,
    upvalue: u32,
) -> Option<u8> {
    let get = function.code.get(pc)?;
    if get.op != Opcode::GetUpvalue {
        return None;
    }
    let index = upvalue_index(get.aux).ok()?;
    if upvalues.get(index).copied() == Some(upvalue) {
        Some(get.a)
    } else {
        None
    }
}

fn set_upvalue_refresh_skip_count(
    function: &Function,
    upvalues: &[u32],
    register_cells: &[Option<u32>],
    pc: usize,
    register: u8,
    upvalue: u32,
) -> usize {
    if !matches!(register_cells.get(register as usize), Some(None)) {
        return 0;
    }
    let Some(clear) = function.code.get(pc) else {
        return 0;
    };
    if clear.op != Opcode::ClearLocalCell || clear.a != register {
        return 0;
    }
    let Some(get) = function.code.get(pc + 1) else {
        return 0;
    };
    if get.op != Opcode::GetUpvalue || get.a != register {
        return 0;
    }
    let Ok(index) = upvalue_index(get.aux) else {
        return 0;
    };
    if upvalues.get(index).copied() == Some(upvalue) {
        2
    } else {
        0
    }
}

struct NestedExpandWindow {
    outer_prefix_start: usize,
    middle_callee: usize,
    middle_prefix_start: usize,
    inner_callee: usize,
    inner_args_start: usize,
    inner_args_end: usize,
}

impl NestedExpandWindow {
    fn new(
        base: usize,
        outer_prefix_count: usize,
        middle_prefix_count: usize,
        inner_argc: usize,
    ) -> Result<Self> {
        let outer_prefix_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let middle_callee = outer_prefix_start
            .checked_add(outer_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested expanded call window overflow",
                })
            })?;
        let middle_prefix_start = middle_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let inner_callee = middle_prefix_start
            .checked_add(middle_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested expanded call window overflow",
                })
            })?;
        let inner_args_start = inner_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let inner_args_end = inner_args_start.checked_add(inner_argc).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        Ok(Self {
            outer_prefix_start,
            middle_callee,
            middle_prefix_start,
            inner_callee,
            inner_args_start,
            inner_args_end,
        })
    }

    fn validate(&self, register_count: usize) -> Result<()> {
        if self.inner_args_end > register_count {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: self.inner_args_end,
                max: register_count,
            }));
        }
        Ok(())
    }

    fn middle_callee_register(&self) -> Result<u8> {
        u8::try_from(self.middle_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested middle callee register overflow",
            })
        })
    }

    fn inner_callee_register(&self) -> Result<u8> {
        u8::try_from(self.inner_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested inner callee register overflow",
            })
        })
    }
}

struct NestedVarargWindow {
    outer_prefix_start: usize,
    middle_callee: usize,
    middle_prefix_start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
struct NestedVarargSpec {
    base: usize,
    aux: i32,
}

struct NestedExpand2VarargWindow {
    outer_prefix_start: usize,
    middle_callee: usize,
    middle_prefix_start: usize,
    inner_callee: usize,
    inner_prefix_start: usize,
    end: usize,
}

struct NestedExpand3Window {
    outer_prefix_start: usize,
    middle_callee: usize,
    middle_prefix_start: usize,
    inner_callee: usize,
    inner_prefix_start: usize,
    leaf_callee: usize,
    leaf_args_start: usize,
    leaf_args_end: usize,
}

impl NestedExpand2VarargWindow {
    fn new(
        base: usize,
        outer_prefix_count: usize,
        middle_prefix_count: usize,
        inner_prefix_count: usize,
    ) -> Result<Self> {
        let outer_prefix_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested vararg call window overflow",
            })
        })?;
        let middle_callee = outer_prefix_start
            .checked_add(outer_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested vararg call window overflow",
                })
            })?;
        let middle_prefix_start = middle_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested vararg call window overflow",
            })
        })?;
        let inner_callee = middle_prefix_start
            .checked_add(middle_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested vararg call window overflow",
                })
            })?;
        let inner_prefix_start = inner_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested vararg call window overflow",
            })
        })?;
        let end = inner_prefix_start
            .checked_add(inner_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested vararg call window overflow",
                })
            })?;
        Ok(Self {
            outer_prefix_start,
            middle_callee,
            middle_prefix_start,
            inner_callee,
            inner_prefix_start,
            end,
        })
    }

    fn validate(&self, register_count: usize) -> Result<()> {
        if self.end > register_count {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: self.end,
                max: register_count,
            }));
        }
        Ok(())
    }

    fn middle_callee_register(&self) -> Result<u8> {
        u8::try_from(self.middle_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested middle callee register overflow",
            })
        })
    }

    fn inner_callee_register(&self) -> Result<u8> {
        u8::try_from(self.inner_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested inner callee register overflow",
            })
        })
    }
}

impl NestedExpand3Window {
    fn new(
        base: usize,
        outer_prefix_count: usize,
        middle_prefix_count: usize,
        inner_prefix_count: usize,
        leaf_argc: usize,
    ) -> Result<Self> {
        let outer_prefix_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let middle_callee = outer_prefix_start
            .checked_add(outer_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested expanded call window overflow",
                })
            })?;
        let middle_prefix_start = middle_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let inner_callee = middle_prefix_start
            .checked_add(middle_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested expanded call window overflow",
                })
            })?;
        let inner_prefix_start = inner_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let leaf_callee = inner_prefix_start
            .checked_add(inner_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested expanded call window overflow",
                })
            })?;
        let leaf_args_start = leaf_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        let leaf_args_end = leaf_args_start.checked_add(leaf_argc).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested expanded call window overflow",
            })
        })?;
        Ok(Self {
            outer_prefix_start,
            middle_callee,
            middle_prefix_start,
            inner_callee,
            inner_prefix_start,
            leaf_callee,
            leaf_args_start,
            leaf_args_end,
        })
    }

    fn validate(&self, register_count: usize) -> Result<()> {
        if self.leaf_args_end > register_count {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: self.leaf_args_end,
                max: register_count,
            }));
        }
        Ok(())
    }

    fn middle_callee_register(&self) -> Result<u8> {
        u8::try_from(self.middle_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested middle callee register overflow",
            })
        })
    }

    fn inner_callee_register(&self) -> Result<u8> {
        u8::try_from(self.inner_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested inner callee register overflow",
            })
        })
    }

    fn leaf_callee_register(&self) -> Result<u8> {
        u8::try_from(self.leaf_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested leaf callee register overflow",
            })
        })
    }
}

impl NestedVarargWindow {
    fn new(base: usize, outer_prefix_count: usize, middle_prefix_count: usize) -> Result<Self> {
        let outer_prefix_start = base.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested vararg call window overflow",
            })
        })?;
        let middle_callee = outer_prefix_start
            .checked_add(outer_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested vararg call window overflow",
                })
            })?;
        let middle_prefix_start = middle_callee.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested vararg call window overflow",
            })
        })?;
        let end = middle_prefix_start
            .checked_add(middle_prefix_count)
            .ok_or_else(|| {
                Error::new(ErrorKind::InvalidProgram {
                    reason: "nested vararg call window overflow",
                })
            })?;
        Ok(Self {
            outer_prefix_start,
            middle_callee,
            middle_prefix_start,
            end,
        })
    }

    fn validate(&self, register_count: usize) -> Result<()> {
        if self.end > register_count {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: self.end,
                max: register_count,
            }));
        }
        Ok(())
    }

    fn middle_callee_register(&self) -> Result<u8> {
        u8::try_from(self.middle_callee).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "nested middle callee register overflow",
            })
        })
    }
}

struct NumericForRegisters {
    index: u8,
    limit: u8,
    step: u8,
    visible: u8,
}

impl NumericForRegisters {
    fn new(index: u8) -> Result<Self> {
        let limit = index.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "numeric for register window overflow",
            })
        })?;
        let step = index.checked_add(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "numeric for register window overflow",
            })
        })?;
        let visible = index.checked_add(3).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "numeric for register window overflow",
            })
        })?;
        Ok(Self {
            index,
            limit,
            step,
            visible,
        })
    }
}

struct NumericForStepOneRegisters {
    index: u8,
    limit: u8,
    visible: u8,
}

impl NumericForStepOneRegisters {
    fn new(index: u8) -> Result<Self> {
        let limit = index.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "numeric for step-one register window overflow",
            })
        })?;
        let visible = index.checked_add(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "numeric for step-one register window overflow",
            })
        })?;
        Ok(Self {
            index,
            limit,
            visible,
        })
    }
}

#[derive(Clone, Copy)]
struct ArrayIteratorRegisters {
    table: u8,
    index: u8,
    key: u8,
    value: u8,
}

impl ArrayIteratorRegisters {
    fn new(table: u8) -> Result<Self> {
        let index = table.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "array iterator register window overflow",
            })
        })?;
        let key = table.checked_add(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "array iterator register window overflow",
            })
        })?;
        let value = table.checked_add(3).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "array iterator register window overflow",
            })
        })?;
        Ok(Self {
            table,
            index,
            key,
            value,
        })
    }
}

const GENERALIZED_ITERATOR_MODE_CALL: f64 = 0.0;
const GENERALIZED_ITERATOR_MODE_TABLE: f64 = 1.0;

#[derive(Clone, Copy, Eq, PartialEq)]
enum InvalidIteratorCallee {
    Call,
    Iterate,
}

struct CallIteratorStepContext<'a> {
    program: &'a Program,
    registers: &'a mut [Value],
    register_cells: &'a mut [Option<u32>],
    window: CallIteratorRegisters,
    depth: usize,
    fuel: &'a mut Option<u64>,
    invalid_callee: InvalidIteratorCallee,
}

#[derive(Clone, Copy)]
struct CallIteratorRegisters {
    mode: u8,
    iterator: u8,
    state: u8,
    control: u8,
    first_visible: u8,
    visible_count: u8,
}

impl CallIteratorRegisters {
    fn generic(iterator: u8, visible_count: u8) -> Result<Self> {
        if visible_count == 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "generic call iterator requires at least one visible variable",
            }));
        }
        let state = iterator.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generic call iterator register window overflow",
            })
        })?;
        let control = iterator.checked_add(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generic call iterator register window overflow",
            })
        })?;
        let first_visible = iterator.checked_add(3).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generic call iterator register window overflow",
            })
        })?;
        Ok(Self::new(
            iterator,
            iterator,
            state,
            control,
            first_visible,
            visible_count,
        ))
    }

    const fn new(
        mode: u8,
        iterator: u8,
        state: u8,
        control: u8,
        first_visible: u8,
        visible_count: u8,
    ) -> Self {
        Self {
            mode,
            iterator,
            state,
            control,
            first_visible,
            visible_count,
        }
    }

    fn validate(&self, register_count: usize, reason: &'static str) -> Result<()> {
        let addressable_end = usize::from(u8::MAX) + 1;
        let end = usize::from(self.first_visible)
            .checked_add(usize::from(self.visible_count))
            .ok_or_else(|| Error::new(ErrorKind::InvalidProgram { reason }))?;
        if end > addressable_end {
            return Err(Error::new(ErrorKind::InvalidProgram { reason }));
        }
        if end > register_count {
            return Err(Error::new(ErrorKind::RegisterOutOfBounds {
                register: end,
                max: register_count,
            }));
        }
        Ok(())
    }
}

struct GeneralizedIteratorRegisters {
    mode: u8,
    call: CallIteratorRegisters,
}

impl GeneralizedIteratorRegisters {
    fn new(mode: u8, visible_count: u8) -> Result<Self> {
        if visible_count == 0 {
            return Err(Error::new(ErrorKind::InvalidProgram {
                reason: "generalized iterator requires at least one visible variable",
            }));
        }
        let iterator = mode.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generalized iterator register window overflow",
            })
        })?;
        let state = mode.checked_add(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generalized iterator register window overflow",
            })
        })?;
        let control = mode.checked_add(3).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generalized iterator register window overflow",
            })
        })?;
        let first_visible = mode.checked_add(4).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "generalized iterator register window overflow",
            })
        })?;
        Ok(Self {
            mode,
            call: CallIteratorRegisters::new(
                mode,
                iterator,
                state,
                control,
                first_visible,
                visible_count,
            ),
        })
    }

    fn validate(&self, register_count: usize) -> Result<()> {
        self.call.validate(
            register_count,
            "generalized iterator register window overflow",
        )
    }
}

#[derive(Clone, Copy)]
enum VectorScalarSide {
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum VectorCallStyle {
    Function,
    Method,
}

impl VectorCallStyle {
    fn missing_message(
        self,
        index: usize,
        function: &'static str,
        expected: &'static str,
    ) -> String {
        match self {
            VectorCallStyle::Function => format!(
                "missing argument #{} to '{function}' ({expected} expected)",
                index + 1
            ),
            VectorCallStyle::Method => {
                format!("missing argument #{} ({expected} expected)", index + 1)
            }
        }
    }

    fn invalid_message(
        self,
        index: usize,
        function: &'static str,
        expected: &'static str,
        value: Value,
    ) -> String {
        match self {
            VectorCallStyle::Function => format!(
                "invalid argument #{} to '{function}' ({expected} expected, got {})",
                index + 1,
                value_type_name(value)
            ),
            VectorCallStyle::Method => format!(
                "invalid argument #{} ({expected} expected, got {})",
                index + 1,
                value_type_name(value)
            ),
        }
    }
}

#[derive(Clone, Copy)]
enum VectorUnary {
    Abs,
    Floor,
    Ceil,
    Sign,
}

impl VectorUnary {
    const fn name(self) -> &'static str {
        match self {
            VectorUnary::Abs => "abs",
            VectorUnary::Floor => "floor",
            VectorUnary::Ceil => "ceil",
            VectorUnary::Sign => "sign",
        }
    }
}

#[derive(Clone, Copy)]
enum VectorMinMax {
    Min,
    Max,
}

impl VectorMinMax {
    const fn name(self) -> &'static str {
        match self {
            VectorMinMax::Min => "min",
            VectorMinMax::Max => "max",
        }
    }
}

#[derive(Clone, Copy)]
enum MathUnary {
    Floor,
    Ceil,
    Round,
    Sqrt,
    Deg,
    Rad,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Log10,
    Sign,
}

#[derive(Clone, Copy)]
enum FastNativeMathUnary {
    Abs,
    Unary(MathUnary),
}

fn fast_native_math_unary(native: NativeFunction) -> Option<FastNativeMathUnary> {
    let op = match native {
        NativeFunction::MathAbs => return Some(FastNativeMathUnary::Abs),
        NativeFunction::MathFloor => MathUnary::Floor,
        NativeFunction::MathCeil => MathUnary::Ceil,
        NativeFunction::MathRound => MathUnary::Round,
        NativeFunction::MathSign => MathUnary::Sign,
        NativeFunction::MathSqrt => MathUnary::Sqrt,
        NativeFunction::MathDeg => MathUnary::Deg,
        NativeFunction::MathRad => MathUnary::Rad,
        NativeFunction::MathSin => MathUnary::Sin,
        NativeFunction::MathCos => MathUnary::Cos,
        NativeFunction::MathTan => MathUnary::Tan,
        NativeFunction::MathAsin => MathUnary::Asin,
        NativeFunction::MathAcos => MathUnary::Acos,
        NativeFunction::MathAtan => MathUnary::Atan,
        NativeFunction::MathSinh => MathUnary::Sinh,
        NativeFunction::MathCosh => MathUnary::Cosh,
        NativeFunction::MathTanh => MathUnary::Tanh,
        NativeFunction::MathExp => MathUnary::Exp,
        NativeFunction::MathLog10 => MathUnary::Log10,
        _ => return None,
    };
    Some(FastNativeMathUnary::Unary(op))
}

#[inline(always)]
fn math_unary_result(value: f64, op: MathUnary) -> f64 {
    match op {
        MathUnary::Floor => libm::floor(value),
        MathUnary::Ceil => libm::ceil(value),
        MathUnary::Round => libm::round(value),
        MathUnary::Sqrt => libm::sqrt(value),
        MathUnary::Deg => value * (180.0 / LUAU_PI),
        MathUnary::Rad => value * (LUAU_PI / 180.0),
        MathUnary::Sin => libm::sin(value),
        MathUnary::Cos => libm::cos(value),
        MathUnary::Tan => libm::tan(value),
        MathUnary::Asin => libm::asin(value),
        MathUnary::Acos => libm::acos(value),
        MathUnary::Atan => libm::atan(value),
        MathUnary::Sinh => libm::sinh(value),
        MathUnary::Cosh => libm::cosh(value),
        MathUnary::Tanh => libm::tanh(value),
        MathUnary::Exp => libm::exp(value),
        MathUnary::Log10 => libm::log10(value),
        MathUnary::Sign => {
            if value > 0.0 {
                1.0
            } else if value < 0.0 {
                -1.0
            } else {
                0.0
            }
        }
    }
}

#[derive(Clone, Copy)]
enum MathBinary {
    Fmod,
    Pow,
    Atan2,
}

#[derive(Clone, Copy)]
enum MathPredicate {
    Nan,
    Inf,
    Finite,
}

#[derive(Clone, Copy)]
enum MathMinMax {
    Min,
    Max,
}

const LUAU_PI: f64 = f64::from_bits(0x4009_21fb_5444_2d18);
const PCG32_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const PCG32_INCREMENT: u64 = 105 | 1;
const PERLIN_HASH: [u8; 257] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180, 151,
];
const PERLIN_GRAD: [[f32; 3]; 16] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
    [1.0, 1.0, 0.0],
    [0.0, -1.0, 1.0],
    [-1.0, 1.0, 0.0],
    [0.0, -1.0, -1.0],
];

fn pcg32_seed(seed: u64) -> u64 {
    let (state, _) = pcg32_random(0);
    let (state, _) = pcg32_random(state.wrapping_add(seed));
    state
}

fn pcg32_random(state: u64) -> (u64, u32) {
    let next_state = state
        .wrapping_mul(PCG32_MULTIPLIER)
        .wrapping_add(PCG32_INCREMENT);
    let xorshifted = (((state >> 18) ^ state) >> 27) as u32;
    let rot = (state >> 59) as u32;
    (next_state, xorshifted.rotate_right(rot))
}

fn perlin_noise(x: f32, y: f32, z: f32) -> f32 {
    let x_floor = libm::floorf(x);
    let y_floor = libm::floorf(y);
    let z_floor = libm::floorf(z);

    let xi = (x_floor as i32 & 255) as usize;
    let yi = y_floor as i32 & 255;
    let zi = z_floor as i32 & 255;

    let xf = x - x_floor;
    let yf = y - y_floor;
    let zf = z - z_floor;

    let u = perlin_fade(xf);
    let v = perlin_fade(yf);
    let w = perlin_fade(zf);

    let a = (i32::from(PERLIN_HASH[xi]) + yi) & 255;
    let aa = (i32::from(PERLIN_HASH[a as usize]) + zi) & 255;
    let ab = (i32::from(PERLIN_HASH[(a + 1) as usize]) + zi) & 255;

    let b = (i32::from(PERLIN_HASH[xi + 1]) + yi) & 255;
    let ba = (i32::from(PERLIN_HASH[b as usize]) + zi) & 255;
    let bb = (i32::from(PERLIN_HASH[(b + 1) as usize]) + zi) & 255;

    let la = perlin_lerp(
        u,
        perlin_grad(PERLIN_HASH[aa as usize], xf, yf, zf),
        perlin_grad(PERLIN_HASH[ba as usize], xf - 1.0, yf, zf),
    );
    let lb = perlin_lerp(
        u,
        perlin_grad(PERLIN_HASH[ab as usize], xf, yf - 1.0, zf),
        perlin_grad(PERLIN_HASH[bb as usize], xf - 1.0, yf - 1.0, zf),
    );
    let la1 = perlin_lerp(
        u,
        perlin_grad(PERLIN_HASH[(aa + 1) as usize], xf, yf, zf - 1.0),
        perlin_grad(PERLIN_HASH[(ba + 1) as usize], xf - 1.0, yf, zf - 1.0),
    );
    let lb1 = perlin_lerp(
        u,
        perlin_grad(PERLIN_HASH[(ab + 1) as usize], xf, yf - 1.0, zf - 1.0),
        perlin_grad(PERLIN_HASH[(bb + 1) as usize], xf - 1.0, yf - 1.0, zf - 1.0),
    );

    perlin_lerp(w, perlin_lerp(v, la, lb), perlin_lerp(v, la1, lb1))
}

fn perlin_fade(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn perlin_lerp(t: f32, a: f32, b: f32) -> f32 {
    a + t * (b - a)
}

fn perlin_grad(hash: u8, x: f32, y: f32, z: f32) -> f32 {
    let grad = PERLIN_GRAD[usize::from(hash & 15)];
    grad[0] * x + grad[1] * y + grad[2] * z
}

fn array_iterator_index(value: f64) -> Option<usize> {
    if value.is_finite() && (1.0..=MAX_LUAU_TABLE_INDEX).contains(&value) && value.fract() == 0.0 {
        Some(value as usize)
    } else {
        None
    }
}

fn checked_jump(aux: i32) -> Result<usize> {
    if aux < 0 {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "negative absolute jump target",
        }));
    }
    Ok(aux as usize)
}

fn get_register(registers: &[Value], register: u8) -> Result<Value> {
    registers.get(register as usize).copied().ok_or_else(|| {
        Error::new(ErrorKind::RegisterOutOfBounds {
            register: register as usize,
            max: registers.len(),
        })
    })
}

fn set_register(registers: &mut [Value], register: u8, value: Value) -> Result<()> {
    if let Some(slot) = registers.get_mut(register as usize) {
        *slot = value;
        Ok(())
    } else {
        Err(Error::new(ErrorKind::RegisterOutOfBounds {
            register: register as usize,
            max: registers.len(),
        }))
    }
}

fn extend_weak_collectable_roots(roots: &mut Vec<Value>, values: &[Value]) {
    roots.extend(
        values
            .iter()
            .copied()
            .filter(|value| value_is_weak_collectable_root(*value)),
    );
}

fn extend_bytecode_continuation_roots(roots: &mut Vec<Value>, continuation: &BytecodeContinuation) {
    match continuation {
        BytecodeContinuation::CallExpandAfterTrailing {
            outer_callee, args, ..
        } => {
            roots.push(*outer_callee);
            roots.extend_from_slice(args);
        }
        BytecodeContinuation::ReturnWithPrefix { prefix } => {
            roots.extend_from_slice(prefix);
        }
        BytecodeContinuation::WriteFirst { .. }
        | BytecodeContinuation::WriteFixed { .. }
        | BytecodeContinuation::PCallWriteFixed { .. }
        | BytecodeContinuation::AppendToTable { .. } => {}
    }
}

fn extend_bytecode_continuation_weak_roots(
    roots: &mut Vec<Value>,
    continuation: &BytecodeContinuation,
) {
    match continuation {
        BytecodeContinuation::CallExpandAfterTrailing {
            outer_callee, args, ..
        } => {
            extend_weak_collectable_roots(roots, &[*outer_callee]);
            extend_weak_collectable_roots(roots, args);
        }
        BytecodeContinuation::ReturnWithPrefix { prefix } => {
            extend_weak_collectable_roots(roots, prefix);
        }
        BytecodeContinuation::WriteFirst { .. }
        | BytecodeContinuation::WriteFixed { .. }
        | BytecodeContinuation::PCallWriteFixed { .. }
        | BytecodeContinuation::AppendToTable { .. } => {}
    }
}

fn extend_stackless_self_call_frame_roots(roots: &mut Vec<Value>, frame: &StacklessSelfCallFrame) {
    roots.extend_from_slice(&frame.registers);
    roots.extend_from_slice(&frame.varargs);
    roots.extend_from_slice(&frame.pending_return_prefixes);
    extend_bytecode_continuation_roots(roots, &frame.continuation);
}

fn extend_stackless_self_call_frame_weak_roots(
    roots: &mut Vec<Value>,
    frame: &StacklessSelfCallFrame,
) {
    extend_weak_collectable_roots(roots, &frame.registers);
    extend_weak_collectable_roots(roots, &frame.varargs);
    extend_weak_collectable_roots(roots, &frame.pending_return_prefixes);
    extend_bytecode_continuation_weak_roots(roots, &frame.continuation);
}

fn value_is_weak_collectable_root(value: Value) -> bool {
    matches!(
        value.kind(),
        ValueKind::Table | ValueKind::Function | ValueKind::Userdata | ValueKind::Buffer
    )
}

fn reset_current_function_frame(
    function: &Function,
    registers: &mut [Value],
    register_cells: &mut [Option<u32>],
    varargs: &mut Vec<Value>,
    args: &[Value],
) -> Result<()> {
    registers.fill(Value::nil());
    register_cells.fill(None);
    for (index, arg) in args
        .iter()
        .copied()
        .take(function.parameter_count)
        .enumerate()
    {
        let register = u8::try_from(index).map_err(|_| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "function parameter register index overflow",
            })
        })?;
        set_register(registers, register, arg)?;
    }
    if function.is_vararg && args.len() > function.parameter_count {
        *varargs = args[function.parameter_count..].to_vec();
    } else {
        varargs.clear();
    }
    Ok(())
}

fn fresh_register_cells(function: &Function) -> Vec<Option<u32>> {
    if function_uses_register_cells(function) {
        vec![None; function.max_registers]
    } else {
        Vec::new()
    }
}

fn function_uses_register_cells(function: &Function) -> bool {
    function.code.iter().any(|instruction| {
        matches!(
            instruction.op,
            Opcode::GetLocalCell | Opcode::SetLocalCell | Opcode::LoadFunction
        )
    })
}

fn empty_return_follows(function: &Function, next_pc: usize) -> bool {
    function
        .code
        .get(next_pc)
        .is_some_and(|instruction| instruction.op == Opcode::Return && instruction.b == 0)
}

fn get_number(registers: &[Value], register: u8) -> Result<f64> {
    get_register(registers, register)?
        .as_number()
        .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "number" }))
}

fn get_numeric_for_number(
    registers: &[Value],
    register: u8,
    value_name: &'static str,
) -> Result<f64> {
    let value = get_register(registers, register)?;
    value
        .as_number()
        .ok_or_else(|| numeric_for_type_error(value, value_name))
}

fn numeric_for_type_error(value: Value, value_name: &'static str) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: format!(
            "invalid 'for' {value_name} (number expected, got {})",
            value_type_name(value)
        ),
    })
}

fn numeric_for_value_name(code: u8) -> &'static str {
    match code {
        0 => "initial value",
        1 => "limit",
        2 => "step",
        _ => "value",
    }
}

fn validate_math_arity(
    instruction: Instruction,
    expected: i32,
    reason: &'static str,
) -> Result<()> {
    if instruction.aux == expected {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidProgram { reason }))
    }
}

fn validate_math_minmax_arity(instruction: Instruction) -> Result<u8> {
    if instruction.aux < 1 || instruction.aux > i32::from(u8::MAX) {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "invalid math.min/math.max arity",
        }));
    }
    Ok(instruction.aux as u8)
}

fn table_concat_range(
    registers: &[Value],
    instruction: Instruction,
) -> Result<(Option<usize>, Option<usize>)> {
    let arity = instruction.aux;
    if !(1..=4).contains(&arity) {
        return Err(Error::new(ErrorKind::InvalidProgram {
            reason: "invalid table.concat arity",
        }));
    }
    let start = if arity >= 3 {
        let register = instruction.c.checked_add(1).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table.concat start register overflow",
            })
        })?;
        Some(table_concat_index(get_number(registers, register)?)?)
    } else {
        None
    };
    let end = if arity >= 4 {
        let register = instruction.c.checked_add(2).ok_or_else(|| {
            Error::new(ErrorKind::InvalidProgram {
                reason: "table.concat end register overflow",
            })
        })?;
        Some(table_concat_index(get_number(registers, register)?)?)
    } else {
        None
    };
    Ok((start, end))
}

fn new_array_capacity_hints(aux: i32) -> (usize, usize) {
    match aux.cmp(&0) {
        std::cmp::Ordering::Greater => (aux as usize, 0),
        std::cmp::Ordering::Equal => (0, 0),
        std::cmp::Ordering::Less => {
            let raw = aux.checked_neg().unwrap_or(i32::MAX) as usize;
            if raw <= u16::MAX as usize {
                (0, raw)
            } else {
                (raw >> 16, raw & u16::MAX as usize)
            }
        }
    }
}

fn table_concat_index(value: f64) -> Result<usize> {
    if !value.is_finite() || value.fract() != 0.0 || !(0.0..=MAX_LUAU_TABLE_INDEX).contains(&value)
    {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer table.concat range",
        }));
    }
    Ok(value as usize)
}

fn table_find_start(value: f64) -> Result<usize> {
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer table.find init",
        }));
    }
    if !(1.0..=MAX_LUAU_TABLE_INDEX).contains(&value) {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "index out of range",
        }));
    }
    Ok(value as usize)
}

fn table_create_size(value: f64) -> Result<usize> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer table.create size",
        }));
    }
    let size = value as i32;
    if size < 0 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "size out of range",
        }));
    }
    let size = size as usize;
    if size > LUAU_TABLE_MAX_SIZE {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "table overflow",
        }));
    }
    if size > MAX_TABLE_CREATE_ARRAY_SIZE {
        return Err(Error::new(ErrorKind::OutOfMemory).without_source_location());
    }
    Ok(size)
}

fn table_len_to_i32(len: usize) -> Result<i32> {
    i32::try_from(len).map_err(|_| {
        Error::new(ErrorKind::TypeError {
            expected: "table overflow",
        })
    })
}

fn table_integer_arg(value: f64, expected: &'static str) -> Result<i32> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError { expected }));
    }
    Ok(value as i32)
}

fn table_insert_position_arg(value: f64) -> Result<i32> {
    if value.is_nan() {
        return Ok(0);
    }
    table_integer_arg(value, "integer table.insert position")
}

fn table_move_count(start: i32, end: i32) -> Result<usize> {
    if end < start {
        return Ok(0);
    }
    let count = i64::from(end) - i64::from(start) + 1;
    if count > i64::from(i32::MAX) {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "too many elements to move".to_owned(),
        }));
    }
    usize::try_from(count).map_err(|_| {
        Error::new(ErrorKind::RuntimeError {
            message: "too many elements to move".to_owned(),
        })
    })
}

fn table_move_destination_end(destination_start: i32, count: usize) -> Result<i32> {
    let count = i64::try_from(count).map_err(|_| {
        Error::new(ErrorKind::RuntimeError {
            message: "too many elements to move".to_owned(),
        })
    })?;
    if count == 0 {
        return Ok(destination_start);
    }
    let destination_end = i64::from(destination_start) + count - 1;
    if destination_end < i64::from(i32::MIN) || destination_end > i64::from(i32::MAX) {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "destination wrap around".to_owned(),
        }));
    }
    Ok(destination_end as i32)
}

const LUAU_PACK_MAX_ALIGN: usize = 8;
const LUAU_PACK_MAX_OPTION_SIZE: usize = 16;
const LUAU_PACK_DEFAULT_INT_SIZE: usize = 4;
const LUAU_PACK_LONG_SIZE: usize = 8;
const LUAU_PACK_SIZE_T_SIZE: usize = 4;
const LUAU_MAX_STRING_BYTES: usize = 1 << 30;
const BIT32_MODULUS: f64 = 4_294_967_296.0; // 2^32

#[derive(Clone, Copy)]
enum Bit32Fold {
    And,
    Or,
    Xor,
}

#[derive(Clone, Copy)]
enum Bit32Shift {
    LogicalLeft,
    LogicalRight,
    ArithmeticRight,
}

#[derive(Clone, Copy)]
enum Bit32Rotate {
    Left,
    Right,
}

#[derive(Clone, Copy)]
enum Bit32Count {
    Leading,
    Trailing,
}

fn bit32_number_to_u32(value: f64) -> Result<u32> {
    if !value.is_finite() {
        return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
    }
    let reduced = value.trunc().rem_euclid(BIT32_MODULUS);
    Ok(reduced as u32)
}

fn bit32_number_to_i32(value: f64) -> Result<i32> {
    let truncated = value.trunc();
    if !truncated.is_finite() || truncated < i32::MIN as f64 || truncated > i32::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer",
        }));
    }
    Ok(truncated as i32)
}

fn bit32_lshift(value: u32, shift: i32) -> u32 {
    if shift <= -32 || shift >= 32 {
        0
    } else if shift < 0 {
        value >> (-shift as u32)
    } else {
        value << (shift as u32)
    }
}

fn bit32_rshift(value: u32, shift: i32) -> u32 {
    if shift <= -32 || shift >= 32 {
        0
    } else if shift < 0 {
        value << (-shift as u32)
    } else {
        value >> (shift as u32)
    }
}

fn bit32_arshift(value: u32, shift: i32) -> u32 {
    if shift < 0 {
        return bit32_lshift(value, shift.saturating_neg());
    }
    if shift >= 32 {
        if (value & 0x8000_0000) == 0 {
            0
        } else {
            u32::MAX
        }
    } else {
        ((value as i32) >> (shift as u32)) as u32
    }
}

fn validate_bit32_field_width(field: u32, width: u32) -> Result<()> {
    if width == 0 || field >= 32 || width > 32 || field.saturating_add(width) > 32 {
        return Err(bit32_range_error());
    }
    Ok(())
}

fn bit32_width_mask(width: u32) -> u32 {
    if width == 32 {
        u32::MAX
    } else {
        (1u32 << width) - 1
    }
}

fn bit32_range_error() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: "trying to access non-existent bits".to_owned(),
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum PackEndian {
    Little,
    Big,
}

#[derive(Clone, Copy)]
struct PackState {
    endian: PackEndian,
    max_align: usize,
}

impl Default for PackState {
    fn default() -> Self {
        Self {
            endian: PackEndian::Little,
            max_align: 1,
        }
    }
}

#[derive(Clone, Copy)]
enum PackItem {
    Data(PackData),
    AlignOnly { align: usize },
}

#[derive(Clone, Copy)]
struct PackData {
    kind: PackDataKind,
    size: usize,
    align: usize,
    variable_size: bool,
}

#[derive(Clone, Copy)]
enum PackDataKind {
    Pad,
    SignedInteger { size: usize },
    UnsignedInteger { size: usize },
    Float32,
    Float64,
    FixedString { size: usize },
    ZeroTerminatedString,
    SizedString { size: usize },
}

struct PackFormatReader<'a> {
    format: &'a [u8],
    cursor: usize,
}

impl<'a> PackFormatReader<'a> {
    fn new(format: &'a [u8]) -> Self {
        Self { format, cursor: 0 }
    }

    fn next_item(&mut self, state: &mut PackState) -> Result<Option<PackItem>> {
        loop {
            self.skip_spaces();
            let Some(option) = self.next_byte() else {
                return Ok(None);
            };
            match option {
                b'<' => state.endian = PackEndian::Little,
                b'>' => state.endian = PackEndian::Big,
                b'=' => state.endian = PackEndian::Little,
                b'!' => {
                    let align = self.parse_number(LUAU_PACK_MAX_ALIGN)?;
                    validate_pack_align_limit(align)?;
                    state.max_align = align;
                }
                b'X' => {
                    let data = self.next_x_alignment_data(state)?;
                    return Ok(Some(PackItem::AlignOnly { align: data.align }));
                }
                option => return self.data_item_for_option(option, state).map(Some),
            }
        }
    }

    fn next_x_alignment_data(&mut self, state: &PackState) -> Result<PackData> {
        let Some(option) = self.next_byte() else {
            return Err(pack_runtime_error("invalid next option for option 'X'"));
        };
        if matches!(option, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c | b'X') {
            return Err(pack_runtime_error("invalid next option for option 'X'"));
        }
        let item = self.data_item_for_option(option, state)?;
        let PackItem::Data(data) = item else {
            unreachable!("data_item_for_option returns data");
        };
        if data.variable_size || matches!(data.kind, PackDataKind::FixedString { .. }) {
            return Err(pack_runtime_error("invalid next option for option 'X'"));
        }
        Ok(data)
    }

    fn data_item_for_option(&mut self, option: u8, state: &PackState) -> Result<PackItem> {
        let data = match option {
            b'x' => PackData {
                kind: PackDataKind::Pad,
                size: 1,
                align: 1,
                variable_size: false,
            },
            b'b' => self.integer_data(PackDataKind::SignedInteger { size: 1 }, 1, state)?,
            b'B' => self.integer_data(PackDataKind::UnsignedInteger { size: 1 }, 1, state)?,
            b'h' => self.integer_data(PackDataKind::SignedInteger { size: 2 }, 2, state)?,
            b'H' => self.integer_data(PackDataKind::UnsignedInteger { size: 2 }, 2, state)?,
            b'l' => self.integer_data(
                PackDataKind::SignedInteger {
                    size: LUAU_PACK_LONG_SIZE,
                },
                LUAU_PACK_LONG_SIZE,
                state,
            )?,
            b'L' => self.integer_data(
                PackDataKind::UnsignedInteger {
                    size: LUAU_PACK_LONG_SIZE,
                },
                LUAU_PACK_LONG_SIZE,
                state,
            )?,
            b'j' => self.integer_data(
                PackDataKind::SignedInteger {
                    size: LUAU_PACK_DEFAULT_INT_SIZE,
                },
                LUAU_PACK_DEFAULT_INT_SIZE,
                state,
            )?,
            b'J' => self.integer_data(
                PackDataKind::UnsignedInteger {
                    size: LUAU_PACK_DEFAULT_INT_SIZE,
                },
                LUAU_PACK_DEFAULT_INT_SIZE,
                state,
            )?,
            b'T' => self.integer_data(
                PackDataKind::UnsignedInteger {
                    size: LUAU_PACK_SIZE_T_SIZE,
                },
                LUAU_PACK_SIZE_T_SIZE,
                state,
            )?,
            b'i' => {
                let size = self.parse_number(LUAU_PACK_DEFAULT_INT_SIZE)?;
                validate_pack_option_size(size)?;
                self.integer_data(PackDataKind::SignedInteger { size }, size, state)?
            }
            b'I' => {
                let size = self.parse_number(LUAU_PACK_DEFAULT_INT_SIZE)?;
                validate_pack_option_size(size)?;
                self.integer_data(PackDataKind::UnsignedInteger { size }, size, state)?
            }
            b'f' => PackData {
                kind: PackDataKind::Float32,
                size: 4,
                align: pack_alignment(4, state.max_align)?,
                variable_size: false,
            },
            b'd' | b'n' => PackData {
                kind: PackDataKind::Float64,
                size: 8,
                align: pack_alignment(8, state.max_align)?,
                variable_size: false,
            },
            b'c' => {
                let size = self.parse_required_number("missing size for format option 'c'")?;
                PackData {
                    kind: PackDataKind::FixedString { size },
                    size,
                    align: 1,
                    variable_size: false,
                }
            }
            b'z' => PackData {
                kind: PackDataKind::ZeroTerminatedString,
                size: 0,
                align: 1,
                variable_size: true,
            },
            b's' => {
                let size = self.parse_number(LUAU_PACK_SIZE_T_SIZE)?;
                validate_pack_option_size(size)?;
                PackData {
                    kind: PackDataKind::SizedString { size },
                    size,
                    align: pack_alignment(size, state.max_align)?,
                    variable_size: true,
                }
            }
            other => {
                return Err(pack_runtime_error(format!(
                    "invalid format option '{}'",
                    char::from(other)
                )));
            }
        };
        Ok(PackItem::Data(data))
    }

    fn integer_data(&self, kind: PackDataKind, size: usize, state: &PackState) -> Result<PackData> {
        Ok(PackData {
            kind,
            size,
            align: pack_alignment(size, state.max_align)?,
            variable_size: false,
        })
    }

    fn parse_required_number(&mut self, missing: &'static str) -> Result<usize> {
        if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            return Err(pack_runtime_error(missing));
        }
        self.parse_number(0)
    }

    fn parse_number(&mut self, default: usize) -> Result<usize> {
        if !self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            return Ok(default);
        }
        let mut value = 0usize;
        while let Some(byte) = self.peek() {
            if !byte.is_ascii_digit() {
                break;
            }
            let digit = usize::from(byte - b'0');
            value = value
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| pack_runtime_error("size specifier is too large"))?;
            if value > LUAU_MAX_STRING_BYTES {
                return Err(pack_runtime_error("size specifier is too large"));
            }
            self.cursor += 1;
        }
        Ok(value)
    }

    fn skip_spaces(&mut self) {
        while self
            .peek()
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c))
        {
            self.cursor += 1;
        }
    }

    fn next_byte(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.cursor += 1;
        Some(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.format.get(self.cursor).copied()
    }
}

fn validate_pack_align_limit(align: usize) -> Result<()> {
    if align == 0 || align > LUAU_PACK_MAX_OPTION_SIZE {
        return Err(pack_runtime_error(
            "format asks for alignment out of limits",
        ));
    }
    if !align.is_power_of_two() {
        return Err(pack_runtime_error(
            "format asks for alignment not power of 2",
        ));
    }
    Ok(())
}

fn validate_pack_option_size(size: usize) -> Result<()> {
    if size == 0 || size > LUAU_PACK_MAX_OPTION_SIZE {
        return Err(pack_runtime_error(format!(
            "integral size ({size}) out of limits [1,{LUAU_PACK_MAX_OPTION_SIZE}]"
        )));
    }
    Ok(())
}

fn pack_alignment(size: usize, max_align: usize) -> Result<usize> {
    let align = size.min(max_align).max(1);
    if !align.is_power_of_two() {
        return Err(pack_runtime_error(
            "format asks for alignment not power of 2",
        ));
    }
    Ok(align)
}

fn push_pack_padding(output: &mut Vec<u8>, align: usize) -> Result<()> {
    let padding = pack_padding(output.len(), align)?;
    let padded_len = checked_pack_position(output.len(), padding)?;
    output.resize(padded_len, 0);
    Ok(())
}

fn pack_padding(position: usize, align: usize) -> Result<usize> {
    if align <= 1 {
        return Ok(0);
    }
    if !align.is_power_of_two() {
        return Err(pack_runtime_error(
            "format asks for alignment not power of 2",
        ));
    }
    Ok((align - (position % align)) % align)
}

fn checked_pack_position(position: usize, added: usize) -> Result<usize> {
    let next = position
        .checked_add(added)
        .ok_or_else(|| pack_runtime_error("format result too large"))?;
    if next > LUAU_MAX_STRING_BYTES {
        return Err(pack_runtime_error("format result too large"));
    }
    Ok(next)
}

fn pack_signed_integer(value: f64, size: usize) -> Result<i64> {
    if !value.is_finite() {
        return Err(Error::new(ErrorKind::TypeError { expected: "number" }));
    }
    if size < 8 {
        let bits = (size * 8) as i32;
        let min = -2_f64.powi(bits - 1);
        let max = 2_f64.powi(bits - 1) - 1.0;
        if value < min || value > max {
            return Err(pack_runtime_error("integer overflow"));
        }
    } else if value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(pack_runtime_error("integer overflow"));
    }
    Ok(value as i64)
}

fn pack_unsigned_integer(value: f64, size: usize) -> Result<u64> {
    if !value.is_finite() || value < 0.0 {
        return Err(pack_runtime_error("unsigned overflow"));
    }
    let max = unsigned_integer_max(size);
    if value > max {
        return Err(pack_runtime_error("unsigned overflow"));
    }
    Ok(value as u64)
}

fn unsigned_integer_max(size: usize) -> f64 {
    if size >= 8 {
        u64::MAX as f64
    } else {
        2_f64.powi((size * 8) as i32) - 1.0
    }
}

fn push_pack_integer(
    output: &mut Vec<u8>,
    raw: u64,
    size: usize,
    endian: PackEndian,
    signed: bool,
) {
    let extension = if signed { 0xff } else { 0x00 };
    match endian {
        PackEndian::Little => {
            for index in 0..size {
                output.push(pack_integer_byte(raw, index, extension));
            }
        }
        PackEndian::Big => {
            for index in (0..size).rev() {
                output.push(pack_integer_byte(raw, index, extension));
            }
        }
    }
}

fn pack_integer_byte(raw: u64, index: usize, extension: u8) -> u8 {
    if index < 8 {
        ((raw >> (index * 8)) & 0xff) as u8
    } else {
        extension
    }
}

fn unpack_start_position(position: Option<Value>, len: usize) -> Result<usize> {
    let Some(position) = position else {
        return Ok(0);
    };
    if position.kind() == ValueKind::Nil {
        return Ok(0);
    }
    let index = string_library_index(number_arg(position)?)?;
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let index = if index < 0 { len + index + 1 } else { index };
    if index == 0 {
        return Ok(0);
    }
    if index < 1 || index > len + 1 {
        return Err(pack_runtime_error("initial position out of string"));
    }
    usize::try_from(index - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.unpack position underflow",
        })
    })
}

fn ensure_unpack_available(data: &[u8], cursor: usize, size: usize) -> Result<()> {
    let Some(end) = cursor.checked_add(size) else {
        return Err(pack_runtime_error("data string too short"));
    };
    if end > data.len() {
        return Err(pack_runtime_error("data string too short"));
    }
    Ok(())
}

fn unpack_bytes(data: &[u8], cursor: usize, size: usize) -> Result<&[u8]> {
    ensure_unpack_available(data, cursor, size)?;
    Ok(&data[cursor..cursor + size])
}

fn unpack_unsigned_integer(bytes: &[u8], size: usize, endian: PackEndian) -> Result<u64> {
    if size > 8 {
        for index in 8..size {
            if ordered_pack_byte(bytes, size, index, endian) != 0 {
                return Err(pack_runtime_error(format!(
                    "{size}-byte integer does not fit"
                )));
            }
        }
    }
    let mut raw = 0u64;
    for index in 0..size.min(8) {
        raw |= u64::from(ordered_pack_byte(bytes, size, index, endian)) << (index * 8);
    }
    Ok(raw)
}

fn unpack_signed_integer(bytes: &[u8], size: usize, endian: PackEndian) -> Result<i64> {
    let sign = (ordered_pack_byte(bytes, size, size - 1, endian) & 0x80) != 0;
    let extension = if sign { 0xff } else { 0x00 };
    if size > 8 {
        for index in 8..size {
            if ordered_pack_byte(bytes, size, index, endian) != extension {
                return Err(pack_runtime_error(format!(
                    "{size}-byte integer does not fit"
                )));
            }
        }
    }
    let mut raw = 0u64;
    for index in 0..size.min(8) {
        raw |= u64::from(ordered_pack_byte(bytes, size, index, endian)) << (index * 8);
    }
    if sign && size < 8 {
        raw |= !0u64 << (size * 8);
    }
    Ok(raw as i64)
}

fn ordered_pack_byte(bytes: &[u8], size: usize, little_index: usize, endian: PackEndian) -> u8 {
    match endian {
        PackEndian::Little => bytes[little_index],
        PackEndian::Big => bytes[size - little_index - 1],
    }
}

fn pack_runtime_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: message.into(),
    })
}

fn string_library_index(value: f64) -> Result<i64> {
    let value = value.trunc();
    if !value.is_finite() || value < i64::MIN as f64 || value > i64::MAX as f64 {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer string library index",
        }));
    }
    Ok(value as i64)
}

fn luau_string_sub_range(len: usize, start: i64, end: i64) -> Result<Option<(usize, usize)>> {
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let start = if start < 0 { len + start + 1 } else { start }.max(1);
    let end = if end < 0 { len + end + 1 } else { end }.min(len);
    if start > end {
        return Ok(None);
    }
    let start = usize::try_from(start - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.sub start index underflow",
        })
    })?;
    let end = usize::try_from(end).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.sub end index underflow",
        })
    })?;
    Ok(Some((start, end)))
}

fn normalize_string_byte_start(
    len: usize,
    index: i64,
    clamp_to_first: bool,
) -> Result<Option<usize>> {
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let mut one_based = if index < 0 { len + index + 1 } else { index };
    if one_based < 1 && clamp_to_first && len > 0 {
        one_based = 1;
    }
    if one_based < 1 || one_based > len {
        return Ok(None);
    }
    Ok(Some(usize::try_from(one_based - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string index underflow",
        })
    })?))
}

fn normalize_string_byte_end(len: usize, index: i64) -> Result<Option<usize>> {
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let one_based = if index < 0 { len + index + 1 } else { index };
    if one_based < 1 {
        return Ok(None);
    }
    let one_based = one_based.min(len);
    Ok(Some(usize::try_from(one_based - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string index underflow",
        })
    })?))
}

fn reverse_string_bytes(text: &[u8]) -> Result<Vec<u8>> {
    Ok(text.iter().rev().copied().collect::<Vec<_>>())
}

fn encode_luau_utf8_char(codepoint: u32, output: &mut Vec<u8>) {
    if codepoint < 0x80 {
        output.push(codepoint as u8);
    } else if codepoint < 0x800 {
        output.push((0xc0 | (codepoint >> 6)) as u8);
        output.push((0x80 | (codepoint & 0x3f)) as u8);
    } else if codepoint < 0x1_0000 {
        output.push((0xe0 | (codepoint >> 12)) as u8);
        output.push((0x80 | ((codepoint >> 6) & 0x3f)) as u8);
        output.push((0x80 | (codepoint & 0x3f)) as u8);
    } else {
        output.push((0xf0 | (codepoint >> 18)) as u8);
        output.push((0x80 | ((codepoint >> 12) & 0x3f)) as u8);
        output.push((0x80 | ((codepoint >> 6) & 0x3f)) as u8);
        output.push((0x80 | (codepoint & 0x3f)) as u8);
    }
}

fn decode_luau_utf8_at(text: &[u8], index: usize) -> Option<(usize, u32)> {
    let first = *text.get(index)?;
    if first < 0x80 {
        return Some((index + 1, u32::from(first)));
    }

    let (continuations, min_codepoint) = match first {
        0xc2..=0xdf => (1usize, 0x7f),
        0xe0..=0xef => (2usize, 0x7ff),
        0xf0..=0xf4 => (3usize, 0xffff),
        _ => return None,
    };
    let next = index.checked_add(continuations + 1)?;
    if next > text.len() {
        return None;
    }

    let mut codepoint = u32::from(first & (0x7f >> continuations));
    for byte in &text[index + 1..next] {
        if (byte & 0xc0) != 0x80 {
            return None;
        }
        codepoint = (codepoint << 6) | u32::from(byte & 0x3f);
    }
    if codepoint <= min_codepoint || codepoint > 0x10ffff || (0xd800..=0xdfff).contains(&codepoint)
    {
        return None;
    }
    Some((next, codepoint))
}

fn utf8_is_continuation_at(text: &[u8], index: i64) -> bool {
    let Ok(index) = usize::try_from(index) else {
        return false;
    };
    text.get(index).is_some_and(|byte| (byte & 0xc0) == 0x80)
}

fn luau_utf8_posrelat(position: i64, len: i64) -> i64 {
    if position >= 0 {
        return position;
    }
    let back = position.checked_neg().unwrap_or(i64::MAX);
    if back > len {
        0
    } else {
        len + position + 1
    }
}

const MAX_STRING_REPEAT_BYTES: usize = 1 << 30;

fn string_repeat_count(value: f64) -> Result<i64> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < i64::MIN as f64
        || value > i64::MAX as f64
    {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "integer string.rep count",
        }));
    }
    Ok(value as i64)
}

fn repeat_string(text: &[u8], count: i64) -> Result<Vec<u8>> {
    if count <= 0 || text.is_empty() {
        return Ok(Vec::new());
    }
    let count = usize::try_from(count).map_err(|_| {
        Error::new(ErrorKind::TypeError {
            expected: "non-negative string.rep count",
        })
    })?;
    let total_len = text.len().checked_mul(count).ok_or_else(|| {
        Error::new(ErrorKind::TypeError {
            expected: "string.rep result below allocation limit",
        })
    })?;
    if total_len > MAX_STRING_REPEAT_BYTES {
        return Err(Error::new(ErrorKind::TypeError {
            expected: "string.rep result below allocation limit",
        }));
    }
    Ok(text.repeat(count))
}

fn luau_string_find_first(
    text: &[u8],
    pattern: &[u8],
    start: i64,
    plain: bool,
) -> Result<Option<usize>> {
    Ok(luau_string_find_range(text, pattern, start, plain)?.map(|(start, _end)| start))
}

fn luau_string_find_range(
    text: &[u8],
    pattern: &[u8],
    start: i64,
    plain: bool,
) -> Result<Option<(usize, usize)>> {
    let Some(start) = normalize_string_find_start(text.len(), start)? else {
        return Ok(None);
    };
    if plain || !has_luau_pattern_special(pattern) {
        return Ok(find_plain_bytes(text, pattern, start)
            .map(|start| (start, string_find_end(start, pattern.len()))));
    }
    if pattern == b"^[%u_]" {
        return Ok(find_start_upper_or_underscore_pattern(text, start).map(|index| (index, index)));
    }
    if let Some(range) = find_anchored_string_chunk_prefix_pattern(text, pattern, start) {
        return Ok(range);
    }
    if let Some(range) = find_anchored_utf8_charpattern(text, pattern, start) {
        return Ok(range);
    }
    if let Some(range) = find_anchored_simple_luau_pattern(text, pattern, start) {
        return Ok(range);
    }
    if let Some(pattern) = parse_capturing_luau_pattern(pattern)? {
        return Ok(
            find_capturing_luau_pattern_match(text, &pattern, start)?.map(|matched| {
                (
                    matched.start + 1,
                    string_find_end(matched.start + 1, matched.end - matched.start),
                )
            }),
        );
    }
    if let Some(pattern) = parse_noncapturing_luau_pattern(pattern)? {
        return Ok(find_noncapturing_luau_pattern(text, &pattern, start));
    }
    if let Some(parsed) = parse_simple_luau_pattern(pattern) {
        return Ok(find_simple_luau_pattern(text, &parsed, start)
            .map(|start| (start, string_find_end(start, parsed.len()))));
    }
    Err(Error::new(ErrorKind::Unsupported {
        feature: "string.find pattern",
    }))
}

fn parse_gsub_pattern(pattern: &[u8]) -> Result<GSubPattern> {
    if pattern.is_empty() {
        return Ok(GSubPattern::Empty);
    }
    if pattern == b"%s+" {
        return Ok(GSubPattern::AsciiWhitespacePlus);
    }
    if pattern == b"(%p)" {
        return Ok(GSubPattern::AsciiPunctuationCapture);
    }
    if pattern == b"%%([0-9])" {
        return Ok(GSubPattern::PercentDigitCapture);
    }
    if pattern == b"^(^?)" {
        return Ok(GSubPattern::StartOptionalCaretCapture);
    }
    if pattern == b"($?)$" {
        return Ok(GSubPattern::EndOptionalDollarCapture);
    }
    if pattern == b"." {
        return Ok(GSubPattern::ByteClass(luau_any_byte_class()));
    }
    if pattern.len() == 4 && pattern[0] == b'%' && pattern[1] == b'b' {
        return Ok(GSubPattern::Balanced {
            open: pattern[2],
            close: pattern[3],
        });
    }
    if pattern.len() == 2 && pattern.first() == Some(&b'%') {
        if let Some(byte_class) = luau_percent_class(pattern[1]) {
            return Ok(GSubPattern::ByteClass(byte_class));
        }
    }
    if let Some(byte_class) = parse_luau_byte_class_pattern(pattern) {
        return Ok(GSubPattern::ByteClass(byte_class));
    }
    if let Some(pattern) = parse_capturing_luau_pattern(pattern)? {
        return Ok(GSubPattern::Capturing(pattern));
    }
    if let Some(pattern) = parse_noncapturing_luau_pattern(pattern)? {
        return Ok(GSubPattern::NonCapturing(pattern));
    }
    if has_luau_pattern_special(pattern) {
        return Err(Error::new(ErrorKind::Unsupported {
            feature: "string.gsub pattern",
        }));
    }
    Ok(GSubPattern::Plain(pattern.to_vec()))
}

fn find_next_gsub_match(
    text: &[u8],
    pattern: &GSubPattern,
    cursor: usize,
) -> Result<Option<GSubMatch>> {
    Ok(match pattern {
        GSubPattern::Empty => {
            if cursor > text.len() {
                return Ok(None);
            }
            Some(GSubMatch {
                start: cursor,
                end: cursor,
                captures: Vec::new(),
            })
        }
        GSubPattern::Plain(pattern) => {
            let Some(one_based_start) = find_plain_bytes(text, pattern, cursor) else {
                return Ok(None);
            };
            let start = one_based_start - 1;
            Some(GSubMatch {
                start,
                end: start + pattern.len(),
                captures: Vec::new(),
            })
        }
        GSubPattern::AsciiWhitespacePlus => {
            let Some(tail) = text.get(cursor..) else {
                return Ok(None);
            };
            let Some(start) = tail
                .iter()
                .position(|byte| byte.is_ascii_whitespace())
                .map(|offset| cursor + offset)
            else {
                return Ok(None);
            };
            let mut end = start + 1;
            while end < text.len() && text[end].is_ascii_whitespace() {
                end += 1;
            }
            Some(GSubMatch {
                start,
                end,
                captures: Vec::new(),
            })
        }
        GSubPattern::AsciiPunctuationCapture => {
            let Some(tail) = text.get(cursor..) else {
                return Ok(None);
            };
            let Some(start) = tail
                .iter()
                .position(|byte| byte.is_ascii_punctuation())
                .map(|offset| cursor + offset)
            else {
                return Ok(None);
            };
            Some(GSubMatch {
                start,
                end: start + 1,
                captures: vec![GSubCapture::Slice {
                    start,
                    end: start + 1,
                }],
            })
        }
        GSubPattern::ByteClass(byte_class) => {
            let Some(tail) = text.get(cursor..) else {
                return Ok(None);
            };
            let Some(start) = tail
                .iter()
                .position(|byte| byte_class.contains(*byte))
                .map(|offset| cursor + offset)
            else {
                return Ok(None);
            };
            Some(GSubMatch {
                start,
                end: start + 1,
                captures: Vec::new(),
            })
        }
        GSubPattern::PercentDigitCapture => {
            let Some(tail) = text.get(cursor..) else {
                return Ok(None);
            };
            let Some(start) = tail
                .windows(2)
                .position(|window| window[0] == b'%' && window[1].is_ascii_digit())
                .map(|offset| cursor + offset)
            else {
                return Ok(None);
            };
            Some(GSubMatch {
                start,
                end: start + 2,
                captures: vec![GSubCapture::Slice {
                    start: start + 1,
                    end: start + 2,
                }],
            })
        }
        GSubPattern::Balanced { open, close } => find_balanced_match(text, cursor, *open, *close)
            .map(|(start, end)| GSubMatch {
                start,
                end,
                captures: Vec::new(),
            }),
        GSubPattern::StartOptionalCaretCapture => {
            if cursor != 0 {
                return Ok(None);
            }
            let end = usize::from(text.first() == Some(&b'^'));
            Some(GSubMatch {
                start: 0,
                end,
                captures: vec![GSubCapture::Slice { start: 0, end }],
            })
        }
        GSubPattern::EndOptionalDollarCapture => {
            if cursor > text.len() {
                return Ok(None);
            }
            let starts_with_dollar = text.last() == Some(&b'$');
            let start = if starts_with_dollar {
                text.len().saturating_sub(1)
            } else {
                text.len()
            };
            if cursor > start {
                return Ok(None);
            }
            Some(GSubMatch {
                start,
                end: text.len(),
                captures: vec![GSubCapture::Slice {
                    start,
                    end: text.len(),
                }],
            })
        }
        GSubPattern::NonCapturing(pattern) => find_noncapturing_luau_pattern(text, pattern, cursor)
            .map(gsub_match_from_noncapturing_range),
        GSubPattern::Capturing(pattern) => {
            find_capturing_luau_pattern_match(text, pattern, cursor)?
                .map(gsub_match_from_capturing_match)
        }
    })
}

fn gsub_match_from_noncapturing_range((start, end): (usize, usize)) -> GSubMatch {
    let start = start.saturating_sub(1);
    let end = if end < start { start } else { end };
    GSubMatch {
        start,
        end,
        captures: Vec::new(),
    }
}

fn gsub_match_from_capturing_match(matched: StringCaptureMatch) -> GSubMatch {
    GSubMatch {
        start: matched.start,
        end: matched.end,
        captures: matched
            .captures
            .into_iter()
            .map(gsub_capture_from_string_capture)
            .collect(),
    }
}

fn gsub_capture_from_string_capture(capture: StringCapture) -> GSubCapture {
    match capture {
        StringCapture::Slice { start, end } => GSubCapture::Slice { start, end },
        StringCapture::Position(position) => GSubCapture::Position(position),
    }
}

fn find_balanced_match(text: &[u8], cursor: usize, open: u8, close: u8) -> Option<(usize, usize)> {
    let mut start = cursor;
    while start < text.len() && text[start] != open {
        start += 1;
    }
    if start >= text.len() {
        return None;
    }
    match_balanced_at(text, start, open, close).map(|end| (start, end))
}

fn match_balanced_at(text: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    if text.get(start) != Some(&open) {
        return None;
    }
    if open == close {
        let end = text
            .get(start + 1..)?
            .iter()
            .position(|byte| *byte == close)?;
        return Some(start + 1 + end + 1);
    }

    let mut depth = 1usize;
    let mut cursor = start + 1;
    while cursor < text.len() {
        if text[cursor] == open {
            depth = depth.saturating_add(1);
        } else if text[cursor] == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(cursor + 1);
            }
        }
        cursor += 1;
    }
    None
}

fn push_gsub_output(output: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let next_len = output.len().checked_add(bytes.len()).ok_or_else(|| {
        Error::new(ErrorKind::RuntimeError {
            message: "string.gsub result too large".to_owned(),
        })
    })?;
    if next_len > LUAU_MAX_STRING_BYTES {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "string.gsub result too large".to_owned(),
        }));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

fn expand_gsub_replacement(
    replacement: &[u8],
    text: &[u8],
    matched: &GSubMatch,
) -> Result<Vec<u8>> {
    let mut output = Vec::with_capacity(replacement.len());
    let whole_match = &text[matched.start..matched.end];
    let mut cursor = 0;
    while cursor < replacement.len() {
        let byte = replacement[cursor];
        if byte != b'%' {
            output.push(byte);
            cursor += 1;
            continue;
        }
        cursor += 1;
        let Some(escaped) = replacement.get(cursor).copied() else {
            return Err(Error::new(ErrorKind::RuntimeError {
                message: "invalid use of '%' in replacement string".to_owned(),
            }));
        };
        match escaped {
            b'%' => output.push(b'%'),
            b'0' => output.extend_from_slice(whole_match),
            b'1'..=b'9' => {
                let index = usize::from(escaped - b'1');
                if matched.captures.is_empty() && index == 0 {
                    output.extend_from_slice(whole_match);
                } else if let Some(capture) = matched.captures.get(index) {
                    append_gsub_capture(&mut output, text, capture)?;
                } else {
                    return Err(Error::new(ErrorKind::RuntimeError {
                        message: "invalid capture index".to_owned(),
                    }));
                }
            }
            _ => {
                return Err(Error::new(ErrorKind::RuntimeError {
                    message: "invalid use of '%' in replacement string".to_owned(),
                }));
            }
        }
        cursor += 1;
    }
    Ok(output)
}

fn append_gsub_capture(output: &mut Vec<u8>, text: &[u8], capture: &GSubCapture) -> Result<()> {
    match *capture {
        GSubCapture::Slice { start, end } => {
            if start > end || end > text.len() {
                return Err(Error::new(ErrorKind::InvalidProgram {
                    reason: "string.gsub capture lies outside the match",
                }));
            }
            output.extend_from_slice(&text[start..end]);
        }
        GSubCapture::Position(position) => {
            output.extend_from_slice(position.to_string().as_bytes());
        }
    }
    Ok(())
}

fn gsub_effective_captures(_text: &[u8], matched: &GSubMatch) -> Vec<GSubCapture> {
    if matched.captures.is_empty() {
        vec![GSubCapture::Slice {
            start: matched.start,
            end: matched.end,
        }]
    } else {
        matched.captures.clone()
    }
}

fn string_find_end(start: usize, match_len: usize) -> usize {
    if match_len == 0 {
        start - 1
    } else {
        start + match_len - 1
    }
}

fn find_start_upper_or_underscore_pattern(text: &[u8], start: usize) -> Option<usize> {
    if start != 0 {
        return None;
    }
    let first = *text.first()?;
    (first == b'_' || first.is_ascii_uppercase()).then_some(1)
}

fn find_anchored_string_chunk_prefix_pattern(
    text: &[u8],
    pattern: &[u8],
    start: usize,
) -> Option<Option<(usize, usize)>> {
    if pattern != br#"^%[string ".*"%]:"# {
        return None;
    }
    if start != 0 {
        return Some(None);
    }
    let prefix = br#"[string ""#;
    let suffix = br#""]:"#;
    if !text.starts_with(prefix) {
        return Some(None);
    }
    let Some(suffix_start) = text
        .get(prefix.len()..)?
        .windows(suffix.len())
        .rposition(|window| window == suffix)
    else {
        return Some(None);
    };
    let end = prefix.len() + suffix_start + suffix.len();
    Some(Some((1, end)))
}

fn find_anchored_utf8_charpattern(
    text: &[u8],
    pattern: &[u8],
    start: usize,
) -> Option<Option<(usize, usize)>> {
    if pattern.first() != Some(&b'^') || pattern.last() != Some(&b'$') {
        return None;
    }
    let body = &pattern[1..pattern.len().saturating_sub(1)];
    if body != LUAU_UTF8_CHARPATTERN {
        return None;
    }
    if start != 0 {
        return Some(None);
    }
    let Some(matched_end) = match_utf8_charpattern_at(text, 0) else {
        return Some(None);
    };
    Some((matched_end == text.len()).then_some((1, text.len())))
}

fn find_anchored_simple_luau_pattern(
    text: &[u8],
    pattern: &[u8],
    start: usize,
) -> Option<Option<(usize, usize)>> {
    if pattern.first() != Some(&b'^') || pattern.last() != Some(&b'$') {
        return None;
    }
    let body = &pattern[1..pattern.len().saturating_sub(1)];
    let atoms = parse_simple_luau_pattern(body)?;
    if start != 0 {
        return Some(None);
    }
    if text.len() != atoms.len() {
        return Some(None);
    }
    Some(pattern_matches_simple(text, &atoms).then_some((1, text.len())))
}

fn luau_string_match_range(
    text: &[u8],
    pattern: &[u8],
    start: i64,
) -> Result<Option<(usize, usize)>> {
    let Some(start) = normalize_string_find_start(text.len(), start)? else {
        return Ok(None);
    };
    if !has_luau_pattern_special(pattern) {
        return Ok(find_plain_bytes(text, pattern, start).map(|index| {
            let start = index - 1;
            (start, start + pattern.len())
        }));
    }
    if let Some(range) = match_anchored_dot_star_luau_pattern(text, pattern, start) {
        return Ok(range);
    }
    if let Some(pattern) = parse_noncapturing_luau_pattern(pattern)? {
        return Ok(
            find_noncapturing_luau_pattern(text, &pattern, start).map(|(start, end)| {
                let start = start - 1;
                (start, end)
            }),
        );
    }
    if let Some(pattern) = parse_simple_luau_pattern(pattern) {
        return Ok(
            find_simple_luau_pattern(text, &pattern, start).map(|index| {
                let start = index - 1;
                (start, start + pattern.len())
            }),
        );
    }
    Err(Error::new(ErrorKind::Unsupported {
        feature: "string.match pattern",
    }))
}

fn luau_string_match_captures(
    text: &[u8],
    pattern: &[u8],
    start: i64,
) -> Result<Option<Vec<StringCapture>>> {
    if pattern == b":(%d+):" {
        let Some(start) = normalize_string_find_start(text.len(), start)? else {
            return Ok(None);
        };
        return Ok(find_colon_decimal_capture(text, start).map(|capture| vec![capture]));
    }

    if let Some(pattern) = parse_capturing_luau_pattern(pattern)? {
        let Some(start) = normalize_string_find_start(text.len(), start)? else {
            return Ok(None);
        };
        return find_capturing_luau_pattern(text, &pattern, start);
    }

    let Some((start, end)) = luau_string_match_range(text, pattern, start)? else {
        return Ok(None);
    };
    Ok(Some(vec![StringCapture::Slice { start, end }]))
}

fn find_colon_decimal_capture(text: &[u8], start: usize) -> Option<StringCapture> {
    let mut cursor = start;
    while cursor < text.len() {
        if text[cursor] != b':' {
            cursor += 1;
            continue;
        }
        let digits_start = cursor + 1;
        let mut digits_end = digits_start;
        while digits_end < text.len() && text[digits_end].is_ascii_digit() {
            digits_end += 1;
        }
        if digits_end > digits_start && text.get(digits_end) == Some(&b':') {
            return Some(StringCapture::Slice {
                start: digits_start,
                end: digits_end,
            });
        }
        cursor += 1;
    }
    None
}

fn luau_gmatch_next(text: &[u8], pattern: &[u8], cursor: usize) -> Result<Option<GMatchResult>> {
    match luau_gmatch_pattern_kind(pattern)? {
        GMatchPatternKind::Plain => Ok(find_plain_bytes(text, pattern, cursor).map(|index| {
            let start = index - 1;
            let end = start + pattern.len();
            GMatchResult {
                next_cursor: end,
                captures: vec![GMatchCapture::Slice { start, end }],
            }
        })),
        GMatchPatternKind::LineExceptNewlineStar => Ok(next_non_newline_run(text, cursor)),
        GMatchPatternKind::AsciiWordPlus => Ok(next_ascii_word_run(text, cursor)),
        GMatchPatternKind::Utf8CharPattern => Ok(next_utf8_charpattern(text, cursor).map(
            |(start, end)| GMatchResult {
                next_cursor: end,
                captures: vec![GMatchCapture::Slice { start, end }],
            },
        )),
        GMatchPatternKind::Utf8PositionCapture => Ok(next_utf8_charpattern(text, cursor).map(
            |(start, end)| GMatchResult {
                next_cursor: end,
                captures: vec![
                    GMatchCapture::Position(start + 1),
                    GMatchCapture::Slice { start, end },
                ],
            },
        )),
        GMatchPatternKind::Capturing(pattern) => {
            Ok(find_capturing_luau_pattern_match(text, &pattern, cursor)?
                .map(gmatch_result_from_capturing_match))
        }
    }
}

fn luau_gmatch_pattern_kind(pattern: &[u8]) -> Result<GMatchPatternKind> {
    if pattern == b"[^\n]*" {
        return Ok(GMatchPatternKind::LineExceptNewlineStar);
    }
    if pattern == b"%w+" {
        return Ok(GMatchPatternKind::AsciiWordPlus);
    }
    if pattern == LUAU_UTF8_CHARPATTERN {
        return Ok(GMatchPatternKind::Utf8CharPattern);
    }
    if is_luau_utf8_position_capture_pattern(pattern) {
        return Ok(GMatchPatternKind::Utf8PositionCapture);
    }
    if let Some(pattern) = parse_capturing_luau_pattern(pattern)? {
        return Ok(GMatchPatternKind::Capturing(pattern));
    }
    if !pattern.is_empty() && !has_luau_pattern_special(pattern) {
        return Ok(GMatchPatternKind::Plain);
    }
    Err(Error::new(ErrorKind::Unsupported {
        feature: "string.gmatch pattern",
    }))
}

fn is_luau_utf8_position_capture_pattern(pattern: &[u8]) -> bool {
    pattern.starts_with(b"()(")
        && pattern.ends_with(b")")
        && pattern.len() == LUAU_UTF8_CHARPATTERN.len() + 4
        && &pattern[3..pattern.len() - 1] == LUAU_UTF8_CHARPATTERN
}

fn next_non_newline_run(text: &[u8], cursor: usize) -> Option<GMatchResult> {
    if cursor > text.len() {
        return None;
    }
    let start = cursor;
    let end = text[start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(text.len(), |offset| start + offset);
    let next_cursor = if end < text.len() {
        end + 1
    } else {
        text.len() + 1
    };
    Some(GMatchResult {
        next_cursor,
        captures: vec![GMatchCapture::Slice { start, end }],
    })
}

fn next_ascii_word_run(text: &[u8], cursor: usize) -> Option<GMatchResult> {
    let mut start = cursor;
    while start < text.len() && !text[start].is_ascii_alphanumeric() {
        start += 1;
    }
    if start >= text.len() {
        return None;
    }
    let mut end = start + 1;
    while end < text.len() && text[end].is_ascii_alphanumeric() {
        end += 1;
    }
    Some(GMatchResult {
        next_cursor: end,
        captures: vec![GMatchCapture::Slice { start, end }],
    })
}

fn next_utf8_charpattern(text: &[u8], cursor: usize) -> Option<(usize, usize)> {
    let mut start = cursor;
    while start < text.len() {
        if let Some(end) = match_utf8_charpattern_at(text, start) {
            return Some((start, end));
        }
        start += 1;
    }
    None
}

fn match_utf8_charpattern_at(text: &[u8], start: usize) -> Option<usize> {
    let first = *text.get(start)?;
    if first <= 0x7f {
        return Some(start + 1);
    }
    if (0xc2..=0xf4).contains(&first) {
        let mut end = start + 1;
        while end < text.len() && (0x80..=0xbf).contains(&text[end]) {
            end += 1;
        }
        return Some(end);
    }
    None
}

fn gmatch_result_from_capturing_match(matched: StringCaptureMatch) -> GMatchResult {
    let next_cursor = if matched.start == matched.end {
        matched.end.saturating_add(1)
    } else {
        matched.end
    };
    GMatchResult {
        next_cursor,
        captures: matched
            .captures
            .into_iter()
            .map(gmatch_capture_from_string_capture)
            .collect(),
    }
}

fn gmatch_capture_from_string_capture(capture: StringCapture) -> GMatchCapture {
    match capture {
        StringCapture::Slice { start, end } => GMatchCapture::Slice { start, end },
        StringCapture::Position(position) => GMatchCapture::Position(position),
    }
}

#[derive(Clone, Copy)]
enum StringCapture {
    Slice { start: usize, end: usize },
    Position(usize),
}

struct StringCaptureMatch {
    start: usize,
    end: usize,
    captures: Vec<StringCapture>,
}

struct GMatchResult {
    next_cursor: usize,
    captures: Vec<GMatchCapture>,
}

enum GMatchCapture {
    Slice { start: usize, end: usize },
    Position(usize),
}

enum GMatchPatternKind {
    Plain,
    LineExceptNewlineStar,
    AsciiWordPlus,
    Utf8CharPattern,
    Utf8PositionCapture,
    Capturing(CapturingPattern),
}

enum GSubReplacement {
    String(Vec<u8>),
    Function(Value),
    Table(Value),
}

enum GSubPattern {
    Empty,
    Plain(Vec<u8>),
    AsciiWhitespacePlus,
    AsciiPunctuationCapture,
    ByteClass(LuauByteClass),
    Balanced { open: u8, close: u8 },
    PercentDigitCapture,
    StartOptionalCaretCapture,
    EndOptionalDollarCapture,
    NonCapturing(NonCapturingPattern),
    Capturing(CapturingPattern),
}

struct GSubMatch {
    start: usize,
    end: usize,
    captures: Vec<GSubCapture>,
}

#[derive(Clone, Copy)]
enum GSubCapture {
    Slice { start: usize, end: usize },
    Position(usize),
}

struct CapturingPattern {
    anchored_start: bool,
    steps: Vec<CaptureStep>,
    capture_count: usize,
}

struct ParsedCaptureSteps {
    steps: Vec<CaptureStep>,
    has_capture_feature: bool,
}

#[derive(Clone)]
struct CaptureMatchState {
    cursor: usize,
    captures: Vec<Option<StringCapture>>,
}

enum CaptureStep {
    StartAnchor,
    EndAnchor,
    Frontier(LuauByteClass),
    Item {
        item: PatternItem,
        repetition: PatternRepetition,
    },
    Capture {
        capture_id: usize,
        steps: Vec<CaptureStep>,
    },
    PositionCapture(usize),
    BackReference(usize),
}

fn normalize_string_find_start(len: usize, index: i64) -> Result<Option<usize>> {
    let len = i64::try_from(len).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string length exceeds i64",
        })
    })?;
    let one_based = if index < 0 {
        len.checked_add(index)
            .and_then(|value| value.checked_add(1))
            .unwrap_or(i64::MIN)
    } else {
        index
    }
    .max(1);
    if one_based > len + 1 {
        return Ok(None);
    }
    Ok(Some(usize::try_from(one_based - 1).map_err(|_| {
        Error::new(ErrorKind::InvalidProgram {
            reason: "string.find start index underflow",
        })
    })?))
}

fn find_plain_bytes(text: &[u8], pattern: &[u8], start: usize) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start + 1);
    }
    text.get(start..)
        .and_then(|tail| {
            tail.windows(pattern.len())
                .position(|window| window == pattern)
        })
        .map(|offset| start + offset + 1)
}

fn find_simple_luau_pattern(
    text: &[u8],
    pattern: &[SimplePatternAtom],
    start: usize,
) -> Option<usize> {
    if pattern.is_empty() {
        return Some(start + 1);
    }
    text.get(start..)
        .and_then(|tail| {
            tail.windows(pattern.len())
                .position(|window| pattern_matches_simple(window, pattern))
        })
        .map(|offset| start + offset + 1)
}

fn match_anchored_dot_star_luau_pattern(
    text: &[u8],
    pattern: &[u8],
    start: usize,
) -> Option<Option<(usize, usize)>> {
    if pattern.first() != Some(&b'^') {
        return None;
    }
    let body = &pattern[1..];
    let dot_star = body.windows(2).position(|window| window == b".*")?;
    let prefix = &body[..dot_star];
    let suffix = &body[dot_star + 2..];
    if has_luau_pattern_special(prefix) || has_luau_pattern_special(suffix) {
        return None;
    }
    if start != 0 {
        return Some(None);
    }
    if !text.starts_with(prefix) {
        return Some(None);
    }
    if suffix.is_empty() {
        return Some(Some((0, text.len())));
    }
    let search_start = prefix.len();
    let suffix_start = text
        .get(search_start..)?
        .windows(suffix.len())
        .rposition(|window| window == suffix);
    Some(suffix_start.map(|suffix_start| {
        let suffix_start = suffix_start + search_start;
        (0, suffix_start + suffix.len())
    }))
}

fn find_noncapturing_luau_pattern(
    text: &[u8],
    pattern: &NonCapturingPattern,
    start: usize,
) -> Option<(usize, usize)> {
    if pattern.anchored_start {
        if start != 0 {
            return None;
        }
        return match_noncapturing_luau_pattern_at(text, &pattern.steps, 0)
            .map(|end| (1, string_find_end(1, end)));
    }

    for index in start..=text.len() {
        if let Some(end) = match_noncapturing_luau_pattern_at(text, &pattern.steps, index) {
            return Some((index + 1, string_find_end(index + 1, end - index)));
        }
    }
    None
}

fn match_noncapturing_luau_pattern_at(
    text: &[u8],
    steps: &[PatternStep],
    start: usize,
) -> Option<usize> {
    match_noncapturing_luau_pattern_from(text, steps, 0, start)
}

fn match_noncapturing_luau_pattern_from(
    text: &[u8],
    steps: &[PatternStep],
    step_index: usize,
    cursor: usize,
) -> Option<usize> {
    let Some(step) = steps.get(step_index) else {
        return Some(cursor);
    };
    match step {
        PatternStep::Frontier(class) => frontier_matches(text, cursor, class)
            .then(|| match_noncapturing_luau_pattern_from(text, steps, step_index + 1, cursor))?,
        PatternStep::EndAnchor => (cursor == text.len())
            .then(|| match_noncapturing_luau_pattern_from(text, steps, step_index + 1, cursor))?,
        PatternStep::Item { item, repetition } => match repetition {
            PatternRepetition::Once => {
                let next = match_pattern_item(text, cursor, item)?;
                match_noncapturing_luau_pattern_from(text, steps, step_index + 1, next)
            }
            PatternRepetition::ZeroOrOne => {
                if let Some(next) = match_pattern_item(text, cursor, item) {
                    if let Some(end) =
                        match_noncapturing_luau_pattern_from(text, steps, step_index + 1, next)
                    {
                        return Some(end);
                    }
                }
                match_noncapturing_luau_pattern_from(text, steps, step_index + 1, cursor)
            }
            PatternRepetition::ZeroOrMoreGreedy => {
                let max = greedy_pattern_item_end(text, cursor, item);
                for next in (cursor..=max).rev() {
                    if let Some(end) =
                        match_noncapturing_luau_pattern_from(text, steps, step_index + 1, next)
                    {
                        return Some(end);
                    }
                }
                None
            }
            PatternRepetition::OneOrMoreGreedy => {
                let first = match_pattern_item(text, cursor, item)?;
                let max = greedy_pattern_item_end(text, first, item);
                for next in (first..=max).rev() {
                    if let Some(end) =
                        match_noncapturing_luau_pattern_from(text, steps, step_index + 1, next)
                    {
                        return Some(end);
                    }
                }
                None
            }
            PatternRepetition::ZeroOrMoreMinimal => {
                let mut next = cursor;
                loop {
                    if let Some(end) =
                        match_noncapturing_luau_pattern_from(text, steps, step_index + 1, next)
                    {
                        return Some(end);
                    }
                    let after_item = match_pattern_item(text, next, item)?;
                    next = after_item;
                }
            }
        },
    }
}

fn greedy_pattern_item_end(text: &[u8], cursor: usize, item: &PatternItem) -> usize {
    let mut next = cursor;
    while let Some(after_item) = match_pattern_item(text, next, item) {
        next = after_item;
    }
    next
}

fn match_pattern_item(text: &[u8], cursor: usize, item: &PatternItem) -> Option<usize> {
    match item {
        PatternItem::Balanced { open, close } => match_balanced_at(text, cursor, *open, *close),
        PatternItem::Any => text.get(cursor).map(|_| cursor + 1),
        PatternItem::Byte(expected) => text
            .get(cursor)
            .is_some_and(|byte| byte == expected)
            .then_some(cursor + 1),
        PatternItem::ByteClass(class) => text
            .get(cursor)
            .is_some_and(|byte| class.contains(*byte))
            .then_some(cursor + 1),
    }
}

fn frontier_matches(text: &[u8], cursor: usize, class: &LuauByteClass) -> bool {
    let previous = if cursor == 0 {
        0
    } else {
        text.get(cursor - 1).copied().unwrap_or(0)
    };
    let current = text.get(cursor).copied().unwrap_or(0);
    !class.contains(previous) && class.contains(current)
}

const CAPTURING_PATTERN_MATCH_BUDGET: usize = 1_000_000;

fn find_capturing_luau_pattern(
    text: &[u8],
    pattern: &CapturingPattern,
    start: usize,
) -> Result<Option<Vec<StringCapture>>> {
    Ok(find_capturing_luau_pattern_match(text, pattern, start)?.map(|matched| matched.captures))
}

fn find_capturing_luau_pattern_match(
    text: &[u8],
    pattern: &CapturingPattern,
    start: usize,
) -> Result<Option<StringCaptureMatch>> {
    let candidates: Box<dyn Iterator<Item = usize>> = if pattern.anchored_start {
        if start != 0 {
            return Ok(None);
        }
        Box::new(std::iter::once(0))
    } else {
        Box::new(start..=text.len())
    };

    let mut budget = CAPTURING_PATTERN_MATCH_BUDGET;
    for cursor in candidates {
        let state = CaptureMatchState {
            cursor,
            captures: vec![None; pattern.capture_count],
        };
        let mut states = match_capturing_steps(text, &pattern.steps, 0, state, &mut budget)?;
        if states.is_empty() {
            continue;
        }
        let state = states.remove(0);
        {
            if pattern.capture_count == 0 {
                return Ok(Some(StringCaptureMatch {
                    start: cursor,
                    end: state.cursor,
                    captures: vec![StringCapture::Slice {
                        start: cursor,
                        end: state.cursor,
                    }],
                }));
            }
            let captures = state.captures.into_iter().flatten().collect();
            return Ok(Some(StringCaptureMatch {
                start: cursor,
                end: state.cursor,
                captures,
            }));
        }
    }
    Ok(None)
}

fn match_capturing_steps(
    text: &[u8],
    steps: &[CaptureStep],
    step_index: usize,
    state: CaptureMatchState,
    budget: &mut usize,
) -> Result<Vec<CaptureMatchState>> {
    spend_capturing_pattern_budget(budget)?;
    let Some(step) = steps.get(step_index) else {
        return Ok(vec![state]);
    };

    match step {
        CaptureStep::StartAnchor => {
            if state.cursor == 0 {
                match_capturing_steps(text, steps, step_index + 1, state, budget)
            } else {
                Ok(Vec::new())
            }
        }
        CaptureStep::EndAnchor => {
            if state.cursor == text.len() {
                match_capturing_steps(text, steps, step_index + 1, state, budget)
            } else {
                Ok(Vec::new())
            }
        }
        CaptureStep::Frontier(class) => {
            if frontier_matches(text, state.cursor, class) {
                match_capturing_steps(text, steps, step_index + 1, state, budget)
            } else {
                Ok(Vec::new())
            }
        }
        CaptureStep::Item { item, repetition } => {
            let cursors = repetition_cursor_candidates(text, state.cursor, item, *repetition);
            let mut matches = Vec::new();
            for cursor in cursors {
                let mut next_state = state.clone();
                next_state.cursor = cursor;
                matches.extend(match_capturing_steps(
                    text,
                    steps,
                    step_index + 1,
                    next_state,
                    budget,
                )?);
            }
            Ok(matches)
        }
        CaptureStep::BackReference(capture_id) => {
            let Some(Some(StringCapture::Slice { start, end })) =
                state.captures.get(*capture_id).copied()
            else {
                return Ok(Vec::new());
            };
            let capture = &text[start..end];
            if text
                .get(state.cursor..)
                .is_some_and(|tail| tail.starts_with(capture))
            {
                let mut next_state = state;
                next_state.cursor += capture.len();
                match_capturing_steps(text, steps, step_index + 1, next_state, budget)
            } else {
                Ok(Vec::new())
            }
        }
        CaptureStep::PositionCapture(capture_id) => {
            let mut next_state = state;
            if let Some(capture) = next_state.captures.get_mut(*capture_id) {
                *capture = Some(StringCapture::Position(next_state.cursor + 1));
            }
            match_capturing_steps(text, steps, step_index + 1, next_state, budget)
        }
        CaptureStep::Capture {
            capture_id,
            steps: inner,
        } => {
            let start = state.cursor;
            let inner_states = match_capturing_steps(text, inner, 0, state, budget)?;
            let mut matches = Vec::new();
            for mut inner_state in inner_states {
                if let Some(capture) = inner_state.captures.get_mut(*capture_id) {
                    *capture = Some(StringCapture::Slice {
                        start,
                        end: inner_state.cursor,
                    });
                }
                matches.extend(match_capturing_steps(
                    text,
                    steps,
                    step_index + 1,
                    inner_state,
                    budget,
                )?);
            }
            Ok(matches)
        }
    }
}

fn repetition_cursor_candidates(
    text: &[u8],
    cursor: usize,
    item: &PatternItem,
    repetition: PatternRepetition,
) -> Vec<usize> {
    match repetition {
        PatternRepetition::Once => match_pattern_item(text, cursor, item).into_iter().collect(),
        PatternRepetition::ZeroOrOne => match_pattern_item(text, cursor, item)
            .into_iter()
            .chain(std::iter::once(cursor))
            .collect(),
        PatternRepetition::ZeroOrMoreGreedy => {
            let max = greedy_pattern_item_end(text, cursor, item);
            (cursor..=max).rev().collect()
        }
        PatternRepetition::OneOrMoreGreedy => {
            let Some(first) = match_pattern_item(text, cursor, item) else {
                return Vec::new();
            };
            let max = greedy_pattern_item_end(text, first, item);
            (first..=max).rev().collect()
        }
        PatternRepetition::ZeroOrMoreMinimal => {
            let max = greedy_pattern_item_end(text, cursor, item);
            (cursor..=max).collect()
        }
    }
}

fn spend_capturing_pattern_budget(budget: &mut usize) -> Result<()> {
    let Some(next) = budget.checked_sub(1) else {
        return Err(Error::new(ErrorKind::RuntimeError {
            message: "pattern match too complex".to_owned(),
        }));
    };
    *budget = next;
    Ok(())
}

fn parse_capturing_luau_pattern(pattern: &[u8]) -> Result<Option<CapturingPattern>> {
    let mut index = 0;
    let mut next_capture = 0;
    let mut open_captures = Vec::new();
    let anchored_start = pattern.first() == Some(&b'^');
    let mut steps = Vec::new();
    if anchored_start {
        steps.push(CaptureStep::StartAnchor);
        index = 1;
    }
    let parsed = parse_capture_steps_until(
        pattern,
        &mut index,
        None,
        &mut next_capture,
        &mut open_captures,
    )?;
    steps.extend(parsed.steps);
    if !parsed.has_capture_feature {
        return Ok(None);
    }
    if index != pattern.len() {
        return Ok(None);
    }
    Ok(Some(CapturingPattern {
        anchored_start,
        steps,
        capture_count: next_capture,
    }))
}

fn parse_capture_steps_until(
    pattern: &[u8],
    index: &mut usize,
    terminator: Option<u8>,
    next_capture: &mut usize,
    open_captures: &mut Vec<usize>,
) -> Result<ParsedCaptureSteps> {
    let mut steps = Vec::new();
    let mut has_capture_feature = false;
    while *index < pattern.len() {
        if Some(pattern[*index]) == terminator {
            *index += 1;
            return Ok(ParsedCaptureSteps {
                steps,
                has_capture_feature,
            });
        }
        if terminator.is_none() && pattern[*index] == b'$' && *index + 1 == pattern.len() {
            steps.push(CaptureStep::EndAnchor);
            *index += 1;
            continue;
        }
        match pattern[*index] {
            b'(' => {
                has_capture_feature = true;
                let capture_id = *next_capture;
                *next_capture += 1;
                *index += 1;
                if pattern.get(*index) == Some(&b')') {
                    *index += 1;
                    steps.push(CaptureStep::PositionCapture(capture_id));
                    continue;
                }
                open_captures.push(capture_id);
                let parsed = parse_capture_steps_until(
                    pattern,
                    index,
                    Some(b')'),
                    next_capture,
                    open_captures,
                )?;
                open_captures.pop();
                steps.push(CaptureStep::Capture {
                    capture_id,
                    steps: parsed.steps,
                });
            }
            b')' => {
                return if terminator.is_some() {
                    unreachable!("terminator branch handles closing captures")
                } else {
                    Err(malformed_pattern("invalid pattern capture"))
                };
            }
            b'%' if pattern.get(*index + 1).is_some_and(u8::is_ascii_digit) => {
                has_capture_feature = true;
                let capture_number = usize::from(pattern[*index + 1] - b'0');
                if capture_number == 0 {
                    return Err(malformed_pattern("invalid pattern capture"));
                }
                let capture_id = capture_number - 1;
                if capture_id >= *next_capture || open_captures.contains(&capture_id) {
                    return Err(malformed_pattern("invalid pattern capture"));
                }
                steps.push(CaptureStep::BackReference(capture_id));
                *index += 2;
            }
            b'%' if pattern.get(*index + 1) == Some(&b'f') => {
                let Some((class, next_index)) = parse_luau_byte_class_at(pattern, *index + 2)?
                else {
                    return Err(malformed_pattern("missing '[' after '%f'"));
                };
                steps.push(CaptureStep::Frontier(class));
                *index = next_index;
            }
            _ => {
                let Some((item, next_index)) = parse_luau_pattern_item(pattern, *index)? else {
                    return Ok(ParsedCaptureSteps {
                        steps,
                        has_capture_feature,
                    });
                };
                *index = next_index;
                let repetition = match pattern.get(*index).copied() {
                    Some(b'*') => {
                        *index += 1;
                        PatternRepetition::ZeroOrMoreGreedy
                    }
                    Some(b'+') => {
                        *index += 1;
                        PatternRepetition::OneOrMoreGreedy
                    }
                    Some(b'-') => {
                        *index += 1;
                        PatternRepetition::ZeroOrMoreMinimal
                    }
                    Some(b'?') => {
                        *index += 1;
                        PatternRepetition::ZeroOrOne
                    }
                    _ => PatternRepetition::Once,
                };
                steps.push(CaptureStep::Item { item, repetition });
            }
        }
    }

    if terminator.is_some() {
        return Err(malformed_pattern("unfinished capture"));
    }
    Ok(ParsedCaptureSteps {
        steps,
        has_capture_feature,
    })
}

fn parse_noncapturing_luau_pattern(pattern: &[u8]) -> Result<Option<NonCapturingPattern>> {
    let mut index = 0;
    let anchored_start = pattern.first() == Some(&b'^');
    if anchored_start {
        index = 1;
    }

    let mut steps = Vec::new();
    while index < pattern.len() {
        if pattern[index] == b'$' && index + 1 == pattern.len() {
            steps.push(PatternStep::EndAnchor);
            index += 1;
            continue;
        }
        if pattern.get(index) == Some(&b'%') && pattern.get(index + 1) == Some(&b'f') {
            let Some((class, next_index)) = parse_luau_byte_class_at(pattern, index + 2)? else {
                return Err(malformed_pattern("missing '[' after '%f'"));
            };
            steps.push(PatternStep::Frontier(class));
            index = next_index;
            continue;
        }

        let Some((item, next_index)) = parse_luau_pattern_item(pattern, index)? else {
            return Ok(None);
        };
        index = next_index;

        let repetition = match pattern.get(index).copied() {
            Some(b'*') => {
                index += 1;
                PatternRepetition::ZeroOrMoreGreedy
            }
            Some(b'+') => {
                index += 1;
                PatternRepetition::OneOrMoreGreedy
            }
            Some(b'-') => {
                index += 1;
                PatternRepetition::ZeroOrMoreMinimal
            }
            Some(b'?') => {
                index += 1;
                PatternRepetition::ZeroOrOne
            }
            _ => PatternRepetition::Once,
        };
        steps.push(PatternStep::Item { item, repetition });
    }

    Ok(Some(NonCapturingPattern {
        anchored_start,
        steps,
    }))
}

fn parse_luau_pattern_item(pattern: &[u8], index: usize) -> Result<Option<(PatternItem, usize)>> {
    let byte = pattern[index];
    match byte {
        b'.' => Ok(Some((PatternItem::Any, index + 1))),
        b'%' => {
            let Some(escaped) = pattern.get(index + 1).copied() else {
                return Err(malformed_pattern("malformed pattern"));
            };
            if escaped == b'b' {
                let Some(open) = pattern.get(index + 2).copied() else {
                    return Err(malformed_pattern("malformed pattern"));
                };
                let Some(close) = pattern.get(index + 3).copied() else {
                    return Err(malformed_pattern("malformed pattern"));
                };
                Ok(Some((PatternItem::Balanced { open, close }, index + 4)))
            } else if escaped == b'f' {
                Err(malformed_pattern("malformed pattern"))
            } else if let Some(class) = luau_percent_class(escaped) {
                Ok(Some((PatternItem::ByteClass(class), index + 2)))
            } else {
                Ok(Some((PatternItem::Byte(escaped), index + 2)))
            }
        }
        b'[' => {
            let Some((class, next_index)) = parse_luau_byte_class_at(pattern, index)? else {
                return Err(malformed_pattern("malformed pattern"));
            };
            Ok(Some((PatternItem::ByteClass(class), next_index)))
        }
        b'(' | b')' => Ok(None),
        b'*' | b'+' | b'-' | b'?' => Err(malformed_pattern("malformed pattern")),
        b'^' | b'$' => Ok(Some((PatternItem::Byte(byte), index + 1))),
        byte => Ok(Some((PatternItem::Byte(byte), index + 1))),
    }
}

fn parse_luau_byte_class_at(
    pattern: &[u8],
    start: usize,
) -> Result<Option<(LuauByteClass, usize)>> {
    if pattern.get(start) != Some(&b'[') {
        return Ok(None);
    }
    let mut index = start + 1;
    if index >= pattern.len() {
        return Ok(None);
    }
    let negated = if pattern[index] == b'^' {
        index += 1;
        true
    } else {
        false
    };
    if index >= pattern.len() {
        return Ok(None);
    }

    let mut matches = LuauByteClass::empty();
    let mut has_element = false;
    while index < pattern.len() {
        if pattern[index] == b']' && has_element {
            if negated {
                matches.invert();
            }
            return Ok(Some((matches, index + 1)));
        }

        let (element, next_index) = parse_luau_byte_class_element(pattern, index)?;
        has_element = true;
        if pattern.get(next_index) == Some(&b'-') && pattern.get(next_index + 1) != Some(&b']') {
            let (end_element, after_end) = parse_luau_byte_class_element(pattern, next_index + 1)?;
            let (ByteClassElement::Single(start), ByteClassElement::Single(end)) =
                (element, end_element)
            else {
                return Ok(None);
            };
            if start > end {
                return Ok(None);
            }
            for byte in start..=end {
                matches.insert(byte);
            }
            index = after_end;
        } else {
            add_byte_class_element(&mut matches, element);
            index = next_index;
        }
    }
    Ok(None)
}

fn parse_luau_byte_class_element(
    pattern: &[u8],
    index: usize,
) -> Result<(ByteClassElement, usize)> {
    let Some(byte) = pattern.get(index).copied() else {
        return Err(malformed_pattern("malformed pattern"));
    };
    if byte == b'%' {
        let Some(escaped) = pattern.get(index + 1).copied() else {
            return Err(malformed_pattern("malformed pattern"));
        };
        if let Some(class) = luau_percent_class(escaped) {
            Ok((ByteClassElement::Class(class), index + 2))
        } else {
            Ok((ByteClassElement::Single(escaped), index + 2))
        }
    } else {
        Ok((ByteClassElement::Single(byte), index + 1))
    }
}

fn add_byte_class_element(matches: &mut LuauByteClass, element: ByteClassElement) {
    match element {
        ByteClassElement::Single(byte) => matches.insert(byte),
        ByteClassElement::Class(class) => matches.union(class),
    }
}

fn malformed_pattern(message: &str) -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: message.to_owned(),
    })
}

fn pattern_matches_simple(text: &[u8], pattern: &[SimplePatternAtom]) -> bool {
    text.iter()
        .zip(pattern)
        .all(|(text, pattern)| match pattern {
            SimplePatternAtom::Any => true,
            SimplePatternAtom::Byte(pattern) => text == pattern,
            SimplePatternAtom::Control => text.is_ascii_control(),
        })
}

fn parse_simple_luau_pattern(pattern: &[u8]) -> Option<Vec<SimplePatternAtom>> {
    let mut atoms = Vec::with_capacity(pattern.len());
    let mut index = 0;
    while index < pattern.len() {
        match pattern[index] {
            b'.' => atoms.push(SimplePatternAtom::Any),
            b'%' => {
                let escaped = *pattern.get(index + 1)?;
                if escaped == b'c' {
                    atoms.push(SimplePatternAtom::Control);
                } else {
                    if escaped.is_ascii_alphanumeric() {
                        return None;
                    }
                    atoms.push(SimplePatternAtom::Byte(escaped));
                }
                index += 1;
            }
            byte if is_luau_pattern_special(byte) => return None,
            byte => atoms.push(SimplePatternAtom::Byte(byte)),
        }
        index += 1;
    }
    Some(atoms)
}

fn parse_luau_byte_class_pattern(pattern: &[u8]) -> Option<LuauByteClass> {
    match parse_luau_byte_class_at(pattern, 0) {
        Ok(Some((byte_class, end))) if end == pattern.len() => Some(byte_class),
        _ => None,
    }
}

fn has_luau_pattern_special(pattern: &[u8]) -> bool {
    pattern.iter().copied().any(is_luau_pattern_special)
}

fn is_luau_pattern_special(byte: u8) -> bool {
    matches!(
        byte,
        b'^' | b'$' | b'(' | b')' | b'%' | b'.' | b'[' | b']' | b'*' | b'+' | b'-' | b'?'
    )
}

struct NonCapturingPattern {
    anchored_start: bool,
    steps: Vec<PatternStep>,
}

enum PatternStep {
    Frontier(LuauByteClass),
    Item {
        item: PatternItem,
        repetition: PatternRepetition,
    },
    EndAnchor,
}

enum PatternItem {
    Any,
    Byte(u8),
    ByteClass(LuauByteClass),
    Balanced { open: u8, close: u8 },
}

#[derive(Clone, Copy)]
enum PatternRepetition {
    Once,
    ZeroOrOne,
    ZeroOrMoreGreedy,
    OneOrMoreGreedy,
    ZeroOrMoreMinimal,
}

#[derive(Clone, Copy)]
enum ByteClassElement {
    Single(u8),
    Class(LuauByteClass),
}

#[derive(Clone, Copy)]
struct LuauByteClass {
    bits: [u64; 4],
}

impl LuauByteClass {
    const fn empty() -> Self {
        Self { bits: [0; 4] }
    }

    const fn any() -> Self {
        Self {
            bits: [u64::MAX; 4],
        }
    }

    fn insert(&mut self, byte: u8) {
        let index = usize::from(byte / 64);
        let bit = u64::from(byte % 64);
        self.bits[index] |= 1_u64 << bit;
    }

    fn contains(&self, byte: u8) -> bool {
        let index = usize::from(byte / 64);
        let bit = u64::from(byte % 64);
        self.bits[index] & (1_u64 << bit) != 0
    }

    fn invert(&mut self) {
        for bits in &mut self.bits {
            *bits = !*bits;
        }
    }

    fn union(&mut self, other: Self) {
        for (lhs, rhs) in self.bits.iter_mut().zip(other.bits) {
            *lhs |= rhs;
        }
    }
}

fn luau_percent_class(byte: u8) -> Option<LuauByteClass> {
    let mut matches = LuauByteClass::empty();
    let lower = byte.to_ascii_lowercase();
    match lower {
        b'a' => {
            for byte in b'A'..=b'Z' {
                matches.insert(byte);
            }
            for byte in b'a'..=b'z' {
                matches.insert(byte);
            }
        }
        b'c' => {
            for byte in u8::MIN..=u8::MAX {
                if byte.is_ascii_control() {
                    matches.insert(byte);
                }
            }
        }
        b'd' => {
            for byte in b'0'..=b'9' {
                matches.insert(byte);
            }
        }
        b'g' => {
            for byte in u8::MIN..=u8::MAX {
                if byte.is_ascii_graphic() {
                    matches.insert(byte);
                }
            }
        }
        b'l' => {
            for byte in b'a'..=b'z' {
                matches.insert(byte);
            }
        }
        b'p' => {
            for byte in u8::MIN..=u8::MAX {
                if byte.is_ascii_punctuation() {
                    matches.insert(byte);
                }
            }
        }
        b's' => {
            for byte in [b' ', b'\x0c', b'\n', b'\r', b'\t', 0x0b] {
                matches.insert(byte);
            }
        }
        b'u' => {
            for byte in b'A'..=b'Z' {
                matches.insert(byte);
            }
        }
        b'w' => {
            for byte in b'0'..=b'9' {
                matches.insert(byte);
            }
            for byte in b'A'..=b'Z' {
                matches.insert(byte);
            }
            for byte in b'a'..=b'z' {
                matches.insert(byte);
            }
        }
        b'x' => {
            for byte in b'0'..=b'9' {
                matches.insert(byte);
            }
            for byte in b'A'..=b'F' {
                matches.insert(byte);
            }
            for byte in b'a'..=b'f' {
                matches.insert(byte);
            }
        }
        b'z' => matches.insert(0),
        _ => return None,
    }

    if byte.is_ascii_uppercase() {
        matches.invert();
    }
    Some(matches)
}

fn luau_any_byte_class() -> LuauByteClass {
    LuauByteClass::any()
}

#[derive(Clone, Copy)]
enum SimplePatternAtom {
    Byte(u8),
    Any,
    Control,
}

fn split_luau_string(text: &[u8], separator: &[u8]) -> Result<Vec<Vec<u8>>> {
    if separator.is_empty() {
        return Ok(text.iter().map(|byte| vec![*byte]).collect());
    }

    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(offset) = text[start..]
        .windows(separator.len())
        .position(|window| window == separator)
    {
        let end = start + offset;
        parts.push(text[start..end].to_vec());
        start = end + separator.len();
    }
    parts.push(text[start..].to_vec());
    Ok(parts)
}

fn get_table(registers: &[Value], register: u8) -> Result<u32> {
    get_register(registers, register)?
        .as_table()
        .ok_or_else(|| Error::new(ErrorKind::TypeError { expected: "table" }))
}

fn validate_table_key(key: Value) -> Result<()> {
    if key.kind() == ValueKind::Nil {
        Err(Error::new(ErrorKind::TableIndexNil))
    } else if key.as_number().is_some_and(f64::is_nan) {
        Err(Error::new(ErrorKind::TableIndexNaN))
    } else {
        Ok(())
    }
}

fn readonly_table_error() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: READONLY_TABLE_ERROR.to_owned(),
    })
}

fn table_expected_error() -> Error {
    Error::new(ErrorKind::RuntimeError {
        message: "table expected".to_owned(),
    })
}
