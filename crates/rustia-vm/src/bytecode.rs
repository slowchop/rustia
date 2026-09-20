use std::mem::size_of;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Program {
    pub main: Function,
    /// Immutable compiled constants shared by every clone of this program.
    ///
    /// VMs retain a program clone for closures and coroutines. Keeping the pool behind an
    /// `Arc` makes that retention cheap and, in particular, stores compiled string bytes once
    /// even when the same program is loaded into multiple VMs.
    constants: Arc<ConstantStorage>,
    pub functions: Vec<Function>,
    pub source_name: Option<Arc<str>>,
    /// Host diagnostic source, accounted separately from guest heap quotas.
    pub diagnostic_source: Option<Arc<crate::diagnostic::SourceSnapshot>>,
    pub diagnostic_prefix_bytes: usize,
    pub diagnostic_mapped_length: Option<usize>,
    /// Optional per-instruction logical-unit schedule produced by an
    /// optimizing lowering (`None` for the identity schedule). See
    /// [`ProgramLogicalUnits`].
    logical_units: Option<Arc<ProgramLogicalUnits>>,
}

/// Per-emitted-instruction logical units for a whole program, main first.
///
/// `units[pc]` is how many logical source instructions the emitted
/// instruction at `pc` represents; an optimizing lowering folds every deleted
/// instruction's unit into a surviving neighbor of the same basic block. The
/// VM charges fuel, `executed_instructions`, and deterministic-collection
/// scheduling by these units, which restores the unoptimized logical schedule
/// over optimized code (the C2.3 receipt contract in `design/58-vm-core2.md`).
/// Every folded instruction has fuel cost one, so an instruction's fuel is its
/// own cost plus `unit - 1`. Charging a unit greater than one advances the
/// instruction clock by several steps at once and may cross several
/// deterministic-collection boundaries; the VM fires one collection per
/// crossed boundary at that instruction's edge, so the collection lattice is
/// the unoptimized program's.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramLogicalUnits {
    main: Box<[u32]>,
    functions: Vec<Box<[u32]>>,
}

impl ProgramLogicalUnits {
    /// Validates that the schedule covers every function body exactly and
    /// that every unit is at least one.
    pub fn new(
        main: Vec<u32>,
        functions: Vec<Vec<u32>>,
    ) -> std::result::Result<Self, &'static str> {
        if main.contains(&0) || functions.iter().any(|units| units.contains(&0)) {
            return Err("logical units must be at least one per instruction");
        }
        Ok(Self {
            main: main.into_boxed_slice(),
            functions: functions.into_iter().map(Vec::into_boxed_slice).collect(),
        })
    }

    /// True when every unit is one: the identity schedule carries no
    /// information and should be represented as `None` on the program.
    pub fn is_identity(&self) -> bool {
        self.function_is_identity(None)
            && (0..self.functions.len()).all(|index| self.function_is_identity(Some(index)))
    }

    /// True when one function's slice of the schedule is all ones (`None`
    /// prototype = main). The VM gates its fixed-charge executors (HotIR and
    /// the leaf-call plans, which assume one unit per instruction) on this
    /// per prototype, so a rewrite in one function never deselects them in
    /// another. A prototype the schedule does not cover has no units and is
    /// identity, matching the VM's treatment of a missing unit as one.
    pub fn function_is_identity(&self, prototype: Option<usize>) -> bool {
        self.function_units(prototype).iter().all(|unit| *unit == 1)
    }

    fn function_units(&self, prototype: Option<usize>) -> &[u32] {
        match prototype {
            None => &self.main,
            Some(index) => self
                .functions
                .get(index)
                .map(|units| &units[..])
                .unwrap_or(&[]),
        }
    }

    fn memory_bytes(&self) -> usize {
        let mut bytes = size_of::<Self>()
            .saturating_add(self.main.len().saturating_mul(size_of::<u32>()))
            .saturating_add(self.functions.len().saturating_mul(size_of::<Box<[u32]>>()));
        for units in &self.functions {
            bytes = bytes.saturating_add(units.len().saturating_mul(size_of::<u32>()));
        }
        bytes
    }
}

