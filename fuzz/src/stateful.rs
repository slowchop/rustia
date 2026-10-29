use rustia::{
    Error, Exports, Function, HostFunction, ModuleSource, NewObject, ObjectType, Profile, Rustia,
    RustiaBuilder, ServiceModuleResolver, VmErrorKind,
};

struct ObjectPayload(i64);

fn object_type(scale: i64) -> ObjectType<ObjectPayload> {
    ObjectType::new("StatefulObject")
        .method("value", 1, move |value: &ObjectPayload| value.0 * scale)
        .method("copy", 1, |value: &ObjectPayload| {
            NewObject::new(ObjectPayload(value.0))
        })
}

pub const MAX_STATEFUL_INPUT_BYTES: usize = 128;

struct RetainedExports {
    exports: Exports,
    version: usize,
    epoch: u64,
}

struct RetainedFunction {
    function: Function,
    version: usize,
    epoch: u64,
}

fn assert_stale(error: Error, retained_epoch: u64, current_epoch: u64) {
    assert!(
        matches!(
            error.as_vm_error().map(|error| error.kind()),
            Some(VmErrorKind::StaleRetainedValue {
                retained_epoch: actual_retained,
                current_epoch: actual_current,
            }) if *actual_retained == retained_epoch && *actual_current == current_epoch
        ),
        "expected epochs {retained_epoch}/{current_epoch}: {error:?}"
    );
}

fn verify_exports(vm: &mut Rustia, retained: &RetainedExports, expected: i64, epoch: u64) {
    assert_eq!(retained.epoch, epoch);
    assert_eq!(retained.exports.decision_epoch(), epoch);
    assert_eq!(
        retained.exports.get::<i64>(vm, "version").unwrap(),
        retained.version as i64 + 1
    );
    let state = retained
        .exports
        .get::<rustia::OwnedTable>(vm, "state")
        .unwrap();
    assert_eq!(state.get_string_as::<i64>("value").unwrap(), Some(expected));
    let read = retained.exports.function(vm, "read").unwrap();
    assert_eq!(
        read.call::<_, (i64, i64, i64)>(vm, ()).unwrap(),
        (retained.version as i64 + 1, expected, expected)
    );
}

fn verify_function(vm: &mut Rustia, retained: &RetainedFunction, expected: i64, epoch: u64) {
    assert_eq!(retained.epoch, epoch);
    assert_eq!(retained.function.decision_epoch(), epoch);
    assert_eq!(
        retained
            .function
            .call::<_, (i64, i64, i64)>(vm, ())
            .unwrap(),
        (retained.version as i64 + 1, expected, expected)
    );
}

