use super::*;
use crate::test_ownership::{Cell, Rc};

struct Counter(i64);

fn definition(value: f64) -> HostObjectTypeDefinition {
    HostObjectTypeDefinition {
        hooks: crate::HostObjectHooks::default(),
        rust_type_id: TypeId::of::<Counter>(),
        rust_type_name: type_name::<Counter>(),
        name: "Counter".to_owned(),
        methods: vec![HostFunctionOp::new(
            "value",
            1,
            OpCapability::Pure,
            Arc::new(move |vm, args| {
                let data = vm.checked_host_object(
                    args[0],
                    TypeId::of::<Counter>(),
                    type_name::<Counter>(),
                )?;
                let counter = data.try_borrow::<Counter>()?;
                Ok(vec![Value::number(counter.0 as f64 + value)])
            }),
        )],
    }
}

fn call(vm: &mut Vm, object: &RetainedValue) -> f64 {
    let raw = object.value(vm).unwrap();
    let method = vm.host_object_method(raw, "value").unwrap();
    vm.call_retained_function(method, &[raw]).unwrap()[0]
        .as_number()
        .unwrap()
}

#[test]
fn replacement_preserves_old_instances_and_reset_restores_only_current_definition() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(definition(1.0)).unwrap();
    let old = vm.create_retained_host_object(Counter(10)).unwrap();
    let alias = old.clone();
    vm.register_host_object_type(definition(2.0)).unwrap();
    let new = vm.create_retained_host_object(Counter(10)).unwrap();
    assert_eq!(call(&mut vm, &old), 11.0);
    assert_eq!(call(&mut vm, &alias), 11.0);
    assert_eq!(call(&mut vm, &new), 12.0);
    assert_eq!(vm.host_object_types.len(), 1);
    vm.reset_decision().unwrap();
    assert!(old.value(&vm).is_err());
    assert!(alias.value(&vm).is_err());
    assert!(new.value(&vm).is_err());
    let after_reset = vm.create_retained_host_object(Counter(10)).unwrap();
    assert_eq!(call(&mut vm, &after_reset), 12.0);
}

#[test]
fn forbidden_method_and_duplicate_names_reject_the_complete_replacement() {
    let mut vm = Vm::new(Profile::DeterministicSim);
    vm.register_host_object_type(definition(1.0)).unwrap();
    let before = vm.current_live_bytes();
    let mut candidate = definition(2.0);
    let mut forbidden = candidate.methods[0].clone();
    forbidden.name = "forbidden".to_owned();
    forbidden.capability = OpCapability::ReadsExternalMutableState;
    candidate.methods.push(forbidden);
    assert!(matches!(
        vm.register_host_object_type(candidate).unwrap_err().kind,
        ErrorKind::CapabilityRemoved { .. }
    ));
    assert_eq!(vm.current_live_bytes(), before);
    for empty_rust_name in [false, true] {
        let mut empty_name = definition(3.0);
        if empty_rust_name {
            empty_name.rust_type_name = "";
        } else {
            empty_name.name.clear();
        }
        assert!(vm.register_host_object_type(empty_name).is_err());
        assert_eq!(vm.current_live_bytes(), before);
    }
    let mut duplicate = definition(3.0);
    duplicate.methods.push(duplicate.methods[0].clone());
    assert!(vm.register_host_object_type(duplicate).is_err());
    assert_eq!(vm.current_live_bytes(), before);
    let mut forbidden_iterator = definition(3.0);
    forbidden_iterator.hooks.iterator = Some(HostFunctionOp::new(
        "__iter",
        1,
        OpCapability::ReadsExternalMutableState,
        Arc::new(|_, _| Ok(vec![Value::nil()])),
    ));
    forbidden_iterator.hooks.iterator_step = Some(HostFunctionOp::new(
        "__iter_step",
        1,
        OpCapability::Pure,
        Arc::new(|_, _| Ok(vec![Value::nil()])),
    ));
    assert!(matches!(
        vm.register_host_object_type(forbidden_iterator)
            .unwrap_err()
            .kind,
        ErrorKind::CapabilityRemoved { .. }
    ));
    assert_eq!(vm.current_live_bytes(), before);
    let object = vm.create_retained_host_object(Counter(10)).unwrap();
    assert_eq!(call(&mut vm, &object), 11.0);
    vm.begin_guest_transaction().unwrap();
    assert!(matches!(
        vm.register_host_object_type(definition(4.0))
            .unwrap_err()
            .kind,
        ErrorKind::HostConfigurationWhileRunning
    ));
    vm.rollback_guest_transaction();
}

