//! Native bench harness for the rustia walking-skeleton VM. Mirrors spike-A's harness shape so the
//! numbers are apples-to-apples: us/decision = tick_time / (n*ticks), warmup = ticks/10.

use spike_sk_rustia::*;
use std::hint::black_box;
use std::time::Instant;

fn bench_native(n: usize, ticks: usize) -> (f64, u64) {
    let mut w = World::new(n, 0xA1_BACE_50);
    for _ in 0..(ticks / 10).max(1) {
        w.tick_native();
    }
    let t0 = Instant::now();
    for _ in 0..ticks {
        w.tick_native();
    }
    let el = t0.elapsed();
    let h = black_box(w.state_hash());
    (el.as_nanos() as f64 / (n as f64 * ticks as f64) / 1000.0, h)
}

fn bench_vm_realistic<const FUEL: bool>(n: usize, ticks: usize) -> (f64, u64) {
    let prog = build_argmax_f64();
    let mut vm = Vm::new();
    let mut w = World::new(n, 0xA1_BACE_50);
    for _ in 0..(ticks / 10).max(1) {
        w.tick_vm_realistic::<FUEL>(&mut vm, &prog);
    }
    let t0 = Instant::now();
    for _ in 0..ticks {
        w.tick_vm_realistic::<FUEL>(&mut vm, &prog);
    }
    let el = t0.elapsed();
    let h = black_box(w.state_hash());
    (el.as_nanos() as f64 / (n as f64 * ticks as f64) / 1000.0, h)
}

fn main() {
    println!("# spike-sk-rustia: from-scratch register-bytecode VM on the grabby AI kernel");
    println!("# machine: aarch64-mac (Apple M4 Max). single-threaded --release opt-level 3 lto.\n");

    // --- program shape report ---
    let luts = Luts::new();
    let pf = build_argmax_f64();
    let pq = build_argmax_q16();
    println!(
        "argmax_f64 program: {} instrs, {} consts, dispatches/call = {}",
        pf.code.len(),
        pf.consts.len(),
        count_dispatches(&pf, &luts, false)
    );
    println!(
        "argmax_q16 program: {} instrs, {} consts, dispatches/call = {}\n",
        pq.code.len(),
        pq.consts.len(),
        count_dispatches(&pq, &luts, true)
    );

    let sizes = [1000usize, 5000, 10000];
    println!(
        "{:>7} | {:>12} | {:>16} | {:>16}",
        "agents", "native us/d", "vm-real(fuel) us/d", "vm-real(nofuel)"
    );
    println!("{}", "-".repeat(66));
    let mut sink: u64 = 0;
    for &n in &sizes {
        let nat_ticks = if n >= 10000 { 200 } else { 500 };
        let vm_ticks = if n >= 10000 { 150 } else { 300 };
        let (nat, h1) = bench_native(n, nat_ticks);
        let (vmf, h2) = bench_vm_realistic::<true>(n, vm_ticks);
        let (vmn, h3) = bench_vm_realistic::<false>(n, vm_ticks);
        sink ^= h1 ^ h2 ^ h3;
        println!(
            "{:>7} | {:>12.4} | {:>16.4} | {:>16.4}",
            n, nat, vmf, vmn
        );
    }
    // --- VM argmax in ISOLATION: pure us per argmax call (no Rust kernel around it). This is the
    // VM's own contribution; in grabby only argmax is scripted, so realistic above is the relevant
    // figure, while a full kernel-in-bytecode "vm-internal" (deferred) would be strictly slower. ---
    {
        let prog = build_argmax_f64();
        let mut vm = Vm::new();
        let luts2 = Luts::new();
        let iters: u64 = 20_000_000;
        // warm + rotate inputs so it isn't constant-folded
        let mut acc = 0u64;
        for w in 0..(iters / 20) {
            let f = (w & 63) as f64 / 64.0;
            vm.arr[0] = Value::num(f);
            vm.arr[1] = Value::num(1.0 - f);
            vm.arr[2] = Value::num(f * 0.5);
            vm.arr[3] = Value::num(0.25 + f * 0.5);
            vm.reg[0] = Value::num((w % 6) as f64);
            acc ^= vm.run::<true>(&prog.code, &prog.consts, &luts2, u64::MAX).0;
        }
        let t0 = Instant::now();
        for w in 0..iters {
            let f = (w & 63) as f64 / 64.0;
            vm.arr[0] = Value::num(f);
            vm.arr[1] = Value::num(1.0 - f);
            vm.arr[2] = Value::num(f * 0.5);
            vm.arr[3] = Value::num(0.25 + f * 0.5);
            vm.reg[0] = Value::num((w % 6) as f64);
            acc ^= vm.run::<true>(&prog.code, &prog.consts, &luts2, u64::MAX).0;
        }
        let el = t0.elapsed();
        black_box(acc);
        let us_call = el.as_nanos() as f64 / iters as f64 / 1000.0;
        let disp = count_dispatches(&prog, &luts2, false) as f64;
        println!(
            "\nVM argmax ISOLATION (fuel on): {:.4} us/call, {} dispatches/call -> {:.3} ns/dispatch",
            us_call,
            disp as u64,
            us_call * 1000.0 / disp
        );
    }

    println!("\n# us/d = microseconds per decision (per agent per tick) = tick_time / (n*ticks)");
    println!("# anchor: native should reproduce the ~0.149 us/d grabby floor.");
    println!("# fuel cost = (vm-real fuel) - (vm-real nofuel).");
    black_box(sink);

    // --- TIER 2: determinism. Q16.16 VM argmax must reproduce the golden state_hash. ---
    println!("\n# TIER 2 determinism: Q16.16 VM argmax (host fxmul/curve), 4000 agents, 200 ticks");
    const GOLDEN: u64 = 0x31c6_ec27_34ea_6fa2;
    let prog_q = build_argmax_q16();
    let mut vm = Vm::new();
    let mut w = World::new(4000, 0xA1_BACE_50);
    for _ in 0..200 {
        w.tick_vm_q16(&mut vm, &prog_q);
    }
    let h = w.state_hash();
    println!("golden  = 0x{GOLDEN:016x}");
    println!("vm-q16  = 0x{h:016x}");
    println!(
        "MATCH   = {}",
        if h == GOLDEN { "YES (bit-identical)" } else { "NO" }
    );

    // sanity: native floor reproduces the golden too (validates the harness/World copy)
    let mut wn = World::new(4000, 0xA1_BACE_50);
    for _ in 0..200 {
        wn.tick_native();
    }
    let hn = wn.state_hash();
    println!(
        "native floor hash = 0x{hn:016x}  (== golden: {})",
        hn == GOLDEN
    );
}
