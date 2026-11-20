//! Type grammar regressions against the pinned Luau parser.
use rustia::{ErrorKind, Profile, RustiaBuilder, SourceError};

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

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn type_function_declarations_erase_and_reject_outer_locals_before_execution() {
    let fixture = include_str!(
        "../../../conformance/fixtures/upstream-luau-0.709/type_function_declarations.luau"
    );
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard().profile(profile).build().unwrap();
        vm.insert_global("observed", 0).unwrap();
        assert_eq!(vm.eval::<String>(fixture).unwrap(), "OK", "{profile:?}");

        for source in [
            "type function (x) return x end",
            "type function t.f() end",
            "type function f<T = number>() end",
            "type function f(x:) end",
            "export type function f() if true then end",
            "type function f() break end",
            "type function f() return ... end",
        ] {
            let error = vm.exec(format!("observed=1; {source}")).unwrap_err();
            assert!(
                matches!(error.kind(), ErrorKind::Source(SourceError::Parse(_))),
                "{profile:?}, {source}: {error}"
            );
        }

        // The first two cases come from Luau 0.709's invalid_user_defined_type_functions.
        for (source, name) in [
            ("local foo=1; type function bar() print(foo) end", "foo"),
            (
                "type function foo() local v1=1; type function bar() print(v1) end end",
                "v1",
            ),
            (
                "local outer=1; type function f() local function g() outer=2 end end",
                "outer",
            ),
            ("local x=1; type function f() local x=x end", "x"),
            ("local x=1; type function f(x: typeof(x)) end", "x"),
            (
                "local function enclosing(p) type function f() return p end end",
                "p",
            ),
            ("for i=1,2 do type function f() return i end end", "i"),
        ] {
            let error = vm.exec(format!("observed=1; {source}")).unwrap_err();
            let ErrorKind::Source(SourceError::Parse(parse)) = error.kind() else {
                panic!("{profile:?}, {source}: {error}");
            };
            assert_eq!(
                parse.message,
                format!("Type function cannot reference outer local '{name}'"),
                "{profile:?}, {source}"
            );
        }
        assert_eq!(vm.eval::<i32>("return observed").unwrap(), 0, "{profile:?}");
    }
}
