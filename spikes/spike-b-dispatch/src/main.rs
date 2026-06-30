use std::time::Instant;
use spike_b_dispatch::{bench, bench_threaded, dispatches_per_run, last_fuel};

fn best_of(f: extern "C" fn(u64) -> f64, iters: u64, runs: u32, sink: &mut f64) -> (f64, f64, f64) {
    let mut best = f64::INFINITY;
    let mut chk = 0.0;
    let mut fuel = 0.0;
    for _ in 0..runs {
        let t0 = Instant::now();
        chk = f(iters);
        let dt = t0.elapsed().as_secs_f64();
        fuel = last_fuel();
        if dt < best {
            best = dt;
        }
        *sink += chk;
    }
    (best, chk, fuel)
}

fn report(label: &str, secs: f64, chk: f64, fuel: f64) {
    let ns = (secs * 1e9) / fuel;
    println!("  {label}:");
    println!("    total_dispatches = {fuel}");
    println!("    checksum         = {chk}");
    println!("    best_elapsed_s   = {secs:.6}");
    println!("    ns_per_dispatch  = {ns:.4}");
    println!("    dispatch_per_sec = {:.3e}", fuel / secs);
}

fn main() {
    let per_run = dispatches_per_run();
    let mut sink = 0.0f64;
    // warm up
    sink += bench(2_000);
    sink += bench_threaded(2_000);

    let iters: u64 = 400_000; // ~5.2e9 dispatches
    println!("native (best of 5):");
    println!("  dispatches_per_run = {}", per_run);
    println!("  iters              = {}", iters);

    let (s1, c1, f1) = best_of(bench, iters, 5, &mut sink);
    report("match_dispatch", s1, c1, f1);

    let (s2, c2, f2) = best_of(bench_threaded, iters, 5, &mut sink);
    report("fnptr_threaded", s2, c2, f2);

    println!("  sink = {}", sink);
}