struct DropProbe(Rc<Cell<usize>>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn probe_definition() -> HostObjectTypeDefinition {
    HostObjectTypeDefinition {
        hooks: crate::HostObjectHooks::default(),
        rust_type_id: TypeId::of::<DropProbe>(),
        rust_type_name: type_name::<DropProbe>(),
        name: "DropProbe".into(),
        methods: Vec::new(),
    }
}

#[test]
fn collection_guest_aliases_reset_and_vm_drop_release_each_payload_once() {
    let drops = Rc::new(Cell::new(0));
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(probe_definition()).unwrap();
    let object = vm
        .create_retained_host_object(DropProbe(drops.clone()))
        .unwrap();
    let alias = object.clone();
    vm.set_global_value("object", object.value(&vm).unwrap())
        .unwrap();
    drop(object);
    drop(alias);
    vm.collect_garbage();
    assert_eq!(drops.get(), 0);
    vm.set_global_value("object", Value::nil()).unwrap();
    vm.collect_garbage();
    assert_eq!(drops.get(), 1);
    let stale = vm
        .create_retained_host_object(DropProbe(drops.clone()))
        .unwrap();
    vm.reset_decision().unwrap();
    assert_eq!(drops.get(), 2);
    drop(stale);
    assert_eq!(drops.get(), 2);
    let object = vm
        .create_retained_host_object(DropProbe(drops.clone()))
        .unwrap();
    drop(vm);
    assert_eq!(drops.get(), 3);
    drop(object);
    assert_eq!(drops.get(), 3);
}

#[test]
fn snapshots_share_existing_payload_mutations_and_discard_provisional_instances() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(definition(0.0)).unwrap();
    vm.register_host_object_type(probe_definition()).unwrap();
    let existing = vm.create_retained_host_object(Counter(1)).unwrap();
    let drops = Rc::new(Cell::new(0));
    vm.begin_guest_transaction().unwrap();
    let provisional = vm
        .create_retained_host_object(DropProbe(drops.clone()))
        .unwrap();
    let data = vm
        .checked_host_object(
            existing.value(&vm).unwrap(),
            TypeId::of::<Counter>(),
            "Counter",
        )
        .unwrap();
    data.try_borrow_mut::<Counter>().unwrap().0 = 7;
    drop(data);
    vm.rollback_guest_transaction();
    assert_eq!(call(&mut vm, &existing), 7.0);
    assert_eq!(drops.get(), 1);
    assert!(provisional.value(&vm).is_err());
    drop(provisional);
    assert_eq!(drops.get(), 1);
    vm.begin_guest_transaction().unwrap();
    let committed = vm
        .create_retained_host_object(DropProbe(drops.clone()))
        .unwrap();
    vm.commit_guest_transaction().unwrap();
    assert!(committed.value(&vm).is_ok());
    drop(committed);
    vm.collect_garbage();
    assert_eq!(drops.get(), 2);
}

#[test]
fn receiver_and_borrow_failures_are_errors_and_missing_methods_do_not_allocate() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(definition(0.0)).unwrap();
    let object = vm.create_retained_host_object(Counter(1)).unwrap();
    let raw = object.value(&vm).unwrap();
    assert_eq!(vm.host_object_type_id(raw), Some(TypeId::of::<Counter>()));
    assert_eq!(vm.host_object_type_id(Value::number(1.0)), None);
    let ordinary = vm.heap.alloc_userdata(None).unwrap();
    assert_eq!(vm.host_object_type_id(Value::userdata(ordinary)), None);
    assert!(vm
        .create_host_object(TypeId::of::<Counter>(), "Counter", Box::new(String::new()))
        .is_err());
    assert!(vm
        .checked_host_object(Value::nil(), TypeId::of::<Counter>(), "Counter")
        .is_err());
    assert!(vm
        .checked_host_object(raw, TypeId::of::<String>(), "String")
        .is_err());
    let data = vm
        .checked_host_object(raw, TypeId::of::<Counter>(), "Counter")
        .unwrap();
    let borrowed = data.try_borrow::<Counter>().unwrap();
    assert!(data.try_borrow_mut::<Counter>().is_err());
    drop(borrowed);
    let borrowed = data.try_borrow_mut::<Counter>().unwrap();
    assert!(data.try_borrow::<Counter>().is_err());
    assert!(data.try_borrow_mut::<Counter>().is_err());
    drop(borrowed);
    assert!(data.try_borrow::<String>().is_err());
    let before = vm.current_live_bytes();
    assert!(vm
        .host_object_method(
            raw,
            "a deliberately long missing method name that is not interned"
        )
        .is_err());
    assert_eq!(vm.current_live_bytes(), before);
}

