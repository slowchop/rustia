mod support {
    pub mod mixed_heap_scenario;
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn mixed_heap_pressure_plateaus_and_recovers_atomically() {
    let vm = rustia::RustiaBuilder::standard()
        .memory_limit_bytes(support::mixed_heap_scenario::MEMORY_LIMIT_BYTES)
        .build()
        .expect("mixed-heap pressure VM");
    support::mixed_heap_scenario::run_pressure_scenario(
        vm,
        support::mixed_heap_scenario::PlateauChecks::DecisionWaves,
    );
}
