use rustia::{ModuleSource, Rustia, ServiceEntropy, ServiceModuleResolver};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = raceOutput, js_name = write)]
    fn write_output(text: &str);
}

#[wasm_bindgen]
pub fn run_race(source: &str, rules: String, initial_seed: u32) -> Result<(), String> {
    let modules = ServiceModuleResolver::from_fn(move |name, _| match name {
        "./demos/web-live/race" => Ok(ModuleSource::new("race.luau", rules.as_bytes())),
        _ => Err(rustia::Error::embed(format!("Unknown module: {name}"))),
    });

    let mut vm = Rustia::builder()
        .entropy(ServiceEntropy::fixed(u64::from(initial_seed)))
        .module_resolver(modules)
        .stdout(write_output)
        .fuel(250_000)
        .memory_limit_bytes(8 * 1024 * 1024)
        .build()
        .map_err(|e| e.to_string())?;
    vm.load(source)
        .source_name("standings.luau")
        .exec()
        .map_err(|e| e.to_string())
}
