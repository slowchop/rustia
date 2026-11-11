//! Pinned-oracle regressions exercised through the public chunk API.
use rustia::{Profile, RustiaBuilder};

#[test]
fn metamethod_sites_and_erased_types_match_the_oracle_on_each_profile() {
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        for source in [
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/generic_function_types.luau"
            ),
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/metamethod_error_sites.luau"
            ),
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/export_type_assertions.luau"
            ),
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/index_write_error_messages.luau"
            ),
        ] {
            assert_eq!(
                vm.load(source)
                    .source_name("regression.luau")
                    .eval::<String>()
                    .unwrap(),
                "OK",
                "{profile:?}"
            );
        }
    }
}

#[test]
fn metamethod_error_level_selects_the_callers_chunk() {
    let mut vm = RustiaBuilder::standard().build().unwrap();
    vm.load("t = setmetatable({}, {__add=function() error('bad add', 2) end})")
        .source_name("prelude.luau")
        .exec()
        .unwrap();
    let message: String = vm
        .load("local ok,e = pcall(function()\nreturn t + 1\nend)\nreturn e")
        .source_name("user.luau")
        .eval()
        .unwrap();
    assert_eq!(message, "user.luau:2: bad add");
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
fn cross_chunk_error_reports_quote_the_selected_source() {
    for profile in profiles() {
        for same_name in [false, true] {
            for level in [2, 3] {
                let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
                let prelude = if level == 2 {
                    "-- prelude marker\nfunction bad()\nerror('bad operand', 2)\nend"
                } else {
                    "-- prelude marker\nfunction bad()\nlocal function inner() error('bad operand', 3) end\ninner()\nend"
                };
                vm.load(prelude)
                    .source_name(if same_name {
                        "user.luau"
                    } else {
                        "prelude.luau"
                    })
                    .exec()
                    .unwrap();
                let error = vm
                    .load("-- user marker\n\nbad()")
                    .source_name("user.luau")
                    .exec()
                    .unwrap_err();
                let report = error.report().to_string();
                assert!(report.contains("user.luau:3"), "{profile:?}: {report}");
                assert!(report.contains("3 | bad()"), "{report}");
                assert!(
                    !report.contains("3 | error(") && !report.contains("local function inner"),
                    "{report}"
                );
                let span = error.source_span().unwrap();
                assert_eq!(&"-- user marker\n\nbad()"[span.start..span.end], "bad()");
                vm.reset_decision().unwrap();
                assert!(error.report().to_string().contains("3 | bad()"));
            }
        }
    }
}

#[test]
fn selected_caller_diagnostics_respect_mapping_and_source_budgets() {
    for profile in profiles() {
        for retain_source in [false, true] {
            let mut vm = RustiaBuilder::standard()
                .profile(profile)
                .diagnostic_limits(
                    rustia::DiagnosticLimits::default()
                        .with_retained_source_bytes(if retain_source { 1024 * 1024 } else { 0 }),
                )
                .build()
                .unwrap();
            vm.load("function bad() error('bad operand', 2) end")
                .source_name("prelude.luau")
                .exec()
                .unwrap();
            let error = vm
                .load("return (function() bad() end)()")
                .source_name("mapped.luau")
                .source_map(rustia::SourceMap::generated_prefix(7))
                .exec()
                .unwrap_err();
            let diagnostic = error.diagnostic().unwrap();
            assert_eq!(diagnostic.source.as_deref(), Some("mapped.luau"));
            assert_eq!(diagnostic.snapshot.is_some(), retain_source);
            assert!(!diagnostic.generated_source);
            let report = error.report().to_string();
            assert_eq!(
                report.contains("1 | (function() bad() end)()"),
                retain_source,
                "{report}"
            );
        }
    }
}

#[test]
fn wrapped_coroutine_report_keeps_the_original_chunk() {
    for profile in profiles() {
        for yielded in [false, true] {
            let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
            let source = format!("-- coroutine marker\nworker = coroutine.wrap(function()\n{}\nerror('worker failed')\nend)", if yielded { "coroutine.yield()" } else { "-- no yield" });
            vm.load(&source).source_name("worker.luau").exec().unwrap();
            if yielded {
                vm.load("worker()")
                    .source_name("first-resume.luau")
                    .exec()
                    .unwrap();
            }
            let error = vm
                .load("worker()")
                .source_name("resume.luau")
                .exec()
                .unwrap_err();
            assert_eq!(error.source_location().unwrap().source, "worker.luau");
            let report = error.report().to_string();
            assert!(
                report.contains("--> worker.luau:4"),
                "{profile:?}: {report}"
            );
            assert!(report.contains("4 | error('worker failed')"), "{report}");
        }
    }
}

#[test]
fn manual_coroutine_string_rethrow_keeps_its_explicit_error_site() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        vm.load("worker = coroutine.create(function() error('worker failed') end)")
            .source_name("worker.luau")
            .exec()
            .unwrap();
        let error = vm
            .load("local ok, message = coroutine.resume(worker)\nif not ok then error(message) end")
            .source_name("manual-rethrow.luau")
            .exec()
            .unwrap_err();
        assert!(error
            .report()
            .to_string()
            .contains("--> manual-rethrow.luau:2"));
    }
}

