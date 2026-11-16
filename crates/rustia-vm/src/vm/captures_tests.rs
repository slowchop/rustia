use super::*;

fn fixture(shared: bool, callback: bool) -> (Vm, RetainedValue) {
    let mut code = if callback {
        vec![
            Instruction::with_aux(Opcode::GetGlobal, 1, 0, 0, 1),
            Instruction::new(Opcode::Call, 1, 1, 0),
        ]
    } else {
        Vec::new()
    };
    code.extend([
        Instruction::with_aux(Opcode::GetUpvalue, 0, 0, 0, 0),
        Instruction::new(Opcode::Return, 0, 1, 0),
    ]);
    let mut main = Function::new(
        vec![
            Instruction::with_aux(Opcode::LoadConst, 0, 0, 0, 0),
            Instruction::with_aux(Opcode::LoadFunction, 1, 0, 0, 0),
            Instruction::new(Opcode::Return, 1, 1, 0),
        ],
        2,
    );
    main.captured_registers = vec![0];
    let program = Program::with_functions(
        main,
        vec![Constant::Number(7.0), Constant::String(b"edit".to_vec())],
        vec![Function::with_params_and_upvalues(
            code,
            2,
            0,
            vec![if shared {
                UpvalueDescriptor::ParentLocal(0)
            } else {
                UpvalueDescriptor::ParentLocalValue(0)
            }],
            Vec::new(),
        )],
    )
    .with_source_name(Some("legacy.luau".into()));
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let raw = vm.run(&program).unwrap();
    let function = vm.retain_value(raw).unwrap();
    (vm, function)
}

fn current(vm: &mut Vm, function: &RetainedValue) -> f64 {
    vm.retained_function_capture(function, 0)
        .unwrap()
        .unwrap()
        .value
        .value(vm)
        .unwrap()
        .as_number()
        .unwrap()
}

#[test]
fn legacy_names_are_absent_and_capture_and_table_edits_roll_back() {
    for shared in [false, true] {
        let (mut vm, function) = fixture(shared, false);
        let capture = vm.retained_function_capture(&function, 0).unwrap().unwrap();
        assert!(capture.name.is_none());
        assert_eq!(
            capture.storage,
            if shared {
                CaptureStorage::SharedCell
            } else {
                CaptureStorage::Direct
            }
        );
        assert_eq!(
            vm.retained_function_info(&function)
                .unwrap()
                .source
                .as_deref(),
            Some("legacy.luau")
        );
        let forty = vm.retain_value(Value::number(40.0)).unwrap();
        let table = vm.create_retained_table().unwrap();
        let metatable = vm.create_retained_table().unwrap();
        vm.set_retained_table_metatable(&table, Some(&metatable))
            .unwrap();
        vm.set_retained_table_readonly(&table, true).unwrap();
        vm.begin_guest_transaction().unwrap();
        vm.set_retained_function_capture(&function, 0, &forty)
            .unwrap();
        vm.set_retained_table_readonly(&table, false).unwrap();
        vm.set_retained_table_metatable(&table, None).unwrap();
        assert_eq!(current(&mut vm, &function), 40.0);
        vm.rollback_guest_transaction();
        assert_eq!(current(&mut vm, &function), 7.0);
        assert!(vm.retained_table_is_readonly(&table).unwrap());
        let restored = vm.retained_table_metatable(&table).unwrap().unwrap();
        assert!(vm
            .raw_equal_values(restored.value(&vm).unwrap(), metatable.value(&vm).unwrap())
            .unwrap());
    }
}

