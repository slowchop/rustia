// Correctness regressions for the isolated continuation candidate.
use super::*;

#[test]
fn borrowed_stateless_host_call_never_installs_a_loaded_program() {
    for hardened in [false, true] {
        let mut settings = VmSettings {
            initial_fuel: Some(10_000),
            memory_limit_bytes: Some(1024 * 1024),
            ..VmSettings::default()
        };
        if hardened {
            settings.guest_surface = GuestSurface::HardenedLeaf;
        }
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
        vm.install_host_function(HostFunctionOp::dispatched(
            "probe",
            1,
            OpCapability::Pure,
            Arc::new(()),
        ))
        .unwrap();
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                    Instruction::with_aux(Opcode::Call, 1, 0, 0, 0),
                    Instruction::new(Opcode::Return, 1, 1, 0),
                ],
                2,
            ),
            vec![Constant::String(b"probe".to_vec())],
        );
        assert!(vm.dynamic_programs.slots.is_empty());
        let mut calls = 0;
        let mut host = |vm: &mut Vm, _: &(dyn std::any::Any + Send + Sync), _: &[Value]| {
            calls += 1;
            assert!(vm.dynamic_programs.slots.is_empty(), "hardened={hardened}");
            let _ = vm.memory_stats();
            let mut exact = MemoryUsage::default();
            add_vm_cached_control_memory(vm, &mut exact);
            assert_eq!(vm.control_live_bytes, exact.live_bytes);
            Ok(vec![Value::number(42.0)])
        };
        let mut execution = vm.begin_transactional_execution_with_fuel(Some(10_000));
        let returned = if hardened {
            let entry = HardenedLeafProgram::new(program).unwrap();
            vm.run_hardened_leaf_entry_in(&entry, &mut execution, &mut host)
        } else {
            vm.run_stateless_entry_in(&program, &mut execution, &mut host)
        }
        .unwrap();
        assert_eq!(returned.as_number(), Some(42.0));
        assert_eq!(calls, 1);
        assert!(vm.dynamic_programs.slots.is_empty());
        assert!(vm.active_coroutine_frames.is_empty());
        assert!(vm.call_stack.is_empty());
    }
}

#[test]
fn callback_depth_rejection_precedes_new_continuation_quota_admission() {
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            recursion_limit: 1,
            ..Default::default()
        },
    );
    vm.install_host_function(HostFunctionOp::new(
        "callback",
        1,
        OpCapability::Pure,
        Arc::new(|_, _| Ok(vec![Value::nil()])),
    ))
    .unwrap();
    let function = vm.get_global_value("callback").unwrap();
    let text = vm.string("x");
    let object = vm.create_retained_table().unwrap();
    let meta = vm.create_retained_table().unwrap();
    let key = vm.string("__tostring");
    vm.raw_table_set_value(meta.value(&vm).unwrap(), key, function)
        .unwrap();
    vm.native_setmetatable(&[object.value(&vm).unwrap(), meta.value(&vm).unwrap()])
        .unwrap();
    let program = Program::new(
        Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
        Vec::new(),
    );
    // At this trusted test seam both boundaries are exceeded. Existing callback
    // entry rejects depth before admitting newly saved continuation state.
    vm.settings.memory_limit_bytes = Some(0);
    for tostring in [false, true] {
        let result = if tostring {
            vm.native_tostring_transfer(&program, &[object.value(&vm).unwrap()], 1)
        } else {
            vm.native_gsub_transfer(&program, &[text, text, function], 1)
        };
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("depth must fail"),
        };
        assert!(
            matches!(error.kind, ErrorKind::RecursionLimit { limit: 1 }),
            "{error}"
        );
    }
    assert!(vm.dynamic_programs.slots.is_empty());
    assert!(vm.active_coroutine_frames.is_empty());
}

