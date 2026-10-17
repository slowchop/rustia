use rustia::{
    HostFunction, HostModule, OpCapability, Profile, Rustia, RustiaBuilder, Value, VmErrorKind,
};
use std::sync::{Arc, Weak};

fn callback(value: i64) -> (HostFunction, Weak<()>) {
    let capture = Arc::new(());
    let weak = Arc::downgrade(&capture);
    (
        HostFunction::pure(move || {
            let _keep = &capture;
            value
        }),
        weak,
    )
}

fn collect(rustia: &mut Rustia) {
    rustia
        .exec("collectgarbage('collect'); collectgarbage('collect')")
        .unwrap();
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn created_function_lives_with_guest_aliases_and_collects_after_final_reference() {
    let mut rustia = Rustia::default();
    let globals = rustia.globals().unwrap();
    let (definition, capture) = callback(7);
    let function = rustia.create_function(definition).unwrap();
    globals
        .set_live(&mut rustia, "old", &function.as_value())
        .unwrap();
    rustia
        .exec("saved = old; cycle = {}; cycle.self = cycle; cycle.callback = old")
        .unwrap();
    drop(function);
    collect(&mut rustia);
    assert!(capture.upgrade().is_some());
    let replacement = rustia.create_function(|| 9_i64).unwrap();
    globals
        .set_live(&mut rustia, "old", &replacement.as_value())
        .unwrap();
    assert!(rustia
        .eval::<bool>("return old() == 9 and saved() == 7")
        .unwrap());
    rustia.exec("saved = nil; cycle = nil").unwrap();
    collect(&mut rustia);
    assert!(capture.upgrade().is_none());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn host_root_and_reset_have_explicit_ownership() {
    let mut rustia = Rustia::default();
    let (definition, capture) = callback(3);
    let function = rustia.create_function(definition).unwrap();
    let clone = function.clone();
    drop(function);
    collect(&mut rustia);
    assert_eq!(clone.call::<_, i64>(&mut rustia, ()).unwrap(), 3);
    rustia.reset_decision().unwrap();
    assert!(
        capture.upgrade().is_none(),
        "stale roots cannot own callback captures"
    );
    assert!(clone.call::<_, i64>(&mut rustia, ()).is_err());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn persistent_replacement_releases_old_configuration_but_preserves_guest_aliases() {
    let mut rustia = Rustia::default();
    let (definition, old_capture) = callback(1);
    rustia.register_function("f", definition).unwrap();
    rustia.exec("old = f").unwrap();
    let (definition, new_capture) = callback(2);
    rustia.register_function("f", definition).unwrap();
    collect(&mut rustia);
    assert!(rustia
        .eval::<bool>("return f() == 2 and old() == 1")
        .unwrap());
    assert!(old_capture.upgrade().is_some());
    rustia.exec("old = nil").unwrap();
    collect(&mut rustia);
    assert!(old_capture.upgrade().is_none());
    rustia.reset_decision().unwrap();
    assert!(new_capture.upgrade().is_some());
    assert_eq!(rustia.eval::<i64>("return f()").unwrap(), 2);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn module_and_global_replacement_share_configuration_ownership() {
    let mut rustia = Rustia::default();
    let (definition, capture) = callback(4);
    rustia
        .install_host_module(HostModule::new("api").function("f", definition))
        .unwrap();
    assert_eq!(rustia.eval::<i64>("return api.f()").unwrap(), 4);
    rustia.register_function("api", || 8_i64).unwrap();
    collect(&mut rustia);
    assert!(capture.upgrade().is_none());
    rustia.reset_decision().unwrap();
    assert_eq!(rustia.eval::<i64>("return api()").unwrap(), 8);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn live_callback_preserves_functions_binary_data_and_cyclic_identity() {
    let mut rustia = Rustia::default();
    rustia
        .register_function(
            "identity",
            |table: rustia::Table, function: rustia::Function, bytes: Value| {
                (table, function, bytes, 7_i64)
            },
        )
        .unwrap();
    assert!(rustia
        .eval::<bool>(
            r#"
        local t = {}; t.self = t
        local f = function() return 12 end
        local a, b, c, d = identity(t, f, string.char(0, 255))
        return rawequal(a, t) and a.self == t and rawequal(b, f)
            and b() == 12 and c == string.char(0, 255) and d == 7
    "#
        )
        .unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn ordinary_callback_shapes_support_live_tables_and_functions() {
    let mut rustia = Rustia::default();
    rustia
        .register_function("value_id", |value: Value| value)
        .unwrap();
    rustia
        .register_function(
            "table_id",
            |value: rustia::Table| -> rustia::Result<rustia::Table> { Ok(value) },
        )
        .unwrap();
    rustia
        .register_function("function_id", |value: rustia::Function| value)
        .unwrap();
    assert!(rustia
        .eval::<bool>(
            r#"
        local t = {}; t.self = t
        local f = function() return 3 end
        return rawequal(value_id(t), t) and rawequal(table_id(t), t)
            and rawequal(function_id(f), f) and function_id(f)() == 3
    "#
        )
        .unwrap());
    assert!(rustia
        .exec("table_id(3)")
        .unwrap_err()
        .to_string()
        .contains("table_id"));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn constructor_supports_standard_profiles_and_rejects_cross_vm_calls() {
    for profile in [
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::LuauCompat,
        Profile::Puc51Compat,
    ] {
        let mut rustia = Rustia::new(profile);
        let function = rustia.create_function(|x: i64| x + 1).unwrap();
        assert_eq!(function.call::<_, i64>(&mut rustia, (4_i64,)).unwrap(), 5);
        let mut other = Rustia::new(profile);
        assert!(function.call::<_, i64>(&mut other, (4_i64,)).is_err());
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn rejected_constructors_release_their_captures() {
    let mut rustia = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(512 * 1024)
        .build_for_host::<()>()
        .unwrap();
    let before = rustia.memory_stats().live_bytes;
    let (definition, capture) = callback(1);
    assert!(matches!(
        (rustia.create_function(definition)).as_ref().err().map(|error| error.kind()),
        Some(rustia::ErrorKind::Vm(error))
        if matches!(error.kind, VmErrorKind::HardenedLeafViolation { .. })));
    assert!(capture.upgrade().is_none());
    assert_eq!(before, rustia.memory_stats().live_bytes);

    let mut rustia = RustiaBuilder::deterministic().build().unwrap();
    assert!(rustia
        .create_function(HostFunction::with_capability(
            OpCapability::NondeterministicEffect,
            || 1_i64
        ))
        .is_err());
    assert!(rustia
        .create_function(HostFunction::pure(|| 1_i64).fuel_cost(0))
        .is_err());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn named_created_callbacks_use_the_real_name_in_conversion_errors() {
    let mut rustia = Rustia::default();
    let function = rustia
        .create_named_function("grid_size", |x: i64| x)
        .unwrap();
    let error = function.call::<_, i64>(&mut rustia, ("bad",)).unwrap_err();
    assert!(error.to_string().contains("grid_size"), "{error}");
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn repeated_create_and_collect_reuses_function_storage() {
    let mut rustia = Rustia::default();
    for _ in 0..8 {
        drop(rustia.create_function(|| 1_i64).unwrap());
        collect(&mut rustia);
    }
    rustia.reset_decision().unwrap();
    let baseline = rustia.memory_stats().live_bytes;
    for _ in 0..64 {
        let (definition, capture) = callback(1);
        drop(rustia.create_function(definition).unwrap());
        collect(&mut rustia);
        assert!(capture.upgrade().is_none());
        rustia.reset_decision().unwrap();
        assert_eq!(baseline, rustia.memory_stats().live_bytes);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn constructor_quota_failure_drops_candidate_and_recovers_after_root_release() {
    let limit = Rustia::default().memory_stats().live_bytes + 4096;
    let mut rustia = RustiaBuilder::standard()
        .memory_limit_bytes(limit)
        .build()
        .unwrap();
    let mut functions = Vec::new();
    loop {
        let (definition, capture) = callback(1);
        match rustia.create_function(definition) {
            Ok(function) => functions.push(function),
            Err(error) => {
                assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind, VmErrorKind::OutOfMemory)));
                assert!(
                    capture.upgrade().is_none(),
                    "failed admission must release the candidate"
                );
                break;
            }
        }
        assert!(functions.len() < 1000, "quota must bound constructors");
    }
    assert!(!functions.is_empty());
    functions.pop();
    assert!(rustia.create_function(|| 2_i64).is_ok());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn failed_installation_restores_callback_aliases_and_releases_snapshot_ownership() {
    let mut rustia = Rustia::default();
    let (definition, capture) = callback(5);
    let function = rustia.create_function(definition).unwrap();
    rustia
        .globals()
        .unwrap()
        .set_live(&mut rustia, "original", &function.as_value())
        .unwrap();
    drop(function);
    let globals = rustia.globals().unwrap();
    assert!(rustia
        .load("_G.original = nil; error('fail')")
        .environment(globals)
        .install()
        .is_err());
    assert_eq!(rustia.eval::<i64>("return original()").unwrap(), 5);
    rustia.exec("original = nil").unwrap();
    collect(&mut rustia);
    assert!(capture.upgrade().is_none());
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn created_host_panics_poison_and_reset_releases_callback() {
    let mut rustia = Rustia::default();
    let capture = Arc::new(());
    let weak = Arc::downgrade(&capture);
    let function = rustia
        .create_function(move || -> i64 {
            let _keep = &capture;
            panic!("host failed")
        })
        .unwrap();
    assert!(matches!(
        (function.call::<_, i64>(&mut rustia, ())).as_ref().err().map(|error| error.kind()),
        Some(rustia::ErrorKind::Vm(error))
        if matches!(error.kind, VmErrorKind::HostPanicked { .. })));
    assert!(rustia.eval::<i64>("return 1").is_err());
    rustia.reset_decision().unwrap();
    assert!(weak.upgrade().is_none());
    assert_eq!(rustia.eval::<i64>("return 1").unwrap(), 1);
}
