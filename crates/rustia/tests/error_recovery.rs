//! Observable failure/recovery contracts, independent of report wording.
use rustia::{ErrorKind, Profile, RustiaBuilder};

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
fn configured_recursion_exhaustion_escapes_protected_calls() {
    use rustia::VmErrorKind;
    for profile in profiles() {
        for body in [
            "local function recur() return 1+recur() end; pcall(recur)",
            "local function recur() return string.len('abc',recur()) end; pcall(recur)",
            "local function recur() return string.len('abc',recur()) end; xpcall(recur,function() observed=3 end)",
        ] {
            let mut vm = RustiaBuilder::standard().profile(profile).recursion_limit(8)
                .fuel(100_000).build().unwrap();
            vm.insert_global("observed", 0).unwrap();
            let source = format!("observed=1; {body}; observed=2; return 'caught'");
            let error = vm.eval::<String>(&source).unwrap_err();
            assert!(matches!(error.kind(), ErrorKind::Vm(error)
                if matches!(error.kind(), VmErrorKind::RecursionLimit { limit: 8 })),
                "{profile:?}: {source}: {error}");
            assert_eq!(vm.eval::<i32>("return observed").unwrap(), 1);
            vm.reset_decision().unwrap();
            assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
        }
    }
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
    for profile in [
        rustia_vm::Profile::LuauCompat,
        rustia_vm::Profile::ReproducibleStandard,
        rustia_vm::Profile::DeterministicSim,
        rustia_vm::Profile::Puc51Compat,
    ] {
        for experimental_o2 in [
            ExperimentalO2Options::default(),
            ExperimentalO2Options::reachability_exact(),
        ] {
            let program = compile_with_options(
                &chunk,
                CompileOptions {
                    experimental_o2,
                    puc51_constructor_order: profile == rustia_vm::Profile::Puc51Compat,
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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
                error.as_vm_error().map(|error| error.kind()),
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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
        let mut vm = rustia::RustiaBuilder::standard()
            .profile(profile)
            .build()
            .unwrap();
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

#[test]
fn quota_oom_releases_failed_call_roots_before_the_next_allocating_call() {
    use rustia::{InvocationLimits, OwnedValue, VmErrorKind};
    for profile in profiles() {
        for transactional in [false, true] {
            let mut vm = RustiaBuilder::standard()
                .profile(profile)
                .without_stdout()
                .fuel(50_000_000)
                .memory_limit_bytes(1024 * 1024)
                .build()
                .unwrap();
            let exports = vm
                .load(
                    r#"return {
    bomb = function()
        local t, i = {}, 0
        while true do
            i = i + 1
            t[i] = { i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i }
        end
    end,
    synth = function(note) return { kind = 'sine', freq = note } end,
}"#,
                )
                .source_name("oom-recovery.luau")
                .install()
                .unwrap();
            let bomb = exports.function(&mut vm, "bomb").unwrap();
            let synth = exports.function(&mut vm, "synth").unwrap();
            for _ in 0..2 {
                let error = if transactional {
                    vm.invoke(&mut (), InvocationLimits::new(50_000_000), |invocation| {
                        bomb.call_in::<_, _, ()>(invocation, ())
                    })
                    .unwrap_err()
                } else {
                    bomb.call::<_, ()>(&mut vm, ()).unwrap_err()
                };
                assert!(
                    matches!(
                        error.as_vm_error().map(|e| e.kind()),
                        Some(VmErrorKind::OutOfMemory)
                    ),
                    "{profile:?}: {error}"
                );
                assert_eq!(
                    error.as_vm_error().unwrap().to_string(),
                    "not enough memory"
                );
                let report = error.report().to_string();
                assert!(
                    report.contains("oom-recovery.luau:6"),
                    "{profile:?}: {report}"
                );
                for _ in 0..3 {
                    let result: OwnedValue = if transactional {
                        vm.invoke(&mut (), InvocationLimits::new(1_000_000), |invocation| {
                            synth.call_in(invocation, 60)
                        })
                        .unwrap_or_else(|e| panic!("{profile:?}, transactional: {e}"))
                        .into_parts()
                        .0
                    } else {
                        synth
                            .call(&mut vm, 60)
                            .unwrap_or_else(|e| panic!("{profile:?}, ordinary: {e}"))
                    };
                    assert!(matches!(result, OwnedValue::Table(_)));
                }
            }
        }
    }
}

#[test]
fn quota_oom_chunk_recovery_preserves_published_values_and_captured_locals() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard()
            .profile(profile)
            .without_stdout()
            .fuel(50_000_000)
            .memory_limit_bytes(1024 * 1024)
            .build()
            .unwrap();
        let retained = vm.load("return { answer = 42 }").eval_live().unwrap();
        let error = vm
            .load(
                r#"published = { answer = 41 }
local captured = 0
saved = function() captured += 1; return captured end
local function bomb()
    local t, i = {}, 0
    while true do
        i += 1
        t[i] = { i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i }
    end
end
-- Expanded argument evaluation uses the explicit continuation driver.
return select(1, bomb())"#,
            )
            .source_name("expanded-oom.luau")
            .exec()
            .unwrap_err();
        assert!(
            matches!(
                error.as_vm_error().map(|e| e.kind()),
                Some(rustia::VmErrorKind::OutOfMemory)
            ),
            "{profile:?}: {error}"
        );
        assert_eq!(
            vm.eval::<i32>("local allocated = { 1 }; return published.answer + saved()")
                .unwrap(),
            42
        );
        assert_eq!(
            retained
                .as_table(&vm)
                .unwrap()
                .unwrap()
                .get(&mut vm, "answer")
                .unwrap()
                .as_number(&vm)
                .unwrap(),
            Some(42.0)
        );
    }
}

#[test]
fn protected_and_dead_coroutine_oom_release_local_allocations() {
    for profile in profiles() {
        for wrapper in [
            "local ok, err = pcall(bomb); assert(not ok and err == 'not enough memory')",
            "local ok, err = xpcall(bomb, function(e) return e end); assert(not ok and err == 'not enough memory')",
            "worker = coroutine.create(bomb); local ok, err = coroutine.resume(worker); assert(not ok and err == 'not enough memory'); assert(coroutine.status(worker) == 'dead')",
        ] {
            let mut vm = RustiaBuilder::standard()
                .profile(profile)
                .without_stdout()
                .fuel(50_000_000)
                .memory_limit_bytes(1024 * 1024)
                .build()
                .unwrap();
            vm.exec(r#"function bomb()
    local t, i = {}, 0
    while true do
        i += 1
        t[i] = { i, i, i, i, i, i, i, i, i, i, i, i, i, i, i, i }
    end
end"#).unwrap();
            let source = format!("{wrapper}; local fresh = {{ answer = 42 }}; return fresh.answer");
            assert_eq!(vm.eval::<i32>(&source).unwrap_or_else(|e| panic!("{profile:?}: {wrapper}: {e}")), 42);
            assert_eq!(vm.eval::<i32>("local fresh = { 42 }; return fresh[1]").unwrap(), 42);
        }
    }
}
