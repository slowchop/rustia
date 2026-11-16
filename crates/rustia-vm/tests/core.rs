#[path = "support/ownership.rs"]
mod ownership;
use ownership::{Rc, RefCell};
use rustia_vm::number::{binary, NumericOp, CANON_NAN_BITS};
use rustia_vm::{
    ClockProvider, Constant, EntropyProvider, ErrorKind, ExtensionOp, Function, HostFunctionOp,
    Instruction, OpCapability, Opcode, Profile, Program, StdoutSink, Value, ValueKind, Vm,
    VmServices, VmSettings, DECISION_RESET_ABI_VERSION, MAX_RECURSION_LIMIT,
};
use std::num::NonZeroU64;
use std::sync::Arc;

fn first_arg(args: &[Value]) -> rustia_vm::Result<Value> {
    Ok(args.first().copied().unwrap_or_else(Value::nil))
}

fn loadstring_program_returns_42(
    _source: &[u8],
    _source_name: Option<&[u8]>,
    _profile: Profile,
) -> std::result::Result<Program, String> {
    Ok(Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::Number(42.0)],
    ))
}

fn loadstring_compiler_should_not_run(
    _source: &[u8],
    _source_name: Option<&[u8]>,
    _profile: Profile,
) -> std::result::Result<Program, String> {
    panic!("loadstring compiler should not run after fuel exhaustion")
}

fn panicking_loadstring_compiler(
    _source: &[u8],
    _source_name: Option<&[u8]>,
    _profile: Profile,
) -> std::result::Result<Program, String> {
    panic!("loadstring compiler panic")
}

fn panicking_extension(_args: &[Value]) -> rustia_vm::Result<Value> {
    panic!("extension panic")
}

fn assert_host_panic_poison_and_reset(vm: &mut Vm, program: &Program, function: &str) {
    let error = vm.run(program).unwrap_err();
    assert!(matches!(
        &error.kind,
        ErrorKind::HostPanicked {
            function: actual
        } if actual == function
    ));
    assert!(vm.is_poisoned());

    let idle = Program::new(
        Function::new(vec![Instruction::new(Opcode::Return, 0, 1, 0)], 1),
        Vec::new(),
    );
    assert!(matches!(
        vm.run(&idle).unwrap_err().kind,
        ErrorKind::VmPoisoned
    ));
    vm.reset_decision()
        .expect("contained host panic must leave the VM resettable");
    assert!(!vm.is_poisoned());
    assert_eq!(vm.run(&idle).unwrap().kind(), ValueKind::Nil);
}

fn global_kind(profile: Profile, name: &[u8]) -> ValueKind {
    global_kind_in(&mut Vm::new(profile), name)
}

fn global_kind_in(vm: &mut Vm, name: &[u8]) -> ValueKind {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::String(name.to_vec())],
    );
    vm.run(&program).unwrap().kind()
}

/// Reads `os.<member>` without calling it.
fn os_member_program(member: &[u8]) -> Program {
    Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::GetIndex, 0, 0, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"os".to_vec()),
            Constant::String(member.to_vec()),
        ],
    )
}

/// Calls `os.<member>()` with no arguments and returns the result.
fn os_member_call_program(member: &[u8]) -> Program {
    Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::GetIndex, 0, 0, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"os".to_vec()),
            Constant::String(member.to_vec()),
        ],
    )
}

/// Calls `math.random()` with no arguments and returns the result.
fn math_random_program() -> Program {
    Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::GetIndex, 0, 0, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"math".to_vec()),
            Constant::String(b"random".to_vec()),
        ],
    )
}

/// Calls `print(...)` with the given constants as arguments.
fn print_call_program(args: &[Constant]) -> Program {
    let mut code = vec![Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0)];
    let mut constants = vec![Constant::String(b"print".to_vec())];
    for (index, argument) in args.iter().enumerate() {
        constants.push(argument.clone());
        code.push(Instruction::new(
            Opcode::LoadConst,
            index as u8 + 1,
            0,
            index as u8 + 1,
        ));
    }
    code.push(Instruction::with_aux(
        Opcode::Call,
        0,
        0,
        1,
        args.len() as i32,
    ));
    code.push(Instruction::new(Opcode::Return, 0, 1, 0));
    Program::new(Function::new(code, args.len() + 1), constants)
}

fn settings_with_stdout(captured: &Rc<RefCell<String>>) -> VmSettings {
    let sink = Rc::clone(captured);
    VmSettings {
        services: VmServices {
            stdout: Some(StdoutSink::new(move |text| {
                sink.borrow_mut().push_str(text);
            })),
            ..VmServices::default()
        },
        ..VmSettings::default()
    }
}

fn import_program(path: &[&[u8]]) -> Program {
    let mut constants = path
        .iter()
        .map(|component| Constant::String(component.to_vec()))
        .collect::<Vec<_>>();
    let descriptor = constants.len();
    constants.push(Constant::Import(
        (0..path.len()).map(|index| index as u32).collect(),
    ));
    Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetImport, 0, path.len() as u8, 0, descriptor as i32),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        constants,
    )
}

#[test]
fn value_is_sixteen_byte_tagged_newtype() {
    assert_eq!(std::mem::size_of::<Value>(), 16);
    assert_eq!(std::mem::align_of::<Value>(), 8);
}

#[test]
fn value_debug_keeps_reference_payloads_opaque() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::String(b"debug".to_vec())],
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let value = vm.run(&program).unwrap();
    assert_eq!(format!("{value:?}"), "string");
}

#[test]
fn indexed_hash_lookup_preserves_tombstone_insertion_order() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let table = vm.empty_table();
    let mut keys = Vec::new();

    for index in 0..128 {
        let key = vm.string(format!("field_{index:03}"));
        vm.raw_table_set_value(table, key, Value::number(index as f64))
            .unwrap();
        keys.push(key);
    }
    for (index, key) in keys.iter().copied().enumerate() {
        assert_eq!(
            vm.raw_table_get_value(table, key).unwrap().as_number(),
            Some(index as f64)
        );
    }

    let tombstone = keys[37];
    vm.raw_table_set_value(table, tombstone, Value::nil())
        .unwrap();
    vm.raw_table_set_value(table, tombstone, Value::number(999.0))
        .unwrap();

    let entries = vm.raw_table_entries_value(table).unwrap();
    let position = entries
        .iter()
        .position(|(key, _)| {
            vm.string_value_bytes(*key).unwrap().as_deref() == Some(b"field_037".as_slice())
        })
        .expect("reinserted tombstone should be iterable");
    assert_eq!(position, 37);
    assert_eq!(entries[position].1.as_number(), Some(999.0));
}

#[test]
fn number_canon_preserves_negative_zero_outside_table_keys() {
    let value = Value::number(-0.0);
    assert_eq!(value.as_number().unwrap().to_bits(), (-0.0f64).to_bits());
}

#[test]
fn nan_is_canonical_at_value_ingress() {
    let value = Value::number(f64::NAN);
    assert_eq!(value.as_number().unwrap().to_bits(), CANON_NAN_BITS);
}

