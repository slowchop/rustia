#![cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    not(feature = "web-live")
))]

use rustia::{Error, RustiaBuilder, ServiceClock, ServiceEntropy, VmErrorKind};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn bare_wasm_live_accepts_explicit_host_inputs() {
    let mut rustia = RustiaBuilder::live()
        .wall_clock(ServiceClock::from_nondeterministic_fn(|| 1_700_000_042.0))
        .monotonic_clock(ServiceClock::from_nondeterministic_fn(|| 7.5))
        .entropy(ServiceEntropy::from_nondeterministic_fn(|| 42))
        .build()
        .unwrap();

    assert_eq!(
        rustia.run::<f64>("return os.time()").unwrap(),
        1_700_000_042.0
    );
    assert_eq!(rustia.run::<f64>("return os.clock()").unwrap(), 7.5);
}

#[wasm_bindgen_test]
fn bare_wasm_live_reports_a_missing_platform_adapter() {
    let result = RustiaBuilder::live().build();
    let Err(Error::Vm(error)) = result else {
        panic!("expected a typed missing-adapter error");
    };
    assert!(matches!(error.kind, VmErrorKind::Unsupported { .. }));
}