#[derive(Debug)]
struct ConstantStorage {
    values: Box<[Constant]>,
    string_hashes: Box<[u64]>,
}

#[derive(Clone, Debug)]
pub(crate) struct SharedProgramString {
    storage: Arc<ConstantStorage>,
    index: usize,
}

impl SharedProgramString {
    #[inline]
    pub(crate) fn as_bytes(&self) -> &[u8] {
        match &self.storage.values[self.index] {
            Constant::String(bytes) => bytes,
            _ => unreachable!("shared string descriptors are validated at construction"),
        }
    }

    #[inline]
    pub(crate) fn stable_hash(&self) -> u64 {
        self.storage.string_hashes[self.index]
    }
}

#[derive(Clone, Debug)]
pub struct Function {
    pub code: Vec<Instruction>,
    pub max_registers: usize,
    pub parameter_count: usize,
    pub is_vararg: bool,
    pub upvalues: Vec<UpvalueDescriptor>,
    pub captured_registers: Vec<u8>,
    pub source_map: Vec<InstructionSource>,
    pub debug_name: Option<String>,
    pub debug_line_defined: Option<u32>,
    pub parameter_types: Vec<RuntimeType>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeType {
    Any,
    Nil,
    Number,
    String,
    OptionalString,
    Table,
    Function,
    Thread,
    Userdata,
    Vector,
    Buffer,
    OptionalBuffer,
}

#[derive(Clone, Debug)]
pub enum Constant {
    Nil,
    Boolean(bool),
    Number(f64),
    String(Vec<u8>),
    /// A one-to-three component import path whose entries index string constants.
    Import(Vec<u32>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstructionSource {
    pub pc: usize,
    pub line: u32,
    /// Exact source range, when provided by the compiler. Hand-written
    /// bytecode can retain line-only debug information.
    pub span: Option<crate::diagnostic::DiagnosticSpan>,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Opcode {
    Halt = 0,
    LoadConst = 1,
    Move = 2,
    Add = 3,
    Sub = 4,
    Mul = 5,
    Div = 6,
    Mod = 7,
    Neg = 8,
    Jump = 9,
    JumpIfNot = 10,
    JumpIf = 11,
    Return = 12,
    CallExtension = 13,
    Assert = 14,
    Equal = 15,
    NotEqual = 16,
    Not = 17,
    Less = 18,
    LessEqual = 19,
    Greater = 20,
    GreaterEqual = 21,
    FloorDiv = 22,
    Pow = 23,
    Concat = 24,
    ForPrep = 25,
    ForLoop = 26,
    ToNumber = 27,
    NewArray = 28,
    IpairsPrep = 29,
    IpairsLoop = 30,
    PairsPrep = 31,
    PairsLoop = 32,
    Len = 33,
    GetIndex = 34,
    SetIndex = 35,
    TableConcat = 36,
    ConcatArgs = 37,
    ToString = 38,
    NextKey = 39,
    TypeOf = 40,
    Abs = 41,
    StringSub = 42,
    StringLen = 43,
    StringByte = 44,
    StringUpper = 45,
    StringLower = 46,
    StringReverse = 47,
    StringRep = 48,
    StringFind = 49,
    StringSplit = 50,
    TableGetN = 51,
    TableMaxN = 52,
    MathFloor = 53,
    MathCeil = 54,
    MathRound = 55,
    MathSign = 56,
    MathClamp = 57,
    MathMap = 58,
    MathLerp = 59,
    MathMin = 60,
    MathMax = 61,
    MathFmod = 62,
    MathPow = 63,
    MathSqrt = 64,
    MathDeg = 65,
    MathRad = 66,
    MathIsNan = 67,
    MathIsInf = 68,
    MathIsFinite = 69,
    MathSin = 70,
    MathCos = 71,
    MathTan = 72,
    MathAsin = 73,
    MathAcos = 74,
    MathAtan = 75,
    MathAtan2 = 76,
    MathSinh = 77,
    MathCosh = 78,
    MathTanh = 79,
    MathExp = 80,
    MathLog = 81,
    MathLog10 = 82,
    MathLdexp = 83,
    RawGet = 84,
    RawSet = 85,
    RawLen = 86,
    TableFind = 87,
    TableCreate = 88,
    TableInsert = 89,
    TableRemove = 90,
    LoadFunction = 91,
    Call = 92,
    LoadCurrentFunction = 93,
    GetGlobal = 94,
    SetGlobal = 95,
    GetLocalCell = 96,
    SetLocalCell = 97,
    GetUpvalue = 98,
    SetUpvalue = 99,
    ClearLocalCell = 100,
    CallMulti = 101,
    ReturnCall = 102,
    CallExpand = 103,
    LoadVarargs = 104,
    ReturnVarargs = 105,
    CallVarargs = 106,
    ReturnCallVarargs = 107,
    AppendVarargs = 108,
    ReturnCallExpand = 109,
    CallExpand2 = 110,
    CallExpandVarargs = 111,
    ReturnCallExpand2 = 112,
    ReturnCallExpandVarargs = 113,
    AppendCall = 114,
    AppendCallVarargs = 115,
    AppendCallExpand = 116,
    ReturnCallExpand2Varargs = 117,
    GenericCallPrep = 118,
    GenericCallLoop = 119,
    GeneralizedIterPrep = 120,
    GeneralizedIterLoop = 121,
    LoadConstWide = 122,
    CallExpand3 = 123,
    CheckMethod = 124,
    CoerceForNumber = 125,
    GetImport = 126,
    FastCall = 127,
    /// Constant-string table lookup. A nonzero `C` also receives the loaded key value so a
    /// compound update can reuse the original logical constant-load step without forcing a new
    /// short-string atom on a miss.
    GetTableKs = 128,
    /// Constant-string table write. `C = 0` fuses the logical key load (fuel 2); nonzero `C`
    /// names a key register loaded earlier and specializes only the final write (fuel 1), so
    /// compiler lowering can preserve evaluation and GC order across an intervening RHS.
    SetTableKs = 129,
    /// Constant-name method lookup. `A` receives the callee, `B` names the receiver register,
    /// and `aux` names the string constant. `C = 0` performs lookup only (fuel 2); `C = 1`
    /// also checks for a missing method (fuel 3). Other `C` values are invalid.
    NameCall = 130,
}

impl Opcode {
    /// Every production opcode in ascending tag order.
    ///
    /// This is the canonical tag-to-opcode table for tooling (the opcode
    /// census, disassembly, fuzz coverage). A unit test pins each entry to its
    /// declared tag so the table cannot drift from the enum.
    pub const ALL: [Opcode; 131] = [
        Opcode::Halt,
        Opcode::LoadConst,
        Opcode::Move,
        Opcode::Add,
        Opcode::Sub,
        Opcode::Mul,
        Opcode::Div,
        Opcode::Mod,
        Opcode::Neg,
        Opcode::Jump,
        Opcode::JumpIfNot,
        Opcode::JumpIf,
        Opcode::Return,
        Opcode::CallExtension,
        Opcode::Assert,
        Opcode::Equal,
        Opcode::NotEqual,
        Opcode::Not,
        Opcode::Less,
        Opcode::LessEqual,
        Opcode::Greater,
        Opcode::GreaterEqual,
        Opcode::FloorDiv,
        Opcode::Pow,
        Opcode::Concat,
        Opcode::ForPrep,
        Opcode::ForLoop,
        Opcode::ToNumber,
        Opcode::NewArray,
        Opcode::IpairsPrep,
        Opcode::IpairsLoop,
        Opcode::PairsPrep,
        Opcode::PairsLoop,
        Opcode::Len,
        Opcode::GetIndex,
        Opcode::SetIndex,
        Opcode::TableConcat,
        Opcode::ConcatArgs,
        Opcode::ToString,
        Opcode::NextKey,
        Opcode::TypeOf,
        Opcode::Abs,
        Opcode::StringSub,
        Opcode::StringLen,
        Opcode::StringByte,
        Opcode::StringUpper,
        Opcode::StringLower,
        Opcode::StringReverse,
        Opcode::StringRep,
        Opcode::StringFind,
        Opcode::StringSplit,
        Opcode::TableGetN,
        Opcode::TableMaxN,
        Opcode::MathFloor,
        Opcode::MathCeil,
        Opcode::MathRound,
        Opcode::MathSign,
        Opcode::MathClamp,
        Opcode::MathMap,
        Opcode::MathLerp,
        Opcode::MathMin,
        Opcode::MathMax,
        Opcode::MathFmod,
        Opcode::MathPow,
        Opcode::MathSqrt,
        Opcode::MathDeg,
        Opcode::MathRad,
        Opcode::MathIsNan,
        Opcode::MathIsInf,
        Opcode::MathIsFinite,
        Opcode::MathSin,
        Opcode::MathCos,
        Opcode::MathTan,
        Opcode::MathAsin,
        Opcode::MathAcos,
        Opcode::MathAtan,
        Opcode::MathAtan2,
        Opcode::MathSinh,
        Opcode::MathCosh,
        Opcode::MathTanh,
        Opcode::MathExp,
        Opcode::MathLog,
        Opcode::MathLog10,
        Opcode::MathLdexp,
        Opcode::RawGet,
        Opcode::RawSet,
        Opcode::RawLen,
        Opcode::TableFind,
        Opcode::TableCreate,
        Opcode::TableInsert,
        Opcode::TableRemove,
        Opcode::LoadFunction,
        Opcode::Call,
        Opcode::LoadCurrentFunction,
        Opcode::GetGlobal,
        Opcode::SetGlobal,
        Opcode::GetLocalCell,
        Opcode::SetLocalCell,
        Opcode::GetUpvalue,
        Opcode::SetUpvalue,
        Opcode::ClearLocalCell,
        Opcode::CallMulti,
        Opcode::ReturnCall,
        Opcode::CallExpand,
        Opcode::LoadVarargs,
        Opcode::ReturnVarargs,
        Opcode::CallVarargs,
        Opcode::ReturnCallVarargs,
        Opcode::AppendVarargs,
        Opcode::ReturnCallExpand,
        Opcode::CallExpand2,
        Opcode::CallExpandVarargs,
        Opcode::ReturnCallExpand2,
        Opcode::ReturnCallExpandVarargs,
        Opcode::AppendCall,
        Opcode::AppendCallVarargs,
        Opcode::AppendCallExpand,
        Opcode::ReturnCallExpand2Varargs,
        Opcode::GenericCallPrep,
        Opcode::GenericCallLoop,
        Opcode::GeneralizedIterPrep,
        Opcode::GeneralizedIterLoop,
        Opcode::LoadConstWide,
        Opcode::CallExpand3,
        Opcode::CheckMethod,
        Opcode::CoerceForNumber,
        Opcode::GetImport,
        Opcode::FastCall,
        Opcode::GetTableKs,
        Opcode::SetTableKs,
        Opcode::NameCall,
    ];

    /// Decodes a raw tag byte, or `None` for an unassigned tag.
    pub fn from_tag(tag: u8) -> Option<Opcode> {
        Self::ALL.get(usize::from(tag)).copied()
    }
}

/// Iterator-call instruction `b` lane emitted when the caller has no captured locals.
///
/// Zero retains the legacy conservative behavior, so existing and hand-authored bytecode still
/// synchronizes its register cells after a guest iterator call.
pub const ITERATOR_CALLER_HAS_NO_CAPTURED_LOCALS: u8 = 1;

pub const ASSERT_MESSAGE_NONE: i32 = -1;
pub const ASSERT_MESSAGE_MISSING_ARGUMENT: i32 = -2;

/// Fixed-width VM instruction. The `aux` word is reserved from day one for jump targets,
/// extension operands, and P2 inline-cache side-table keys.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub op: Opcode,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub aux: i32,
}

impl Instruction {
    #[inline]
    pub const fn new(op: Opcode, a: u8, b: u8, c: u8) -> Self {
        Self {
            op,
            a,
            b,
            c,
            aux: 0,
        }
    }

    #[inline]
    pub const fn with_aux(op: Opcode, a: u8, b: u8, c: u8, aux: i32) -> Self {
        Self { op, a, b, c, aux }
    }
}

impl Program {
    pub fn new(main: Function, constants: Vec<Constant>) -> Self {
        Self {
            main,
            constants: Arc::new(ConstantStorage::new(constants)),
            functions: Vec::new(),
            source_name: None,
            diagnostic_source: None,
            diagnostic_prefix_bytes: 0,
            diagnostic_mapped_length: None,
            logical_units: None,
        }
    }

    pub fn with_functions(
        main: Function,
        constants: Vec<Constant>,
        functions: Vec<Function>,
    ) -> Self {
        Self {
            main,
            constants: Arc::new(ConstantStorage::new(constants)),
            functions,
            source_name: None,
            diagnostic_source: None,
            diagnostic_prefix_bytes: 0,
            diagnostic_mapped_length: None,
            logical_units: None,
        }
    }

    pub fn with_source_name(mut self, source_name: Option<String>) -> Self {
        self.source_name = source_name.map(Arc::from);
        self
    }

    /// Attaches a validated logical-unit schedule. A `None` or identity
    /// schedule clears the field so unoptimized programs pay nothing.
    pub fn with_logical_units(mut self, units: Option<ProgramLogicalUnits>) -> Self {
        self.logical_units = units.filter(|units| !units.is_identity()).map(Arc::new);
        self
    }

    /// The attached logical-unit schedule, if any.
    #[inline]
    pub fn logical_units(&self) -> Option<&ProgramLogicalUnits> {
        self.logical_units.as_deref()
    }

    /// Per-instruction units for one function (`None` prototype = main).
    /// Empty when no schedule is attached.
    #[inline]
    pub(crate) fn logical_units_for(&self, prototype: Option<usize>) -> Option<&[u32]> {
        self.logical_units
            .as_deref()
            .map(|units| units.function_units(prototype))
    }

    /// True when one function charges one unit per instruction: no schedule
    /// is attached, or its slice for `prototype` is all ones. This is the
    /// condition under which the fixed per-instruction charges of HotIR and
    /// the leaf-call plans equal the scalar dispatcher's unit charge.
    #[inline]
    pub(crate) fn logical_units_are_identity_for(&self, prototype: Option<usize>) -> bool {
        self.logical_units
            .as_deref()
            .is_none_or(|units| units.function_is_identity(prototype))
    }

    #[inline]
    pub fn constants(&self) -> &[Constant] {
        &self.constants.values
    }

    #[inline]
    pub(crate) fn shared_string(&self, index: usize) -> Option<SharedProgramString> {
        matches!(self.constants.values.get(index), Some(Constant::String(_))).then(|| {
            SharedProgramString {
                storage: Arc::clone(&self.constants),
                index,
            }
        })
    }

    pub(crate) fn constant_storage_identity(&self) -> *const () {
        Arc::as_ptr(&self.constants).cast()
    }

    pub(crate) fn memory_usage(&self, include_shared_constants: bool) -> (usize, usize) {
        let mut live = size_of::<Self>();
        let mut reserved = size_of::<Self>();
        add_function_memory(&self.main, &mut live, &mut reserved);
        live = live.saturating_add(self.functions.len().saturating_mul(size_of::<Function>()));
        reserved = reserved.saturating_add(
            self.functions
                .capacity()
                .saturating_mul(size_of::<Function>()),
        );
        for function in &self.functions {
            add_function_memory(function, &mut live, &mut reserved);
        }
        if let Some(source_name) = &self.source_name {
            live = live.saturating_add(source_name.len());
            reserved = reserved.saturating_add(source_name.len());
        }
        if let Some(units) = &self.logical_units {
            let bytes = units.memory_bytes();
            live = live.saturating_add(bytes);
            reserved = reserved.saturating_add(bytes);
        }
        if include_shared_constants {
            live = live
                .saturating_add(size_of::<ConstantStorage>())
                .saturating_add(
                    self.constants
                        .values
                        .len()
                        .saturating_mul(size_of::<Constant>()),
                )
                .saturating_add(
                    self.constants
                        .string_hashes
                        .len()
                        .saturating_mul(size_of::<u64>()),
                );
            reserved = reserved
                .saturating_add(size_of::<ConstantStorage>())
                .saturating_add(
                    self.constants
                        .values
                        .len()
                        .saturating_mul(size_of::<Constant>()),
                )
                .saturating_add(
                    self.constants
                        .string_hashes
                        .len()
                        .saturating_mul(size_of::<u64>()),
                );
            for constant in self.constants() {
                match constant {
                    Constant::String(bytes) => {
                        live = live.saturating_add(bytes.len());
                        reserved = reserved.saturating_add(bytes.capacity());
                    }
                    Constant::Import(path) => {
                        live = live.saturating_add(path.len().saturating_mul(size_of::<u32>()));
                        reserved = reserved
                            .saturating_add(path.capacity().saturating_mul(size_of::<u32>()));
                    }
                    Constant::Nil | Constant::Boolean(_) | Constant::Number(_) => {}
                }
            }
        }
        (live, reserved)
    }
}

fn add_function_memory(function: &Function, live: &mut usize, reserved: &mut usize) {
    macro_rules! add_vec {
        ($field:expr, $element:ty) => {
            *live = live.saturating_add($field.len().saturating_mul(size_of::<$element>()));
            *reserved =
                reserved.saturating_add($field.capacity().saturating_mul(size_of::<$element>()));
        };
    }
    add_vec!(function.code, Instruction);
    add_vec!(function.upvalues, UpvalueDescriptor);
    add_vec!(function.captured_registers, u8);
    add_vec!(function.source_map, InstructionSource);
    add_vec!(function.parameter_types, RuntimeType);
    if let Some(debug_name) = &function.debug_name {
        *live = live.saturating_add(debug_name.len());
        *reserved = reserved.saturating_add(debug_name.capacity());
    }
}

impl ConstantStorage {
    fn new(values: Vec<Constant>) -> Self {
        let string_hashes = values
            .iter()
            .map(|constant| match constant {
                Constant::String(bytes) => stable_bytes_hash(bytes),
                _ => 0,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            values: values.into_boxed_slice(),
            string_hashes,
        }
    }
}

#[inline]
pub(crate) fn stable_bytes_hash(value: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in value {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix_stable_hash(hash ^ value.len() as u64)
}

#[inline]
const fn mix_stable_hash(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

impl Function {
    pub fn new(code: Vec<Instruction>, max_registers: usize) -> Self {
        Self {
            code,
            max_registers,
            parameter_count: 0,
            is_vararg: false,
            upvalues: Vec::new(),
            captured_registers: Vec::new(),
            source_map: Vec::new(),
            debug_name: None,
            debug_line_defined: None,
            parameter_types: Vec::new(),
        }
    }

    pub fn with_params(
        code: Vec<Instruction>,
        max_registers: usize,
        parameter_count: usize,
    ) -> Self {
        Self {
            code,
            max_registers,
            parameter_count,
            is_vararg: false,
            upvalues: Vec::new(),
            captured_registers: Vec::new(),
            source_map: Vec::new(),
            debug_name: None,
            debug_line_defined: None,
            parameter_types: Vec::new(),
        }
    }

    pub fn with_params_and_vararg(
        code: Vec<Instruction>,
        max_registers: usize,
        parameter_count: usize,
        is_vararg: bool,
    ) -> Self {
        Self {
            code,
            max_registers,
            parameter_count,
            is_vararg,
            upvalues: Vec::new(),
            captured_registers: Vec::new(),
            source_map: Vec::new(),
            debug_name: None,
            debug_line_defined: None,
            parameter_types: Vec::new(),
        }
    }

    pub fn with_params_and_upvalues(
        code: Vec<Instruction>,
        max_registers: usize,
        parameter_count: usize,
        upvalues: Vec<UpvalueDescriptor>,
        captured_registers: Vec<u8>,
    ) -> Self {
        Self {
            code,
            max_registers,
            parameter_count,
            is_vararg: false,
            upvalues,
            captured_registers,
            source_map: Vec::new(),
            debug_name: None,
            debug_line_defined: None,
            parameter_types: Vec::new(),
        }
    }

    pub fn with_params_vararg_and_upvalues(
        code: Vec<Instruction>,
        max_registers: usize,
        parameter_count: usize,
        is_vararg: bool,
        upvalues: Vec<UpvalueDescriptor>,
        captured_registers: Vec<u8>,
    ) -> Self {
        Self {
            code,
            max_registers,
            parameter_count,
            is_vararg,
            upvalues,
            captured_registers,
            source_map: Vec::new(),
            debug_name: None,
            debug_line_defined: None,
            parameter_types: Vec::new(),
        }
    }

    pub fn with_source_map(mut self, source_map: Vec<InstructionSource>) -> Self {
        self.source_map = source_map;
        self
    }

    pub fn with_debug_name(mut self, debug_name: Option<String>) -> Self {
        self.debug_name = debug_name;
        self
    }

    pub fn with_debug_line_defined(mut self, line: Option<u32>) -> Self {
        self.debug_line_defined = line;
        self
    }

    pub fn with_parameter_types(mut self, parameter_types: Vec<RuntimeType>) -> Self {
        self.parameter_types = parameter_types;
        self
    }

    pub fn source_line(&self, pc: usize) -> Option<u32> {
        self.source_map
            .iter()
            .rev()
            .find(|source| source.pc == pc)
            .map(|source| source.line)
    }

    pub fn source_line_at_or_before(&self, pc: usize) -> Option<u32> {
        self.source_map
            .iter()
            .rev()
            .find(|source| source.pc <= pc)
            .map(|source| source.line)
    }

    pub fn coverage_lines(&self) -> Vec<u32> {
        let mut lines = self
            .source_map
            .iter()
            .map(|source| source.line)
            .collect::<Vec<_>>();
        lines.sort_unstable();
        lines.dedup();
        lines
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpvalueDescriptor {
    /// Capture a parent local through a shared mutable cell.
    ParentLocal(u8),
    /// Capture a parent upvalue through its shared mutable cell.
    ParentUpvalue(u8),
    /// Copy a compiler-proven immutable parent local into the closure.
    ParentLocalValue(u8),
    /// Copy a compiler-proven immutable parent upvalue into the closure.
    ParentUpvalueValue(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_clones_share_compiled_constants_and_string_storage() {
        let bytes = b"shared compiled string storage".to_vec();
        let bytes_pointer = bytes.as_ptr();
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
            vec![Constant::String(bytes)],
        );
        let cloned = program.clone();

        assert!(Arc::ptr_eq(&program.constants, &cloned.constants));
        let original = program.shared_string(0).expect("shared string");
        let cloned = cloned.shared_string(0).expect("shared string");
        assert!(Arc::ptr_eq(&original.storage, &cloned.storage));
        assert_eq!(original.as_bytes().as_ptr(), bytes_pointer);
        assert_eq!(original.as_bytes().as_ptr(), cloned.as_bytes().as_ptr());
        assert_eq!(
            original.stable_hash(),
            stable_bytes_hash(original.as_bytes())
        );
    }

    #[test]
    fn shared_string_descriptors_reject_non_strings_and_out_of_bounds_indices() {
        let program = Program::new(
            Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
            vec![Constant::Number(1.0)],
        );

        assert!(program.shared_string(0).is_none());
        assert!(program.shared_string(1).is_none());
    }

    #[test]
    fn opcode_table_matches_declared_tags_exactly() {
        for (index, opcode) in Opcode::ALL.iter().enumerate() {
            assert_eq!(*opcode as usize, index, "{opcode:?}");
        }
        assert_eq!(Opcode::from_tag(0), Some(Opcode::Halt));
        assert_eq!(Opcode::from_tag(130), Some(Opcode::NameCall));
        assert_eq!(Opcode::from_tag(131), None);
        assert_eq!(Opcode::from_tag(u8::MAX), None);
    }
}
