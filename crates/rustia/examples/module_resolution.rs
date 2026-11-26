//! Teach Luau’s `require` where to find a module. When the script asks for
//! `game/config`, Rust supplies the module from a string.

use rustia::{ModuleSource, RustiaBuilder, ServiceModuleResolver};

fn main() -> rustia::Result<()> {
    let resolver = ServiceModuleResolver::from_fn(|requested, referrer| match requested {
        "game/config" => Ok(ModuleSource::with_source_name(
            "game/config@v1",
            "modules/game/config.luau",
            "return { difficulty = 6, party_size = 7 }",
        )),
        _ => Err(rustia::Error::embed(format!(
            "module {requested:?} was not found for {}",
            referrer.unwrap_or("<unknown>")
        ))),
    });

    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()?;
    let answer: f64 = vm
        .load(
            r#"
            local config = require("game/config")
            return config.difficulty * config.party_size
        "#,
        )
        .source_name("app/main.luau")
        .eval()?;

    assert_eq!(answer, 42.0);
    Ok(())
}