#[test]
fn deterministic_sim_omits_environment_dependent_gc_and_os_globals() {
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::Puc51Compat,
    ] {
        assert_eq!(global_kind(profile, b"collectgarbage"), ValueKind::Function);
        assert_eq!(global_kind(profile, b"gcinfo"), ValueKind::Function);
        assert_eq!(global_kind(profile, b"os"), ValueKind::Table);
    }

    for name in [b"collectgarbage".as_slice(), b"gcinfo", b"os"] {
        assert_eq!(global_kind(Profile::DeterministicSim, name), ValueKind::Nil);
    }
}

#[test]
fn getimport_supports_one_to_three_live_components_with_one_three_five_fuel() {
    let cases = [
        (import_program(&[b"math"]), 1_u64, None),
        (
            import_program(&[b"math", b"pi"]),
            3_u64,
            Some(std::f64::consts::PI),
        ),
        (
            import_program(&[b"root", b"child", b"value"]),
            5_u64,
            Some(42.0),
        ),
    ];

    for (program, import_fuel, expected_number) in cases {
        let settings = VmSettings {
            initial_fuel: Some(import_fuel),
            ..VmSettings::default()
        };
        let mut starved = Vm::with_settings(Profile::ReproducibleStandard, settings);
        if expected_number == Some(42.0) {
            let child = starved.empty_table();
            let value = starved.string("value");
            starved
                .raw_table_set_value(child, value, Value::number(42.0))
                .unwrap();
            let root = starved.empty_table();
            let child_key = starved.string("child");
            starved.raw_table_set_value(root, child_key, child).unwrap();
            starved.set_global_value("root", root).unwrap();
        }
        let error = starved
            .run(&program)
            .expect_err("Return still needs one fuel");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::FuelExhausted { limit } if limit == import_fuel
        ));
        assert_eq!(error.pc, Some(1));

        let settings = VmSettings {
            initial_fuel: Some(import_fuel + 1),
            ..VmSettings::default()
        };
        let mut enough = Vm::with_settings(Profile::ReproducibleStandard, settings);
        if expected_number == Some(42.0) {
            let child = enough.empty_table();
            let value = enough.string("value");
            enough
                .raw_table_set_value(child, value, Value::number(42.0))
                .unwrap();
            let root = enough.empty_table();
            let child_key = enough.string("child");
            enough.raw_table_set_value(root, child_key, child).unwrap();
            enough.set_global_value("root", root).unwrap();
        }
        let result = enough.run(&program).expect("exact import fuel should pass");
        if let Some(expected) = expected_number {
            assert_eq!(result.as_number(), Some(expected));
        } else {
            assert_eq!(result.kind(), ValueKind::Table);
        }
    }
}

#[test]
fn getimport_reserves_its_fused_fuel_before_lookup_steps() {
    let settings = VmSettings {
        initial_fuel: Some(4),
        deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
    let child = vm.empty_table();
    let value = vm.string("value");
    vm.raw_table_set_value(child, value, Value::number(42.0))
        .unwrap();
    let root = vm.empty_table();
    let child_key = vm.string("child");
    vm.raw_table_set_value(root, child_key, child).unwrap();
    vm.set_global_value("root", root).unwrap();

    let error = vm
        .run(&import_program(&[b"root", b"child", b"value"]))
        .expect_err("a five-fuel import must not start with four fuel");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 4 }
    ));
    assert_eq!(error.pc, Some(0));
    assert_eq!(vm.deterministic_collection_count(), 0);
}

#[test]
fn getimport_rejects_malformed_descriptors_and_value_loads() {
    let malformed = [
        Program::new(
            Function::new(
                vec![Instruction::with_aux(Opcode::GetImport, 0, 2, 0, 1)],
                1,
            ),
            vec![
                Constant::String(b"math".to_vec()),
                Constant::Import(vec![0]),
            ],
        ),
        Program::new(
            Function::new(
                vec![Instruction::with_aux(Opcode::GetImport, 0, 1, 0, 1)],
                1,
            ),
            vec![Constant::Number(1.0), Constant::Import(vec![0])],
        ),
        Program::new(
            Function::new(
                vec![Instruction::with_aux(Opcode::GetImport, 0, 1, 0, 0)],
                1,
            ),
            vec![Constant::String(b"math".to_vec())],
        ),
        Program::new(
            Function::new(vec![Instruction::new(Opcode::LoadConst, 0, 0, 0)], 1),
            vec![Constant::Import(vec![1])],
        ),
    ];

    for program in malformed {
        let error = Vm::new(Profile::ReproducibleStandard)
            .run(&program)
            .expect_err("malformed import bytecode must be rejected");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::InvalidProgram { .. }
        ));
    }
}

#[test]
fn getimport_rejects_invalid_encoding_fields() {
    let constants = vec![
        Constant::String(b"math".to_vec()),
        Constant::Import(vec![0]),
    ];
    let invalid_program =
        |instruction| Program::new(Function::new(vec![instruction], 1), constants.clone());

    for instruction in [
        Instruction::with_aux(Opcode::GetImport, 0, 1, 0, -1),
        Instruction::with_aux(Opcode::GetImport, 0, 0, 0, 1),
        Instruction::with_aux(Opcode::GetImport, 0, 4, 0, 1),
        Instruction::with_aux(Opcode::GetImport, 0, 1, 1, 1),
    ] {
        let error = Vm::new(Profile::ReproducibleStandard)
            .run(&invalid_program(instruction))
            .expect_err("invalid import encoding must be rejected");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::InvalidProgram { .. }
        ));
    }

    let error = Vm::new(Profile::ReproducibleStandard)
        .run(&invalid_program(Instruction::with_aux(
            Opcode::GetImport,
            0,
            1,
            0,
            9,
        )))
        .expect_err("out-of-range import descriptor must be rejected");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::ConstantOutOfBounds { index: 9, len: 2 }
    ));
}

#[test]
fn constant_key_and_namecall_opcodes_reserve_fused_fuel_atomically() {
    let cases = [
        (Instruction::with_aux(Opcode::GetTableKs, 0, 1, 0, 0), 1),
        (Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, 0), 1),
        (Instruction::with_aux(Opcode::NameCall, 0, 1, 0, 0), 1),
        (Instruction::with_aux(Opcode::NameCall, 0, 1, 1, 0), 2),
    ];

    for (instruction, limit) in cases {
        let settings = VmSettings {
            initial_fuel: Some(limit),
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let program = Program::new(
            Function::new(vec![instruction], 2),
            vec![Constant::String(b"field".to_vec())],
        );
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
        let error = vm
            .run(&program)
            .expect_err("a fused opcode must reserve its full fuel before starting");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::FuelExhausted { limit: actual } if actual == limit
        ));
        assert_eq!(error.pc, Some(0));
        assert_eq!(vm.deterministic_collection_count(), 0);
    }
}

