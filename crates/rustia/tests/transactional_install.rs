use rustia::{
    ModuleSource, OwnedValue, Profile, Rustia, RustiaBuilder, RustiaExtension,
    ServiceModuleResolver, VmErrorKind, RUSTIA_INSTALL_TRANSACTION_ABI_VERSION,
};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[test]
fn default_install_uses_an_isolated_environment_that_inherits_globals() {
    assert_eq!(RUSTIA_INSTALL_TRANSACTION_ABI_VERSION, 1);
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);
    rustia.insert_global("host_value", 40).unwrap();

    let exports = rustia
        .load(
            r#"
                module_local = host_value + 2
                return {
                    answer = module_local,
                    owns_global = _G.module_local == module_local,
                    read = function() return module_local end,
                }
            "#,
        )
        .source_name("isolated.luau")
        .install()
        .unwrap();

    assert_eq!(exports.get::<i32>(&mut rustia, "answer").unwrap(), 42);
    assert!(exports.get::<bool>(&mut rustia, "owns_global").unwrap());
    assert_eq!(
        exports
            .function(&mut rustia, "read")
            .unwrap()
            .call::<_, i32>(&mut rustia, ())
            .unwrap(),
        42
    );
    assert!(matches!(
        rustia.eval::<OwnedValue>("return module_local").unwrap(),
        OwnedValue::Nil
    ));
}

#[test]
fn runtime_compilation_inherits_the_installation_environment() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();
    rustia.insert_global("host_value", 42).unwrap();

    let exports = rustia
        .load(
            r#"
                module_local = "candidate"
                local loaded = assert(loadstring("return _G, host_value, module_local"))
                local loaded_globals, host_value, module_local = loaded()
                local worker = coroutine.create(function()
                    local deferred = assert(loadstring("return _G == getfenv(0), module_local"))
                    return deferred()
                end)
                return {
                    same_globals = loaded_globals == _G,
                    host_value = host_value,
                    module_local = module_local,
                    deferred = function()
                        local loaded_later = assert(loadstring("return _G == getfenv(0), module_local"))
                        return loaded_later()
                    end,
                    resume_worker = function()
                        local ok, same_globals, value = coroutine.resume(worker)
                        assert(ok)
                        return same_globals, value
                    end,
                }
            "#,
        )
        .install()
        .unwrap();

    assert!(exports.get::<bool>(&mut rustia, "same_globals").unwrap());
    assert_eq!(exports.get::<i32>(&mut rustia, "host_value").unwrap(), 42);
    assert_eq!(
        exports.get::<String>(&mut rustia, "module_local").unwrap(),
        "candidate"
    );
    let deferred = exports.function(&mut rustia, "deferred").unwrap();
    assert_eq!(
        deferred.call::<_, (bool, String)>(&mut rustia, ()).unwrap(),
        (true, "candidate".to_owned())
    );
    let resume_worker = exports.function(&mut rustia, "resume_worker").unwrap();
    assert_eq!(
        resume_worker
            .call::<_, (bool, String)>(&mut rustia, ())
            .unwrap(),
        (true, "candidate".to_owned())
    );
    assert!(matches!(
        rustia.eval::<OwnedValue>("return module_local").unwrap(),
        OwnedValue::Nil
    ));
}

#[test]
fn first_coroutine_resume_uses_its_creator_environment() {
    let mut rustia = RustiaBuilder::puc51_compat().build().unwrap();
    let exports = rustia
        .load(
            r#"
                module_local = "candidate"
                return coroutine.create(function()
                    local loaded = assert(loadstring(
                        "return _G == getfenv(0), module_local"
                    ))
                    return loaded()
                end)
            "#,
        )
        .source_name("coroutine-environment.luau")
        .install()
        .unwrap();
    let worker = exports.as_value();
    let globals = rustia.globals().unwrap();
    globals
        .set_live(&mut rustia, "deferred_worker", &worker)
        .unwrap();

    let result: (bool, String) = rustia
        .eval_serde(
            r#"
                local ok, same_globals, value = coroutine.resume(deferred_worker)
                assert(ok)
                return { same_globals, value }
            "#,
        )
        .unwrap();
    assert_eq!(result, (true, "candidate".to_owned()));
}