#[test]
fn transferred_native_service_waits_for_control_quota_admission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let effects = Arc::new(AtomicUsize::new(0));
    let observed = effects.clone();
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            services: VmServices {
                stdout: Some(StdoutSink::new(move |_| {
                    observed.fetch_add(1, Ordering::SeqCst);
                })),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let program = Program::new(
        Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
        Vec::new(),
    );
    let print = vm.get_global_value("print").unwrap();
    let message = vm.string("must not escape");
    let request = vm
        .defer_non_bytecode_call(&program, print, &[message], 0, None)
        .unwrap();
    // Inject exhaustion after request construction, before dispatch; proves the
    // internal transfer itself cannot bypass host-service admission.
    vm.settings.memory_limit_bytes = Some(0);
    let mut state = vm.begin_execution();
    let mut hosts = RejectHostDispatcher;
    let result = vm.drive_deferred_bytecode_flow(
        Ok(request),
        0,
        &mut Execution {
            state: &mut state,
            hosts: &mut hosts,
        },
    );
    assert_eq!(
        effects.load(Ordering::SeqCst),
        0,
        "native service ran above quota"
    );
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::OutOfMemory,
            ..
        })
    ));
    assert!(vm.active_coroutine_frames.is_empty());
    assert!(vm.call_stack.is_empty());
}

#[test]
fn callable_native_depth_rejection_precedes_transfer_admission() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let effects = Arc::new(AtomicUsize::new(0));
    let observed = effects.clone();
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            recursion_limit: 1,
            services: VmServices {
                stdout: Some(StdoutSink::new(move |_| {
                    observed.fetch_add(1, Ordering::SeqCst);
                })),
                ..Default::default()
            },
            ..Default::default()
        },
    );
    let program = Program::new(
        Function::new(vec![Instruction::new(Opcode::Halt, 0, 0, 0)], 1),
        Vec::new(),
    );
    let print = vm.get_global_value("print").unwrap();
    let object = vm.create_retained_table().unwrap();
    let meta = vm.create_retained_table().unwrap();
    let key = vm.string("__call");
    vm.raw_table_set_value(meta.value(&vm).unwrap(), key, print)
        .unwrap();
    vm.native_setmetatable(&[object.value(&vm).unwrap(), meta.value(&vm).unwrap()])
        .unwrap();
    let request = vm
        .defer_non_bytecode_call(&program, object.value(&vm).unwrap(), &[], 1, None)
        .unwrap();
    vm.settings.memory_limit_bytes = Some(0);
    let mut state = vm.begin_execution();
    let mut hosts = RejectHostDispatcher;
    let result = vm.drive_deferred_bytecode_flow(
        Ok(request),
        0,
        &mut Execution {
            state: &mut state,
            hosts: &mut hosts,
        },
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("resolved depth must fail"),
    };
    assert!(
        matches!(error.kind, ErrorKind::RecursionLimit { limit: 1 }),
        "{error}"
    );
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(vm.active_coroutine_frames.is_empty());
    assert!(vm.call_stack.is_empty());
}

