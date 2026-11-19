//! Application-side JavaScript boundary for the browser setup recipe.
#![forbid(unsafe_code)]

use rustia::RustiaBuilder;
use wasm_bindgen::prelude::*;

/// Runs the race on a fresh deterministic VM and returns three distances per tick.
pub fn race(seed: u32) -> rustia::Result<Vec<u32>> {
    let mut vm = RustiaBuilder::deterministic()
        .fuel(100_000)
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()?;
    vm.insert_global("seed", seed)?;
    vm.load(include_str!("../race.luau"))
        .source_name("race.luau")
        .eval()
}

/// Runs the same race inside the browser Wasm module.
#[wasm_bindgen]
pub fn race_frames(seed: u32) -> Result<Vec<u32>, JsValue> {
    race(seed).map_err(|error| JsValue::from_str(&error.report().to_string()))
}

/// Returns a script value to JavaScript; script `print` goes to the console.
#[wasm_bindgen]
pub fn run_example() -> Result<i32, JsValue> {
    let mut vm = RustiaBuilder::live()
        .fuel(10_000)
        .memory_limit_bytes(2 * 1024 * 1024)
        .build()
        .map_err(|error| JsValue::from_str(&error.report().to_string()))?;

    vm.load("print('Hello from Luau'); return 6 * 7")
        .source_name("browser-example.luau")
        .eval::<i32>()
        .map_err(|error| JsValue::from_str(&error.report().to_string()))
}
