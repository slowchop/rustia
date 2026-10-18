use super::*;

fn pending(vm: &mut Vm, identity: &str, owner: Option<u32>) -> SuspendedRequireFrame {
    let load = ModuleLoad {
        identity: identity.to_owned(),
        owner,
    };
    vm.module_cohorts
        .get_mut(&0)
        .unwrap()
        .loading
        .push(load.clone());
    vm.mark_control_memory_dirty();
    SuspendedRequireFrame { cohort: 0, load }
}

fn empty_program() -> Program {
    Program::new(
        Function::new(vec![Instruction::new(Opcode::Return, 0, 0, 0)], 1),
        Vec::new(),
    )
}

#[test]
fn completion_caches_zero_or_one_result_and_releases_failed_results() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let frame = pending(&mut vm, "empty", None);
    assert!(vm
        .finish_required_module(frame, Vec::new())
        .unwrap()
        .is_empty());
    assert!(matches!(
        vm.module_cohorts[&0].cache.get("empty"),
        Some(None)
    ));
    let frame = pending(&mut vm, "value", None);
    let result = vm
        .finish_required_module(frame, vec![Value::number(42.0)])
        .unwrap();
    assert_eq!(result[0].as_number(), Some(42.0));
    assert_eq!(
        vm.module_cohorts[&0].cache["value"].unwrap().as_number(),
        Some(42.0)
    );
    let frame = pending(&mut vm, "many", None);
    let error = vm
        .finish_required_module(frame, vec![Value::nil(), Value::nil()])
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("module must return a single value"));
    assert!(!vm.module_cohorts[&0].cache.contains_key("many"));
    assert!(vm.module_cohorts[&0].loading.is_empty());
}

#[test]
fn wrong_owner_cannot_complete_or_release_another_load() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let mut frame = pending(&mut vm, "shared", Some(11));
    frame.load.owner = Some(12);
    vm.release_module_load(&frame);
    assert!(matches!(
        vm.finish_required_module(frame, vec![Value::number(42.0)])
            .unwrap_err()
            .kind,
        ErrorKind::InvalidProgram { .. }
    ));
    assert_eq!(vm.module_cohorts[&0].loading.len(), 1);
    assert_eq!(vm.module_cohorts[&0].loading[0].owner, Some(11));
    assert!(vm.module_cohorts[&0].cache.is_empty());
}

#[test]
fn cycle_and_other_owner_refusal_leave_loading_ownership_unchanged() {
    let program = empty_program();
    let resolved_program = program.clone();
    let mut vm = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            module_resolver: Some(ModuleResolver::new(
                move |_, _| Ok(ResolvedModule::new("a", resolved_program.clone())),
                true,
            )),
            ..VmSettings::default()
        },
    );
    pending(&mut vm, "a", None);
    pending(&mut vm, "b", None);
    let requested = vm.string("a");
    let mut state = vm.begin_execution();
    let mut hosts = RejectHostDispatcher;
    let mut execution = Execution {
        state: &mut state,
        hosts: &mut hosts,
    };
    let error = match vm.native_require_resumable(&[requested], &program, 0, None, &mut execution) {
        Err(error) => error,
        Ok(_) => panic!("cycle must fail"),
    };
    assert!(error.to_string().contains("a -> b -> a"));
    vm.module_cohorts.get_mut(&0).unwrap().loading[0].owner = Some(11);
    let error = match vm.native_require_resumable(&[requested], &program, 0, None, &mut execution) {
        Err(error) => error,
        Ok(_) => panic!("another owner's load must fail"),
    };
    assert!(error.to_string().contains("another coroutine"));
    assert_eq!(vm.module_cohorts[&0].loading.len(), 2);
    assert_eq!(vm.module_cohorts[&0].loading[0].owner, Some(11));
}

#[test]
fn cache_admission_oom_releases_ownership_and_allows_retry() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let frame = pending(&mut vm, "oom", None);
    vm.settings.memory_limit_bytes = Some(vm.current_live_bytes());
    assert!(matches!(
        vm.finish_required_module(frame, vec![Value::number(42.0)])
            .unwrap_err()
            .kind,
        ErrorKind::OutOfMemory
    ));
    assert!(vm.module_cohorts[&0].loading.is_empty());
    assert!(!vm.module_cohorts[&0].cache.contains_key("oom"));
    vm.settings.memory_limit_bytes = None;
    let retry = pending(&mut vm, "oom", None);
    assert_eq!(
        vm.finish_required_module(retry, vec![Value::number(42.0)])
            .unwrap()[0]
            .as_number(),
        Some(42.0)
    );
}

#[test]
fn transaction_snapshot_restores_pending_load_and_removes_provisional_cache() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let frame = pending(&mut vm, "pending", Some(11));
    vm.begin_guest_transaction().unwrap();
    vm.finish_required_module(frame, vec![Value::number(42.0)])
        .unwrap();
    assert!(vm.module_cohorts[&0].loading.is_empty());
    vm.rollback_guest_transaction();
    assert!(!vm.module_cohorts[&0].cache.contains_key("pending"));
    assert_eq!(vm.module_cohorts[&0].loading[0].identity, "pending");
    assert_eq!(vm.module_cohorts[&0].loading[0].owner, Some(11));
}

