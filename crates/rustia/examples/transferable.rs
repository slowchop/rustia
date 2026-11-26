//! Load a Luau function on the main thread, then move it and the idle
//! interpreter to a worker thread. The worker calls the function and sends
//! the result back.
//!
//! From the repository root, run
//! `cargo run --locked -p rustia --features send --example transferable`.
//! It prints `Worker script call returned 42`.

use rustia::Rustia;

fn main() -> rustia::Result<()> {
    let mut vm = Rustia::builder().build()?;
    let exports = vm
        .load("return { double = function(value) return value * 2 end }")
        .source_name("double.luau")
        .install()?;
    let double = exports.function(&mut vm, "double")?;

    // Move both together and let the worker own the interpreter for the call.
    let score = std::thread::spawn(move || double.call::<_, i64>(&mut vm, (21,)))
        .join()
        .expect("script worker panicked")?;

    assert_eq!(score, 42);
    println!("Worker script call returned {score}");
    Ok(())
}