#[test]
fn special_fuel_lookup_preserves_validation_and_operand_error_order() {
    let invalid_before_fuel = [
        (
            Program::new(
                Function::new(vec![Instruction::with_aux(Opcode::NameCall, 0, 1, 2, 0)], 2),
                vec![Constant::String(b"method".to_vec())],
            ),
            "namecall mode must be zero or one",
        ),
        (
            Program::new(
                Function::new(
                    vec![Instruction::with_aux(Opcode::GetImport, 0, 0, 0, 0)],
                    1,
                ),
                Vec::new(),
            ),
            "import path length must be between one and three",
        ),
        (
            Program::new(
                Function::new(
                    vec![Instruction::with_aux(Opcode::CallExtension, 0, 0, 0, 0)],
                    1,
                ),
                Vec::new(),
            ),
            "unknown extension op id",
        ),
    ];

    for (program, expected_reason) in invalid_before_fuel {
        let settings = VmSettings {
            initial_fuel: Some(0),
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
        let error = vm
            .run(&program)
            .expect_err("special opcode validation must happen before fuel charging");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::InvalidProgram { reason } if reason == expected_reason
        ));
        assert_eq!(error.pc, Some(0));
        assert_eq!(vm.deterministic_collection_count(), 0);
    }

    for instruction in [
        Instruction::with_aux(Opcode::GetTableKs, 2, 0, 0, 0),
        Instruction::with_aux(Opcode::SetTableKs, 0, 2, 0, 0),
    ] {
        let settings = VmSettings {
            initial_fuel: Some(1),
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let program = Program::new(
            Function::new(vec![instruction], 1),
            vec![Constant::String(b"field".to_vec())],
        );
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
        let error = vm
            .run(&program)
            .expect_err("fused fuel reservation must precede operand validation");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::FuelExhausted { limit: 1 }
        ));
        assert_eq!(error.pc, Some(0));
        assert_eq!(vm.deterministic_collection_count(), 0);
    }
}

#[test]
fn constant_key_and_checked_namecall_opcodes_preserve_successful_gc_cadence() {
    let cases = [
        (
            vec![
                Instruction::new(Opcode::NewArray, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, 0),
                Instruction::with_aux(Opcode::GetTableKs, 2, 0, 0, 0),
                Instruction::new(Opcode::Return, 2, 1, 0),
            ],
            7,
        ),
        (
            vec![
                Instruction::new(Opcode::NewArray, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, 0),
                Instruction::with_aux(Opcode::NameCall, 2, 0, 1, 0),
                Instruction::new(Opcode::Return, 2, 1, 0),
            ],
            8,
        ),
    ];

    for (code, expected_collections) in cases {
        let settings = VmSettings {
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let program = Program::new(
            Function::new(code, 3),
            vec![Constant::String(b"field".to_vec()), Constant::Number(42.0)],
        );
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);

        assert_eq!(vm.run(&program).unwrap().as_number(), Some(42.0));
        assert_eq!(vm.deterministic_collection_count(), expected_collections);
    }
}

#[test]
fn compound_gettableks_key_output_matches_unfused_fuel_and_gc_cadence() {
    let setup = vec![
        Instruction::new(Opcode::NewArray, 0, 0, 0),
        Instruction::new(Opcode::LoadConst, 1, 0, 1),
        Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, 0),
    ];
    let mut fused = setup.clone();
    fused.extend([
        Instruction::with_aux(Opcode::GetTableKs, 2, 0, 3, 0),
        Instruction::new(Opcode::LoadConst, 4, 0, 1),
        Instruction::new(Opcode::Add, 2, 2, 4),
        Instruction::new(Opcode::SetIndex, 0, 3, 2),
        Instruction::new(Opcode::Return, 2, 1, 0),
    ]);
    let mut unfused = setup;
    unfused.extend([
        Instruction::new(Opcode::LoadConst, 3, 0, 0),
        Instruction::new(Opcode::GetIndex, 2, 0, 3),
        Instruction::new(Opcode::LoadConst, 4, 0, 1),
        Instruction::new(Opcode::Add, 2, 2, 4),
        Instruction::new(Opcode::SetIndex, 0, 3, 2),
        Instruction::new(Opcode::Return, 2, 1, 0),
    ]);

    for code in [fused, unfused] {
        let settings = VmSettings {
            initial_fuel: Some(10),
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let program = Program::new(
            Function::new(code, 5),
            vec![Constant::String(b"field".to_vec()), Constant::Number(1.0)],
        );
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);

        assert_eq!(vm.run(&program).unwrap().as_number(), Some(2.0));
        assert_eq!(vm.deterministic_collection_count(), 10);
    }
}

#[test]
fn preloaded_settableks_matches_unfused_fuel_and_gc_cadence() {
    let preloaded = vec![
        Instruction::new(Opcode::NewArray, 0, 0, 0),
        Instruction::new(Opcode::LoadConst, 1, 0, 0),
        Instruction::new(Opcode::LoadConst, 2, 0, 1),
        Instruction::with_aux(Opcode::SetTableKs, 0, 2, 1, 0),
        Instruction::new(Opcode::Return, 2, 1, 0),
    ];
    let unfused = vec![
        Instruction::new(Opcode::NewArray, 0, 0, 0),
        Instruction::new(Opcode::LoadConst, 1, 0, 0),
        Instruction::new(Opcode::LoadConst, 2, 0, 1),
        Instruction::new(Opcode::SetIndex, 0, 1, 2),
        Instruction::new(Opcode::Return, 2, 1, 0),
    ];

    let variants = [preloaded, unfused];
    for limit in 0..=5 {
        let outcomes = variants
            .iter()
            .map(|code| {
                let settings = VmSettings {
                    initial_fuel: Some(limit),
                    deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
                    ..VmSettings::default()
                };
                let program = Program::new(
                    Function::new(code.clone(), 3),
                    vec![Constant::String(b"field".to_vec()), Constant::Number(42.0)],
                );
                let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
                let outcome = match vm.run(&program) {
                    Ok(value) => format!("ok:{:?}", value.as_number()),
                    Err(error) => format!("err:{:?}:{error}", error.pc),
                };
                (outcome, vm.deterministic_collection_count())
            })
            .collect::<Vec<_>>();
        assert_eq!(outcomes[0], outcomes[1], "fuel cutoff {limit} diverged");
        if limit == 5 {
            assert_eq!(outcomes[0], ("ok:Some(42.0)".to_owned(), 5));
        }
    }
}

