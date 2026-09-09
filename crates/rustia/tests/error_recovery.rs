//! Observable failure/recovery contracts, independent of report wording.
use rustia::{ErrorKind, Profile, Rustia, RustiaBuilder};

#[test]
fn builder_profile_globals_can_be_rebound_after_reset() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard()
            .profile(profile)
            .fuel(4096)
            .memory_limit_bytes(2 * 1024 * 1024)
            .build()
            .unwrap();
        vm.insert_global("counter", 0).unwrap();
        assert_eq!(
            vm.eval::<i32>("return counter")
                .unwrap_or_else(|e| panic!("{profile:?}: {e:?}")),
            0
        );
        let old = vm.create_table().unwrap();
        vm.reset_decision().unwrap();
        assert!(old.get(&mut vm, "anything").is_err());
        vm.insert_global("counter", 42).unwrap();
        assert_eq!(vm.eval::<i32>("return counter").unwrap(), 42, "{profile:?}");
    }
}

fn profiles() -> [Profile; 4] {
    [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ]
}

#[test]
fn invalid_uncalled_function_prevents_all_execution() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        vm.insert_global("seen", 0).unwrap();
        let error = vm
            .load("seen = 1; local function unused() local x = end")
            .source_name("invalid-function.luau")
            .exec()
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Source(_)));
        assert_eq!(vm.eval::<i32>("return seen").unwrap(), 0);
        assert_eq!(vm.eval::<i32>("return 6 * 7").unwrap(), 42);
    }
}

#[test]
fn ordinary_failure_keeps_prior_mutations_and_vm_remains_usable() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        vm.insert_global("seen", 0).unwrap();
        let error = vm
            .load("seen = seen + 1; error('deliberate', 0)")
            .source_name("ordinary-failure.luau")
            .exec()
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Vm(_)));
        assert_eq!(vm.eval::<i32>("return seen").unwrap(), 1);
        assert_eq!(vm.eval::<i32>("return 6 * 7").unwrap(), 42);
    }
}

#[test]
fn protected_errors_preserve_object_identity_and_close_captured_locals() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        let ok = vm
            .eval::<bool>(
                r#"
            local token = {}
            local saved
            local ok, caught = pcall(function()
                local n = 41
                saved = function() n = n + 1; return n end
                error(token)
            end)
            return not ok and rawequal(caught, token) and saved() == 42 and saved() == 43
        "#,
            )
            .unwrap();
        assert!(ok, "profile {profile:?}");
        assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
    }
}

#[test]
fn retained_reports_survive_reset_without_reviving_handles() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        let table = vm.create_table().unwrap();
        let error = vm
            .load("local missing = nil; return missing.health")
            .source_name("retained-error.luau")
            .exec()
            .unwrap_err();
        let before = error.report().to_string();
        assert!(before.contains("retained-error.luau"));
        vm.reset_decision().unwrap();
        assert!(table.get(&mut vm, "health").is_err());
        assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
        drop(vm);
        assert_eq!(error.report().to_string(), before);
    }
}