#[test]
fn failed_install_restores_shared_graphs_upvalues_buffers_metatables_and_rng() {
    let mut rustia = RustiaBuilder::standard().build().unwrap();
    let mut control = RustiaBuilder::standard().build().unwrap();
    let setup = r#"
                nested = { answer = 42 }
                bytes = buffer.fromstring(string.char(1, 2, 3))
                local count = 0
                bump = function() count += 1; return count end
                thread = coroutine.create(function()
                    local step = 0
                    while true do
                        step += 1
                        coroutine.yield(step)
                    end
                end)
                assert(select(2, coroutine.resume(thread)) == 1)
            "#;
    rustia.exec(setup).unwrap();
    control.exec(setup).unwrap();
    let globals = rustia.globals().unwrap();
    let logical_time_before = rustia.eval::<f64>("return os.clock()").unwrap();
    let control_logical_time_before = control.eval::<f64>("return os.clock()").unwrap();
    assert_eq!(logical_time_before, control_logical_time_before);

    let error = rustia
        .load(
            r#"
                candidate = "partial"
                nested.answer = 99
                bump()
                buffer.writeu8(bytes, 0, 255)
                setmetatable(nested, { tag = "partial" })
                assert(select(2, coroutine.resume(thread)) == 2)
                math.random()
                error("install failed")
            "#,
        )
        .source_name("failed-install.luau")
        .environment(&globals)
        .install()
        .unwrap_err();
    assert!(error.to_string().contains("install failed"), "{error}");

    let restoration_check = r#"
                local resumed, step = coroutine.resume(thread)
                return candidate == nil
                    and nested.answer == 42
                    and buffer.readu8(bytes, 0) == 1
                    and getmetatable(nested) == nil
                    and bump() == 1
                    and resumed
                    and step == 2
            "#;
    let restored: bool = rustia.eval(restoration_check).unwrap();
    assert!(restored);
    assert!(control.eval::<bool>(restoration_check).unwrap());
    assert_eq!(
        rustia.eval::<f64>("return os.clock()").unwrap(),
        control.eval::<f64>("return os.clock()").unwrap(),
        "a failed install must restore deterministic logical time"
    );
    assert_eq!(
        rustia.eval::<f64>("return math.random()").unwrap(),
        control.eval::<f64>("return math.random()").unwrap(),
        "a failed install must restore the RNG stream"
    );
}

#[test]
fn explicit_globals_environment_is_shared_across_installs_and_later_execution() {
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);
    let globals = rustia.globals().unwrap();

    let first = rustia
        .load("installed_value = 42; return { answer = installed_value }")
        .environment(&globals)
        .install()
        .unwrap();
    let second = rustia
        .load("derived_value = installed_value + 1; return { answer = derived_value }")
        .environment(&globals)
        .install()
        .unwrap();
    rustia.exec("observed_value = derived_value + 1").unwrap();

    assert_eq!(first.get::<i32>(&mut rustia, "answer").unwrap(), 42);
    assert_eq!(second.get::<i32>(&mut rustia, "answer").unwrap(), 43);
    assert_eq!(rustia.eval::<i32>("return installed_value").unwrap(), 42);
    assert_eq!(rustia.eval::<i32>("return observed_value").unwrap(), 44);
}

#[test]
fn weak_table_collection_is_observable_on_commit_and_reversed_on_rollback() {
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);
    rustia
        .exec(
            r#"
                weak = setmetatable({}, { __mode = "v" })
                strong = { answer = 42 }
                weak.key = strong
            "#,
        )
        .unwrap();
    let globals = rustia.globals().unwrap();

    assert!(rustia
        .load("strong = nil; collectgarbage('collect'); error('rollback')")
        .environment(&globals)
        .install()
        .is_err());
    assert!(rustia
        .eval::<bool>("return strong ~= nil and weak.key == strong")
        .unwrap());

    rustia
        .load("strong = nil; collectgarbage('collect'); return {}")
        .environment(&globals)
        .install()
        .unwrap();
    assert!(rustia.eval::<bool>("return weak.key == nil").unwrap());
}