#[test]
fn constant_key_and_namecall_opcodes_reject_malformed_encodings() {
    let cases = [
        (
            Instruction::with_aux(Opcode::SetTableKs, 0, 1, 1, 0),
            vec![Constant::String(b"field".to_vec())],
        ),
        (
            Instruction::with_aux(Opcode::NameCall, 0, 1, 2, 0),
            vec![Constant::String(b"method".to_vec())],
        ),
        (
            Instruction::with_aux(Opcode::GetTableKs, 0, 1, 0, -1),
            vec![Constant::String(b"field".to_vec())],
        ),
        (
            Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, -1),
            vec![Constant::String(b"field".to_vec())],
        ),
        (
            Instruction::with_aux(Opcode::NameCall, 0, 1, 0, -1),
            vec![Constant::String(b"method".to_vec())],
        ),
        (
            Instruction::with_aux(Opcode::GetTableKs, 0, 1, 0, 0),
            vec![Constant::Number(1.0)],
        ),
        (
            Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, 0),
            vec![Constant::Number(1.0)],
        ),
        (
            Instruction::with_aux(Opcode::NameCall, 0, 1, 0, 0),
            vec![Constant::Number(1.0)],
        ),
    ];

    for (instruction, constants) in cases {
        let program = Program::new(Function::new(vec![instruction], 2), constants);
        let error = Vm::new(Profile::ReproducibleStandard)
            .run(&program)
            .expect_err("malformed fused table bytecode must be rejected");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::InvalidProgram { .. }
        ));
    }

    let invalid_key_output = Program::new(
        Function::new(
            vec![Instruction::with_aux(Opcode::GetTableKs, 0, 0, 1, 0)],
            1,
        ),
        vec![Constant::String(b"field".to_vec())],
    );
    let error = Vm::new(Profile::ReproducibleStandard)
        .run(&invalid_key_output)
        .expect_err("GETTABLEKS key output must reference a live register");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::RegisterOutOfBounds {
            register: 1,
            max: 1
        }
    ));

    let mismatched_preload = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::SetTableKs, 0, 1, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"field".to_vec()),
            Constant::String(b"other".to_vec()),
        ],
    );
    let error = Vm::new(Profile::ReproducibleStandard)
        .run(&mismatched_preload)
        .expect_err("preloaded SETTABLEKS key must match its constant");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::InvalidProgram { .. }
    ));

    let invalid_preload_register = Program::new(
        Function::new(
            vec![Instruction::with_aux(Opcode::SetTableKs, 0, 1, 2, 0)],
            2,
        ),
        vec![Constant::String(b"field".to_vec())],
    );
    let error = Vm::new(Profile::ReproducibleStandard)
        .run(&invalid_preload_register)
        .expect_err("preloaded SETTABLEKS key register must be in bounds");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::RegisterOutOfBounds {
            register: 2,
            max: 2
        }
    ));

    for instruction in [
        Instruction::with_aux(Opcode::GetTableKs, 0, 1, 0, 9),
        Instruction::with_aux(Opcode::SetTableKs, 0, 1, 0, 9),
        Instruction::with_aux(Opcode::NameCall, 0, 1, 0, 9),
    ] {
        let out_of_bounds = Program::new(
            Function::new(vec![instruction], 2),
            vec![Constant::String(b"field".to_vec())],
        );
        let error = Vm::new(Profile::ReproducibleStandard)
            .run(&out_of_bounds)
            .expect_err("out-of-range table key constants must be rejected");
        assert!(matches!(
            error.kind,
            rustia_vm::ErrorKind::ConstantOutOfBounds { index: 9, len: 1 }
        ));
    }
}

#[test]
fn fastcall_reuses_call_argument_validation() {
    let invalid_arity = Program::new(
        Function::new(
            vec![Instruction::with_aux(Opcode::FastCall, 0, 0, 0, -1)],
            1,
        ),
        vec![],
    );
    let error = Vm::new(Profile::ReproducibleStandard)
        .run(&invalid_arity)
        .expect_err("negative FASTCALL arity must be rejected");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::InvalidProgram { .. }
    ));

    let invalid_window = Program::new(
        Function::new(vec![Instruction::with_aux(Opcode::FastCall, 0, 0, 1, 1)], 1),
        vec![],
    );
    let error = Vm::new(Profile::ReproducibleStandard)
        .run(&invalid_window)
        .expect_err("out-of-range FASTCALL argument window must be rejected");
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::RegisterOutOfBounds {
            register: 2,
            max: 1
        }
    ));
}

#[test]
fn loadstring_hook_is_profile_gated() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::String(b"loadstring".to_vec())],
    );
    let settings = VmSettings {
        loadstring_compiler: Some(loadstring_program_returns_42),
        ..VmSettings::default()
    };

    let mut standard = Vm::with_settings(Profile::ReproducibleStandard, settings.clone());
    let standard_result = standard.run(&program).unwrap();
    assert_eq!(standard_result.kind(), rustia_vm::ValueKind::Function);

    let mut deterministic = Vm::with_settings(Profile::DeterministicSim, settings);
    let deterministic_result = deterministic.run(&program).unwrap();
    assert_eq!(deterministic_result.kind(), rustia_vm::ValueKind::Nil);
}

#[test]
fn loadstring_executes_host_compiled_dynamic_program() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"loadstring".to_vec()),
            Constant::String(b"return 42".to_vec()),
        ],
    );
    let settings = VmSettings {
        loadstring_compiler: Some(loadstring_program_returns_42),
        ..VmSettings::default()
    };

    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let result = vm.run(&program).unwrap();
    assert_eq!(result.as_number(), Some(42.0));
}

#[test]
fn conformance_top_level_coroutine_yield_is_gated() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::GetIndex, 0, 0, 1),
                Instruction::new(Opcode::LoadConst, 1, 0, 2),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"coroutine".to_vec()),
            Constant::String(b"yield".to_vec()),
            Constant::Number(7.0),
        ],
    );

    let mut default_vm = Vm::new(Profile::ReproducibleStandard);
    let error = default_vm.run(&program).unwrap_err();
    assert!(error
        .to_string()
        .contains("attempt to yield across metamethod/C-call boundary"));

    let settings = VmSettings {
        enable_conformance_top_level_yield: true,
        ..VmSettings::default()
    };
    let mut conformance_vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let value = conformance_vm.run(&program).unwrap();
    assert_eq!(value.as_number(), Some(7.0));
}

