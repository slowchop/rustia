//! Return/table grammar regressions against the pinned Luau parser.
use rustia::{ErrorKind, Profile, RustiaBuilder};

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn return_and_table_grammar_match_on_all_profiles_before_guest_effects() {
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        for source in [
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/return_type_annotations.luau"
            ),
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/table_type_grammar.luau"
            ),
        ] {
            assert_eq!(vm.eval::<String>(source).unwrap(), "OK", "{profile:?}");
        }
        for annotation in [
            "(number)?",
            "(number) | nil",
            "(number,T...)?",
            "(number,...string) | nil",
            "((number))?",
            "(number,U...) & nil",
        ] {
            assert_eq!(
                vm.eval::<i32>(format!(
                    "local function f(): {annotation} return 7 end; return f()"
                ))
                .unwrap(),
                7,
                "{profile:?}: {annotation}"
            );
        }
        vm.exec(
            "type T = {x: | number | string, y: & A & B}; type U = {[| number | string]:number}",
        )
        .unwrap();
        vm.insert_global("observed", 0).unwrap();
        for invalid in [
            "local function f(): ((number,string)) return nil end",
            "local function f(): T...? return nil end",
            "local function f(): (number,string)? return nil end",
            "local function f(): (number,string)->() | nil return nil end",
            "local function f(): (number,string)->(number,string)? return nil end",
            "type T = {x:}",
            "type T = {this is not a type}",
            "type T = {[string]:number,[number]:string}",
            "type T = {number,}",
            "type T = {x:(number,string)}",
            "type T = {[`field{1}`]:number}",
            "type T = {[`plain`]:number}",
            "type T = {read [`field{1}`]:number}",
            "type T = {x: A | B & C}",
            "type T = {x: A & B?}",
        ] {
            let error = vm.exec(format!("observed=1; {invalid}")).unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Source(_)),
                "{profile:?}: {error}"
            );
            assert_eq!(vm.eval::<i32>("return observed").unwrap(), 0);
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn typeof_expressions_parse_without_executing_on_any_profile() {
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        assert_eq!(
            vm.eval::<String>(include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/typeof_expressions.luau"
            ))
            .unwrap(),
            "OK"
        );
        vm.insert_global("observed", 0).unwrap();
        assert_eq!(
            vm.eval::<i32>(
                r#"
            type T = typeof((function() observed=1; return 7 end)())
            type U = typeof(`value {error('never execute')}`)
            type V = typeof(function(x:number):number return x end)
            local value = 42 :: typeof(error('never execute'))
            return value + observed
        "#
            )
            .unwrap(),
            42
        );
        for expression in [
            "",
            "1+",
            "this is not an expression",
            "local x = 1",
            "return 1",
            "1,2",
            "function() break end",
            "function() continue end",
            "function() return ... end",
            "function() local x=... end",
            "function(...) return function() return ... end end",
        ] {
            let error = vm
                .exec(format!("observed=1; type T = typeof({expression})"))
                .unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Source(_)),
                "{profile:?}: {error}"
            );
            assert_eq!(vm.eval::<i32>("return observed").unwrap(), 0);
        }
        for ordinary in [
            "local function f() break end",
            "local function f() return ... end",
        ] {
            let error = vm.exec(ordinary).unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Compile(_)),
                "{profile:?}: {error}"
            );
        }
        let mut deep = "number".to_owned();
        for _ in 0..1200 {
            deep = format!("typeof(function(): {deep} end)");
        }
        let error = vm.exec(format!("observed=1; type T={deep}")).unwrap_err();
        assert!(
            matches!(error.kind(), ErrorKind::Source(_)),
            "{profile:?}: {error}"
        );
        assert_eq!(vm.eval::<i32>("return observed").unwrap(), 0);
    }
}
