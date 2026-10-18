use rustia::{Error, HostFunction, Profile, RustiaBuilder, VmErrorKind};

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
    let mut retained = None;
    for (index, operation) in data.iter().enumerate() {
        match operation % 10 {
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
            _ => {
                let table = vm.create_table().unwrap();
                let value = table.as_value();
                table.set_live(&mut vm, "self", &value).unwrap();
                let alias = table.get(&mut vm, "self").unwrap();
                assert!(alias.raw_equal(&mut vm, &value).unwrap());
                vm.exec("host_fail()").unwrap_err();
                assert!(alias.raw_equal(&mut vm, &value).unwrap());
            }
        }
        assert_eq!(vm.eval::<i32>("return shared.value").unwrap(), 7);
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
