#![cfg(all(target_arch = "wasm32", target_os = "unknown", feature = "web-live"))]

use rustia::RustiaBuilder;
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn default_web_live_adapter_supplies_browser_host_services() {
    let mut rustia = RustiaBuilder::live().build().unwrap();

    let wall: f64 = rustia.run("return os.time()").unwrap();
    assert!(wall > 1_700_000_000.0);

    let monotonic: f64 = rustia.run("return os.clock()").unwrap();
    assert!(monotonic >= 0.0);

    let random: f64 = rustia.run("return math.random()").unwrap();
    assert!((0.0..1.0).contains(&random));

    let answer: f64 = rustia.run("return loadstring('return 40 + 2')()").unwrap();
    assert_eq!(answer, 42.0);
}
