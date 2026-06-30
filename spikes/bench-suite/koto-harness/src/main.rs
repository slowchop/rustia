//! Koto 0.16 baseline harness + capability probe. Mirrors the Luau harness: each koto/*.koto file
//! `export`s `f = |n| ...` returning a checksum; we auto-calibrate n, take min of reps, report ns/unit.
//! Run with arg "probe" to just run the probe script and print results (capability discovery).

use koto::prelude::*;
use std::time::Instant;

struct W { name: &'static str, file: &'static str }

fn workloads() -> Vec<W> {
    vec![
        W { name: "dense_tables",      file: "01_dense_tables.koto" },
        W { name: "sparse_tables",     file: "02_sparse_tables.koto" },
        W { name: "oop_metamethods",   file: "03_oop_metamethods.koto" },
        W { name: "closures_upvalues", file: "04_closures_upvalues.koto" },
        W { name: "string_format",     file: "06_string_format.koto" },
        W { name: "coroutines",        file: "07_coroutines.koto" },
        W { name: "pcall_xpcall",      file: "08_pcall_xpcall.koto" },
        W { name: "generic_iteration", file: "09_generic_iteration.koto" },
        W { name: "stdlib_math_table", file: "10_stdlib_math_table.koto" },
    ]
}

const TARGET_SECS: f64 = 0.25;
const REPS: usize = 7;

fn call_secs(koto: &mut Koto, f: &KValue, n: i64) -> f64 {
    let t = Instant::now();
    koto.call_function(f.clone(), &[KValue::Number(KNumber::I64(n))])
        .expect("call failed");
    t.elapsed().as_secs_f64()
}

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../koto");
    let args: Vec<String> = std::env::args().collect();

    if args.get(1).map(|s| s.as_str()) == Some("probe") {
        let src = std::fs::read_to_string(dir.join("00_probe.koto")).unwrap();
        let mut koto = Koto::default();
        match koto.compile_and_run(&src) {
            Ok(_) => println!("PROBE OK"),
            Err(e) => println!("PROBE ERR: {e}"),
        }
        return;
    }

    println!("# rustia broad benchmark suite -- KOTO 0.16.1 baseline (the incumbent being replaced)");
    println!("# engine: koto 0.16.1, default-features=false, features=[arc]; --release lto; single thread");
    println!("# metric: ns per UNIT (min of {} reps, ~{}s/rep). Compare to the Luau column for koto multiplier.", REPS, TARGET_SECS);
    println!();
    println!("{:<20} | {:>12} | {:>14}", "workload", "ns/unit(min)", "units/sec");
    println!("{}", "-".repeat(54));

    for w in workloads() {
        let path = dir.join(w.file);
        if !path.exists() {
            println!("{:<20} | {:>12} | {}", w.name, "SKIP", "(no koto equivalent: see feasibility notes)");
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let mut koto = Koto::default();
        if let Err(e) = koto.compile_and_run(&src) {
            println!("{:<20} | ERR compile/run: {}", w.name, e);
            continue;
        }
        let f = match koto.exports().get("f") {
            Some(v) => v,
            None => { println!("{:<20} | ERR no export f", w.name); continue; }
        };

        let mut n: i64 = 256;
        loop {
            let s = call_secs(&mut koto, &f, n);
            if s >= TARGET_SECS || n >= 1_000_000_000 {
                if s > 0.0 { n = ((n as f64) * (TARGET_SECS / s)).max(1.0) as i64; }
                break;
            }
            n = n.saturating_mul(2);
        }
        let _ = call_secs(&mut koto, &f, n);
        let mut times: Vec<f64> = (0..REPS).map(|_| call_secs(&mut koto, &f, n)).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let min_s = times[0];
        let ns_min = min_s * 1e9 / n as f64;
        let per_sec = n as f64 / min_s;
        println!("{:<20} | {:>12.2} | {:>14.0}  (n={})", w.name, ns_min, per_sec, n);
    }
}