#[test]
fn object_admission_and_rollback_root_oom_release_payloads_atomically() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(probe_definition()).unwrap();
    let drops = Rc::new(Cell::new(0));
    let before = vm.current_live_bytes();
    vm.settings.memory_limit_bytes =
        Some(before + VmCell::host_object_live_bytes() + external_root_node_live_bytes() - 1);
    assert!(matches!(
        vm.create_retained_host_object(DropProbe(drops.clone()))
            .unwrap_err()
            .kind,
        ErrorKind::OutOfMemory
    ));
    assert_eq!(drops.get(), 1);
    assert_eq!(vm.current_live_bytes(), before);
    vm.settings.memory_limit_bytes = None;
    vm.begin_guest_transaction().unwrap();
    let before = vm.current_live_bytes();
    vm.settings.memory_limit_bytes =
        Some(before + VmCell::host_object_live_bytes() + external_root_node_live_bytes());
    assert!(matches!(
        vm.create_retained_host_object(DropProbe(drops.clone()))
            .unwrap_err()
            .kind,
        ErrorKind::OutOfMemory
    ));
    assert_eq!(drops.get(), 2);
    vm.rollback_guest_transaction();
    vm.settings.memory_limit_bytes = None;
    let after = vm
        .create_retained_host_object(DropProbe(drops.clone()))
        .unwrap();
    assert!(after.value(&vm).is_ok());
}

#[test]
fn registration_oom_does_not_replace_configuration_or_keep_provisional_functions() {
    let mut failed = 0;
    let mut succeeded = 0;
    for extra in (0..32_768).step_by(1024) {
        let mut vm = Vm::new(Profile::ReproducibleStandard);
        vm.register_host_object_type(definition(1.0)).unwrap();
        let base = vm.current_live_bytes();
        let snapshot =
            vm.heap.transaction_snapshot_live_bytes() + guest_transaction_snapshot_live_bytes(&vm);
        vm.settings.memory_limit_bytes = Some(base + snapshot + extra);
        let mut candidate = definition(2.0);
        let owner = Arc::new(());
        let ownership = Arc::downgrade(&owner);
        candidate.methods[0] = HostFunctionOp::new(
            "value",
            1,
            OpCapability::Pure,
            Arc::new(move |_, _| {
                let _ = &owner;
                Ok(vec![Value::number(2.0)])
            }),
        );
        candidate.methods = (0..24)
            .map(|index| {
                let mut method = candidate.methods[0].clone();
                method.name = format!("method_{index}");
                method
            })
            .collect();
        match vm.register_host_object_type(candidate) {
            Ok(()) => {
                succeeded += 1;
                assert!(ownership.upgrade().is_some());
            }
            Err(error) => {
                assert!(matches!(error.kind, ErrorKind::OutOfMemory), "{error}");
                failed += 1;
                assert!(ownership.upgrade().is_none());
                vm.settings.memory_limit_bytes = None;
                let object = vm.create_retained_host_object(Counter(10)).unwrap();
                assert_eq!(call(&mut vm, &object), 11.0);
            }
        }
        assert_eq!(vm.host_object_types.len(), 1);
    }
    assert!(failed > 0 && succeeded > 0);
}

#[test]
fn hardened_creation_is_rejected_before_guest_allocation() {
    let mut vm = Vm::with_settings(
        Profile::DeterministicSim,
        VmSettings {
            guest_surface: GuestSurface::HardenedLeaf,
            memory_limit_bytes: Some(1024 * 1024),
            ..VmSettings::default()
        },
    );
    let before = vm.current_live_bytes();
    assert!(matches!(
        vm.register_host_object_type(definition(0.0))
            .unwrap_err()
            .kind,
        ErrorKind::HardenedLeafViolation { .. }
    ));
    assert!(matches!(
        vm.create_retained_host_object(Counter(1)).unwrap_err().kind,
        ErrorKind::HardenedLeafViolation { .. }
    ));
    assert_eq!(vm.current_live_bytes(), before);
}