#[test]
fn decision_reset_requires_idle_execution_and_preserves_installed_host_functions() {
    assert_eq!(DECISION_RESET_ABI_VERSION, 1);
    let mut vm = Vm::new(Profile::DeterministicSim);
    let extension_id = vm
        .register_extension_op(ExtensionOp::new(
            "identity",
            1,
            OpCapability::Pure,
            first_arg,
        ))
        .unwrap();
    vm.install_host_function(HostFunctionOp::new(
        "try_reset",
        1,
        OpCapability::Pure,
        Arc::new(|vm, _| {
            let epoch = vm.decision_epoch();
            let error = vm
                .reset_decision()
                .expect_err("a host callback must not reset an active VM");
            assert!(matches!(error.kind, ErrorKind::DecisionResetWhileRunning));
            assert_eq!(vm.decision_epoch(), epoch);
            Ok(vec![Value::boolean(true)])
        }),
    ))
    .unwrap();
    let call = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::String(b"try_reset".to_vec())],
    );
    let extension_call = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::with_aux(Opcode::CallExtension, 1, 0, 1, i32::from(extension_id)),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        ),
        vec![Constant::Number(7.0)],
    );
    let retained_table = vm.empty_table();
    let retained = vm.retain_value(retained_table).unwrap();

    assert_eq!(vm.run(&call).unwrap().as_bool(), Some(true));
    assert_eq!(vm.run(&extension_call).unwrap().as_number(), Some(7.0));
    assert!(retained.belongs_to(&vm));
    assert!(retained.value(&vm).is_ok());
    let other_vm = Vm::new(Profile::DeterministicSim);
    assert!(matches!(
        retained.value(&other_vm).unwrap_err().kind,
        ErrorKind::RetainedValueFromDifferentVm
    ));
    assert_eq!(vm.decision_epoch(), 0);
    vm.reset_decision().unwrap();
    assert_eq!(vm.decision_epoch(), 1);
    assert!(!retained.belongs_to(&vm));
    assert!(matches!(
        retained.value(&vm).unwrap_err().kind,
        ErrorKind::StaleRetainedValue {
            retained_epoch: 0,
            current_epoch: 1
        }
    ));
    assert_eq!(vm.run(&call).unwrap().as_bool(), Some(true));
    assert_eq!(vm.run(&extension_call).unwrap().as_number(), Some(7.0));
}

#[test]
fn deterministic_f64_golden_exercises_vm_ops() {
    let cases = [
        (Opcode::Div, 0.0, 0.0, CANON_NAN_BITS),
        (Opcode::Sub, f64::INFINITY, f64::INFINITY, CANON_NAN_BITS),
        (Opcode::Div, 1.0, -0.0, f64::NEG_INFINITY.to_bits()),
        (Opcode::Mod, -3.0, 2.0, 1.0f64.to_bits()),
        (Opcode::Mod, -0.0, 3.0, (-0.0f64).to_bits()),
        (Opcode::FloorDiv, -3.0, 2.0, (-2.0f64).to_bits()),
        (Opcode::FloorDiv, 5.0, 0.0, f64::INFINITY.to_bits()),
        (Opcode::Pow, -2.0, 0.5, CANON_NAN_BITS),
    ];

    for (op, lhs, rhs, expected_bits) in cases {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::LoadConst, 1, 0, 1),
                    Instruction::new(op, 2, 0, 1),
                    Instruction::new(Opcode::Return, 2, 1, 0),
                ],
                3,
            ),
            vec![Constant::Number(lhs), Constant::Number(rhs)],
        );
        let mut vm = Vm::new(Profile::DeterministicSim);
        let result = vm.run(&program).unwrap();
        assert_eq!(result.as_number().unwrap().to_bits(), expected_bits);
    }
}

#[test]
fn compiler_and_vm_share_numeric_constant_folding_rules() {
    assert_eq!(
        binary(NumericOp::Mod, -3.0, 2.0).to_bits(),
        1.0f64.to_bits()
    );
    assert_eq!(
        binary(NumericOp::FloorDiv, -3.0, 2.0).to_bits(),
        (-2.0f64).to_bits()
    );
    assert_eq!(binary(NumericOp::Pow, -2.0, 0.5).to_bits(), CANON_NAN_BITS);
}

#[test]
fn fuel_exhaustion_is_structured() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::Number(1.0)],
    );
    let settings = VmSettings {
        initial_fuel: Some(1),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 1 }
    ));
}

#[test]
fn function_calls_share_the_run_fuel_budget_and_preserve_callee_pc() {
    let program = Program::with_functions(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::LoadFunction, 0, 0, 0, 0),
                Instruction::with_aux(Opcode::Call, 1, 0, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        ),
        vec![Constant::Number(1.0), Constant::Number(2.0)],
        vec![Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        )],
    );
    let settings = VmSettings {
        initial_fuel: Some(4),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 4 }
    ));
    assert_eq!(error.pc, Some(2));
}

#[test]
fn native_stdlib_aliases_charge_variable_work_fuel() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::GetIndex, 0, 0, 1),
                Instruction::new(Opcode::NewArray, 1, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 2),
                Instruction::new(Opcode::LoadConst, 3, 0, 2),
                Instruction::new(Opcode::SetIndex, 1, 2, 3),
                Instruction::new(Opcode::LoadConst, 2, 0, 3),
                Instruction::new(Opcode::LoadConst, 3, 0, 3),
                Instruction::new(Opcode::SetIndex, 1, 2, 3),
                Instruction::new(Opcode::LoadConst, 2, 0, 4),
                Instruction::new(Opcode::LoadConst, 3, 0, 4),
                Instruction::new(Opcode::SetIndex, 1, 2, 3),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            4,
        ),
        vec![
            Constant::String(b"table".to_vec()),
            Constant::String(b"concat".to_vec()),
            Constant::Number(1.0),
            Constant::Number(2.0),
            Constant::Number(3.0),
        ],
    );
    let settings = VmSettings {
        initial_fuel: Some(14),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 14 }
    ));
    assert_eq!(error.pc, Some(13));
}

#[test]
fn table_insert_opcode_charges_shift_work_fuel() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::NewArray, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 0),
                Instruction::new(Opcode::SetIndex, 0, 1, 2),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::LoadConst, 2, 0, 1),
                Instruction::new(Opcode::SetIndex, 0, 1, 2),
                Instruction::new(Opcode::LoadConst, 1, 0, 2),
                Instruction::new(Opcode::LoadConst, 2, 0, 2),
                Instruction::new(Opcode::SetIndex, 0, 1, 2),
                Instruction::new(Opcode::LoadConst, 1, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 3),
                Instruction::with_aux(Opcode::TableInsert, 3, 0, 1, 3),
                Instruction::new(Opcode::Return, 3, 1, 0),
            ],
            4,
        ),
        vec![
            Constant::Number(1.0),
            Constant::Number(2.0),
            Constant::Number(3.0),
            Constant::Number(9.0),
        ],
    );
    let settings = VmSettings {
        initial_fuel: Some(15),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 15 }
    ));
    assert_eq!(error.pc, Some(12));
}

#[test]
fn string_find_opcode_charges_text_and_pattern_fuel() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::StringFind, 2, 0, 1, 2),
                Instruction::new(Opcode::Return, 2, 1, 0),
            ],
            3,
        ),
        vec![
            Constant::String(b"abcdefgh".to_vec()),
            Constant::String(b"def".to_vec()),
        ],
    );

    let settings = VmSettings {
        initial_fuel: Some(13),
        ..VmSettings::default()
    };
    let mut starved = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let error = starved.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 13 }
    ));
    assert_eq!(error.pc, Some(2));

    let settings = VmSettings {
        initial_fuel: Some(15),
        ..VmSettings::default()
    };
    let mut enough = Vm::with_settings(Profile::ReproducibleStandard, settings);
    assert_eq!(enough.run(&program).unwrap().as_number(), Some(4.0));
}

