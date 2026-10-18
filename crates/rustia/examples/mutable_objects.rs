//! Standard/live objects can change application state immediately.

use rustia::{Error, ObjectType, Result, RustiaBuilder};

struct Counter {
    value: i64,
}

fn main() -> Result<()> {
    let mut lua = RustiaBuilder::standard().build()?;
    lua.register_object_type(
        ObjectType::<Counter>::new("Counter")
            .method("value", 1, |counter: &Counter| counter.value)
            .method_mut(
                "add",
                1,
                |counter: &mut Counter, amount: i64| -> Result<()> {
                    counter.value = counter
                        .value
                        .checked_add(amount)
                        .ok_or_else(|| Error::embed("counter is out of range"))?;
                    Ok(())
                },
            ),
    )?;
    let counter = lua.create_object(Counter { value: 0 })?;
    lua.globals()?
        .set_live(&mut lua, "counter", &counter.as_value())?;

    let error = lua
        .exec("counter:add(3); error('later failure')")
        .unwrap_err();
    // An immediate application change survives a later script error.
    assert_eq!(counter.call_method::<_, i64>(&mut lua, "value", ())?, 3);
    println!("Counter is 3 after: {error}");
    Ok(())
}
