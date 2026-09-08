use rustia::{HostFunction, HostModule, Profile, Result, Rustia, Table, ValueType};

fn main() -> Result<()> {
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);
    let root = rustia.create_table()?;
    let shared = rustia.create_table()?;
    shared.set(&mut rustia, "answer", 42_i32)?;
    root.set_live(&mut rustia, "self", &root.as_value())?;
    root.set_live(&mut rustia, "left", &shared.as_value())?;
    root.set_live(&mut rustia, "right", &shared.as_value())?;
    let identity = rustia.create_function(|table: Table| table)?;
    root.set_live(&mut rustia, "identity", &identity.as_value())?;
    rustia
        .globals()?
        .set_live(&mut rustia, "host_root", &root.as_value())?;

    let returned = rustia
        .load("host_root.bytes = buffer.fromstring(string.char(1, 0, 254)); return host_root.identity(host_root)")
        .eval_live()?;
    let self_value = root.get(&mut rustia, "self")?;
    let left = root.get(&mut rustia, "left")?;
    let right = root.get(&mut rustia, "right")?;
    let bytes = root.get(&mut rustia, "bytes")?;

    assert_eq!(root.as_value().kind(&rustia)?, ValueType::Table);
    assert!(root.as_value().raw_equal(&mut rustia, &returned)?);
    assert!(root.as_value().raw_equal(&mut rustia, &self_value)?);
    assert!(left.raw_equal(&mut rustia, &right)?);
    assert_eq!(bytes.buffer_bytes(&mut rustia)?, Some(vec![1, 0, 254]));

    // One callback definition works for persistent configuration too.
    rustia.register_function("twice", HostFunction::pure(|n: i64| n * 2).fuel_cost(2))?;
    rustia.install_host_module(HostModule::new("maths").function("answer", || 42_i64))?;
    rustia.reset_decision()?;
    assert_eq!(
        rustia.load("return twice(maths.answer())").eval::<i64>()?,
        84
    );
    assert!(identity.call::<_, ()>(&mut rustia, ()).is_err());

    println!(
        "host-created values preserved graph identity; reset kept registrations and invalidated live callbacks"
    );
    Ok(())
}
