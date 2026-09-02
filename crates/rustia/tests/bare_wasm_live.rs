#![cfg(all(
    target_arch = "wasm32",
    target_os = "unknown",
    not(feature = "web-live")
))]

use rustia::{Error, RustiaBuilder, ServiceClock, ServiceEntropy, VmErrorKind};
use serde::Deserialize;
use wasm_bindgen_test::wasm_bindgen_test;

#[derive(Debug, Deserialize, PartialEq)]
struct LiveSerdeProbe {
    label: String,
    values: Vec<u32>,
}

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

#[wasm_bindgen_test]
fn wasm_buffer_float_round_trips_preserve_nan_payload_bits() {
    let mut rustia = RustiaBuilder::deterministic().build().unwrap();
    let exact: bool = rustia
        .run(
            r#"
            local f32_source = buffer.create(4)
            buffer.writeu32(f32_source, 0, 0x7fc00001)
            local f32_target = buffer.create(4)
            buffer.writef32(f32_target, 0, buffer.readf32(f32_source, 0))

            local f64_source = buffer.create(8)
            buffer.writeu32(f64_source, 0, 0x00000001)
            buffer.writeu32(f64_source, 4, 0x7ff80000)
            local f64_target = buffer.create(8)
            buffer.writef64(f64_target, 0, buffer.readf64(f64_source, 0))

            return buffer.readu32(f32_target, 0) == 0x7fc00001
                and buffer.readu32(f64_target, 0) == 0x00000001
                and buffer.readu32(f64_target, 4) == 0x7ff80000
            "#,
        )
        .unwrap();
    assert!(exact);
}

#[wasm_bindgen_test]
fn bare_wasm_deserializes_rooted_live_values() {
    let mut rustia = RustiaBuilder::deterministic().build().unwrap();
    let value = rustia
        .run_value("return { label = 'bare', values = { 1, 2, 3 } }")
        .unwrap();
    assert_eq!(
        value.deserialize::<LiveSerdeProbe>(&mut rustia).unwrap(),
        LiveSerdeProbe {
            label: "bare".to_owned(),
            values: vec![1, 2, 3],
        }
    );
}
