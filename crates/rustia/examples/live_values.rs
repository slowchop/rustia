//! Keep shared and cyclic tables as live handles, then reset guest state.
//!
//! Run from the repository root:
//! ```sh
//! cargo run --locked -p rustia --example live_values
//! ```
//! Expected output:
//! ```text
//! host-created values preserved graph identity; reset kept registrations and invalidated live callbacks
//! ```
//! Assertions check shared table identity, binary bytes, persistent registrations,
//! and rejection of a live function handle after reset.

use rustia::{HostFunction, HostModule, Result, RustiaBuilder, Table, ValueType, VmErrorKind};

fn main() -> Result<()> {
    let mut rustia = RustiaBuilder::standard().build()?;

    // Build a cycle and two references to the same table inside the VM.
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

    // Return a live handle so the table graph and binary buffer retain their identity.
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

    // Persistent registrations are restored by reset; the live `identity` function expires.
    rustia.register_function("twice", HostFunction::pure(|n: i64| n * 2).fuel_cost(2))?;
    rustia.install_host_module(HostModule::new("maths").function("answer", || 42_i64))?;
    rustia.reset_decision()?;
    assert_eq!(
        rustia.load("return twice(maths.answer())").eval::<i64>()?,
        84
    );
    let error = identity
        .call::<_, ()>(&mut rustia, ())
        .expect_err("reset must invalidate the live function handle");
    assert!(matches!(
        error.as_vm_error().map(|error| error.kind()),
        Some(VmErrorKind::StaleRetainedValue { .. })
    ));

    println!(
        "host-created values preserved graph identity; reset kept registrations and invalidated live callbacks"
    );
    Ok(())
}
