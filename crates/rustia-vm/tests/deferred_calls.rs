use rustia_vm::{
    CallDescriptor, CallDescriptorCall, Constant, ErrorKind, Function, HardenedLeafProgram,
    HostFunctionOp, Instruction, OpCapability, Opcode, Profile, Program, ValueKind, Vm, VmSettings,
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
    let calls = (0..levels)
        .map(|level| CallDescriptorCall {
            fixed_args: 0,
            deferred_import: (level + 1 < levels).then_some(1),
            deferred_method: None,
            method_receiver_register: None,
        })
        .collect();
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
            calls,
            append_varargs: matches!(
                op,
                Opcode::CallExpandVarargs
                    | Opcode::ReturnCallExpandVarargs
                    | Opcode::ReturnCallExpand2Varargs
            ),
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
    let mut descriptors = vec![
        CallDescriptor {
            aux: -1,
            calls: vec![
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: Some(1),
                    deferred_method: None,
                    method_receiver_register: None,
                },
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: None,
                    deferred_method: None,
                    method_receiver_register: None,
                },
            ],
            append_varargs: false,
        },
        CallDescriptor {
            aux: 1,
            calls: vec![
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: Some(99),
                    deferred_method: None,
                    method_receiver_register: None,
                },
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: None,
                    deferred_method: None,
                    method_receiver_register: None,
                },
            ],
            append_varargs: false,
        },
        CallDescriptor {
            aux: 1,
            calls: vec![
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: Some(0),
                    deferred_method: None,
                    method_receiver_register: None,
                },
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: None,
                    deferred_method: None,
                    method_receiver_register: None,
                },
            ],
            append_varargs: false,
        },
        CallDescriptor {
            aux: 1,
            calls: vec![CallDescriptorCall {
                fixed_args: 0,
                deferred_import: Some(1),
                deferred_method: None,
                method_receiver_register: None,
            }],
            append_varargs: false,
        },
    ];
    for (fixed_args, deferred_import, deferred_method) in [
        (0, None, Some(0)),        // no receiver
        (1, Some(1), Some(0)),     // conflicting lookup metadata
        (1, None, Some(1)),        // import descriptor, not a string
        (1, None, Some(99)),       // missing constant
        (1, None, Some(u32::MAX)), // index overflow
    ] {
        descriptors.push(CallDescriptor {
            aux: 1,
            calls: vec![
                CallDescriptorCall {
                    fixed_args,
                    deferred_import,
                    deferred_method,
                    method_receiver_register: None,
                },
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: None,
                    deferred_method: None,
                    method_receiver_register: None,
                },
            ],
            append_varargs: false,
        });
    }
    for (deferred_method, method_receiver_register) in [(None, Some(0)), (Some(0), Some(255))] {
        descriptors.push(CallDescriptor {
            aux: 1,
            calls: vec![
                CallDescriptorCall {
                    fixed_args: 1,
                    deferred_import: None,
                    deferred_method,
                    method_receiver_register,
                },
                CallDescriptorCall {
                    fixed_args: 0,
                    deferred_import: None,
                    deferred_method: None,
                    method_receiver_register: None,
                },
            ],
            append_varargs: false,
        });
    }
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
            calls: Vec::new(),
            append_varargs: false,
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

fn recursive_expanded_program(depth: f64) -> Program {
    let mut function = Function::new(
        vec![
            Instruction::new(Opcode::LoadConst, 1, 0, 1),
            Instruction::new(Opcode::Equal, 1, 0, 1),
            Instruction::with_aux(Opcode::JumpIfNot, 1, 0, 0, 6),
            Instruction::new(Opcode::LoadConst, 0, 0, 2),
            Instruction::new(Opcode::LoadConst, 1, 0, 3),
            Instruction::new(Opcode::Return, 0, 2, 0),
            Instruction::new(Opcode::LoadCurrentFunction, 3, 0, 0),
            Instruction::new(Opcode::LoadConst, 4, 0, 4),
            Instruction::new(Opcode::Sub, 4, 0, 4),
            Instruction::new(Opcode::LoadConst, 2, 0, 5),
            Instruction::with_aux(Opcode::ReturnCallExpand, 0, 0, 1, !8),
        ],
        5,
    );
    function.parameter_count = 1;
    let mut program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::LoadFunction, 0, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 1, 0, 0),
                Instruction::with_aux(Opcode::Call, 0, 0, 1, 1),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            2,
        ),
        vec![
            Constant::Number(depth),
            Constant::Number(0.0),
            Constant::Number(4.0),
            Constant::Number(5.0),
            Constant::Number(1.0),
            Constant::String(b"abc".to_vec()),
            Constant::String(b"count".to_vec()),
            Constant::Import(vec![6]),
            Constant::CallDescriptor(Box::new(CallDescriptor {
                aux: 0,
                calls: vec![
                    CallDescriptorCall {
                        fixed_args: 1,
                        deferred_import: Some(7),
                        deferred_method: None,
                        method_receiver_register: None,
                    },
                    CallDescriptorCall {
                        fixed_args: 1,
                        deferred_import: None,
                        deferred_method: None,
                        method_receiver_register: None,
                    },
                ],
                append_varargs: false,
            })),
        ],
    );
    program.functions.push(function);
    program
}

