use rustia::{Function, ModuleSource, Rustia, ServiceModuleResolver, SourceMap};
use std::collections::BTreeMap;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = playgroundOutput, js_name = write)]
    pub(crate) fn write(text: &str);
}

// The resolver can only see files in this editor project.
pub(crate) fn project_sources(
    names: Vec<String>,
    sources: Vec<String>,
) -> rustia::Result<(String, ServiceModuleResolver)> {
    project_entry(names.into_iter().zip(sources).collect(), "main.luau")
}

pub(crate) fn project_entry(
    files: BTreeMap<String, String>,
    entry: &str,
) -> rustia::Result<(String, ServiceModuleResolver)> {
    let main = files
        .get(entry)
        .ok_or_else(|| rustia::Error::embed(format!("Missing {entry}")))?
        .clone();
    let resolver = ServiceModuleResolver::from_fn(move |name, _| {
        let stem = name.strip_prefix("./").ok_or_else(|| {
            rustia::Error::embed(format!(
                "Use a project-relative module name such as './{name}'"
            ))
        })?;
        let file = if stem.ends_with(".luau") {
            stem.to_owned()
        } else {
            format!("{stem}.luau")
        };
        let source = files
            .get(&file)
            .ok_or_else(|| rustia::Error::embed(format!("No project file named {file}")))?;
        Ok(ModuleSource::new(file, source.as_bytes()))
    });
    Ok((main, resolver))
}

#[wasm_bindgen]
pub struct ConsoleProgram {
    vm: Rustia,
    resume: Function,
}

#[wasm_bindgen]
impl ConsoleProgram {
    #[wasm_bindgen(constructor)]
    pub fn new(
        names: Vec<String>,
        sources: Vec<String>,
        seed: u32,
    ) -> Result<ConsoleProgram, String> {
        Self::load(names, sources, seed).map_err(|error| error.to_string())
    }

    // True means the same VM is suspended at readLine().
    pub fn advance(&mut self, line: Option<String>) -> Result<bool, String> {
        match line {
            Some(line) => self.resume.call(&mut self.vm, (line,)),
            None => self.resume.call(&mut self.vm, ()),
        }
        .map_err(|error| error.to_string())
    }
}

impl ConsoleProgram {
    fn load(names: Vec<String>, sources: Vec<String>, seed: u32) -> rustia::Result<Self> {
        let (main, resolver) = project_sources(names, sources)?;
        let mut vm = Rustia::builder()
            .module_resolver(resolver)
            .entropy(rustia::ServiceEntropy::fixed(u64::from(seed)))
            .stdout(write)
            .fuel(250_000)
            .memory_limit_bytes(8 * 1024 * 1024)
            .build()?;
        let driver = vm
            .load(include_str!("console-input.luau"))
            .source_name("console-input.luau")
            .eval_live()?
            .as_function(&vm)?
            .ok_or_else(|| rustia::Error::embed("Missing console input driver"))?;
        // Check the user's source directly so syntax errors point into the editor,
        // including a missing token at EOF before the generated closing 'end'.
        Rustia::compile_entry("main.luau", &main)?;
        const PREFIX: &str = "return function(...) ";
        let program = vm
            .load(&format!("{PREFIX}{main}\nend"))
            .source_name("main.luau")
            .source_map(SourceMap::generated_prefix(PREFIX.len()))
            .eval_live()?;
        let resume = driver
            .call_values(&mut vm, &[program])?
            .into_iter()
            .next()
            .ok_or_else(|| rustia::Error::embed("Missing console resume callback"))?
            .as_function(&vm)?
            .ok_or_else(|| rustia::Error::embed("Invalid console resume callback"))?;
        Ok(Self { vm, resume })
    }
}
#[wasm_bindgen]
pub struct ConsoleRepl {
    vm: Rustia,
    display: Function,
}

#[wasm_bindgen]
impl ConsoleRepl {
    #[wasm_bindgen(constructor)]
    pub fn new(names: Vec<String>, sources: Vec<String>) -> Result<ConsoleRepl, String> {
        Self::load(names, sources).map_err(|error| error.to_string())
    }

    pub fn evaluate(&mut self, source: &str) -> Result<(), String> {
        // Try an expression first. A failed compilation does not run user code.
        let expression = format!("return {source}");
        if Rustia::compile_entry("console", &expression).is_ok() {
            let value = self
                .vm
                .load(&expression)
                .source_name("console")
                .eval_live()
                .map_err(|error| error.to_string())?;
            if !value.is_nil(&self.vm).map_err(|error| error.to_string())? {
                self.display
                    .call_values(&mut self.vm, &[value])
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        } else {
            self.vm
                .load(source)
                .source_name("console")
                .exec()
                .map_err(|error| error.to_string())
        }
    }
}

impl ConsoleRepl {
    fn load(names: Vec<String>, sources: Vec<String>) -> rustia::Result<Self> {
        let (_, resolver) = project_sources(names, sources)?;
        let mut vm = Rustia::builder()
            .module_resolver(resolver)
            .stdout(write)
            .fuel(250_000)
            .memory_limit_bytes(8 * 1024 * 1024)
            .build()?;
        let display = vm
            .globals()?
            .get(&mut vm, "print")?
            .as_function(&vm)?
            .ok_or_else(|| rustia::Error::embed("Missing console output"))?;
        Ok(Self { vm, display })
    }
}
#[wasm_bindgen]
pub fn run_project(names: Vec<String>, sources: Vec<String>) -> Result<(), String> {
    let (main, resolver) = project_sources(names, sources).map_err(|error| error.to_string())?;
    let mut vm = Rustia::builder()
        .module_resolver(resolver)
        .stdout(write)
        .fuel(250_000)
        .memory_limit_bytes(8 * 1024 * 1024)
        .build()
        .map_err(|error| error.to_string())?;
    vm.load(&main)
        .source_name("main.luau")
        .exec()
        .map_err(|error| error.to_string())
}
