use rustia::{Profile, Rustia};

fn main() -> rustia::Result<()> {
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);

    let exports = rustia.hot_reload(
        "ai.luau",
        "\
return {
    decide = function(value)
        return value + 1
    end,
}
",
    )?;
    let old_decide = exports.function(&mut rustia, "decide")?;
    assert_eq!(old_decide.call::<_, f64>(&mut rustia, (41.0,))?, 42.0);

    assert!(rustia.hot_reload("ai.luau", "return {").is_err());
    assert_eq!(old_decide.call::<_, f64>(&mut rustia, (9.0,))?, 10.0);

    let exports = rustia.hot_reload(
        "ai.luau",
        "\
return {
    decide = function(value)
        return value * 2
    end,
}
",
    )?;
    let new_decide = exports.function_current(&mut rustia, "decide")?;
    assert_eq!(new_decide.call::<_, f64>(&mut rustia, (21.0,))?, 42.0);
    assert!(!old_decide.is_current(&rustia));
    assert!(old_decide
        .call_current::<_, f64>(&mut rustia, (1.0,))
        .is_err());
    assert!(new_decide.is_current(&rustia));
    assert_eq!(
        new_decide.call_current::<_, f64>(&mut rustia, (21.0,))?,
        42.0
    );

    println!("hot reload example ok");
    Ok(())
}
