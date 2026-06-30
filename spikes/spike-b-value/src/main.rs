//! Native harness for spike-B. Times each heap microbench + the IAUS kernel (canon on/off) for
//! NaN-box(8B) / tagged(16B) / fat(24B), prints ns/op + us/decision, verifies the golden + the
//! NaN-box soundness scan. Honest: black_box, churned data-dependent inputs, best-of-N.

use spike_b_value::*;
use std::hint::black_box;
use std::time::Instant;

fn time_ns_per_op(iters: u64, runs: u32, mut f: impl FnMut(u64) -> u64) -> (f64, u64) {
    // warm
    let mut chk = 0u64;
    for _ in 0..3 {
        chk ^= f(iters / 10 + 1);
    }
    let mut best = f64::INFINITY;
    for _ in 0..runs {
        let t0 = Instant::now();
        chk ^= black_box(f(iters));
        let el = t0.elapsed().as_nanos() as f64;
        let per = el / iters as f64;
        if per < best {
            best = per;
        }
    }
    (best, chk)
}

fn kernel_us_per_decision<V: VmValue + PartialEq, const CANON: bool>(
    n: usize,
    ticks: usize,
    runs: u32,
) -> (f64, u64) {
    let prog = build_argmax_f64::<V>();
    let mut best = f64::INFINITY;
    let mut h = 0u64;
    for _ in 0..runs {
        let mut vm = Vm::<V>::new();
        let mut w = World::new(n, 0xA1_BACE_50);
        for _ in 0..(ticks / 10).max(1) {
            w.tick_vm_realistic::<V, true, CANON>(&mut vm, &prog);
        }
        let t0 = Instant::now();
        for _ in 0..ticks {
            w.tick_vm_realistic::<V, true, CANON>(&mut vm, &prog);
        }
        let el = t0.elapsed().as_nanos() as f64;
        h = black_box(w.state_hash());
        let us = el / (n as f64 * ticks as f64) / 1000.0;
        if us < best {
            best = us;
        }
    }
    (best, h)
}

