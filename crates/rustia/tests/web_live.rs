#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]

use rustia::RustiaBuilder;
use serde::Deserialize;
use wasm_bindgen_test::wasm_bindgen_test;

#[derive(Debug, Deserialize, PartialEq)]
struct LiveSerdeProbe {
    label: String,
    values: Vec<u32>,
}

#[wasm_bindgen_test]
fn default_web_live_adapter_supplies_browser_host_services() {
    let mut rustia = RustiaBuilder::live().build().unwrap();

    let wall: f64 = rustia.eval("return os.time()").unwrap();
    assert!(wall > 1_700_000_000.0);

    let monotonic: f64 = rustia.eval("return os.clock()").unwrap();
    assert!(monotonic >= 0.0);

    let random: f64 = rustia.eval("return math.random()").unwrap();
    assert!((0.0..1.0).contains(&random));

    let answer: f64 = rustia.eval("return loadstring('return 40 + 2')()").unwrap();
    assert_eq!(answer, 42.0);
}

#[wasm_bindgen_test]
fn web_live_deserializes_rooted_live_values() {
    let mut rustia = RustiaBuilder::live().build().unwrap();
    let value = rustia
        .load("return { label = 'web', values = { 4, 5, 6 } }")
        .eval_live()
        .unwrap();
    assert_eq!(
        value.deserialize::<LiveSerdeProbe>(&mut rustia).unwrap(),
        LiveSerdeProbe {
            label: "web".to_owned(),
            values: vec![4, 5, 6],
        }
    );
}
