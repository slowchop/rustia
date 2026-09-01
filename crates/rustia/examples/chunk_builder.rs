use rustia::RustiaBuilder;

fn main() -> rustia::Result<()> {
    let mut rustia = RustiaBuilder::standard().build()?;

    let answer: f64 = rustia.load("return 40 + 2").name("answer.luau").eval()?;
    assert_eq!(answer, 42.0);

    let environment = rustia.create_table()?;
    environment.set(&mut rustia, "input", 21)?;
    let doubled: f64 = rustia
        .load("return input * 2")
        .name("sandboxed.luau")
        .environment(&environment)
        .eval()?;
    assert_eq!(doubled, 42.0);

    rustia
        .load("chunk_builder_ran = true")
        .name("side-effect.luau")
        .exec()?;
    let ran: bool = rustia.run("return chunk_builder_ran")?;
    assert!(ran);

    Ok(())
}