#[test]
fn each_install_has_a_fresh_lazy_require_cohort() {
    let resolution = Rc::new(Cell::new(0_u32));
    let resolution_for_host = Rc::clone(&resolution);
    let resolver = ServiceModuleResolver::from_fn(move |requested, _| {
        assert_eq!(requested, "dep");
        let version = resolution_for_host.get() + 1;
        resolution_for_host.set(version);
        Ok(ModuleSource::new(
            "dep",
            format!("return {{ version = {version} }}"),
        ))
    });
    let mut rustia = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()
        .unwrap();
    let source = r#"
        local dependency = require("dep")
        return {
            first = dependency.version,
            later = function() return require("dep").version end,
        }
    "#;

    let old = rustia
        .load(source)
        .source_name("root.luau")
        .install()
        .unwrap();
    let old_later = old.function(&mut rustia, "later").unwrap();
    assert_eq!(old.get::<u32>(&mut rustia, "first").unwrap(), 1);
    assert_eq!(old_later.call::<_, u32>(&mut rustia, ()).unwrap(), 1);

    let new = rustia
        .load(source)
        .source_name("root.luau")
        .install()
        .unwrap();
    let new_later = new.function(&mut rustia, "later").unwrap();
    assert_eq!(new.get::<u32>(&mut rustia, "first").unwrap(), 3);
    assert_eq!(old_later.call::<_, u32>(&mut rustia, ()).unwrap(), 1);
    assert_eq!(new_later.call::<_, u32>(&mut rustia, ()).unwrap(), 3);
    assert_eq!(resolution.get(), 5);
}

#[test]
fn quota_failure_rolls_back_installation_and_keeps_the_vm_reusable() {
    let mut baseline = RustiaBuilder::deterministic().build().unwrap();
    let limit = baseline.memory_stats().live_bytes + 128 * 1024;
    drop(baseline);
    let mut rustia = RustiaBuilder::deterministic()
        .memory_limit_bytes(limit)
        .build()
        .unwrap();
    rustia.exec("shared = { value = 1 }").unwrap();
    let globals = rustia.globals().unwrap();

    let error = rustia
        .load(
            r#"
                shared.value = 99
                local values = {}
                for i = 1, 10000 do
                    values[i] = string.rep("x", 100) .. i
                end
                return values
            "#,
        )
        .environment(&globals)
        .install()
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind, VmErrorKind::OutOfMemory)
    ));
    assert_eq!(rustia.eval::<i32>("return shared.value").unwrap(), 1);
    assert_eq!(rustia.eval::<i32>("return 40 + 2").unwrap(), 42);
}

#[test]
fn transaction_snapshot_is_quota_checked_before_guest_mutation() {
    let setup = r#"
        large = table.create(8192, 1)
        bytes = buffer.create(131072)
        shared = { value = 1 }
    "#;
    let mut probe = RustiaBuilder::deterministic().build().unwrap();
    probe.exec(setup).unwrap();
    let limit = probe.memory_stats().live_bytes + 64 * 1024;
    drop(probe);

    let mut rustia = RustiaBuilder::deterministic()
        .memory_limit_bytes(limit)
        .build()
        .unwrap();
    rustia.exec(setup).unwrap();
    let globals = rustia.globals().unwrap();
    let mut failed_live = None;
    for _ in 0..2 {
        let before = rustia.memory_stats().live_bytes;
        let error = rustia
            .load("shared.value = 99; buffer.writeu8(bytes, 0, 255); return {}")
            .environment(&globals)
            .install()
            .unwrap_err();
        assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind, VmErrorKind::OutOfMemory)
        ));
        let after = rustia.memory_stats().live_bytes;
        assert!(
            after <= before,
            "failed admission must not leak live memory"
        );
        if let Some(expected) = failed_live {
            assert_eq!(after, expected);
        }
        failed_live = Some(after);
    }
    assert_eq!(rustia.eval::<i32>("return shared.value").unwrap(), 1);
    assert_eq!(
        rustia
            .eval::<i32>("return buffer.readu8(bytes, 0)")
            .unwrap(),
        0
    );
}

