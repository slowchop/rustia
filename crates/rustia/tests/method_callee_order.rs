//! Colon-call ordering is derived from the pinned Luau 0.709 oracle.
use rustia::{Profile, RustiaBuilder};

#[test]
fn method_selection_observes_arguments_in_every_call_shape() {
    let source = include_str!(
        "../../../conformance/fixtures/upstream-luau-0.709/method_callee_evaluation_order.luau"
    );
    for profile in [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ] {
        let mut vm = RustiaBuilder::standard()
            .profile(profile)
            .fuel(100_000)
            .memory_limit_bytes(8 * 1024 * 1024)
            .build()
            .unwrap();
        assert_eq!(vm.eval::<String>(source).unwrap(), "pass", "{profile:?}");
        vm.reset_decision().unwrap();
        assert_eq!(
            vm.eval::<String>(source).unwrap(),
            "pass",
            "after reset: {profile:?}"
        );
    }
}
