use rustia::{Profile, RustiaBuilder};

pub const MAX_STATEFUL_INPUT_BYTES: usize = 128;

/// Each byte selects an operation; a host-side model tracks decision-local state.
/// Sources are bounded and loop-free. Retained handles/reports are bounded too.
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
    let mut retained = None;
    for (index, operation) in data.iter().enumerate() {
        match operation % 6 {
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
                vm.load("counter = 999; error('candidate', 0)")
                    .install()
                    .unwrap_err();
            }
            3 => {
                let old = vm.create_table().unwrap();
                vm.reset_decision().unwrap();
                assert!(old.get(&mut vm, "anything").is_err());
                expected = 0;
                vm.insert_global("counter", expected).unwrap();
            }
            4 => {
                assert!(vm.eval::<bool>("local t = {}; local ok, e = pcall(error, t); return not ok and rawequal(t, e)").unwrap());
            }
            _ => {
                vm.exec("counter = 999; local function bad() local x = end")
                    .unwrap_err();
            }
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