#[test]
fn loadstring_charges_source_length_fuel_before_compiling() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"loadstring".to_vec()),
            Constant::String(b"return 42".to_vec()),
        ],
    );
    let settings = VmSettings {
        initial_fuel: Some(3),
        loadstring_compiler: Some(loadstring_compiler_should_not_run),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 3 }
    ));
    assert_eq!(error.pc, Some(2));
}

#[test]
fn configured_recursion_limit_is_capped_to_host_safe_maximum() {
    let settings = VmSettings {
        recursion_limit: usize::MAX,
        ..VmSettings::default()
    };
    let vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    assert_eq!(vm.settings().recursion_limit, MAX_RECURSION_LIMIT);
}

#[test]
fn function_metadata_rejects_parameter_windows_outside_registers() {
    let program = Program::new(
        Function::with_params(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1, 2),
        Vec::new(),
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::InvalidProgram { .. }
    ));
}

#[test]
fn nested_expand_aux_rejects_high_bits() {
    let program = Program::new(
        Function::new(
            vec![Instruction::with_aux(
                Opcode::ReturnCallExpand2Varargs,
                0,
                0,
                0,
                0x0100_0000,
            )],
            3,
        ),
        Vec::new(),
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::InvalidProgram { .. }
    ));
}

#[test]
fn deterministic_profile_rejects_effectful_extension_ops() {
    let mut vm = Vm::new(Profile::DeterministicSim);
    let error = vm
        .register_extension_op(ExtensionOp::new(
            "clock",
            1,
            OpCapability::NondeterministicEffect,
            first_arg,
        ))
        .unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::CapabilityRemoved { .. }
    ));
}

#[test]
fn standard_profile_accepts_pure_extension_ops_with_declared_fuel() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let id = vm
        .register_extension_op(ExtensionOp::new("clamp", 1, OpCapability::Pure, first_arg))
        .unwrap();
    assert_eq!(id, 0);
}

#[test]
fn extension_opcode_charges_declared_fuel() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let id = vm
        .register_extension_op(ExtensionOp::new("id", 3, OpCapability::Pure, first_arg))
        .unwrap();
    let program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::with_aux(Opcode::CallExtension, 1, 0, 1, i32::from(id)),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        ),
        vec![Constant::Number(7.0)],
    );
    let settings = VmSettings {
        initial_fuel: Some(3),
        ..VmSettings::default()
    };
    let mut starved = Vm::with_settings(Profile::ReproducibleStandard, settings);
    starved
        .register_extension_op(ExtensionOp::new("id", 3, OpCapability::Pure, first_arg))
        .unwrap();
    assert!(matches!(
        starved.run(&program).unwrap_err().kind,
        rustia_vm::ErrorKind::FuelExhausted { limit: 3 }
    ));

    let settings = VmSettings {
        initial_fuel: Some(5),
        ..VmSettings::default()
    };
    let mut enough = Vm::with_settings(Profile::ReproducibleStandard, settings);
    enough
        .register_extension_op(ExtensionOp::new("id", 3, OpCapability::Pure, first_arg))
        .unwrap();
    assert_eq!(enough.run(&program).unwrap().as_number(), Some(7.0));
}

#[test]
fn extension_opcode_rejects_ids_above_u16() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_extension_op(ExtensionOp::new("id", 1, OpCapability::Pure, first_arg))
        .unwrap();
    let program = Program::new(
        Function::new(
            vec![Instruction::with_aux(
                Opcode::CallExtension,
                0,
                0,
                0,
                i32::from(u16::MAX) + 1,
            )],
            1,
        ),
        Vec::new(),
    );
    let error = vm.run(&program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::InvalidProgram {
            reason: "extension op id out of range"
        }
    ));
}

#[test]
fn assert_opcode_returns_truthy_value_and_errors_on_false() {
    let ok_program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Assert, 1, 0, 0),
                Instruction::new(Opcode::Return, 1, 1, 0),
            ],
            2,
        ),
        vec![Constant::Number(1.0)],
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    assert_eq!(vm.run(&ok_program).unwrap().as_number(), Some(1.0));

    let fail_program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::Assert, 1, 0, 0),
            ],
            2,
        ),
        vec![Constant::Boolean(false)],
    );
    let error = vm.run(&fail_program).unwrap_err();
    assert!(matches!(
        error.kind,
        rustia_vm::ErrorKind::AssertionFailed { .. }
    ));
}

#[test]
fn concat_opcode_joins_string_values() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::new(Opcode::LoadConst, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::Concat, 2, 0, 1),
                Instruction::new(Opcode::Return, 2, 1, 0),
            ],
            3,
        ),
        vec![
            Constant::String(b"ab".to_vec()),
            Constant::String(b"cd".to_vec()),
        ],
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let result = vm.run(&program).unwrap();
    assert_eq!(vm.string_value(result).unwrap().as_deref(), Some("abcd"));
}

#[test]
fn equality_opcodes_follow_luau_number_semantics() {
    let cases = [
        (Opcode::Equal, -0.0, 0.0, true),
        (Opcode::Equal, f64::NAN, f64::NAN, false),
        (Opcode::NotEqual, f64::NAN, f64::NAN, true),
        (Opcode::NotEqual, 1.0, 2.0, true),
    ];

    for (op, lhs, rhs, expected) in cases {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::LoadConst, 1, 0, 1),
                    Instruction::new(op, 2, 0, 1),
                    Instruction::new(Opcode::Return, 2, 1, 0),
                ],
                3,
            ),
            vec![Constant::Number(lhs), Constant::Number(rhs)],
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_bool(), Some(expected));
    }
}

#[test]
fn relational_opcodes_compare_numbers() {
    let cases = [
        (Opcode::Less, 1.0, 2.0, true),
        (Opcode::LessEqual, 2.0, 2.0, true),
        (Opcode::Greater, 1.0, 2.0, false),
        (Opcode::GreaterEqual, 1.0, 2.0, false),
        (Opcode::Less, -0.0, 0.0, false),
        (Opcode::LessEqual, -0.0, 0.0, true),
        (Opcode::Less, f64::NEG_INFINITY, f64::INFINITY, true),
        (Opcode::GreaterEqual, f64::INFINITY, f64::INFINITY, true),
        (Opcode::Less, f64::NAN, 1.0, false),
        (Opcode::Greater, 1.0, f64::NAN, false),
    ];

    for (op, lhs, rhs, expected) in cases {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::LoadConst, 1, 0, 1),
                    Instruction::new(op, 2, 0, 1),
                    Instruction::new(Opcode::Return, 2, 1, 0),
                ],
                3,
            ),
            vec![Constant::Number(lhs), Constant::Number(rhs)],
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_bool(), Some(expected));
    }
}

