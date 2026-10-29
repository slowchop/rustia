//! Application-side JavaScript boundary for the browser setup recipe.
#![forbid(unsafe_code)]

use rustia::RustiaBuilder;
use wasm_bindgen::prelude::*;

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
