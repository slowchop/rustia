use rustia_compiler::compile;
use rustia_parser::parse_source_bytes;
use rustia_vm::{Constant, Function, Instruction, Opcode, Profile, Program, Vm, VmSettings};

mod stateful;
pub use stateful::{fuzz_stateful, MAX_STATEFUL_INPUT_BYTES};
pub mod differential;

pub const MAX_LEXER_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_PARSER_INPUT_BYTES: usize = 32 * 1024;
pub const MAX_COMPILER_INPUT_BYTES: usize = 16 * 1024;

const VM_INSTRUCTION_BYTES: usize = 8;
const MAX_VM_INSTRUCTIONS: usize = 256;
pub const MAX_VM_INPUT_BYTES: usize = VM_INSTRUCTION_BYTES * MAX_VM_INSTRUCTIONS;
const VM_REGISTER_COUNT: u8 = 16;
const VM_FUEL: u64 = 4_096;
const VM_RECURSION_LIMIT: usize = 32;
const VM_MEMORY_LIMIT_BYTES: usize = 512 * 1024;

pub fn fuzz_lexer(data: &[u8]) -> bool {
    if data.len() > MAX_LEXER_INPUT_BYTES {
        return false;
    }
    let _ = rustia_lexer::lex_bytes(data);
    true
}

pub fn fuzz_parser(data: &[u8]) -> bool {
    if data.len() > MAX_PARSER_INPUT_BYTES {
        return false;
    }
    let _ = parse_source_bytes(data);
    true
}

pub fn fuzz_compiler(data: &[u8]) -> bool {
    if data.len() > MAX_COMPILER_INPUT_BYTES {
        return false;
    }
    let Ok(chunk) = parse_source_bytes(data) else {
        return false;
    };
    let _ = compile(&chunk);
    true
}

pub fn fuzz_vm(data: &[u8]) -> bool {
    if data.len() > MAX_VM_INPUT_BYTES {
        return false;
    }

    let profile = match data.first().copied().unwrap_or_default() % 4 {
        0 => Profile::LuauCompat,
        1 => Profile::ReproducibleStandard,
        2 => Profile::DeterministicSim,
        _ => Profile::Puc51Compat,
    };
    let settings = VmSettings {
        initial_fuel: Some(VM_FUEL),
        recursion_limit: VM_RECURSION_LIMIT,
        memory_limit_bytes: Some(VM_MEMORY_LIMIT_BYTES),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(profile, settings);
    let _ = vm.run(&program_from_bytes(data));
    true
}

fn program_from_bytes(data: &[u8]) -> Program {
    let mut code = data
        .chunks(VM_INSTRUCTION_BYTES)
        .take(MAX_VM_INSTRUCTIONS)
        .map(decode_instruction)
        .collect::<Vec<_>>();
    if !matches!(code.last(), Some(instruction) if instruction.op == Opcode::Halt) {
        code.push(Instruction::new(Opcode::Halt, 0, 0, 0));
    }

    let input_string = data.get(..data.len().min(32)).unwrap_or(data).to_vec();
    let number = data
        .first()
        .copied()
        .map_or(0.0, |byte| f64::from(byte as i8));
    let constants = vec![
        Constant::Nil,
        Constant::Boolean(false),
        Constant::Boolean(true),
        Constant::Number(-1.0),
        Constant::Number(0.0),
        Constant::Number(1.0),
        Constant::Number(42.0),
        Constant::Number(number),
        Constant::String(Vec::new()),
        Constant::String(b"x".to_vec()),
        Constant::String(b"math".to_vec()),
        Constant::String(b"abs".to_vec()),
        Constant::String(input_string),
        Constant::Import(vec![10]),
        Constant::Import(vec![10, 11]),
    ];
    let function = Function::new(
        vec![Instruction::new(Opcode::Halt, 0, 0, 0)],
        usize::from(VM_REGISTER_COUNT),
    );

    Program::with_functions(
        Function::new(code, usize::from(VM_REGISTER_COUNT)),
        constants,
        vec![function],
    )
}

fn decode_instruction(bytes: &[u8]) -> Instruction {
    let tag = byte(bytes, 0);
    let op = match tag {
        b'H' => Opcode::Halt,
        b'L' => Opcode::LoadConst,
        b'R' => Opcode::Return,
        _ => VM_OPCODES[usize::from(tag) % VM_OPCODES.len()],
    };
    let a = byte(bytes, 1) % VM_REGISTER_COUNT;
    let b = byte(bytes, 2) % VM_REGISTER_COUNT;
    let c = byte(bytes, 3) % VM_REGISTER_COUNT;
    let raw_aux = i32::from_le_bytes([
        byte(bytes, 4),
        byte(bytes, 5),
        byte(bytes, 6),
        byte(bytes, 7),
    ]);
    let aux = if tag & 0x80 == 0 {
        raw_aux.rem_euclid(32) - 8
    } else {
        raw_aux
    };
    Instruction::with_aux(op, a, b, c, aux)
}

fn byte(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or_default()
}

pub fn fuzzed_vm_opcodes() -> &'static [Opcode] {
    VM_OPCODES
}

// Every production opcode is safe to include now that input length, instruction
// count, fuel, recursion, and VM-owned logical-live memory are all bounded.
const VM_OPCODES: &[Opcode] = &[
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
