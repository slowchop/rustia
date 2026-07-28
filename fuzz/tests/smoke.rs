use rustia_fuzz::{
    fuzz_compiler, fuzz_lexer, fuzz_parser, fuzz_vm, MAX_COMPILER_INPUT_BYTES,
    MAX_LEXER_INPUT_BYTES, MAX_PARSER_INPUT_BYTES, MAX_VM_INPUT_BYTES,
};

const VALID_SOURCES: &[&[u8]] = &[
    b"return 40 + 2",
    b"local function add(a: number, b: number): number return a + b end return add(20, 22)",
    b"local t = {answer = 42}; return t.answer",
    b"for i = 1, 4 do if i == 3 then continue end end return true",
];

#[test]
fn frontend_targets_accept_seed_programs() {
    for source in VALID_SOURCES {
        assert!(fuzz_lexer(source));
        assert!(fuzz_parser(source));
        assert!(fuzz_compiler(source));
    }
}

#[test]
fn vm_target_accepts_bounded_bytecode_inputs() {
    for data in [
        &b""[..],
        &b"H0000000"[..],
        &b"L0000000R0100000"[..],
        &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
        &[2],
    ] {
        assert!(fuzz_vm(data));
    }
}

#[test]
fn every_target_rejects_oversized_inputs() {
    assert!(!fuzz_lexer(&vec![0; MAX_LEXER_INPUT_BYTES + 1]));
    assert!(!fuzz_parser(&vec![0; MAX_PARSER_INPUT_BYTES + 1]));
    assert!(!fuzz_compiler(&vec![0; MAX_COMPILER_INPUT_BYTES + 1]));
    assert!(!fuzz_vm(&vec![0; MAX_VM_INPUT_BYTES + 1]));
}
