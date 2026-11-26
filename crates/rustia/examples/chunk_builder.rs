//! Give a Luau script a filename for error reports, then show how to give it
//! its own globals. Use `eval` to get a value back and `exec` to run a script
//! that sets a flag.
//!
//! From the repository root, run
//! `cargo run --locked -p rustia --example chunk_builder`. It checks that both
//! calculations return 42 and the flag is set, then exits quietly.

use rustia::RustiaBuilder;

fn main() -> rustia::Result<()> {
    let mut rustia = RustiaBuilder::standard().build()?;

    // Label source for diagnostics; eval converts its returned value to Rust.
    let answer: f64 = rustia
        .load("return 40 + 2")
        .source_name("answer.luau")
        .eval()?;
    assert_eq!(answer, 42.0);

    // This table replaces the environment; default globals are not inherited.
    let environment = rustia.create_table()?;
    environment.set(&mut rustia, "input", 21)?;
    let doubled: f64 = rustia
        .load("return input * 2")
        .source_name("sandboxed.luau")
        .environment(&environment)
        .eval()?;
    assert_eq!(doubled, 42.0);

    // Use exec when the script changes guest state but Rust needs no return value.
    rustia
        .load("chunk_builder_ran = true")
        .source_name("side-effect.luau")
        .exec()?;
    let ran: bool = rustia.eval("return chunk_builder_ran")?;
    assert!(ran);

    Ok(())
}