#[test]
fn method_tables_are_frozen_protected_and_callback_captures_follow_aliases() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Capture(Arc<AtomicUsize>);
    impl Drop for Capture {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let releases = Arc::new(AtomicUsize::new(0));
    let capture = Capture(releases.clone());
    let mut first = definition(1.0);
    first.methods[0] = HostFunctionOp::new(
        "value",
        1,
        OpCapability::Pure,
        Arc::new(move |_, _| {
            let _ = &capture;
            Ok(vec![Value::number(1.0)])
        }),
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(first).unwrap();
    let object = vm.create_retained_host_object(Counter(0)).unwrap();
    let raw = object.value(&vm).unwrap();
    assert_eq!(
        vm.native_getmetatable(&[raw]).unwrap().as_bool(),
        Some(false)
    );
    let metatable = vm
        .heap
        .userdata_metatable(raw.as_userdata().unwrap())
        .flatten()
        .unwrap();
    let key = vm.string("__index");
    let methods = vm.heap.table_get(metatable, key).unwrap();
    let key = vm.string("value");
    assert!(vm.raw_table_set_value(methods, key, Value::nil()).is_err());
    assert!(vm
        .raw_table_set_value(Value::table(metatable), key, Value::nil())
        .is_err());
    let method = vm.host_object_method(raw, "value").unwrap();
    let detached = vm.retain_value(method).unwrap();
    vm.register_host_object_type(definition(2.0)).unwrap();
    drop(object);
    vm.collect_garbage();
    assert_eq!(releases.load(Ordering::Relaxed), 0);
    drop(detached);
    vm.collect_garbage();
    assert_eq!(releases.load(Ordering::Relaxed), 1);
}

#[test]
fn reset_rebuilds_admitted_configuration_without_temporary_quota_roots() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(definition(0.0)).unwrap();
    vm.collect_garbage();
    let baseline = vm.current_live_bytes();
    vm.settings.memory_limit_bytes = Some(baseline);
    vm.reset_decision().unwrap();
    assert!(vm.current_live_bytes() <= baseline);
    vm.settings.memory_limit_bytes = Some(baseline + 4096);
    let object = vm.create_retained_host_object(Counter(4)).unwrap();
    assert_eq!(call(&mut vm, &object), 4.0);
}

#[test]
fn callable_userdata_hooks_keep_variable_arity_and_current_dispatcher() {
    struct Dispatcher {
        calls: usize,
    }
    impl VmHostDispatcher for Dispatcher {
        fn invoke(
            &mut self,
            vm: &mut Vm,
            _: &(dyn std::any::Any + Send + Sync),
            args: &[Value],
        ) -> Result<Vec<Value>> {
            let object =
                vm.checked_host_object(args[0], TypeId::of::<Counter>(), type_name::<Counter>())?;
            if args.len() > 2 {
                return Err(object_error("at most one owned argument".to_owned()));
            }
            self.calls += 1;
            let counter = object.try_borrow::<Counter>()?;
            Ok(vec![Value::number(
                counter.0 as f64 + args.get(1).and_then(|v| v.as_number()).unwrap_or(0.0),
            )])
        }
    }
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let mut definition = definition(0.0);
    definition.hooks.call = Some(HostFunctionOp::dispatched(
        "invoke",
        3,
        OpCapability::ReadsExternalMutableState,
        Arc::new(()),
    ));
    vm.register_host_object_type(definition).unwrap();
    let object = vm.create_retained_host_object(Counter(7)).unwrap();
    let mut dispatcher = Dispatcher { calls: 0 };
    let mut execution = vm.begin_execution();
    for (args, expected) in [(Vec::new(), 7.0), (vec![Value::number(4.0)], 11.0)] {
        let result = vm
            .call_retained_function_in(
                object.value(&vm).unwrap(),
                &args,
                &mut execution,
                &mut dispatcher,
            )
            .unwrap();
        assert_eq!(result[0].as_number(), Some(expected));
    }
    assert!(vm
        .call_retained_function_in(
            object.value(&vm).unwrap(),
            &[Value::nil(), Value::nil()],
            &mut execution,
            &mut dispatcher
        )
        .is_err());
    assert_eq!(dispatcher.calls, 2);
}

#[test]
fn hook_storage_is_accounted_and_failed_registration_preserves_old_definition() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.register_host_object_type(definition(1.0)).unwrap();
    let object = vm.create_retained_host_object(Counter(10)).unwrap();
    let before = vm.current_live_bytes();
    let mut replacement = definition(2.0);
    let hook = HostFunctionOp::new(
        "index".repeat(1024),
        1,
        OpCapability::Pure,
        Arc::new(|_, _| Ok(vec![Value::nil()])),
    );
    replacement.hooks.index = Some(hook);
    vm.settings.memory_limit_bytes = Some(before);
    assert!(vm.register_host_object_type(replacement.clone()).is_err());
    vm.settings.memory_limit_bytes = None;
    assert_eq!(call(&mut vm, &object), 11.0);
    let after_failure = vm.create_retained_host_object(Counter(10)).unwrap();
    assert_eq!(call(&mut vm, &after_failure), 11.0);
    vm.register_host_object_type(replacement).unwrap();
    assert!(
        vm.current_live_bytes() > before + 5 * 1024,
        "hook names and op metadata contribute to the census"
    );
    assert_eq!(call(&mut vm, &object), 11.0);
    let newest = vm.create_retained_host_object(Counter(10)).unwrap();
    assert_eq!(call(&mut vm, &newest), 12.0);
}