fn main() {
    println!("# spike-b-value: heap path (interned strings + insertion-ordered tables) for");
    println!("# NaN-box(8B) vs tagged(16B) vs fat(24B). native aarch64-mac, --release lto.\n");

    // ---- sizeof ----
    println!("== sizeof(Value) ==");
    println!("  nanbox-8B : {} bytes", std::mem::size_of::<NbValue>());
    println!("  tagged-16B: {} bytes", std::mem::size_of::<TagValue>());
    println!("  fat-24B   : {} bytes", std::mem::size_of::<E24>());
    println!(
        "  reg file [24]: nb={}B tag={}B e24={}B",
        std::mem::size_of::<[NbValue; 24]>(),
        std::mem::size_of::<[TagValue; 24]>(),
        std::mem::size_of::<[E24; 24]>()
    );

    // ---- NaN-box soundness scan ----
    println!("\n== NaN-box soundness: does the tag space alias canonicalized-NaN f64s? ==");
    let (tested, before, after) = soundness_scan();
    println!("  NaN payloads tested : {}", tested);
    println!("  alias-as-tag BEFORE canon (UNSOUND if >0): {}", before);
    println!("  alias-as-tag AFTER  canon (MUST be 0)     : {}", after);
    // concrete example: the canonical NaN and a dangerous raw NaN
    let dangerous = 0x7ffc_0000_0000_0001u64; // exponent 0x7ff, bits 51&50 set, +1
    println!(
        "  example raw NaN 0x{:016x} (is_nan={}) classifies as: {}",
        dangerous,
        f64::from_bits(dangerous).is_nan(),
        classify_alias(dangerous)
    );
    println!(
        "  CANON_NAN       0x{:016x} classifies as: {}",
        CANON_NAN,
        classify_alias(CANON_NAN)
    );
    let danger_handle = SIGN_DEMO();
    println!(
        "  example raw NaN 0x{:016x} (sign set) classifies as: {}",
        danger_handle,
        classify_alias(danger_handle)
    );

    // ---- golden (determinism intact through both layouts' Q16 path) ----
    println!("\n== TIER-2 golden (Q16.16 VM argmax, 4000 agents, 200 ticks) ==");
    const GOLDEN: u64 = 0x31c6_ec27_34ea_6fa2;
    {
        let prog = build_argmax_q16::<NbValue>();
        let mut vm = Vm::new();
        let mut w = World::new(4000, 0xA1_BACE_50);
        for _ in 0..200 {
            w.tick_vm_q16(&mut vm, &prog);
        }
        let h = w.state_hash();
        println!("  nanbox-8B  = 0x{h:016x}  MATCH={}", h == GOLDEN);
    }
    {
        let prog = build_argmax_q16::<TagValue>();
        let mut vm = Vm::new();
        let mut w = World::new(4000, 0xA1_BACE_50);
        for _ in 0..200 {
            w.tick_vm_q16(&mut vm, &prog);
        }
        let h = w.state_hash();
        println!("  tagged-16B = 0x{h:016x}  MATCH={}", h == GOLDEN);
    }
    println!("  golden     = 0x{GOLDEN:016x}");

    // ---- heap microbenches ----
    let it_tbl: u64 = 40_000_000;
    let it_str: u64 = 40_000_000;
    let it_mv: u64 = 200_000_000;
    let it_bulk: u64 = 2_000_000;
    let runs = 7;

    println!("\n== heap microbenches: ns per op (best-of-{runs}, churned/black_box) ==");
    println!("{:<22} | {:>10} | {:>10} | {:>10}", "op", "nb-8B", "tag-16B", "fat-24B");
    println!("{}", "-".repeat(62));

    let mut sink = 0u64;
    macro_rules! row3 {
        ($label:expr, $f:ident, $iters:expr) => {{
            let (a, c1) = time_ns_per_op($iters, runs, |n| $f::<NbValue>(n));
            let (b, c2) = time_ns_per_op($iters, runs, |n| $f::<TagValue>(n));
            let (c, c3) = time_ns_per_op($iters, runs, |n| $f::<E24>(n));
            sink ^= c1 ^ c2 ^ c3;
            println!("{:<22} | {:>10.3} | {:>10.3} | {:>10.3}", $label, a, b, c);
        }};
    }
    macro_rules! row2 {
        ($label:expr, $f:ident, $iters:expr) => {{
            let (a, c1) = time_ns_per_op($iters, runs, |n| $f::<NbValue>(n));
            let (b, c2) = time_ns_per_op($iters, runs, |n| $f::<TagValue>(n));
            sink ^= c1 ^ c2;
            println!("{:<22} | {:>10.3} | {:>10.3} | {:>10}", $label, a, b, "-");
        }};
    }

    row3!("table get", bench_table_get, it_tbl);
    row3!("table set (overwrite)", bench_table_set, it_tbl);
    row2!("table insert/grow", bench_table_insert, it_tbl);
    row2!("string intern (hit)", bench_string_intern, it_str);
    row2!("string eq (interned)", bench_string_eq, it_str);
    row3!("value move (reg)", bench_clone_move, it_mv);
    row3!("bulk clone [256]", bench_bulk_clone, it_bulk);

    // koto Arc-tax clone
    let (arc_ns, ca) = time_ns_per_op(it_mv, runs, bench_clone_arc);
    sink ^= ca;
    println!(
        "{:<22} | {:>10} | {:>10} | {:>10.3}  (Arc clone+drop; koto-class non-Copy)",
        "value move (Arc 24B)", "-", "-", arc_ns
    );

    // ---- kernel us/decision ----
    println!("\n== IAUS kernel us/decision (realistic f64 argmax, fuel ON) ==");
    println!(
        "{:<14} | {:>14} | {:>14}",
        "layout", "canon-ON us/d", "canon-OFF us/d"
    );
    println!("{}", "-".repeat(48));
    let kn = 4000usize;
    let kt = 200usize;
    let kruns = 7;
    let (nb_on, h1) = kernel_us_per_decision::<NbValue, true>(kn, kt, kruns);
    let (nb_off, h2) = kernel_us_per_decision::<NbValue, false>(kn, kt, kruns);
    let (tg_on, h3) = kernel_us_per_decision::<TagValue, true>(kn, kt, kruns);
    let (tg_off, h4) = kernel_us_per_decision::<TagValue, false>(kn, kt, kruns);
    let (e_on, h5) = kernel_us_per_decision::<E24, true>(kn, kt, kruns);
    sink ^= h1 ^ h2 ^ h3 ^ h4 ^ h5;
    println!("{:<14} | {:>14.4} | {:>14.4}", "nanbox-8B", nb_on, nb_off);
    println!("{:<14} | {:>14.4} | {:>14.4}", "tagged-16B", tg_on, tg_off);
    println!("{:<14} | {:>14.4} | {:>14}", "fat-24B", e_on, "-");
    println!(
        "  canon tax: nb={:.4} us/d ({:+.1}%), tag={:.4} us/d ({:+.1}%)",
        nb_on - nb_off,
        (nb_on / nb_off - 1.0) * 100.0,
        tg_on - tg_off,
        (tg_on / tg_off - 1.0) * 100.0
    );

    black_box(sink);
    println!("\n(sink={sink})");
}

// helper: a NaN with the sign bit set, which the NaN-box reads as a HEAP handle if uncanon.
#[allow(non_snake_case)]
fn SIGN_DEMO() -> u64 {
    0x8000_0000_0000_0000u64 | 0x7ffc_0000_0000_0002
}