fn install_counter(vm: &mut Vm) {
    vm.install_host_function(HostFunctionOp::new(
        "count",
        1,
        OpCapability::Pure,
        Arc::new(|vm, args| {
            vm.collect_garbage();
            Ok(vec![
                rustia_vm::Value::number(args.len() as f64),
                rustia_vm::Value::number(7.0),
            ])
        }),
    ))
    .unwrap();
}

#[test]
fn recursive_expanded_arguments_use_iterative_bytecode_frames() {
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            deterministic_gc_interval: NonZeroU64::new(1).unwrap(),
            memory_limit_bytes: Some(2 * 1024 * 1024),
            ..VmSettings::default()
        },
    );
    install_counter(&mut vm);
    for depth in [1.0, 2.0, 20.0, 100.0] {
        assert_eq!(
            vm.run(&recursive_expanded_program(depth))
                .unwrap_or_else(|e| panic!("depth {depth}: {e:?}"))
                .as_number(),
            Some(3.0)
        );
    }
}

#[test]
fn recursive_expanded_arguments_exhaust_the_structured_recursion_limit() {
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            recursion_limit: 12,
            ..VmSettings::default()
        },
    );
    install_counter(&mut vm);
    assert!(matches!(
        vm.run(&recursive_expanded_program(100.0)).unwrap_err().kind,
        ErrorKind::RecursionLimit { limit: 12 }
    ));

    let original = recursive_expanded_program(100.0);
    let mut constants = original.constants().to_vec();
    constants.push(Constant::String(b"pcall".to_vec()));
    let mut protected = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 9),
                Instruction::with_aux(Opcode::LoadFunction, 1, 0, 0, 0),
                Instruction::new(Opcode::LoadConst, 2, 0, 0),
                Instruction::new(Opcode::CallMulti, 0, 2, 2),
                Instruction::new(Opcode::Return, 0, 2, 0),
            ],
            3,
        ),
        constants,
    );
    protected.functions = original.functions;
    assert!(matches!(
        vm.run(&protected).unwrap_err().kind,
        ErrorKind::RecursionLimit { limit: 12 }
    ));
}

#[test]
fn general_descriptor_preserves_results_through_many_call_levels() {
    let original = expanded_program(Opcode::CallExpand);
    let mut constants = original.constants().to_vec();
    constants[2] = Constant::CallDescriptor(Box::new(CallDescriptor {
        aux: 1,
        calls: (0..100)
            .map(|level| CallDescriptorCall {
                fixed_args: 0,
                deferred_import: (level != 99).then_some(1),
                deferred_method: None,
                method_receiver_register: None,
            })
            .collect(),
        append_varargs: false,
    }));
    let program = Program::new(
        Function::new(
            vec![
                Instruction::with_aux(Opcode::GetGlobal, 99, 0, 0, 3),
                Instruction::with_aux(Opcode::CallExpand, 0, 0, 0, !2),
                Instruction::new(Opcode::Return, 0, 1, 0),
            ],
            100,
        ),
        constants,
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    install_callbacks(&mut vm, Arc::new(AtomicUsize::new(0)), true);
    let result = vm.run(&program).unwrap();
    assert_eq!(
        vm.string_value(result).unwrap().as_deref(),
        Some("fresh argument survives deferred lookup and collection")
    );
}

#[test]
fn stackless_pcall_shortcuts_cannot_swallow_configured_recursion_limits() {
    for tail in [false, true] {
        let original = recursive_expanded_program(100.0);
        let mut constants = original.constants().to_vec();
        constants.push(Constant::String(b"pcall".to_vec()));
        let mut code = vec![
            Instruction::with_aux(Opcode::GetGlobal, 0, 0, 0, 9),
            Instruction::new(Opcode::LoadCurrentFunction, 1, 0, 0),
        ];
        if tail {
            code.push(Instruction::with_aux(
                Opcode::ReturnCall,
                0,
                0,
                0,
                (1 << 8) | 1,
            ));
        } else {
            code.push(Instruction::new(Opcode::CallMulti, 0, 1, 2));
            code.push(Instruction::new(Opcode::Return, 0, 2, 0));
        }
        let mut program = Program::new(original.main, constants);
        program.functions.push(Function::new(code, 2));
        let mut vm = Vm::with_settings(
            Profile::ReproducibleStandard,
            VmSettings {
                recursion_limit: 12,
                ..VmSettings::default()
            },
        );
        assert!(
            matches!(
                vm.run(&program).unwrap_err().kind,
                ErrorKind::RecursionLimit { limit: 12 }
            ),
            "tail={tail}"
        );
    }
}
