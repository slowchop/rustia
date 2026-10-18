use rustia::{Error, HostFunction, NewObject, ObjectType, Profile, RustiaBuilder, VmErrorKind};

struct ObjectPayload(i64);

fn object_type(scale: i64) -> ObjectType<ObjectPayload> {
    ObjectType::new("StatefulObject")
        .method("value", 1, move |value: &ObjectPayload| value.0 * scale)
        .method("copy", 1, |value: &ObjectPayload| {
            NewObject::new(ObjectPayload(value.0))
        })
}

pub const MAX_STATEFUL_INPUT_BYTES: usize = 128;

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
    for (index, operation) in data.iter().enumerate() {
        match operation % 13 {
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
                vm.reset_decision().unwrap();
                assert!(old.get(&mut vm, "anything").is_err());
                if let Some((old, _, _)) = object.take() {
                    assert!(old.call_method::<_, i64>(&mut vm, "value", ()).is_err());
                }
                expected = 0;
                vm.insert_global("counter", expected).unwrap();
                vm.exec("shared = { value = 7 }").unwrap();
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
            _ => {
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
    }
    drop(vm);
    if let Some((error, report)) = retained {
        assert_eq!(error.report().to_string(), report);
    }
    true
}