#[test]
fn selected_error_level_in_generated_caller_does_not_blame_user_input() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        vm.load("function bad() error('bad wrapper', 2) end")
            .source_name("prelude.luau")
            .exec()
            .unwrap();
        let prefix = "bad(); return ";
        let error = vm
            .load(&format!("{prefix}42"))
            .source_name("wrapped.luau")
            .source_map(rustia::SourceMap::generated_prefix(prefix.len()))
            .exec()
            .unwrap_err();
        assert!(error.diagnostic().unwrap().generated_source);
        let report = error.report().to_string();
        assert!(report.contains("generated wrapper code"), "{report}");
        assert!(!report.contains("1 | 42"), "{report}");
    }
}

#[test]
fn invalid_type_assertions_and_defaults_reject_before_guest_effects() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        vm.insert_global("observed", 0).unwrap();
        for declaration in [
            "local value = 1 :: ()",
            "local value = 1 :: (number, string)",
            "local value = 1 :: ((number,string))",
            "local value = 1 :: (number,string)?",
            "local value = 1 :: (number,string) | number",
            "local value = 1 :: ...number",
            "type Box<T = number, U> = T",
            "type Box<T = number, P...> = T",
            "type Box<T = (number,string)> = T",
            "type Box<P... = number> = number",
            "type Box<P... = (...number, string)> = number",
            "local function f<T = number>() end",
        ] {
            let source = format!("observed = 1; {declaration}; return 'wrong'");
            let error = vm
                .load(&source)
                .source_name("invalid-type.luau")
                .exec()
                .unwrap_err();
            assert!(
                matches!(error.kind(), rustia::ErrorKind::Source(_)),
                "{profile:?}: {declaration}: {error}"
            );
            assert_eq!(
                vm.eval::<i32>("return observed").unwrap(),
                0,
                "{declaration}"
            );
        }
        assert_eq!(vm.eval::<i32>("return 42").unwrap(), 42);
    }
}

#[test]
fn generic_function_types_erase_in_assertions_and_alias_defaults() {
    for profile in profiles() {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        assert!(vm
            .eval::<bool>(
                r#"
            type Identity = <T>(T) -> T
            type Box<F = <T>(T) -> T> = { value: F }
            local function id(value) return value end
            local typed = id :: Identity
            local box = { value = id } :: Box
            local function many(...) return ... end
            local packed = many :: <P...>(P...) -> P...
            return typed(42) == 42 and typed('ok') == 'ok'
                and box.value(true) and select('#', packed(1,2)) == 2
                and select('#', packed(1,2) :: (<T>(T)->T)) == 1
        "#
            )
            .unwrap());
        vm.insert_global("observed", 0).unwrap();
        for ty in [
            "<T>(T)",
            "<T=number>(T)->T",
            "<P...,T>(P...)->T",
            "<T>(T...,number)->T",
            "<T>(x:T...)->T",
        ] {
            let error = vm
                .exec(format!("observed=1; local value = nil :: ({ty})"))
                .unwrap_err();
            assert!(
                matches!(error.kind(), rustia::ErrorKind::Source(_)),
                "{profile:?}: {ty}: {error}"
            );
            assert_eq!(vm.eval::<i32>("return observed").unwrap(), 0);
        }
    }
}