#[test]
fn borrowed_implicit_callbacks_do_not_install_loaded_programs() {
    let cases = [
        (
            "__index",
            Instruction::with_aux(Opcode::GetImport, 4, 2, 0, 4),
        ),
        ("__index", Instruction::new(Opcode::GetIndex, 4, 0, 2)),
        (
            "__index",
            Instruction::with_aux(Opcode::GetTableKs, 4, 0, 0, 2),
        ),
        (
            "__index",
            Instruction::with_aux(Opcode::NameCall, 4, 0, 1, 2),
        ),
        ("__newindex", Instruction::new(Opcode::SetIndex, 0, 2, 3)),
        (
            "__newindex",
            Instruction::with_aux(Opcode::SetTableKs, 0, 3, 0, 2),
        ),
        ("__eq", Instruction::new(Opcode::Equal, 4, 0, 1)),
        ("__eq", Instruction::new(Opcode::NotEqual, 4, 0, 1)),
        ("__add", Instruction::new(Opcode::Add, 4, 0, 1)),
        ("__sub", Instruction::new(Opcode::Sub, 4, 0, 1)),
        ("__mul", Instruction::new(Opcode::Mul, 4, 0, 1)),
        ("__div", Instruction::new(Opcode::Div, 4, 0, 1)),
        ("__idiv", Instruction::new(Opcode::FloorDiv, 4, 0, 1)),
        ("__mod", Instruction::new(Opcode::Mod, 4, 0, 1)),
        ("__pow", Instruction::new(Opcode::Pow, 4, 0, 1)),
        ("__unm", Instruction::new(Opcode::Neg, 4, 0, 0)),
        ("__concat", Instruction::new(Opcode::Concat, 4, 0, 1)),
        ("__len", Instruction::new(Opcode::Len, 4, 0, 0)),
        ("__lt", Instruction::new(Opcode::Less, 4, 0, 1)),
        ("__lt", Instruction::new(Opcode::LessEqual, 4, 0, 1)),
        ("__lt", Instruction::new(Opcode::Greater, 4, 0, 1)),
        ("__le", Instruction::new(Opcode::LessEqual, 4, 0, 1)),
        ("__le", Instruction::new(Opcode::GreaterEqual, 4, 0, 1)),
    ];
    for (metamethod, operation) in cases {
        let settings = VmSettings {
            initial_fuel: Some(10_000),
            memory_limit_bytes: Some(1024 * 1024),
            ..Default::default()
        };
        let mut vm = Vm::with_settings(Profile::DeterministicSim, settings);
        vm.install_host_function(HostFunctionOp::dispatched(
            "probe",
            1,
            OpCapability::Pure,
            Arc::new(()),
        ))
        .unwrap();
        let probe = vm.get_global_value("probe").unwrap();
        let left = vm.create_retained_table().unwrap();
        let right = vm.create_retained_table().unwrap();
        let meta = vm.create_retained_table().unwrap();
        let key = vm.string(metamethod);
        vm.raw_table_set_value(meta.value(&vm).unwrap(), key, probe)
            .unwrap();
        for object in [&left, &right] {
            vm.native_setmetatable(&[object.value(&vm).unwrap(), meta.value(&vm).unwrap()])
                .unwrap();
        }
        vm.set_global_value("left", left.value(&vm).unwrap())
            .unwrap();
        vm.set_global_value("right", right.value(&vm).unwrap())
            .unwrap();
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                    Instruction::with_aux(Opcode::GetGlobal, 1, 0, 0, 1),
                    Instruction::new(Opcode::LoadConst, 2, 0, 2),
                    Instruction::new(Opcode::LoadConst, 3, 0, 3),
                    operation,
                    Instruction::new(Opcode::Return, 3, 1, 0),
                ],
                5,
            ),
            vec![
                Constant::String(b"left".to_vec()),
                Constant::String(b"right".to_vec()),
                Constant::String(b"field".to_vec()),
                Constant::Number(42.0),
                Constant::Import(vec![0, 2]),
            ],
        );
        assert!(vm.dynamic_programs.slots.is_empty());
        let mut calls = 0;
        let mut host = |vm: &mut Vm, _: &(dyn std::any::Any + Send + Sync), _: &[Value]| {
            calls += 1;
            assert!(
                vm.dynamic_programs.slots.is_empty(),
                "{metamethod} {:?}",
                operation.op
            );
            let _ = vm.memory_stats();
            let mut exact = MemoryUsage::default();
            add_vm_cached_control_memory(vm, &mut exact);
            assert_eq!(vm.control_live_bytes, exact.live_bytes);
            Ok(vec![Value::number(42.0)])
        };
        let mut execution = vm.begin_transactional_execution_with_fuel(Some(10_000));
        let result = vm.run_stateless_entry_in(&program, &mut execution, &mut host);
        assert_eq!(result.unwrap().as_number(), Some(42.0));
        assert_eq!(calls, 1, "{metamethod} {:?}", operation.op);
        assert!(vm.dynamic_programs.slots.is_empty());
        assert!(vm.active_coroutine_frames.is_empty());
        assert!(vm.call_stack.is_empty());
    }
}