#[test]
fn rollback_invalidates_preexisting_program_caches_before_handle_reuse() {
    const ORIGINAL: &str = "original constant string long enough to require an interned atom";
    const REPLACEMENT: &str = "replacement string that must never alias the original cached atom";
    let mut rustia = RustiaBuilder::standard().build().unwrap();
    let exports = rustia
        .load(
            r#"
                return {
                    read = function()
                        return "original constant string long enough to require an interned atom"
                    end,
                }
            "#,
        )
        .install()
        .unwrap();
    let read = exports.function(&mut rustia, "read").unwrap();
    let globals = rustia.globals().unwrap();
    globals
        .set_live(&mut rustia, "old_read", &read.as_value())
        .unwrap();

    let error = rustia
        .load("assert(old_read() ~= nil); error('rollback after cache fill')")
        .environment(&globals)
        .install()
        .unwrap_err();
    assert!(error.to_string().contains("rollback after cache fill"));

    let replacement: String = rustia.eval(format!("return {REPLACEMENT:?}")).unwrap();
    assert_eq!(replacement, REPLACEMENT);
    assert_eq!(read.call::<_, String>(&mut rustia, ()).unwrap(), ORIGINAL);
}

#[test]
fn failed_install_does_not_claim_to_undo_external_host_effects() {
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_host = Arc::clone(&calls);
    let mut rustia = RustiaBuilder::standard().build().unwrap();
    rustia
        .add_external_state_fn("touch_host", move || {
            calls_for_host.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
    let globals = rustia.globals().unwrap();

    let error = rustia
        .load("guest_value = 42; touch_host(); error('reject candidate')")
        .environment(&globals)
        .install()
        .unwrap_err();
    assert!(error.to_string().contains("reject candidate"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        rustia.eval::<OwnedValue>("return guest_value").unwrap(),
        OwnedValue::Nil
    ));
}

#[test]
fn global_extension_batches_are_atomic_on_quota_failure() {
    let mut baseline = RustiaBuilder::standard().build().unwrap();
    let limit = baseline.memory_stats().live_bytes + 32 * 1024;
    drop(baseline);
    let mut rustia = RustiaBuilder::standard()
        .memory_limit_bytes(limit)
        .build()
        .unwrap();
    let before_live = rustia.memory_stats().live_bytes;
    let oversized_name = "oversized".repeat(64 * 1024);
    let extension = RustiaExtension::global()
        .fn_pure("must_not_leak", || 1_i32)
        .fn_pure(oversized_name, || 2_i32);

    let error = rustia.install_extension(extension).unwrap_err();
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind, VmErrorKind::OutOfMemory)
    ));
    assert_eq!(rustia.memory_stats().live_bytes, before_live);
    assert!(matches!(
        rustia.eval::<OwnedValue>("return must_not_leak").unwrap(),
        OwnedValue::Nil
    ));
}

#[test]
fn namespaced_extension_batches_are_atomic_on_quota_failure() {
    let mut baseline = RustiaBuilder::standard().build().unwrap();
    let limit = baseline.memory_stats().live_bytes + 32 * 1024;
    drop(baseline);
    let mut rustia = RustiaBuilder::standard()
        .memory_limit_bytes(limit)
        .build()
        .unwrap();
    let before_live = rustia.memory_stats().live_bytes;
    let oversized_name = "oversized".repeat(64 * 1024);
    let extension = RustiaExtension::new("candidate")
        .fn_pure("must_not_leak", || 1_i32)
        .fn_pure(oversized_name, || 2_i32);

    let error = rustia.install_extension(extension).unwrap_err();
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind, VmErrorKind::OutOfMemory)
    ));
    assert_eq!(rustia.memory_stats().live_bytes, before_live);
    assert!(matches!(
        rustia.eval::<OwnedValue>("return candidate").unwrap(),
        OwnedValue::Nil
    ));
}