#[test]
fn suspended_require_frame_clone_and_memory_are_independent() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let frame = CoroutineFrame::Require(pending(&mut vm, "owned identity", Some(11)));
    let mut copy = try_clone_coroutine_frame(&frame).unwrap();
    let CoroutineFrame::Require(ref mut copy_frame) = copy else {
        panic!("wrong frame");
    };
    copy_frame.load.identity.push_str(" changed");
    let CoroutineFrame::Require(ref original) = frame else {
        panic!("wrong frame");
    };
    assert_eq!(original.load.identity, "owned identity");
    let mut usage = MemoryUsage::default();
    add_coroutine_frame_memory(&frame, &mut usage);
    assert_eq!(usage.live_bytes, "owned identity".len());
    shrink_coroutine_frame_capacity(&mut copy);
    let mut roots = Vec::new();
    vm.extend_coroutine_frame_roots(&mut roots, &frame);
    assert!(roots.is_empty(), "owner identity must not become a GC root");
}

#[test]
fn hard_error_unwind_releases_all_pending_require_frames() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let outer = pending(&mut vm, "outer", Some(11));
    let inner = pending(&mut vm, "inner", Some(11));
    let mut state = vm.begin_execution();
    let mut hosts = RejectHostDispatcher;
    let mut execution = Execution {
        state: &mut state,
        hosts: &mut hosts,
    };
    let result = vm.propagate_coroutine_error(
        vec![
            CoroutineFrame::Require(inner),
            CoroutineFrame::PCall,
            CoroutineFrame::Require(outer),
        ],
        Error::new(ErrorKind::FuelExhausted { limit: 10 }),
        &mut execution,
    );
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::FuelExhausted { limit: 10 },
            ..
        })
    ));
    assert!(vm.module_cohorts[&0].loading.is_empty());
}

#[test]
fn rejected_suspension_cleans_loads_before_discarding_frames() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let load = pending(&mut vm, "lost", None);
    let globals = vm.globals;
    let context = FunctionCallContext {
        current_function: None,
        captures: &[],
        depth: 0,
        environment: globals,
        thread_environment: globals,
        module_cohort: 0,
        call_site: None,
        prototype: None,
    };
    let result = vm.suspend_bytecode_frame(
        None,
        &context,
        0,
        RegisterValues::new(),
        RegisterCells::new(),
        Vec::new(),
        Vec::new(),
        BytecodeContinuation::WriteFirst { register: 0 },
        Vec::new(),
        0,
        CoroutineYield {
            yielded: Vec::new(),
            frames: vec![CoroutineFrame::Require(load)],
        },
    );
    assert!(matches!(
        result,
        Err(Error {
            kind: ErrorKind::InvalidProgram { .. },
            ..
        })
    ));
    assert!(vm.module_cohorts[&0].loading.is_empty());
}

#[test]
fn active_require_continuation_keeps_its_cache_cohort_and_values_alive() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    vm.begin_guest_transaction().unwrap();
    let cohort = vm.create_module_cohort().unwrap();
    vm.commit_guest_transaction().unwrap();
    let table = vm.empty_table();
    vm.module_cohorts
        .get_mut(&cohort)
        .unwrap()
        .cache
        .insert("completed".to_owned(), Some(table));
    vm.active_coroutine_frames
        .push(vec![CoroutineFrame::Require(SuspendedRequireFrame {
            cohort,
            load: ModuleLoad {
                identity: "pending".to_owned(),
                owner: None,
            },
        })]);
    vm.mark_control_memory_dirty();
    vm.collect_heap_and_programs(&[], true);
    assert!(vm.module_cohorts.contains_key(&cohort));
    assert!(vm.heap.has_table(table.as_table().unwrap()));
    vm.active_coroutine_frames.pop();
    vm.collect_garbage();
    assert!(!vm.module_cohorts.contains_key(&cohort));
    assert!(!vm.heap.has_table(table.as_table().unwrap()));
}

#[test]
fn collection_releases_abandoned_loads_while_other_frames_are_active() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let owner = vm.heap.alloc_userdata(None).unwrap();
    pending(&mut vm, "abandoned", Some(owner));
    vm.active_coroutine_frames.push(vec![CoroutineFrame::PCall]);
    vm.mark_control_memory_dirty();
    vm.collect_heap_and_programs(&[], true);
    assert!(!vm.heap.has_userdata(owner));
    assert!(vm.module_cohorts[&0].loading.is_empty());
    vm.active_coroutine_frames.pop();
}

#[test]
fn snapshot_quota_rejection_preserves_pending_ownership_without_a_savepoint() {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    pending(&mut vm, &"identity".repeat(1024), None);
    vm.settings.memory_limit_bytes = Some(vm.current_live_bytes() + 1);
    assert!(matches!(
        vm.begin_guest_transaction().unwrap_err().kind,
        ErrorKind::OutOfMemory
    ));
    assert!(vm.guest_transactions.is_empty());
    assert_eq!(vm.module_cohorts[&0].loading.len(), 1);
    assert_eq!(vm.module_cohorts[&0].loading[0].owner, None);
}