#[test]
fn relational_opcodes_compare_strings_by_bytes() {
    let cases = [
        (Opcode::Less, "a", "b", true),
        (Opcode::LessEqual, "a", "a", true),
        (Opcode::Greater, "a", "", true),
        (Opcode::GreaterEqual, "abc", "abd", false),
    ];

    for (op, lhs, rhs, expected) in cases {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::LoadConst, 1, 0, 1),
                    Instruction::new(op, 2, 0, 1),
                    Instruction::new(Opcode::Return, 2, 1, 0),
                ],
                3,
            ),
            vec![
                Constant::String(lhs.as_bytes().to_vec()),
                Constant::String(rhs.as_bytes().to_vec()),
            ],
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_bool(), Some(expected));
    }
}

#[test]
fn not_opcode_uses_luau_truthiness() {
    let cases = [
        (Constant::Nil, true),
        (Constant::Boolean(false), true),
        (Constant::Boolean(true), false),
        (Constant::Number(0.0), false),
    ];

    for (constant, expected) in cases {
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::new(Opcode::LoadConst, 0, 0, 0),
                    Instruction::new(Opcode::Not, 1, 0, 0),
                    Instruction::new(Opcode::Return, 1, 1, 0),
                ],
                2,
            ),
            vec![constant],
        );
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        assert_eq!(vm.run(&program).unwrap().as_bool(), Some(expected));
    }
}

#[test]
fn deterministic_gc_instruction_count_persists_across_runs_and_is_profile_specific() {
    let settings = VmSettings {
        deterministic_gc_interval: NonZeroU64::new(2).unwrap(),
        ..VmSettings::default()
    };
    let program = Program::new(
        Function::new(vec![Instruction::new(Opcode::Return, 0, 1, 0)], 1),
        vec![],
    );
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings.clone());
    vm.run(&program).unwrap();
    assert_eq!(vm.deterministic_collection_count(), 0);
    vm.run(&program).unwrap();
    assert_eq!(vm.deterministic_collection_count(), 1);

    let mut standard = Vm::with_settings(Profile::ReproducibleStandard, settings);
    standard.run(&program).unwrap();
    standard.run(&program).unwrap();
    assert_eq!(standard.deterministic_collection_count(), 0);
}

