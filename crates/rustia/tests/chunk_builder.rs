use rustia::{Error, GuestSurface, OwnedValue, Rustia, RustiaBuilder, VmErrorKind};

#[test]
fn chunk_builder_evaluates_typed_live_and_exports_results() {
    let mut rustia = Rustia::default();

    let answer: f64 = rustia
        .load("return 40 + 2")
        .source_name("typed-chunk.luau")
        .eval()
        .expect("typed chunk");
    assert_eq!(answer, 42.0);

    let live = rustia
        .load("local value = {}; value.self = value; return value")
        .source_name("live-chunk.luau")
        .eval_live()
        .expect("live cyclic result");
    let table = live
        .as_table(&rustia)
        .expect("live kind")
        .expect("live table");
    let self_value = table.get(&mut rustia, "self").expect("self field");
    assert!(live
        .raw_equal(&mut rustia, &self_value)
        .expect("cyclic identity"));

    let exports = rustia
        .load("return { answer = 42 }")
        .source_name("exports-chunk.luau")
        .install()
        .expect("exports chunk");
    assert_eq!(
        exports
            .get::<f64>(&mut rustia, "answer")
            .expect("answer export"),
        42.0
    );
}

#[test]
fn chunk_builder_is_available_in_every_ordinary_preset() {
    let presets = [
        ("standard", RustiaBuilder::standard()),
        ("deterministic", RustiaBuilder::deterministic()),
        ("live", RustiaBuilder::live()),
        ("puc51", RustiaBuilder::puc51_compat()),
    ];

    for (name, builder) in presets {
        let mut rustia = builder.build().expect("ordinary preset");
        let environment = rustia.create_table().expect("environment");
        environment
            .set(&mut rustia, "profile_value", 42)
            .expect("environment input");
        let answer: f64 = rustia
            .load("return profile_value")
            .source_name(format!("{name}-chunk.luau"))
            .environment(environment)
            .eval()
            .expect("profile chunk");
        assert_eq!(answer, 42.0, "preset {name}");
    }
}

#[test]
fn exec_discards_the_result_but_keeps_chunk_global_effects() {
    let mut rustia = Rustia::default();
    let globals = rustia.globals().expect("globals");

    rustia
        .load("chunk_side_effect = 42; return 'discarded'")
        .source_name("exec-chunk.luau")
        .exec()
        .expect("execute chunk");

    let value = globals
        .get(&mut rustia, "chunk_side_effect")
        .expect("global side effect");
    assert_eq!(value.as_number(&rustia).expect("number kind"), Some(42.0));
}

#[test]
fn custom_environment_is_live_and_captured_by_nested_functions() {
    let mut rustia = Rustia::default();
    let environment = rustia.create_table().expect("environment");
    environment
        .set(&mut rustia, "answer", 40)
        .expect("initial environment value");

    let function = rustia
        .load("return function() answer += 1; return answer end")
        .source_name("environment-chunk.luau")
        .environment(&environment)
        .eval_live()
        .expect("function result")
        .as_function(&rustia)
        .expect("function kind")
        .expect("function value");

    assert_eq!(
        function
            .call::<_, f64>(&mut rustia, ())
            .expect("first call"),
        41.0
    );
    assert_eq!(
        function
            .call::<_, f64>(&mut rustia, ())
            .expect("second call"),
        42.0
    );
    let answer = environment
        .get(&mut rustia, "answer")
        .expect("captured environment write");
    assert_eq!(answer.as_number(&rustia).expect("number kind"), Some(42.0));

    let globals = rustia.globals().expect("globals");
    assert!(globals
        .get(&mut rustia, "answer")
        .expect("ordinary globals")
        .is_nil(&rustia)
        .expect("nil kind"));
}

#[test]
fn custom_environment_is_validated_before_source_compilation() {
    let mut first = Rustia::default();
    let mut second = Rustia::default();
    let foreign = first.create_table().expect("foreign environment");

    let error = second
        .load("return {")
        .environment(foreign)
        .exec()
        .expect_err("cross-VM environment must reject first");
    assert!(matches!(
        error,
        Error { kind: rustia::ErrorKind::Embed(ref message), .. } if message.contains("different Rustia VM")
    ));
    let stale = second.create_table().expect("soon-stale environment");
    second.reset_decision().expect("reset");
    let error = second
        .load("return {")
        .environment(stale)
        .exec()
        .expect_err("stale environment must reject first");
    assert!(matches!(
        error,
        Error { kind: rustia::ErrorKind::Embed(ref message), .. } if message.contains("stale chunk environment handle")
    ));
}

#[test]
fn borrowed_source_bytes_and_name_feed_runtime_diagnostics() {
    let mut rustia = Rustia::default();
    let source = b"local t = {}; t[0 / 0] = 1".as_slice();
    let error = rustia
        .load(source)
        .source_name("byte-chunk.luau")
        .eval::<OwnedValue>()
        .expect_err("runtime failure");
    let location = error.source_location().expect("runtime source location");
    assert_eq!(location.source, "byte-chunk.luau");
    assert_eq!(location.line, 1);
}

#[test]
fn hardened_leaf_rejects_dynamic_loading_without_mutating_the_vm() {
    let mut rustia = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(512 * 1024)
        .build_for_host::<()>()
        .expect("hardened VM");
    assert_eq!(
        rustia.vm_settings().guest_surface,
        GuestSurface::HardenedLeaf
    );
    let before = rustia.memory_stats();

    let error = rustia
        .load("return 42")
        .source_name("forbidden-chunk.luau")
        .eval::<f64>()
        .expect_err("dynamic loading must reject");
    assert!(matches!(
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "dynamic source loading"
                }
            )
    ));
    assert_eq!(rustia.memory_stats(), before);
}