/// Each byte selects an operation; a host-side model tracks decision-local state.
/// Sources are bounded, with a fuel-limited loop testing hard-abort recovery.
/// Retained handles and reports are bounded too.
pub fn fuzz_stateful(data: &[u8]) -> bool {
    if data.len() > MAX_STATEFUL_INPUT_BYTES {
        return false;
    }
    let profile = match data.first().copied().unwrap_or(0) % 4 {
        0 => Profile::LuauCompat,
        1 => Profile::ReproducibleStandard,
        2 => Profile::DeterministicSim,
        _ => Profile::Puc51Compat,
    };
    let mut vm = RustiaBuilder::standard()
        .profile(profile)
        .fuel(4096)
        .memory_limit_bytes(2 * 1024 * 1024)
        .module_resolver(ServiceModuleResolver::from_fn(|_, _| {
            Ok(ModuleSource::new(
                "stateful-module",
                "local value = coroutine.yield('module'); return { value = value }",
            ))
        }))
        .build()
        .unwrap();
    let mut expected = 0;
    vm.insert_global("counter", expected).unwrap();
    vm.exec("shared = { value = 7 }").unwrap();
    vm.register_function(
        "host_fail",
        HostFunction::pure(|| -> rustia::Result<()> {
            Err(Error::embed("deliberate host failure"))
        }),
    )
    .unwrap();
    let mut object_scale = 1;
    vm.register_object_type(object_type(object_scale)).unwrap();
    let mut object: Option<(rustia::Object<ObjectPayload>, i64, i64)> = None;
    let mut retained = None;
    let mut module_loading = false;
    let mut module_answer = None;
    let mut current: Option<RetainedExports> = None;
    let mut previous: Option<RetainedExports> = None;
    let mut detached: Option<RetainedFunction> = None;
    let mut stale_exports: Option<RetainedExports> = None;
    let mut stale_function: Option<RetainedFunction> = None;
    let mut captured_values = [0i64; MAX_STATEFUL_INPUT_BYTES];
    let mut epoch = 0;
    assert_eq!(vm.decision_epoch(), epoch);
    for (index, operation) in data.iter().enumerate() {
        match operation % 25 {
            0 => {
                vm.exec("counter = counter + 1").unwrap();
                expected += 1;
            }
            1 => {
                let error = vm
                    .load("counter = counter + 1; error('stateful', 0)")
                    .source_name("stateful.luau")
                    .exec()
                    .unwrap_err();
                expected += 1;
                let report = error.report().to_string();
                retained = Some((error, report));
            }
            2 => {
                vm.load("shared.value = 999; counter = 999; error('candidate', 0)")
                    .install()
                    .unwrap_err();
            }
            3 => {
                let old = vm.create_table().unwrap();
                let old_epoch = epoch;
                let old_exports = [current.take(), previous.take()];
                let old_export_handles: Vec<_> = old_exports
                    .iter()
                    .flatten()
                    .map(|retained| {
                        let function = RetainedFunction {
                            function: retained.exports.function(&mut vm, "read").unwrap(),
                            version: retained.version,
                            epoch: retained.epoch,
                        };
                        let value = retained.exports.as_value();
                        let state = retained.exports.get_live(&mut vm, "state").unwrap();
                        let table = state.as_table(&vm).unwrap().unwrap();
                        (function, value, state, table)
                    })
                    .collect();
                let old_detached = detached.take();
                vm.reset_decision().unwrap();
                epoch += 1;
                assert_eq!(vm.decision_epoch(), epoch);
                assert_stale(old.get(&mut vm, "anything").unwrap_err(), old_epoch, epoch);
                if let Some((old, _, _)) = object.take() {
                    assert_stale(
                        old.call_method::<_, i64>(&mut vm, "value", ()).unwrap_err(),
                        old_epoch,
                        epoch,
                    );
                }
                for retained in old_exports.iter().flatten() {
                    assert_stale(
                        retained.exports.get::<i64>(&mut vm, "version").unwrap_err(),
                        retained.epoch,
                        epoch,
                    );
                    assert_stale(
                        retained.exports.function(&mut vm, "read").unwrap_err(),
                        retained.epoch,
                        epoch,
                    );
                }
                for (retained, value, state, table) in &old_export_handles {
                    assert_stale(
                        retained
                            .function
                            .call::<_, (i64, i64, i64)>(&mut vm, ())
                            .unwrap_err(),
                        retained.epoch,
                        epoch,
                    );
                    assert_stale(value.type_name(&vm).unwrap_err(), retained.epoch, epoch);
                    assert_stale(state.type_name(&vm).unwrap_err(), retained.epoch, epoch);
                    assert_stale(
                        table.get(&mut vm, "value").unwrap_err(),
                        retained.epoch,
                        epoch,
                    );
                }
                if let Some(retained) = old_detached {
                    assert_stale(
                        retained
                            .function
                            .call::<_, (i64, i64, i64)>(&mut vm, ())
                            .unwrap_err(),
                        retained.epoch,
                        epoch,
                    );
                    stale_function = Some(retained);
                } else if let Some((retained, _, _, _)) = old_export_handles.into_iter().next() {
                    stale_function = Some(retained);
                }
                if let Some(retained) = old_exports.into_iter().flatten().next() {
                    stale_exports = Some(retained);
                }
                expected = 0;
                vm.insert_global("counter", expected).unwrap();
                vm.exec("shared = { value = 7 }").unwrap();
                module_loading = false;
                module_answer = None;
            }
            4 => {
                assert!(vm.eval::<bool>("local t = {}; local ok, e = pcall(error, t); return not ok and rawequal(t, e)").unwrap());
            }
            5 => {
                vm.exec("counter = 999; local function bad() local x = end")
                    .unwrap_err();
            }
            6 => {
                assert!(vm
                    .eval::<bool>(
                        "counter = counter + 1; local ok = pcall(host_fail); return not ok"
                    )
                    .unwrap());
                expected += 1;
            }
            7 => {
                let error = vm
                    .exec("pcall(function() while true do end end)")
                    .unwrap_err();
                assert!(matches!(
                    error.as_vm_error().map(|e| e.kind()),
                    Some(VmErrorKind::FuelExhausted { .. })
                ));
            }
            8 => {
                let error = vm
                    .load("counter = counter + 1; return {}")
                    .eval::<i32>()
                    .unwrap_err();
                assert!(matches!(error.kind(), rustia::ErrorKind::Embed(_)));
                expected += 1;
            }
            9 => {
                let table = vm.create_table().unwrap();
                let value = table.as_value();
                table.set_live(&mut vm, "self", &value).unwrap();
                let alias = table.get(&mut vm, "self").unwrap();
                assert!(alias.raw_equal(&mut vm, &value).unwrap());
                vm.exec("host_fail()").unwrap_err();
                assert!(alias.raw_equal(&mut vm, &value).unwrap());
            }
            10 => {
                let payload = i64::from(*operation);
                let value = vm.create_object(ObjectPayload(payload)).unwrap();
                vm.globals()
                    .unwrap()
                    .set_live(&mut vm, "object", &value.as_value())
                    .unwrap();
                vm.exec("object_alias = object").unwrap();
                object = Some((value, payload, object_scale));
            }
            11 => {
                object_scale = 3 - object_scale;
                vm.register_object_type(object_type(object_scale)).unwrap();
            }
            12 => {
                if let Some((value, payload, scale)) = &object {
                    assert_eq!(
                        value.call_method::<_, i64>(&mut vm, "value", ()).unwrap(),
                        payload * scale
                    );
                    let copy = value.call_method_values(&mut vm, "copy", &[]).unwrap();
                    let copy = copy[0]
                        .as_object::<ObjectPayload>(&mut vm)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        copy.call_method::<_, i64>(&mut vm, "value", ()).unwrap(),
                        payload * object_scale
                    );
                    assert!(vm
                        .eval::<bool>("return rawequal(object, object_alias)")
                        .unwrap());
                }
            }
            13 => {
                if !module_loading {
                    let result = vm
                        .eval::<rustia::OwnedTable>(
                            "module_owner = coroutine.create(function() return require('module') end); return {coroutine.resume(module_owner)}",
                        )
                        .unwrap();
                    assert_eq!(result.get_array_as::<bool>(0).unwrap(), Some(true));
                    if let Some(answer) = module_answer {
                        let value = result
                            .get_array_as::<rustia::OwnedTable>(1)
                            .unwrap()
                            .unwrap();
                        assert_eq!(value.get_string_as::<i64>("value").unwrap(), Some(answer));
                    } else {
                        assert_eq!(
                            result.get_array_as::<String>(1).unwrap().as_deref(),
                            Some("module")
                        );
                        module_loading = true;
                    }
                }
            }
            14 => {
                if module_loading {
                    let answer = i64::from(*operation);
                    vm.insert_global("module_input", answer).unwrap();
                    assert!(vm.eval::<bool>(
                        "local ok, value = coroutine.resume(module_owner, module_input); return ok and value.value == module_input and require('module') == value",
                    ).unwrap());
                    module_loading = false;
                    module_answer = Some(answer);
                }
            }
            15 => {
                if module_loading {
                    vm.exec("assert(coroutine.close(module_owner)); module_owner = nil")
                        .unwrap();
                    module_loading = false;
                }
            }
            16 => {
                if module_loading {
                    vm.load("local ok = coroutine.resume(module_owner, 999); assert(ok); error('rollback loader', 0)")
                        .install()
                        .unwrap_err();
                }
            }
            17 => {
                if module_loading && profile != Profile::DeterministicSim {
                    vm.exec(
                        "module_owner = nil; collectgarbage('collect'); collectgarbage('collect')",
                    )
                    .unwrap();
                    module_loading = false;
                }
            }
            18 => {
                // Each installation closes over a distinct version and mutable
                // value. The host model never derives expectations from reads.
                captured_values[index] = (index as i64 + 1) * 10;
                let source = format!(
                    "local version={}; local value={}; local state={{value=value}}; \
                     local function read(delta) if delta~=nil then value=value+delta; state.value=value end; \
                     return version,value,state.value end; return {{version=version,state=state,read=read}}",
                    index + 1,
                    captured_values[index],
                );
                let exports = vm
                    .load(&source)
                    .source_name("stateful-retained.luau")
                    .install()
                    .unwrap();
                previous = current.take();
                current = Some(RetainedExports {
                    exports,
                    version: index,
                    epoch,
                });
            }
            19 => {
                let target = if let Some(retained) = current.as_ref().or(previous.as_ref()) {
                    Some((
                        retained.exports.function(&mut vm, "read").unwrap(),
                        retained.version,
                    ))
                } else {
                    detached
                        .as_ref()
                        .map(|retained| (retained.function.clone(), retained.version))
                };
                if let Some((target, version)) = target {
                    vm.globals()
                        .unwrap()
                        .set_live(&mut vm, "retained_target", &target.as_value())
                        .unwrap();
                    let source = format!(
                        "local version,value,table_value=retained_target(1000); \
                         assert(version=={} and value=={} and table_value==value); error('retained candidate',0)",
                        version + 1,
                        captured_values[version] + 1000,
                    );
                    let error = vm.load(&source).install().unwrap_err();
                    assert!(
                        matches!(
                            error.as_vm_error().map(|error| error.kind()),
                            Some(VmErrorKind::RuntimeError { message }) if message == "retained candidate"
                        ),
                        "candidate must execute before failing: {error:?}"
                    );
                    vm.globals()
                        .unwrap()
                        .set(&mut vm, "retained_target", None::<i64>)
                        .unwrap();
                }
            }
            20 => {
                let target = if operation & 32 == 0 {
                    current.as_ref().or(previous.as_ref())
                } else {
                    previous.as_ref().or(current.as_ref())
                };
                if let Some(retained) = target {
                    let function = retained.exports.function(&mut vm, "read").unwrap();
                    captured_values[retained.version] += 1;
                    assert_eq!(
                        function.call::<_, (i64, i64, i64)>(&mut vm, (1,)).unwrap(),
                        (
                            retained.version as i64 + 1,
                            captured_values[retained.version],
                            captured_values[retained.version],
                        )
                    );
                    detached = Some(RetainedFunction {
                        function,
                        version: retained.version,
                        epoch,
                    });
                }
            }
            21 => {
                // No guest alias or live Value/Table survives the failed-install
                // operation. A detached function can now be the only live root.
                current = None;
                previous = None;
            }
            22 => {
                if let Some(retained) = current.as_ref().or(previous.as_ref()) {
                    previous = Some(RetainedExports {
                        exports: retained.exports.clone(),
                        version: retained.version,
                        epoch,
                    });
                }
            }
            23 => {
                // Two MiB of buffers plus the VM baseline exceed the quota.
                // A weak witness proves collection in every profile.
                vm.exec(
                    "local weak=setmetatable({}, {__mode='v'}); local witness={}; \
                     weak[1]=witness; assert(weak[1]==witness); witness=nil; \
                     for i=1,8 do local pressure=buffer.create(262144) end; \
                     if collectgarbage then collectgarbage('collect') end; assert(weak[1]==nil)",
                )
                .unwrap();
            }
            _ => detached = None,
        }
        assert_eq!(vm.eval::<i32>("return shared.value").unwrap(), 7);
        if let Some((value, payload, scale)) = &object {
            assert_eq!(
                value.call_method::<_, i64>(&mut vm, "value", ()).unwrap(),
                payload * scale
            );
        }
        assert_eq!(
            vm.eval::<i32>("return counter")
                .unwrap_or_else(|error| panic!(
                    "profile {profile:?}, operations {:?}: {error:?}",
                    &data[..=index]
                )),
            expected
        );
        if let Some((error, report)) = &retained {
            assert_eq!(&error.report().to_string(), report);
        }
        for retained in current.iter().chain(previous.iter()) {
            verify_exports(&mut vm, retained, captured_values[retained.version], epoch);
        }
        if let Some(retained) = &detached {
            verify_function(&mut vm, retained, captured_values[retained.version], epoch);
        }
        if let Some(retained) = &stale_exports {
            assert_stale(
                retained.exports.get::<i64>(&mut vm, "version").unwrap_err(),
                retained.epoch,
                epoch,
            );
        }
        if let Some(retained) = &stale_function {
            assert_stale(
                retained
                    .function
                    .call::<_, (i64, i64, i64)>(&mut vm, ())
                    .unwrap_err(),
                retained.epoch,
                epoch,
            );
        }
    }
    drop(vm);
    if let Some((error, report)) = retained {
        assert_eq!(error.report().to_string(), report);
    }
    true
}