#[test]
fn clock_providers_feed_exact_os_time_and_os_clock_values() {
    let settings = VmSettings {
        services: VmServices {
            wall_clock: Some(ClockProvider::new(|| 1_234_567.5, true)),
            monotonic_clock: Some(ClockProvider::new(|| 42.25, true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);

    let time = vm.run(&os_member_call_program(b"time")).unwrap();
    assert_eq!(time.as_number(), Some(1_234_567.5));
    let clock = vm.run(&os_member_call_program(b"clock")).unwrap();
    assert_eq!(clock.as_number(), Some(42.25));
}

#[test]
fn every_host_controlled_callback_panic_is_contained_poisoned_and_resettable() {
    let settings = VmSettings {
        services: VmServices {
            stdout: Some(StdoutSink::new(|_| panic!("stdout panic"))),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut stdout_vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    assert_host_panic_poison_and_reset(&mut stdout_vm, &print_call_program(&[]), "service::stdout");

    let settings = VmSettings {
        services: VmServices {
            wall_clock: Some(ClockProvider::new(|| panic!("wall clock panic"), true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut wall_clock_vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    assert_host_panic_poison_and_reset(
        &mut wall_clock_vm,
        &os_member_call_program(b"time"),
        "service::wall_clock",
    );

    let settings = VmSettings {
        services: VmServices {
            monotonic_clock: Some(ClockProvider::new(|| panic!("monotonic clock panic"), true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut monotonic_clock_vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    assert_host_panic_poison_and_reset(
        &mut monotonic_clock_vm,
        &os_member_call_program(b"clock"),
        "service::monotonic_clock",
    );

    let loadstring = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::String(b"loadstring".to_vec()),
            Constant::String(b"return 1".to_vec()),
        ],
    );
    let settings = VmSettings {
        loadstring_compiler: Some(panicking_loadstring_compiler),
        ..VmSettings::default()
    };
    let mut loadstring_vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    assert_host_panic_poison_and_reset(
        &mut loadstring_vm,
        &loadstring,
        "service::loadstring_compiler",
    );

    let mut extension_vm = Vm::new(Profile::ReproducibleStandard);
    let extension_id = extension_vm
        .register_extension_op(ExtensionOp::new(
            "panic",
            1,
            OpCapability::Pure,
            panicking_extension,
        ))
        .unwrap();
    let extension = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::CallExtension, 0, 0, 0, i32::from(extension_id)),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        Vec::new(),
    );
    assert_host_panic_poison_and_reset(&mut extension_vm, &extension, "extension::panic");
}

#[test]
fn clock_provider_nan_is_canonicalized_and_negative_zero_passes_through() {
    let noncanonical_nan = f64::from_bits(0x7ff0_0000_0000_0001);
    assert!(noncanonical_nan.is_nan());
    let settings = VmSettings {
        services: VmServices {
            wall_clock: Some(ClockProvider::new(move || noncanonical_nan, true)),
            monotonic_clock: Some(ClockProvider::new(move || noncanonical_nan, true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    for member in [b"time".as_slice(), b"clock"] {
        let value = vm.run(&os_member_call_program(member)).unwrap();
        assert_eq!(value.as_number().unwrap().to_bits(), CANON_NAN_BITS);
    }

    let settings = VmSettings {
        services: VmServices {
            wall_clock: Some(ClockProvider::new(|| -0.0, true)),
            monotonic_clock: Some(ClockProvider::new(|| -0.0, true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    for member in [b"time".as_slice(), b"clock"] {
        let value = vm
            .run(&os_member_call_program(member))
            .unwrap()
            .as_number()
            .unwrap();
        assert_eq!(value.to_bits(), (-0.0f64).to_bits());
        assert_eq!(1.0 / value, f64::NEG_INFINITY);
    }
}

#[test]
fn entropy_provider_selects_the_initial_random_sequence() {
    let sequence = |seed| {
        let settings = VmSettings {
            services: VmServices {
                entropy: Some(EntropyProvider::new(move || seed, true)),
                ..VmServices::default()
            },
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        [
            vm.run(&math_random_program()).unwrap().as_number().unwrap(),
            vm.run(&math_random_program()).unwrap().as_number().unwrap(),
        ]
    };

    assert_eq!(sequence(42), sequence(42));
    assert_ne!(sequence(42), sequence(43));
    assert_eq!(sequence(0), {
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        [
            vm.run(&math_random_program()).unwrap().as_number().unwrap(),
            vm.run(&math_random_program()).unwrap().as_number().unwrap(),
        ]
    });
}

#[test]
fn deterministic_sim_silently_drops_nondeterministic_entropy() {
    let settings = VmSettings {
        services: VmServices {
            entropy: Some(EntropyProvider::new(
                || panic!("dropped entropy provider must not be read"),
                false,
            )),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
    let seeded = vm.run(&math_random_program()).unwrap().as_number().unwrap();

    let mut baseline = Vm::new(Profile::DeterministicSim);
    assert_eq!(
        seeded,
        baseline
            .run(&math_random_program())
            .unwrap()
            .as_number()
            .unwrap()
    );
}

#[test]
fn print_composes_tab_joined_arguments_with_a_trailing_newline() {
    let captured = Rc::new(RefCell::new(String::new()));
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        settings_with_stdout(&captured),
    );

    let result = vm
        .run(&print_call_program(&[
            Constant::String(b"a".to_vec()),
            Constant::String(b"b".to_vec()),
            Constant::Number(3.0),
        ]))
        .unwrap();
    assert_eq!(result.kind(), ValueKind::Nil);
    assert_eq!(captured.borrow().as_str(), "a\tb\t3\n");
}

#[test]
fn print_with_no_arguments_emits_a_lone_newline() {
    let captured = Rc::new(RefCell::new(String::new()));
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        settings_with_stdout(&captured),
    );

    vm.run(&print_call_program(&[])).unwrap();
    assert_eq!(captured.borrow().as_str(), "\n");
}

#[test]
fn print_honors_the_tostring_metamethod() {
    let metamethod = Function::new(
        vec![
            Instruction::new(Opcode::LoadConst, 0, 0, 3),
            Instruction::new(Opcode::Return, 0, 1, 0),
        ],
        1,
    );
    let program = Program::with_functions(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 1),
                Instruction::new(Opcode::NewArray, 1, 0, 0),
                Instruction::new(Opcode::NewArray, 2, 0, 0),
                Instruction::with_aux(Opcode::LoadFunction, 3, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 4, 0, 0),
                Instruction::with_aux(Opcode::SetTableKs, 2, 3, 4, 0),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 2),
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 2),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            5,
        ),
        vec![
            Constant::String(b"__tostring".to_vec()),
            Constant::String(b"setmetatable".to_vec()),
            Constant::String(b"print".to_vec()),
            Constant::String(b"custom".to_vec()),
        ],
        vec![metamethod],
    );
    let captured = Rc::new(RefCell::new(String::new()));
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        settings_with_stdout(&captured),
    );

    vm.run(&program).unwrap();
    assert_eq!(captured.borrow().as_str(), "custom\n");
}

#[test]
fn print_without_a_sink_is_swallowed_and_returns_nil() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let result = vm
        .run(&print_call_program(&[Constant::String(
            b"ignored".to_vec(),
        )]))
        .unwrap();
    assert_eq!(result.kind(), ValueKind::Nil);
}

#[test]
fn deterministic_sim_silently_drops_nondeterministic_clock_providers() {
    let settings = VmSettings {
        services: VmServices {
            wall_clock: Some(ClockProvider::new(
                || panic!("dropped wall clock must not be read"),
                false,
            )),
            monotonic_clock: Some(ClockProvider::new(
                || panic!("dropped monotonic clock must not be read"),
                false,
            )),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);

    assert_eq!(global_kind_in(&mut vm, b"os"), ValueKind::Nil);
}

#[test]
fn deterministic_sim_registers_os_iff_a_clock_provider_is_plugged() {
    let mut sealed = Vm::new(Profile::DeterministicSim);
    assert_eq!(global_kind_in(&mut sealed, b"os"), ValueKind::Nil);

    let settings = VmSettings {
        services: VmServices {
            wall_clock: Some(ClockProvider::new(|| 1_700_000_042.0, true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
    assert_eq!(global_kind_in(&mut vm, b"os"), ValueKind::Table);
    for member in [b"time".as_slice(), b"clock", b"date", b"difftime"] {
        assert_eq!(
            vm.run(&os_member_program(member)).unwrap().kind(),
            ValueKind::Function,
            "os member {} should be registered",
            String::from_utf8_lossy(member)
        );
    }
    assert_eq!(global_kind_in(&mut vm, b"collectgarbage"), ValueKind::Nil);
    assert_eq!(global_kind_in(&mut vm, b"gcinfo"), ValueKind::Nil);
}

#[test]
fn cloned_settings_share_stdout_sinks_and_clock_providers() {
    let writes: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
    let reads = Rc::new(RefCell::new(0_u64));
    let settings = VmSettings {
        services: VmServices {
            stdout: Some(StdoutSink::new({
                let writes = Rc::clone(&writes);
                move |text| writes.borrow_mut().push(text.to_owned())
            })),
            wall_clock: Some(ClockProvider::new(
                {
                    let reads = Rc::clone(&reads);
                    move || {
                        *reads.borrow_mut() += 1;
                        1.5
                    }
                },
                true,
            )),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let cloned = settings.clone();

    let mut first = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let mut second = Vm::with_settings(Profile::ReproducibleStandard, cloned);
    first
        .run(&print_call_program(&[Constant::String(b"one".to_vec())]))
        .unwrap();
    second
        .run(&print_call_program(&[Constant::String(b"two".to_vec())]))
        .unwrap();
    first.run(&os_member_call_program(b"time")).unwrap();
    second.run(&os_member_call_program(b"time")).unwrap();

    assert_eq!(
        writes.borrow().as_slice(),
        ["one\n".to_owned(), "two\n".to_owned()]
    );
    assert_eq!(*reads.borrow(), 2);
}

#[test]
fn logical_os_clock_reads_identically_across_fresh_vms() {
    let program = os_member_call_program(b"clock");
    let first = Vm::new(Profile::ReproducibleStandard)
        .run(&program)
        .unwrap()
        .as_number()
        .unwrap();
    let second = Vm::new(Profile::ReproducibleStandard)
        .run(&program)
        .unwrap()
        .as_number()
        .unwrap();

    assert!(
        first > 0.0,
        "the logical clock measures executed instructions"
    );
    assert_eq!(first.to_bits(), second.to_bits());
}

#[test]
fn logical_os_clock_is_non_decreasing_within_a_run() {
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 1),
                Instruction::new(Opcode::GetIndex, 0, 0, 1),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 0),
                Instruction::with_aux(Opcode::GetGlobal, 1, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 1),
                Instruction::new(Opcode::GetIndex, 1, 1, 2),
                Instruction::with_aux(Opcode::Call, 1, 1, 2, 0),
                Instruction::new(Opcode::LessEqual, 2, 0, 1),
                Instruction::new(Opcode::Return, 2, 1, 0),
            ],
            3,
        ),
        vec![
            Constant::String(b"os".to_vec()),
            Constant::String(b"clock".to_vec()),
        ],
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    assert_eq!(vm.run(&program).unwrap().as_bool(), Some(true));
}

#[test]
fn plugged_monotonic_clock_overrides_the_logical_default() {
    let settings = VmSettings {
        services: VmServices {
            monotonic_clock: Some(ClockProvider::new(|| 7.5, true)),
            ..VmServices::default()
        },
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);

    let clock = vm.run(&os_member_call_program(b"clock")).unwrap();
    assert_eq!(clock.as_number(), Some(7.5));
    let time = vm.run(&os_member_call_program(b"time")).unwrap();
    assert_eq!(time.as_number(), Some(1_700_000_000.0));
}
