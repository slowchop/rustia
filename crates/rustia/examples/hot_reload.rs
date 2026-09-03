use rustia::{Profile, Rustia};

fn main() -> rustia::Result<()> {
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);

    let active = rustia
        .load(
            "\
return {
    decide = function(value)
        return value + 1
    end,
}
",
        )
        .source_name("ai.luau")
        .install()?;
    let old_decide = active.function(&mut rustia, "decide")?;
    assert_eq!(old_decide.call::<_, f64>(&mut rustia, (41.0,))?, 42.0);

    assert!(rustia
        .load("return {")
        .source_name("ai.luau")
        .install()
        .is_err());
    assert_eq!(old_decide.call::<_, f64>(&mut rustia, (9.0,))?, 10.0);

    let replacement = rustia
        .load(
            "\
return {
    decide = function(value)
        return value * 2
    end,
}
",
        )
        .source_name("ai.luau")
        .install()?;
    let new_decide = replacement.function(&mut rustia, "decide")?;
    assert_eq!(new_decide.call::<_, f64>(&mut rustia, (21.0,))?, 42.0);
    assert_eq!(old_decide.call::<_, f64>(&mut rustia, (1.0,))?, 2.0);

    // The host owns the active slot and swaps only after installation succeeds.
    let active = replacement;
    assert_eq!(
        active
            .function(&mut rustia, "decide")?
            .call::<_, f64>(&mut rustia, (21.0,))?,
        42.0
    );

    println!("hot reload example ok");
    Ok(())
}
