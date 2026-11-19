use rustia::{Function, Rustia, RustiaBuilder};
use wasm_bindgen::prelude::*;

/// Keeps the active movement rule and racer position across editor changes.
#[wasm_bindgen]
pub struct LiveRacer {
    vm: Rustia,
    advance: Function,
    position: u32,
}

fn movement_rule(vm: &mut Rustia, source: &str) -> rustia::Result<Function> {
    let exports = vm.load(source).source_name("movement.luau").install()?;
    exports.function(vm, "advance")
}

fn diagnostic(error: rustia::Error) -> JsValue {
    JsValue::from_str(&error.report().to_string())
}

#[wasm_bindgen]
impl LiveRacer {
    /// Starts one session with an installed Luau movement rule.
    #[wasm_bindgen(constructor)]
    pub fn new(source: &str) -> Result<Self, JsValue> {
        let mut vm = RustiaBuilder::deterministic()
            .fuel(100_000)
            .memory_limit_bytes(2 * 1024 * 1024)
            .build()
            .map_err(diagnostic)?;
        let advance = movement_rule(&mut vm, source).map_err(diagnostic)?;
        Ok(Self {
            vm,
            advance,
            position: 0,
        })
    }

    /// Replaces the active function only after the new export is available.
    pub fn reload(&mut self, source: &str) -> Result<(), JsValue> {
        self.advance = movement_rule(&mut self.vm, source).map_err(diagnostic)?;
        Ok(())
    }

    /// Runs one tick of the active rule from the last position.
    pub fn step(&mut self) -> Result<u32, JsValue> {
        self.position = self
            .advance
            .call(&mut self.vm, self.position)
            .map_err(diagnostic)?;
        Ok(self.position)
    }
}
