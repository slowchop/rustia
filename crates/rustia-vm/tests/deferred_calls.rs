use rustia_vm::{
    CallDescriptor, Constant, ErrorKind, Function, HardenedLeafProgram, HostFunctionOp,
    Instruction, OpCapability, Opcode, Profile, Program, ValueKind, Vm, VmSettings,
};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn expanded_program(op: Opcode) -> Program {
    let levels = match op {
        Opcode::CallExpand3 => 4,
        Opcode::CallExpand2 | Opcode::ReturnCallExpand2 | Opcode::ReturnCallExpand2Varargs => 3,
        _ => 2,
    };
    let append = op == Opcode::AppendCallExpand;
    let returning = matches!(
        op,
        Opcode::ReturnCallExpand
            | Opcode::ReturnCallExpand2
            | Opcode::ReturnCallExpandVarargs
            | Opcode::ReturnCallExpand2Varargs
    );
    let mut imports = [None; 4];
    imports[..levels - 1].fill(Some(1));
    let aux = if append {
        1 << 8
    } else if op == Opcode::CallExpand {
        1
    } else {
        0
    };
    let constants = vec![
        Constant::String(b"target".to_vec()),
        Constant::Import(vec![0]),
        Constant::CallDescriptor(Box::new(CallDescriptor {
            aux,
            deferred_imports: imports,
        })),
        Constant::String(b"mutate".to_vec()),
        Constant::Number(1.0),
    ];
    let base = u8::from(append);
    let mut code = Vec::new();
    if append {
        code.push(Instruction::new(Opcode::NewArray, 0, 0, 0));
    }
    code.push(Instruction::with_aux(
        Opcode::GetGlobal,
        base + levels as u8 - 1,
        0,
        0,
        3,
    ));
    let (a, b, c) = if append {
        (0, 1, 0)
    } else if returning {
        (0, 0, 0)
    } else if op == Opcode::CallExpand3 {
        (0, 1, 0)
    } else if op == Opcode::CallExpand {
        (0, 0, 0)
    } else {
        (0, 0, 1)
    };
    code.push(Instruction::with_aux(op, a, b, c, !2));
    if append {
        code.push(Instruction::new(Opcode::LoadConst, 1, 0, 4));
        code.push(Instruction::new(Opcode::GetIndex, 0, 0, 1));
    }
    if !returning {
        code.push(Instruction::new(Opcode::Return, 0, 1, 0));
    }
    Program::new(Function::new(code, 6), constants)
}

fn install_callbacks(vm: &mut Vm, calls: Arc<AtomicUsize>, install_replacement: bool) {
    vm.install_host_function(HostFunctionOp::new(
        "replacement",
        1,
        OpCapability::Pure,
        Arc::new(|vm, args| {
            vm.collect_garbage();
            Ok(args.to_vec())
        }),
    ))
    .unwrap();
    vm.install_host_function(HostFunctionOp::new(
        "mutate",
        1,
        OpCapability::Pure,
        Arc::new(move |vm, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            if install_replacement {
                let replacement = vm.get_global_value("replacement")?;
                vm.set_global_value("target", replacement)?;
            }
            Ok(vec![vm.string(
                "fresh argument survives deferred lookup and collection",
            )])
        }),
    ))
    .unwrap();
}

#[test]
fn every_expanded_shape_resolves_each_outer_callee_after_arguments() {
    for op in [
        Opcode::CallExpand,
        Opcode::CallExpand2,
        Opcode::CallExpand3,
        Opcode::CallExpandVarargs,
        Opcode::ReturnCallExpand,
        Opcode::ReturnCallExpand2,
        Opcode::ReturnCallExpandVarargs,
        Opcode::ReturnCallExpand2Varargs,
        Opcode::AppendCallExpand,
    ] {
        let settings = VmSettings {
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            ..VmSettings::default()
        };
        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let calls = Arc::new(AtomicUsize::new(0));
        install_callbacks(&mut vm, calls.clone(), true);
        let result = vm
            .run(&expanded_program(op))
            .unwrap_or_else(|e| panic!("{op:?}: {e}"));
        assert_eq!(
            vm.string_value(result).unwrap().as_deref(),
            Some("fresh argument survives deferred lookup and collection"),
            "{op:?}"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1, "{op:?}");
    }
}

#[test]
fn deferred_lookup_and_call_failures_leave_argument_side_effects_in_order() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let calls = Arc::new(AtomicUsize::new(0));
    install_callbacks(&mut vm, calls.clone(), false);
    let error = vm.run(&expanded_program(Opcode::CallExpand)).unwrap_err();
    assert!(error.to_string().contains("attempt to call"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn deferred_lookup_fuel_is_charged_after_argument_effects() {
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            initial_fuel: Some(3),
            ..VmSettings::default()
        },
    );
    let calls = Arc::new(AtomicUsize::new(0));
    install_callbacks(&mut vm, calls.clone(), true);
    let error = vm.run(&expanded_program(Opcode::CallExpand)).unwrap_err();
    assert!(matches!(error.kind, ErrorKind::FuelExhausted { limit: 3 }));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(error.pc, Some(1));

    let mut enough = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            initial_fuel: Some(6),
            ..VmSettings::default()
        },
    );
    install_callbacks(&mut enough, Arc::new(AtomicUsize::new(0)), true);
    enough
        .run(&expanded_program(Opcode::CallExpand))
        .expect("lookup has no placeholder fuel charge");
}

#[test]
fn malformed_call_descriptors_reject_before_running_arguments() {
    let descriptors = [
        CallDescriptor {
            aux: -1,
            deferred_imports: [Some(1), None, None, None],
        },
        CallDescriptor {
            aux: 1,
            deferred_imports: [Some(99), None, None, None],
        },
        CallDescriptor {
            aux: 1,
            deferred_imports: [Some(0), None, None, None],
        },
        CallDescriptor {
            aux: 1,
            deferred_imports: [Some(1), None, Some(1), None],
        },
    ];
    for descriptor in descriptors {
        let original = expanded_program(Opcode::CallExpand);
        let mut constants = original.constants().to_vec();
        constants[2] = Constant::CallDescriptor(Box::new(descriptor));
        let program = Program::new(original.main, constants);
        let calls = Arc::new(AtomicUsize::new(0));
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        install_callbacks(&mut vm, calls.clone(), true);
        assert!(matches!(
            vm.run(&program).unwrap_err().kind,
            ErrorKind::InvalidProgram { .. }
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(HardenedLeafProgram::new(program).is_err());
    }
}

#[test]
fn call_descriptors_are_metadata_and_cannot_be_loaded_as_guest_values() {
    let program = Program::new(
        Function::new(vec![Instruction::new(Opcode::LoadConst, 0, 0, 0)], 1),
        vec![Constant::CallDescriptor(Box::new(CallDescriptor {
            aux: 1,
            deferred_imports: [None; 4],
        }))],
    );
    assert!(matches!(
        Vm::new(Profile::ReproducibleStandard)
            .run(&program)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidProgram { .. }
    ));
}

#[test]
fn expanded_argument_mutation_uses_existing_transaction_rollback() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    install_callbacks(&mut vm, Arc::new(AtomicUsize::new(0)), true);
    vm.begin_guest_transaction().unwrap();
    let result = vm
        .run(&expanded_program(Opcode::ReturnCallExpand2))
        .unwrap();
    assert!(vm.string_value(result).unwrap().is_some());
    vm.rollback_guest_transaction();
    assert_eq!(
        vm.get_global_value("target").unwrap().kind(),
        ValueKind::Nil
    );
}
