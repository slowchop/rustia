use super::*;

fn read_program() -> Program {
    Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            1,
        ),
        vec![Constant::String(b"answer".to_vec())],
    )
    .with_source_name(Some("nested/module.luau".into()))
}

#[test]
fn creation_does_not_execute_and_retains_exact_environment_and_source() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let env = vm.create_retained_table().unwrap();
    let key = vm.create_retained_string_bytes(b"answer").unwrap();
    vm.raw_table_set_value(
        env.value(&vm).unwrap(),
        key.value(&vm).unwrap(),
        Value::number(42.0),
    )
    .unwrap();
    vm.set_retained_table_readonly(&env, true).unwrap();
    let executed = vm.executed_instructions();
    let entry = vm.retain_program_entry(read_program(), &env).unwrap();
    assert_eq!(vm.executed_instructions(), executed);
    let info = vm.retained_function_info(&entry).unwrap();
    assert_eq!(info.kind, InspectedFunctionKind::Script);
    assert_eq!(info.source.as_deref(), Some("nested/module.luau"));
    let actual_env = vm.retained_function_environment(&entry).unwrap();
    assert!(vm
        .raw_equal_values(actual_env.value(&vm).unwrap(), env.value(&vm).unwrap())
        .unwrap());
    drop(actual_env);
    drop(env);
    drop(key);
    vm.collect_garbage();
    let result = vm
        .call_retained_function(entry.value(&vm).unwrap(), &[])
        .unwrap();
    assert_eq!(result[0].as_number(), Some(42.0));
}

#[test]
fn arbitrary_main_parameters_and_execution_validation_are_preserved() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let env = vm.create_retained_table().unwrap();
    let program = Program::new(
        Function::with_params(vec![Instruction::new(Opcode::Return, 0, 1, 0)], 1, 1),
        Vec::new(),
    );
    let entry = vm.retain_program_entry(program, &env).unwrap();
    assert_eq!(
        vm.call_retained_function(entry.value(&vm).unwrap(), &[Value::number(23.0)])
            .unwrap()[0]
            .as_number(),
        Some(23.0)
    );
    for main in [
        Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 0),
        Function::new(
            vec![Instruction::with_aux(Opcode::LoadConst, 0, 0, 0, 99)],
            1,
        ),
    ] {
        let program = Program::new(main, Vec::new());
        let expected = vm.run(&program).expect_err("invalid ordinary program");
        let entry = vm.retain_program_entry(program, &env).unwrap();
        let actual = vm
            .call_retained_function(entry.value(&vm).unwrap(), &[])
            .expect_err("invalid retained program");
        assert_eq!(format!("{:?}", actual.kind), format!("{:?}", expected.kind));
    }
}

#[test]
fn foreign_stale_non_table_and_reclaimed_environments_are_rejected() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let mut other = Vm::new(Profile::ReproducibleStandard);
    let foreign = other.create_retained_table().unwrap();
    assert!(vm.retain_program_entry(read_program(), &foreign).is_err());
    let number = vm.retain_value(Value::number(1.0)).unwrap();
    assert!(matches!(
        vm.retain_program_entry(read_program(), &number)
            .map(|_| ())
            .expect_err("rejected input")
            .kind,
        ErrorKind::TypeError { .. }
    ));
    let env = vm.create_retained_table().unwrap();
    let raw = env.value(&vm).unwrap();
    drop(env);
    vm.collect_garbage();
    let reclaimed = vm.retain_value(raw).unwrap();
    assert!(vm.retain_program_entry(read_program(), &reclaimed).is_err());
    let stale = vm.create_retained_table().unwrap();
    vm.reset_decision().unwrap();
    assert!(vm.retain_program_entry(read_program(), &stale).is_err());
}

#[test]
fn rollback_invalidates_only_new_entries_and_environments() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let env = vm.create_retained_table().unwrap();
    let previous = vm.retain_program_entry(read_program(), &env).unwrap();
    vm.begin_guest_transaction().unwrap();
    let discarded_env = vm.create_retained_table().unwrap();
    let discarded = vm
        .retain_program_entry(read_program(), &discarded_env)
        .unwrap();
    vm.rollback_guest_transaction();
    assert!(discarded.value(&vm).is_err());
    assert!(vm
        .retain_program_entry(read_program(), &discarded_env)
        .is_err());
    let replacement = vm.retain_program_entry(read_program(), &env).unwrap();
    assert!(discarded.value(&vm).is_err());
    assert!(replacement.value(&vm).is_ok());
    assert!(previous.value(&vm).is_ok());
    assert!(vm
        .call_retained_function(previous.value(&vm).unwrap(), &[])
        .unwrap()[0]
        .is_nil());
}

#[test]
fn quota_hardened_and_poison_guards_precede_publication() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let env = vm.create_retained_table().unwrap();
    let instructions = vm.executed_instructions();
    let roots = vm.external_roots.borrow().values.len();
    vm.settings.memory_limit_bytes = Some(0);
    assert!(matches!(
        vm.retain_program_entry(read_program(), &env)
            .map(|_| ())
            .expect_err("rejected input")
            .kind,
        ErrorKind::OutOfMemory
    ));
    assert_eq!(vm.external_roots.borrow().values.len(), roots);
    assert_eq!(vm.executed_instructions(), instructions);
    vm.settings.memory_limit_bytes = None;
    vm.settings.guest_surface = GuestSurface::HardenedLeaf;
    assert!(matches!(
        vm.retain_program_entry(read_program(), &env)
            .map(|_| ())
            .expect_err("rejected input")
            .kind,
        ErrorKind::HardenedLeafViolation { .. }
    ));
    vm.settings.guest_surface = GuestSurface::Standard;
    vm.poisoned = true;
    assert!(vm.retain_program_entry(read_program(), &env).is_err());
    assert_eq!(vm.external_roots.borrow().values.len(), roots);
}

#[test]
fn admission_includes_the_function_and_root_not_only_its_program() {
    let mut reference = Vm::new(Profile::ReproducibleStandard);
    let env = reference.create_retained_table().unwrap();
    let before = reference.memory_stats().live_bytes;
    let _entry = reference
        .retain_program_entry(read_program(), &env)
        .unwrap();
    let delta = reference.memory_stats().live_bytes - before;
    let mut accepted = false;
    for allowance in [delta.saturating_sub(1), delta, delta + 4096] {
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        let env = vm.create_retained_table().unwrap();
        let limit = vm.memory_stats().live_bytes + allowance;
        vm.settings.memory_limit_bytes = Some(limit);
        let roots = vm.external_roots.borrow().values.len();
        match vm.retain_program_entry(read_program(), &env) {
            Ok(entry) => {
                accepted = true;
                assert!(vm.memory_stats().live_bytes <= limit);
                assert!(entry.value(&vm).is_ok());
            }
            Err(error) => {
                assert!(matches!(error.kind, ErrorKind::OutOfMemory));
                assert_eq!(vm.external_roots.borrow().values.len(), roots);
            }
        }
        assert_eq!(vm.executed_instructions(), 0);
    }
    assert!(accepted, "the roomy boundary must admit a real callable");
}
