use rustia_fuzz::{
    fuzz_compiler, fuzz_lexer, fuzz_parser, fuzz_vm, fuzzed_vm_opcodes, MAX_COMPILER_INPUT_BYTES,
    MAX_LEXER_INPUT_BYTES, MAX_PARSER_INPUT_BYTES, MAX_VM_INPUT_BYTES,
};
use rustia_vm::Opcode;

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
fn vm_target_covers_every_production_opcode_once() {
    let opcodes = fuzzed_vm_opcodes();
    let mut seen = vec![false; Opcode::NameCall as usize + 1];
    for opcode in opcodes {
        let slot = &mut seen[*opcode as usize];
        assert!(!*slot, "duplicate fuzz opcode {opcode:?}");
        *slot = true;
    }
    assert!(
        seen.into_iter().all(|present| present),
        "every production opcode must remain in the bounded VM fuzz decoder"
    );
}

#[test]
fn deterministic_frontend_mutation_sweep_stays_bounded() {
    let mut random = DeterministicBytes::new(0x6f_70_63_6f_64_65);
    for case in 0..2_048 {
        let mut data = VALID_SOURCES[case % VALID_SOURCES.len()].to_vec();
        let edits = 1 + random.usize(8);
        for _ in 0..edits {
            match random.usize(4) {
                0 if !data.is_empty() => {
                    let index = random.usize(data.len());
                    data[index] = random.byte();
                }
                1 if data.len() < 1_024 => {
                    let index = random.usize(data.len() + 1);
                    data.insert(index, random.byte());
                }
                2 if !data.is_empty() => {
                    let index = random.usize(data.len());
                    data.remove(index);
                }
                _ => data.truncate(random.usize(data.len() + 1)),
            }
        }
        assert!(fuzz_lexer(&data));
        assert!(fuzz_parser(&data));
        let _ = fuzz_compiler(&data);
    }
}

#[test]
fn deterministic_vm_mutation_sweep_stays_within_all_resource_bounds() {
    let mut random = DeterministicBytes::new(0x72_75_73_74_69_61);
    for _ in 0..2_048 {
        let mut data = vec![0; random.usize(MAX_VM_INPUT_BYTES + 1)];
        for byte in &mut data {
            *byte = random.byte();
        }
        assert!(fuzz_vm(&data));
    }
}

#[test]
fn every_target_rejects_oversized_inputs() {
    assert!(!fuzz_lexer(&vec![0; MAX_LEXER_INPUT_BYTES + 1]));
    assert!(!fuzz_parser(&vec![0; MAX_PARSER_INPUT_BYTES + 1]));
    assert!(!fuzz_compiler(&vec![0; MAX_COMPILER_INPUT_BYTES + 1]));
    assert!(!fuzz_vm(&vec![0; MAX_VM_INPUT_BYTES + 1]));
}

struct DeterministicBytes(u64);

#[test]
#[ignore = "requires the separately built pinned oracle worker; run in differential CI"]
fn generated_differential_smoke() {
    for shape in 8..35 {
        for builtin in 0..7 {
            assert!(rustia_fuzz::differential::fuzz_differential(&[
                shape, 1, builtin, 0,
            ]));
        }
    }
    let mut random = DeterministicBytes::new(0x6572_726f_7273);
    for _ in 0..256 {
        let data: Vec<_> = (0..16).map(|_| random.byte()).collect();
        assert!(rustia_fuzz::differential::fuzz_differential(&data));
    }
}

#[test]
fn deterministic_stateful_failure_and_recovery_sequences() {
    for profile in 0..4 {
        for suffix in [
            &[13, 14, 13][..],
            &[13, 15, 13, 14][..],
            &[13, 16, 14][..],
            &[13, 17, 13, 14][..],
            &[13, 3, 13, 14][..],
        ] {
            let mut data = vec![profile];
            data.extend_from_slice(suffix);
            assert!(rustia_fuzz::fuzz_stateful(&data));
        }
    }
    let mut random = DeterministicBytes::new(0x6572_726f_7273);
    for _ in 0..256 {
        let data: Vec<_> = (0..32).map(|_| random.byte()).collect();
        assert!(rustia_fuzz::fuzz_stateful(&data));
    }
    assert!(!rustia_fuzz::fuzz_stateful(
        &[0; rustia_fuzz::MAX_STATEFUL_INPUT_BYTES + 1]
    ));
}

impl DeterministicBytes {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn usize(&mut self, upper_exclusive: usize) -> usize {
        if upper_exclusive == 0 {
            return 0;
        }
        (self.next() as usize) % upper_exclusive
    }

    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
}
