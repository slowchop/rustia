//! Return/table grammar regressions against the pinned Luau parser.
use rustia::{ErrorKind, Profile, RustiaBuilder};

#[test]
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
