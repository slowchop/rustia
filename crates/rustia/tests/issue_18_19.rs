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
                "../../../conformance/fixtures/upstream-luau-0.709/metamethod_error_sites.luau"
            ),
            include_str!(
                "../../../conformance/fixtures/upstream-luau-0.709/export_type_assertions.luau"
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