#[test]
fn borrowed_generalized_iterator_preparation_does_not_install_loaded_programs() {
    let mut vm = Vm::with_settings(
        Profile::DeterministicSim,
        VmSettings {
            initial_fuel: Some(10_000),
            memory_limit_bytes: Some(1024 * 1024),
            ..Default::default()
        },
    );
    vm.install_host_function(HostFunctionOp::dispatched(
        "probe",
        1,
        OpCapability::Pure,
        Arc::new(()),
    ))
    .unwrap();
    let object = vm.create_retained_table().unwrap();
    let meta = vm.create_retained_table().unwrap();
    let key = vm.string("__iter");
    let probe = vm.get_global_value("probe").unwrap();
    vm.raw_table_set_value(meta.value(&vm).unwrap(), key, probe)
        .unwrap();
    vm.native_setmetatable(&[object.value(&vm).unwrap(), meta.value(&vm).unwrap()])
        .unwrap();
    vm.set_global_value("subject", object.value(&vm).unwrap())
        .unwrap();
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                Instruction::with_aux(Opcode::GeneralizedIterPrep, 0, 0, 1, 4),
                Instruction::new(Opcode::LoadConst, 6, 0, 1),
                Instruction::new(Opcode::Return, 6, 1, 0),
                Instruction::new(Opcode::LoadConst, 6, 0, 2),
                Instruction::new(Opcode::Return, 6, 1, 0),
            ],
            7,
        ),
        vec![
            Constant::String(b"subject".to_vec()),
            Constant::Number(-1.0),
            Constant::Number(42.0),
        ],
    );
    let mut calls = 0;
    let mut host = |vm: &mut Vm, _: &(dyn std::any::Any + Send + Sync), args: &[Value]| {
        calls += 1;
        assert!(vm.dynamic_programs.slots.is_empty());
        let _ = vm.memory_stats();
        let mut exact = MemoryUsage::default();
        add_vm_cached_control_memory(vm, &mut exact);
        assert_eq!(vm.control_live_bytes, exact.live_bytes);
        Ok(vec![vm.get_global_value("next")?, args[0], Value::nil()])
    };
    let mut execution = vm.begin_transactional_execution_with_fuel(Some(10_000));
    assert_eq!(
        vm.run_stateless_entry_in(&program, &mut execution, &mut host)
            .unwrap()
            .as_number(),
        Some(42.0)
    );
    assert_eq!(calls, 1);
    assert!(vm.dynamic_programs.slots.is_empty());
    assert!(vm.active_coroutine_frames.is_empty());
    assert!(vm.call_stack.is_empty());
}

#[test]
fn borrowed_foreach_callbacks_do_not_install_loaded_programs() {
    for name in ["foreach", "foreachi"] {
        let mut vm = Vm::with_settings(
            Profile::DeterministicSim,
            VmSettings {
                initial_fuel: Some(10_000),
                memory_limit_bytes: Some(1024 * 1024),
                ..Default::default()
            },
        );
        vm.install_host_function(HostFunctionOp::dispatched(
            "probe",
            1,
            OpCapability::Pure,
            Arc::new(()),
        ))
        .unwrap();
        let object = vm.create_retained_table().unwrap();
        vm.raw_table_set_value(
            object.value(&vm).unwrap(),
            Value::number(1.0),
            Value::number(42.0),
        )
        .unwrap();
        vm.set_global_value("subject", object.value(&vm).unwrap())
            .unwrap();
        let table = vm.get_global_value("table").unwrap();
        let key = vm.string(name);
        let walk = vm.raw_table_get_value(table, key).unwrap();
        vm.set_global_value("walk", walk).unwrap();
        let program = Program::new(
            Function::new(
                vec![
                    Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 0),
                    Instruction::with_aux(Opcode::GetGlobal, 1, 0, 0, 1),
                    Instruction::with_aux(Opcode::GetGlobal, 2, 0, 0, 2),
                    Instruction::with_aux(Opcode::Call, 3, 0, 1, 2),
                    Instruction::new(Opcode::Return, 3, 1, 0),
                ],
                4,
            ),
            vec![
                Constant::String(b"walk".to_vec()),
                Constant::String(b"subject".to_vec()),
                Constant::String(b"probe".to_vec()),
            ],
        );
        let mut calls = 0;
        let mut host = |vm: &mut Vm, _: &(dyn std::any::Any + Send + Sync), args: &[Value]| {
            calls += 1;
            assert!(vm.dynamic_programs.slots.is_empty());
            assert_eq!(args[0].as_number(), Some(1.0));
            assert_eq!(args[1].as_number(), Some(42.0));
            let _ = vm.memory_stats();
            let mut exact = MemoryUsage::default();
            add_vm_cached_control_memory(vm, &mut exact);
            assert_eq!(vm.control_live_bytes, exact.live_bytes);
            Ok(vec![Value::nil()])
        };
        let mut execution = vm.begin_transactional_execution_with_fuel(Some(10_000));
        assert_eq!(
            vm.run_stateless_entry_in(&program, &mut execution, &mut host)
                .unwrap()
                .kind(),
            ValueKind::Nil
        );
        assert_eq!(calls, 1);
        assert!(vm.dynamic_programs.slots.is_empty());
        assert!(vm.active_coroutine_frames.is_empty());
        assert!(vm.call_stack.is_empty());
    }
}
