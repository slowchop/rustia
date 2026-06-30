//! Native helper: emit the runtime-generated wasm modules to disk (for inspection / node to load
//! as a cross-check) and confirm the interpreter and static-AOT drivers produce an identical
//! checksum natively before we ever touch V8.

use spike_wasmjit::{emit_module_cfg, run_aot_impl, run_interp_impl};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let seed = 0x1234_5678i32;
    let iters = 2_000_000i32;

    // native cross-path checksum sanity
    let a = run_aot_impl(iters, seed);
    let i = run_interp_impl(iters, seed);
    println!("native run_aot    checksum = 0x{a:016x}");
    println!("native run_interp checksum = 0x{i:016x}");
    println!(
        "native interp==aot : {}",
        if a == i { "MATCH" } else { "*** MISMATCH ***" }
    );

    // native steady-state timing (sanity; the real numbers come from V8). black_box the inputs so
    // LLVM cannot CSE these against the checksum calls above (the fn is pure).
    let t0 = Instant::now();
    let h = black_box(run_aot_impl(black_box(iters), black_box(seed)));
    let aot_ns = t0.elapsed().as_nanos() as f64 / iters as f64;
    let t0 = Instant::now();
    let h2 = black_box(run_interp_impl(black_box(iters), black_box(seed)));
    let int_ns = t0.elapsed().as_nanos() as f64 / iters as f64;
    println!("native aot    ns/op = {aot_ns:.3}  (chk 0x{h:016x})");
    println!("native interp ns/op = {int_ns:.3}  (chk 0x{h2:016x})");

    // emit + write the runtime-generated modules (looped + unrolled, pure + shared)
    let gen = emit_module_cfg(false, false);
    let gen_opt = emit_module_cfg(false, true);
    let shared = emit_module_cfg(true, false);
    let shared_opt = emit_module_cfg(true, true);
    std::fs::write("gen_run.wasm", &gen).unwrap();
    std::fs::write("gen_run_opt.wasm", &gen_opt).unwrap();
    std::fs::write("gen_shared.wasm", &shared).unwrap();
    std::fs::write("gen_shared_opt.wasm", &shared_opt).unwrap();
    println!("emitted gen_run.wasm        : {} bytes", gen.len());
    println!("emitted gen_run_opt.wasm    : {} bytes", gen_opt.len());
    println!("emitted gen_shared.wasm     : {} bytes", shared.len());
    println!("emitted gen_shared_opt.wasm : {} bytes", shared_opt.len());
}
