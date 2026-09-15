//! Human-readable reports preserve the ordinary error for programmatic use.
use rustia::{Result, RustiaBuilder};

fn main() -> Result<()> {
    let mut rustia = RustiaBuilder::standard().build()?;
    let result = rustia
        .load("local function attack(target)\n    return target.health\nend\nlocal function take_turn()\n    return attack(nil)\nend\ntake_turn()")
        .source_name("combat.luau")
        .exec();
    if let Err(error) = result {
        eprintln!("{}", error.report());
    }
    Ok(())
}
