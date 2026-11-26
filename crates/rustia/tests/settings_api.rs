use rustia::{
    DiagnosticLimits, GuestSurface, ModuleSource, Profile, RustiaBuilder, ServiceClock,
    ServiceDeterminism, ServiceEntropy, ServiceModuleResolver,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[cfg(not(feature = "send"))]
#[test]
fn default_services_accept_thread_affine_closures() {
    use std::cell::Cell;
    use std::rc::Rc;

    let entropy_calls = Rc::new(Cell::new(0));
    let captured = Rc::clone(&entropy_calls);
    let mut vm = RustiaBuilder::standard()
        .entropy(ServiceEntropy::from_fn(move || {
            captured.set(captured.get() + 1);
            42
        }))
        .build()
        .unwrap();

    assert_eq!(entropy_calls.get(), 1);
    assert!(vm.eval::<f64>("return math.random()").unwrap() >= 0.0);
}

#[test]
fn built_settings_match_the_requested_limits_and_survive_execution_and_reset() {
    let diagnostic_limits = DiagnosticLimits::new()
        .with_retained_source_bytes(8192)
        .with_source_bytes(2048)
        .with_frames(4)
        .with_report_bytes(1024);
    let builder = RustiaBuilder::deterministic()
        .fuel(10_000)
        .recursion_limit(64)
        .memory_limit_bytes(4 * 1024 * 1024)
        .memory_tracking(true)
        .diagnostic_limits(diagnostic_limits);
    let requested = builder.settings();
    assert_eq!(requested.profile(), Profile::DeterministicSim);
    assert_eq!(requested.guest_surface(), GuestSurface::Standard);
    assert_eq!(requested.fuel(), Some(10_000));
    assert_eq!(requested.recursion_limit(), 64);
    assert_eq!(requested.memory_limit_bytes(), Some(4 * 1024 * 1024));
    assert!(requested.memory_tracking());
    assert_eq!(requested.diagnostic_limits(), diagnostic_limits);
    assert!(!requested.runtime_compilation());
    assert!(!requested.stdout());
    assert_eq!(requested.wall_clock(), None);
    assert_eq!(requested.monotonic_clock(), None);
    assert_eq!(requested.entropy(), None);
    assert_eq!(requested.module_resolver(), None);

    let mut vm = builder.build().unwrap();
    assert_eq!(vm.settings(), requested);
    assert_eq!(vm.eval::<i64>("return 6 * 7").unwrap(), 42);
    assert!(vm.exec("error('expected')").is_err());
    assert_eq!(
        vm.settings(),
        requested,
        "remaining fuel is execution state"
    );
    vm.reset_decision().unwrap();
    assert_eq!(vm.settings(), requested);
}

#[test]
fn settings_readback_describes_services_without_calling_them() {
    let calls = Arc::new(AtomicUsize::new(0));
    let output_calls = calls.clone();
    let wall_calls = calls.clone();
    let monotonic_calls = calls.clone();
    let entropy_calls = calls.clone();
    let resolver_calls = calls.clone();
    let builder = RustiaBuilder::standard()
        .stdout(move |_| {
            output_calls.fetch_add(1, Ordering::Relaxed);
        })
        .wall_clock(ServiceClock::from_fn(move || {
            wall_calls.fetch_add(1, Ordering::Relaxed);
            0.0
        }))
        .monotonic_clock(ServiceClock::from_nondeterministic_fn(move || {
            monotonic_calls.fetch_add(1, Ordering::Relaxed);
            0.0
        }))
        .entropy(ServiceEntropy::from_fn(move || {
            entropy_calls.fetch_add(1, Ordering::Relaxed);
            0
        }))
        .module_resolver(ServiceModuleResolver::from_nondeterministic_fn(
            move |_, _| {
                resolver_calls.fetch_add(1, Ordering::Relaxed);
                Ok(ModuleSource::new("example", "return 42"))
            },
        ));
    let settings = builder.settings();
    assert!(settings.stdout());
    assert_eq!(
        settings.wall_clock(),
        Some(ServiceDeterminism::ReplayStable)
    );
    assert_eq!(settings.monotonic_clock(), Some(ServiceDeterminism::Live));
    assert_eq!(settings.entropy(), Some(ServiceDeterminism::ReplayStable));
    assert_eq!(settings.module_resolver(), Some(ServiceDeterminism::Live));
    assert_eq!(settings, builder.settings());
    assert!(!settings.identity_bytes().is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn identity_distinguishes_unlimited_and_zero_budgets() {
    let unlimited = RustiaBuilder::deterministic().settings();
    let zero_fuel = RustiaBuilder::deterministic().fuel(0).settings();
    let zero_memory = RustiaBuilder::deterministic()
        .memory_limit_bytes(0)
        .settings();
    assert_ne!(unlimited.identity_bytes(), zero_fuel.identity_bytes());
    assert_ne!(unlimited.identity_bytes(), zero_memory.identity_bytes());
    assert_ne!(zero_fuel.identity_bytes(), zero_memory.identity_bytes());
}

#[test]
fn memory_samples_expose_peaks_only_when_requested() {
    let mut vm = RustiaBuilder::deterministic().build().unwrap();
    let sample = vm.memory_stats();
    assert_eq!(sample.peak_live_bytes(), None);
    assert_eq!(sample.peak_reserved_bytes(), None);

    let mut tracked = RustiaBuilder::deterministic()
        .memory_tracking(true)
        .build()
        .unwrap();
    tracked.exec("held = string.rep('x', 1024)").unwrap();
    let sample = tracked.memory_stats();
    assert!(sample.peak_live_bytes().unwrap() >= sample.live_bytes());
    assert!(sample.peak_reserved_bytes().unwrap() >= sample.reserved_bytes());
}

#[test]
fn service_declarations_can_be_matched_without_exhaustive_enum_assumptions() {
    fn description(declaration: ServiceDeterminism) -> &'static str {
        match declaration {
            ServiceDeterminism::ReplayStable => "replay-stable",
            ServiceDeterminism::Live => "live",
            _ => "other",
        }
    }
    assert_eq!(
        description(ServiceDeterminism::ReplayStable),
        "replay-stable"
    );
    assert_eq!(description(ServiceDeterminism::Live), "live");
}

#[test]
fn canonical_defaults_and_clamped_limits_have_honest_readback() {
    let default = rustia::Rustia::default();
    let builder = rustia::Rustia::builder();
    assert_eq!(default.settings(), builder.settings());
    assert_eq!(default.settings(), builder.build().unwrap().settings());

    let builder = RustiaBuilder::deterministic().recursion_limit(usize::MAX);
    assert_eq!(builder.settings().recursion_limit(), 512);
    assert_eq!(builder.settings(), builder.build().unwrap().settings());

    let builder = RustiaBuilder::deterministic_leaf().memory_limit_bytes(1024 * 1024);
    let expected = builder.settings();
    let vm = builder.build_for_host::<()>().unwrap();
    assert_eq!(expected, vm.settings());

    assert!(!RustiaBuilder::standard()
        .without_stdout()
        .settings()
        .stdout());
}

#[test]
fn hardened_leaf_rejects_entropy_without_calling_the_provider() {
    let calls = Arc::new(AtomicUsize::new(0));
    let captured = calls.clone();
    let result = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(1024 * 1024)
        .entropy(ServiceEntropy::from_fn(move || {
            captured.fetch_add(1, Ordering::Relaxed);
            1
        }))
        .build_for_host::<()>();
    assert!(matches!(
        result.err().unwrap().as_vm_error().unwrap().kind(),
        rustia::VmErrorKind::HardenedLeafViolation { .. }
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}
