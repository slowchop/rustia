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
fn optimized_protected_failure_and_recovery_match_unoptimized_execution() {
    use rustia_compiler::{compile_with_options, CompileOptions, ExperimentalO2Options};
    let source = r#"
        local token = {}
        local saved
        local ok, caught = pcall(function()
            local n = 41
            saved = function() n = n + 1; return n end
            if true then error(token) end
            n = 999
        end)
        return not ok and rawequal(token, caught) and saved() == 42
    "#;
    let chunk = rustia_parser::parse_source_bytes(source.as_bytes()).unwrap();
    for profile in profiles() {
        for experimental_o2 in [
            ExperimentalO2Options::default(),
            ExperimentalO2Options::reachability_exact(),
        ] {
            let program = compile_with_options(
                &chunk,
                CompileOptions {
                    experimental_o2,
                    puc51_constructor_order: profile == Profile::Puc51Compat,
                    ..CompileOptions::default()
                }
                .with_source("optimized-errors.luau", source),
            )
            .unwrap();
            let mut vm = rustia_vm::Vm::with_settings(
                profile,
                rustia_vm::VmSettings {
                    initial_fuel: Some(10_000),
                    memory_limit_bytes: Some(2 * 1024 * 1024),
                    ..rustia_vm::VmSettings::default()
                },
            );
            assert_eq!(vm.run(&program).unwrap().as_bool(), Some(true));
            assert_eq!(vm.run(&program).unwrap().as_bool(), Some(true));
            vm.reset_decision().unwrap();
            assert_eq!(vm.run(&program).unwrap().as_bool(), Some(true));
        }
    }
}

#[test]
fn compile_rejection_in_uncalled_function_precedes_side_effects() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        vm.insert_global("seen", 0).unwrap();
        let error = vm
            .exec("seen = 1; local function unused() break end")
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Compile(_)), "{error:?}");
        assert_eq!(vm.eval::<i32>("return seen").unwrap(), 0);
    }
}

#[test]
fn owned_conversion_failure_does_not_roll_back_guest_execution() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        let error = vm
            .load("seen = 42; return {}")
            .source_name("conversion.luau")
            .eval::<i32>()
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Embed(_)));
        assert!(error.report().to_string().contains("returned value"));
        assert_eq!(vm.eval::<i32>("return seen").unwrap(), 42);
    }
}

#[test]
fn coroutine_failure_closes_upvalues_and_preserves_error_identity() {
    for profile in profiles() {
        let mut vm = Rustia::new(profile);
        assert!(
            vm.eval::<bool>(
                r#"
            local token = {}
            local saved
            local worker = coroutine.create(function()
                local n = 41
                saved = function() n = n + 1; return n end
                coroutine.yield('pause')
                error(token)
            end)
            local first, message = coroutine.resume(worker)
            local second, caught = coroutine.resume(worker)
            return first and message == 'pause' and not second
                and rawequal(caught, token) and saved() == 42
                and coroutine.status(worker) == 'dead'
        "#
            )
            .unwrap(),
            "{profile:?}"
        );
        assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
    }
}

#[test]
fn ordinary_quota_oom_is_catchable_and_reset_recovers() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard()
            .profile(profile)
            .memory_limit_bytes(1024 * 1024)
            .fuel(10_000_000)
            .build()
            .unwrap();
        let error = vm.exec("return string.rep('x', 2000000)").unwrap_err();
        assert!(
            matches!(
                error.as_vm_error().map(|error| &error.kind),
                Some(rustia::VmErrorKind::OutOfMemory)
            ),
            "{error:?}"
        );
        assert!(
            vm.eval::<bool>(
                "local ok = pcall(function() return string.rep('x', 2000000) end); return not ok"
            )
            .unwrap(),
            "{profile:?}"
        );
        assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
        vm.reset_decision().unwrap();
        assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
    }
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
