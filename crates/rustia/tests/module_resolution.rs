use rustia::{
    GuestSurface, ModuleSource, OwnedTable, Profile, RustiaBuilder, ServiceModuleResolver,
    VmErrorKind, VmSettings, RUSTIA_MODULE_RESOLUTION_ABI_VERSION,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[test]
fn dependency_syntax_error_retains_its_source_and_loading_callers() {
    let resolver = ServiceModuleResolver::from_fn(|requested, _| {
        Ok(match requested {
            "outer" => {
                ModuleSource::with_source_name("outer", "outer.luau", "return require('broken')")
            }
            _ => {
                ModuleSource::with_source_name("broken", "broken.luau", "-- dependency\nlocal = 1")
            }
        })
    });
    let mut vm = RustiaBuilder::standard()
        .module_resolver(resolver)
        .build()
        .unwrap();
    let error = vm
        .load("return require('outer')")
        .source_name("entry.luau")
        .exec()
        .unwrap_err();
    let report = error.report().to_string();
    assert!(report.contains("broken.luau:2"), "{report}");
    assert!(report.contains("2 | local = 1"), "{report}");
    assert!(report.contains("outer.luau:1"), "{report}");
    assert!(report.contains("entry.luau:1"), "{report}");
    vm.reset_decision().unwrap();
    assert!(vm.diagnostic_source_bytes() > 0);
    drop(error);
    assert_eq!(vm.diagnostic_source_bytes(), 0);
}

#[test]
fn dependency_syntax_error_obeys_zero_snapshot_budget() {
    let resolver =
        ServiceModuleResolver::from_fn(|_, _| Ok(ModuleSource::new("broken", "local = 1")));
    let mut settings = VmSettings::default();
    settings.diagnostic_limits.retained_source_bytes = 0;
    let mut vm = RustiaBuilder::standard()
        .settings(settings)
        .module_resolver(resolver)
        .build()
        .unwrap();
    let error = vm.eval::<()>("require('broken')").unwrap_err();
    assert!(error.diagnostic().unwrap().snapshot.is_none());
    assert_eq!(vm.diagnostic_source_bytes(), 0);
}

#[test]
fn require_resolves_by_host_identity_and_executes_once() {
    assert_eq!(RUSTIA_MODULE_RESOLUTION_ABI_VERSION, 2);
    let resolutions = Rc::new(Cell::new(0_u32));
    let resolutions_for_host = Rc::clone(&resolutions);
    let resolver = ServiceModuleResolver::from_fn(move |requested, referrer| {
        resolutions_for_host.set(resolutions_for_host.get() + 1);
        assert_eq!(referrer, Some("entry.luau"));
        assert!(matches!(requested, "first" | "second"));
        Ok(ModuleSource::with_source_name(
            "shared-module",
            "modules/shared.luau",
            br#"
                _G.module_runs = (_G.module_runs or 0) + 1
                return { answer = 42 }
            "#,
        ))
    });
    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()
        .expect("deterministic resolver");

    let result: OwnedTable = vm
        .load(
            r#"
                local first = require("first")
                local second = require("second")
                return { same = first == second, runs = module_runs }
            "#,
        )
        .source_name("entry.luau")
        .eval()
        .expect("module result");

    assert_eq!(result.get_string_as::<bool>("same").unwrap(), Some(true));
    assert_eq!(result.get_string_as::<f64>("runs").unwrap(), Some(1.0));
    assert_eq!(resolutions.get(), 2);
}

#[test]
fn require_compiles_modules_with_the_requesting_profile() {
    let resolver = ServiceModuleResolver::from_fn(|requested, _| {
        assert_eq!(requested, "puc-constructor");
        Ok(ModuleSource::new(
            "puc-constructor",
            "local t = {1, [1] = 9} return t[1]",
        ))
    });
    let mut vm = RustiaBuilder::puc51_compat()
        .module_resolver(resolver)
        .build()
        .expect("PUC resolver");

    let result: f64 = vm
        .eval(r#"return require("puc-constructor")"#)
        .expect("module result");
    assert_eq!(result, 1.0);
}

#[test]
fn require_passes_nested_referrers_and_caches_empty_results() {
    let calls = Rc::new(RefCell::new(Vec::<(String, Option<String>)>::new()));
    let calls_for_host = Rc::clone(&calls);
    let resolver = ServiceModuleResolver::from_fn(move |requested, referrer| {
        calls_for_host
            .borrow_mut()
            .push((requested.to_owned(), referrer.map(str::to_owned)));
        match requested {
            "outer" => Ok(ModuleSource::with_source_name(
                "outer-id",
                "modules/outer.luau",
                r#"
                    require("empty")
                    require("empty")
                    return 42
                "#,
            )),
            "empty" => Ok(ModuleSource::with_source_name(
                "empty-id",
                "modules/empty.luau",
                "_G.empty_runs = (_G.empty_runs or 0) + 1",
            )),
            _ => Err(rustia::Error::embed(format!("unknown module {requested}"))),
        }
    });
    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()
        .expect("deterministic resolver");

    let result: OwnedTable = vm
        .load(r#"return { value = require("outer"), runs = empty_runs }"#)
        .source_name("entry.luau")
        .eval()
        .expect("nested modules");
    assert_eq!(result.get_string_as::<f64>("value").unwrap(), Some(42.0));
    assert_eq!(result.get_string_as::<f64>("runs").unwrap(), Some(1.0));
    assert_eq!(
        calls.borrow().as_slice(),
        [
            ("outer".to_owned(), Some("entry.luau".to_owned())),
            ("empty".to_owned(), Some("modules/outer.luau".to_owned())),
            ("empty".to_owned(), Some("modules/outer.luau".to_owned())),
        ]
    );
}

#[test]
fn require_reports_cycles_and_does_not_cache_failures() {
    let resolver = ServiceModuleResolver::from_fn(|requested, _| match requested {
        "a" => Ok(ModuleSource::new(
            "a",
            r#"
                _G.a_runs = (_G.a_runs or 0) + 1
                require("b")
                return "a"
            "#,
        )),
        "b" => Ok(ModuleSource::new("b", r#"return require("a")"#)),
        _ => Err(rustia::Error::embed(format!("unknown module {requested}"))),
    });
    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()
        .expect("deterministic resolver");

    let result: OwnedTable = vm
        .eval(
            r#"
                local first_ok, first_error = pcall(require, "a")
                local second_ok = pcall(require, "a")
                return {
                    first = first_ok,
                    second = second_ok,
                    runs = a_runs,
                    cycle = string.find(first_error, "a %-> b %-> a") ~= nil,
                }
            "#,
        )
        .expect("cycle is catchable");
    assert_eq!(result.get_string_as::<bool>("first").unwrap(), Some(false));
    assert_eq!(result.get_string_as::<bool>("second").unwrap(), Some(false));
    assert_eq!(result.get_string_as::<f64>("runs").unwrap(), None);
    assert_eq!(result.get_string_as::<bool>("cycle").unwrap(), Some(true));
}

#[test]
fn require_reset_clears_the_execute_once_cache() {
    let output = Rc::new(RefCell::new(String::new()));
    let output_for_host = Rc::clone(&output);
    let resolver = ServiceModuleResolver::from_fn(|requested, _| {
        assert_eq!(requested, "probe");
        Ok(ModuleSource::new("probe", "print('loaded'); return 7"))
    });
    let mut vm = RustiaBuilder::deterministic()
        .stdout(move |text| output_for_host.borrow_mut().push_str(text))
        .module_resolver(resolver)
        .build()
        .expect("deterministic resolver");

    let first: OwnedTable = vm
        .eval(r#"return { require("probe"), require("probe") }"#)
        .expect("first epoch");
    assert_eq!(first.get_array_as::<f64>(0).unwrap(), Some(7.0));
    assert_eq!(first.get_array_as::<f64>(1).unwrap(), Some(7.0));
    vm.reset_decision().expect("decision reset");
    let second: f64 = vm.eval(r#"return require("probe")"#).expect("second epoch");
    assert_eq!(second, 7.0);
    assert_eq!(output.borrow().as_str(), "loaded\nloaded\n");
}

#[test]
fn required_values_remain_rooted_and_modules_share_caller_fuel() {
    let resolver = ServiceModuleResolver::from_fn(|requested, _| match requested {
        "rooted" => Ok(ModuleSource::new("rooted", "return { answer = 42 }")),
        "expensive" => Ok(ModuleSource::new(
            "expensive",
            "local n = 0; for i = 1, 1000 do n += i end; return n",
        )),
        _ => Err(rustia::Error::embed(format!("unknown module {requested}"))),
    });
    let mut vm = RustiaBuilder::standard()
        .module_resolver(resolver.clone())
        .build()
        .expect("standard resolver");
    let rooted: bool = vm
        .eval(
            r#"
                local first = require("rooted")
                collectgarbage("collect")
                local second = require("rooted")
                return first == second and second.answer == 42
            "#,
        )
        .expect("cached value survives collection");
    assert!(rooted);

    let mut fuelled = RustiaBuilder::standard()
        .fuel(20)
        .module_resolver(resolver)
        .build()
        .expect("fuelled resolver");
    let error = fuelled
        .eval::<f64>(r#"return require("expensive")"#)
        .unwrap_err();
    assert!(matches!(
        error,
        rustia::Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(error.kind, VmErrorKind::FuelExhausted { limit: 20 })
    ));
}

#[test]
fn multi_result_modules_fail_without_entering_the_cache() {
    let resolver = ServiceModuleResolver::from_fn(|_, _| {
        Ok(ModuleSource::new(
            "multi",
            "_G.multi_runs = (_G.multi_runs or 0) + 1; return 1, 2",
        ))
    });
    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()
        .expect("deterministic resolver");
    let result: OwnedTable = vm
        .eval(
            r#"
                local first_ok, first_error = pcall(require, "multi")
                local second_ok = pcall(require, "multi")
                return {
                    first = first_ok,
                    second = second_ok,
                    runs = multi_runs,
                    message = string.find(first_error, "module must return a single value", 1, true) ~= nil,
                }
            "#,
        )
        .expect("module errors are catchable");
    assert_eq!(result.get_string_as::<bool>("first").unwrap(), Some(false));
    assert_eq!(result.get_string_as::<bool>("second").unwrap(), Some(false));
    assert_eq!(result.get_string_as::<f64>("runs").unwrap(), None);
    assert_eq!(result.get_string_as::<bool>("message").unwrap(), Some(true));
}

#[test]
fn module_identity_quota_rejection_is_atomic_and_catchable() {
    let mut baseline_vm = RustiaBuilder::standard().build().expect("baseline VM");
    let baseline = baseline_vm.memory_stats().live_bytes;
    let oversized_identity = "m".repeat(256 * 1024);
    let resolver = ServiceModuleResolver::from_fn(move |_, _| {
        Ok(ModuleSource::new(
            oversized_identity.clone(),
            "_G.module_started = true; return 1",
        ))
    });
    let mut vm = RustiaBuilder::standard()
        .memory_limit_bytes(baseline + 128 * 1024)
        .module_resolver(resolver)
        .build()
        .expect("quota VM");

    let stayed_atomic: bool = vm
        .eval(
            r#"
                local ok, message = pcall(require, "oversized")
                return not ok
                    and string.find(message, "not enough memory", 1, true) ~= nil
                    and module_started == nil
            "#,
        )
        .expect("legacy pcall catches quota error");
    assert!(stayed_atomic);
    assert_eq!(
        vm.eval::<f64>("return 42").expect("VM remains usable"),
        42.0
    );
}

#[test]
fn deterministic_and_hardened_profiles_reject_invalid_resolvers() {
    let nondeterministic =
        ServiceModuleResolver::from_nondeterministic_fn(|_, _| Err(rustia::Error::embed("unused")));
    let error = RustiaBuilder::deterministic()
        .module_resolver(nondeterministic)
        .build()
        .err()
        .expect("deterministic profile rejection");
    assert!(matches!(
        error,
        rustia::Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                error.kind,
                VmErrorKind::CapabilityRemoved {
                    feature: "non-deterministic module resolver",
                    ..
                }
            )
    ));

    let deterministic = ServiceModuleResolver::from_fn(|_, _| Err(rustia::Error::embed("unused")));
    let error = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(256 * 1024)
        .module_resolver(deterministic)
        .build()
        .err()
        .expect("hardened leaf rejection");
    assert!(matches!(
        error,
        rustia::Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "dynamic-code or conformance-only facilities"
                }
            )
    ));

    let low_level = VmSettings {
        guest_surface: GuestSurface::HardenedLeaf,
        memory_limit_bytes: Some(256 * 1024),
        module_resolver: Some(rustia_vm::ModuleResolver::new(
            |_, _| {
                Err(rustia_vm::Error::new(rustia_vm::ErrorKind::RuntimeError {
                    message: "unused".to_owned(),
                }))
            },
            true,
        )),
        ..VmSettings::default()
    };
    let vm = rustia_vm::Vm::with_settings(Profile::DeterministicSim, low_level);
    assert!(vm.settings().module_resolver.is_none());
}

#[test]
fn resolver_panics_poison_until_decision_reset() {
    let calls = Rc::new(Cell::new(0_u32));
    let calls_for_host = Rc::clone(&calls);
    let resolver = ServiceModuleResolver::from_fn(move |_, _| {
        calls_for_host.set(calls_for_host.get() + 1);
        if calls_for_host.get() == 1 {
            panic!("resolver panic");
        }
        Ok(ModuleSource::new("recovered", "return 42"))
    });
    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()
        .expect("deterministic resolver");

    let first = vm.eval::<f64>(r#"return require("probe")"#).unwrap_err();
    assert!(matches!(
        first,
        rustia::Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(error.kind, VmErrorKind::HostPanicked { .. })
    ));
    assert!(matches!(
        vm.eval::<f64>("return 1").unwrap_err(),
        rustia::Error { kind: rustia::ErrorKind::Vm(ref error), .. } if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));

    vm.reset_decision().expect("reset poisoned VM");
    assert_eq!(
        vm.eval::<f64>(r#"return require("probe")"#)
            .expect("resolver recovered"),
        42.0
    );
}
