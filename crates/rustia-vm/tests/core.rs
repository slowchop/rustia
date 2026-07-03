use rustia_vm::number::{binary, NumericOp, CANON_NAN_BITS};
use rustia_vm::{
    Constant, ExtensionOp, Function, Instruction, OpCapability, Opcode, Profile, Program, Value,
    ValueKind, Vm, VmSettings, MAX_RECURSION_LIMIT,
};
use std::num::NonZeroU64;

fn first_arg(args: &[Value]) -> rustia_vm::Result<Value> {
    Ok(args.first().copied().unwrap_or_else(Value::nil))
}

fn loadstring_program_returns_42(
    _source: &[u8],
    _source_name: Option<&[u8]>,
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
) -> std::result::Result<Program, String> {
    panic!("loadstring compiler should not run after fuel exhaustion")
}

fn global_kind(profile: Profile, name: &[u8]) -> ValueKind {
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
    Vm::new(profile).run(&program).unwrap().kind()
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
    for profile in [Profile::LuauCompat, Profile::ReproducibleStandard] {
        assert_eq!(global_kind(profile, b"collectgarbage"), ValueKind::Function);
        assert_eq!(global_kind(profile, b"gcinfo"), ValueKind::Function);
        assert_eq!(global_kind(profile, b"os"), ValueKind::Table);
    }

    for name in [b"collectgarbage".as_slice(), b"gcinfo", b"os"] {
        assert_eq!(global_kind(Profile::DeterministicSim, name), ValueKind::Nil);
    }
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