#[test]
fn active_capture_edits_are_rejected_before_mutating_the_running_closure() {
    struct Dispatcher {
        function: RetainedValue,
        replacement: RetainedValue,
        rejected: bool,
    }
    impl VmHostDispatcher for Dispatcher {
        fn invoke(
            &mut self,
            vm: &mut Vm,
            _: &(dyn std::any::Any + Send + Sync),
            _: &[Value],
        ) -> Result<Vec<Value>> {
            let error = vm
                .set_retained_function_capture(&self.function, 0, &self.replacement)
                .unwrap_err();
            self.rejected = error.to_string().contains("idle VM");
            Ok(Vec::new())
        }
    }
    let (mut vm, function) = fixture(false, true);
    let replacement = vm.retain_value(Value::number(40.0)).unwrap();
    let mut dispatcher = Dispatcher {
        function: function.clone(),
        replacement: replacement.clone(),
        rejected: false,
    };
    let callback = vm
        .register_host_function(HostFunctionOp::dispatched(
            "edit",
            1,
            OpCapability::NondeterministicEffect,
            Arc::new(()),
        ))
        .unwrap();
    vm.set_global_value("edit", callback).unwrap();
    let mut execution = vm.begin_execution();
    let result = vm
        .call_retained_function_in(
            function.value(&vm).unwrap(),
            &[],
            &mut execution,
            &mut dispatcher,
        )
        .unwrap();
    assert!(dispatcher.rejected);
    assert_eq!(result[0].as_number(), Some(7.0));
    assert_eq!(current(&mut vm, &function), 7.0);
    vm.set_retained_function_capture(&function, 0, &replacement)
        .unwrap();
    assert_eq!(current(&mut vm, &function), 40.0);
}

#[test]
fn quota_and_hardened_policy_fail_before_capture_or_table_mutation() {
    let (mut vm, function) = fixture(false, false);
    let replacement = vm.retain_value(Value::number(40.0)).unwrap();
    let table = vm.create_retained_table().unwrap();
    vm.set_retained_table_readonly(&table, true).unwrap();
    vm.settings.memory_limit_bytes = Some(0);
    assert!(vm
        .set_retained_function_capture(&function, 0, &replacement)
        .is_err());
    assert!(vm.set_retained_table_readonly(&table, false).is_err());
    assert!(vm.retained_function_capture(&function, 0).is_err());
    vm.settings.memory_limit_bytes = None;
    assert_eq!(current(&mut vm, &function), 7.0);
    assert!(vm.retained_table_is_readonly(&table).unwrap());
    vm.settings.guest_surface = GuestSurface::HardenedLeaf;
    assert!(matches!(
        vm.set_retained_function_capture(&function, 0, &replacement)
            .unwrap_err()
            .kind,
        ErrorKind::HardenedLeafViolation { .. }
    ));
    assert!(matches!(
        vm.set_retained_table_readonly(&table, false)
            .unwrap_err()
            .kind,
        ErrorKind::HardenedLeafViolation { .. }
    ));
    assert!(matches!(
        vm.set_retained_table_metatable(&table, None)
            .unwrap_err()
            .kind,
        ErrorKind::HardenedLeafViolation { .. }
    ));
    assert_eq!(current(&mut vm, &function), 7.0);
    assert!(vm.retained_table_is_readonly(&table).unwrap());
}

#[test]
fn reclaimed_raw_function_handles_cannot_be_reintroduced_by_inspection() {
    let (mut vm, function) = fixture(false, false);
    let raw = function.value(&vm).unwrap();
    drop(function);
    vm.collect_garbage();
    let invalid = vm.retain_value(raw).unwrap();
    assert!(vm.retained_function_info(&invalid).is_err());
    assert!(vm.retained_function_capture(&invalid, 0).is_err());
}

#[test]
fn rollback_reallocation_cannot_make_a_stale_root_valid() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.begin_guest_transaction().unwrap();
    let stale = vm.create_retained_table().unwrap();
    let retired = stale.value(&vm).unwrap();
    vm.rollback_guest_transaction();
    let replacement = vm.create_retained_table().unwrap();
    assert_eq!(
        retired.as_table(),
        replacement.value(&vm).unwrap().as_table(),
        "exercise actual raw slot reuse rather than just a stale epoch"
    );
    assert!(stale.value(&vm).is_err());
    assert!(vm.retained_table_next(&stale, None).is_err());
    assert!(vm.retained_table_is_readonly(&stale).is_err());
    assert!(vm
        .set_retained_table_metatable(&replacement, Some(&stale))
        .is_err());
    assert!(vm.retained_table_metatable(&replacement).unwrap().is_none());
}
