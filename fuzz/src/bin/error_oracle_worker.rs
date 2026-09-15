//! One bounded source per process. The supervisor enforces wall/process limits.
use std::io::{self, Read};

fn main() {
    let mut source = Vec::new();
    io::stdin().take(16_385).read_to_end(&mut source).unwrap();
    assert!(source.len() <= 16_384, "source exceeds worker bound");
    match std::env::args().nth(1).as_deref() {
        Some("luau") => run_luau(&source),
        Some("rustia") => run_rustia(&source),
        _ => panic!("expected luau or rustia engine"),
    }
}

fn run_luau(source: &[u8]) {
    let lua = mlua::Lua::new();
    lua.set_memory_limit(8 * 1024 * 1024).unwrap();
    lua.globals().set("observed", 0).unwrap();
    let function = match lua
        .load(source)
        .set_name("differential.luau")
        .into_function()
    {
        Ok(function) => function,
        Err(_) => {
            println!("frontend\n0");
            return;
        }
    };
    let result = function.call::<mlua::Value>(());
    let observed: i32 = lua.globals().get("observed").unwrap();
    match result {
        Ok(mlua::Value::String(value)) => println!("ok\n{observed}\n{}", value.to_str().unwrap()),
        Ok(_) => panic!("generator must return a string"),
        Err(_) => println!("runtime\n{observed}"),
    }
}

fn run_rustia(source: &[u8]) {
    let mut vm = rustia::RustiaBuilder::standard()
        .profile(rustia::Profile::LuauCompat)
        .fuel(100_000)
        .memory_limit_bytes(8 * 1024 * 1024)
        .build()
        .unwrap();
    vm.insert_global("observed", 0).unwrap();
    let result = vm
        .load(source)
        .source_name("differential.luau")
        .eval::<String>();
    let observed: i32 = vm.eval("return observed").unwrap();
    match result {
        Ok(value) => println!("ok\n{observed}\n{value}"),
        Err(error) => match error.kind() {
            rustia::ErrorKind::Source(_) | rustia::ErrorKind::Compile(_) => {
                println!("frontend\n{observed}")
            }
            rustia::ErrorKind::Vm(_) => println!("runtime\n{observed}"),
            _ => panic!("unexpected boundary: {error:?}"),
        },
    }
}
