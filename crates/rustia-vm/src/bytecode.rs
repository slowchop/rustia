use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Program {
    pub main: Function,
    pub constants: Vec<Constant>,
    pub functions: Vec<Function>,
    pub record_key_sets: Vec<Vec<usize>>,
    pub source_name: Option<Arc<str>>,
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
    pub debug_name: Option<Arc<str>>,
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstructionSource {
    pub pc: usize,
    pub line: u32,
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
    StringFormat = 126,
    NewRecord = 127,
    LenPlusOne = 128,
    StringByteEqual = 129,
    AddConst = 130,
    SubConst = 131,
    MulConst = 132,
    DivConst = 133,
    FloorDivConst = 134,
    ModConst = 135,
    PowConst = 136,
    MathSinCosMul = 137,
    AddUpvalue = 138,
    ForPrepStepOne = 139,
    ForLoopStepOne = 140,
    AddUpvalueConst = 141,
    ConcatConst = 142,
    GetIndexConcatConst = 143,
    SetIndexConcatConst = 144,
    ClearLocalCells = 145,
    GetIndexNumber = 146,
    SetIndexNumber = 147,
    ConcatRhsConst = 148,
    GetIndexNumberConst = 149,
    MulModConst = 150,
    MulAddModConst = 151,
    ModAddConst = 152,
    GetIndexModAddConst = 153,
    SetIndexMulAddModConst = 154,
    SetIndexMulModConst = 155,
    GetIndexMulModConst = 156,
    GetIndexStringConst = 157,
    SetIndexStringConst = 158,
    SetIndexNumberConst = 159,
    CheckMethodStringConst = 160,
    NewRecordConstKeys = 161,
    RawGetStringConst = 162,
    RawSetStringConst = 163,
}

pub const ARRAY_ITERATOR_WRITE_VISIBLE_KEY: u8 = 0;
pub const ARRAY_ITERATOR_SKIP_VISIBLE_KEY: u8 = 1;

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
            constants,
            functions: Vec::new(),
            record_key_sets: Vec::new(),
            source_name: None,
        }
    }

    pub fn with_functions(
        main: Function,
        constants: Vec<Constant>,
        functions: Vec<Function>,
    ) -> Self {
        Self {
            main,
            constants,
            functions,
            record_key_sets: Vec::new(),
            source_name: None,
        }
    }

    pub fn with_record_key_sets(mut self, record_key_sets: Vec<Vec<usize>>) -> Self {
        self.record_key_sets = record_key_sets;
        self
    }

    pub fn with_source_name(mut self, source_name: Option<String>) -> Self {
        self.source_name = source_name.map(Arc::from);
        self
    }
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
        self.debug_name = debug_name.map(Arc::from);
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
    ParentLocal(u8),
    ParentUpvalue(u8),
}
