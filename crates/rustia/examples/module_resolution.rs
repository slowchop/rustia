use rustia::{ModuleSource, RustiaBuilder, ServiceModuleResolver};

fn main() -> rustia::Result<()> {
    let resolver = ServiceModuleResolver::from_fn(|requested, referrer| match requested {
        "game/config" => Ok(ModuleSource::named(
            "game/config@v1",
            "modules/game/config.luau",
            "return { difficulty = 6, party_size = 7 }",
        )),
        _ => Err(format!(
            "module {requested:?} was not found for {}",
            referrer.unwrap_or("<unknown>")
        )),
    });

    let mut vm = RustiaBuilder::deterministic()
        .module_resolver(resolver)
        .build()?;
    let answer: f64 = vm.run_named(
        "app/main.luau",
        r#"
            local config = require("game/config")
            return config.difficulty * config.party_size
        "#,
    )?;

    assert_eq!(answer, 42.0);
    Ok(())
}
