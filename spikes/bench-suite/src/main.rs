//! Reference-Luau baseline harness for the rustia broad benchmark suite ([H2]).
//! Loads each luau/*.luau workload (each `return`s a function f(n) that performs n "units"
//! of representative work and returns a checksum). Auto-calibrates n so a timed batch runs
//! ~target seconds, then takes the MIN over several reps (min = least noise / closest to the
//! machine's true throughput) and reports ns per unit.
//!
//! This measures THE BAR (reference Luau via mlua), not rustia. rustia does not exist yet.

use mlua::{Function, Lua, Value};
use std::time::Instant;

struct Workload {
    name: &'static str,
    file: &'static str,
    unit: &'static str,
}

fn workloads() -> Vec<Workload> {
    vec![
        Workload { name: "dense_tables",      file: "01_dense_tables.luau",      unit: "build+sum+sort of 256-elem array" },
        Workload { name: "sparse_tables",     file: "02_sparse_tables.luau",     unit: "fill+read 128 hash entries" },
        Workload { name: "oop_metamethods",   file: "03_oop_metamethods.luau",   unit: "(p+v)*s + :dot + :len2 step" },
        Workload { name: "closures_upvalues", file: "04_closures_upvalues.luau", unit: "one upvalue-mutating closure call" },
        Workload { name: "string_patterns",   file: "05_string_patterns.luau",   unit: "gmatch+match+gsub over 60-char line" },
        Workload { name: "string_format",     file: "06_string_format.luau",     unit: "one mixed-spec string.format" },
        Workload { name: "coroutines",        file: "07_coroutines.luau",        unit: "one resume->yield round trip" },
        Workload { name: "pcall_xpcall",      file: "08_pcall_xpcall.luau",      unit: "pcall+xpcall pair (1/16 throws)" },
        Workload { name: "generic_iteration", file: "09_generic_iteration.luau", unit: "__iter(32)+pairs(5)+ipairs(32) pass" },
        Workload { name: "stdlib_math_table", file: "10_stdlib_math_table.luau",  unit: "math fastcalls + buffered push" },
        Workload { name: "serialization",     file: "11_serialization.luau",     unit: "serialize+parse 32-record round trip" },
    ]
}

const TARGET_SECS: f64 = 0.25;
const REPS: usize = 9;

fn call_secs(f: &Function, n: u64) -> f64 {
    let t = Instant::now();
    let _: Value = f.call(n).expect("workload call failed");
    t.elapsed().as_secs_f64()
}

fn main() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("luau");
    let lua = Lua::new();
    let luau_ver: String = lua
        .globals()
        .get::<String>("_VERSION")
        .unwrap_or_else(|_| "?".into());

    println!("# rustia broad benchmark suite -- REFERENCE-LUAU BASELINE (the bar, not rustia)");
    println!("# engine: mlua 0.11.6 + luau0-src 0.18.3+luau709 (Luau 0.709), vendored");
    println!("# build: --release, opt-level=3, lto=fat, codegen-units=1; single thread; _VERSION={}", luau_ver);
    println!("# host: aarch64-mac. metric: ns per UNIT (min of {} reps, ~{}s/rep timed).", REPS, TARGET_SECS);
    println!();
    println!("{:<20} | {:>12} | {:>11} | {:>14} | {}", "workload", "ns/unit(min)", "ns/unit(med)", "units/sec", "unit");
    println!("{}", "-".repeat(110));

    for w in workloads() {
        let src = std::fs::read_to_string(dir.join(w.file)).expect("read workload");
        let f: Function = lua
            .load(&src)
            .set_name(w.name)
            .eval()
            .unwrap_or_else(|e| panic!("eval {} failed: {}", w.name, e));

        // calibrate: grow n until a single batch takes >= TARGET_SECS (cap to avoid runaway).
        let mut n: u64 = 256;
        loop {
            let s = call_secs(&f, n);
            if s >= TARGET_SECS || n >= 1_000_000_000 {
                // scale n to land near TARGET_SECS for the timed reps
                if s > 0.0 {
                    let scaled = (n as f64 * (TARGET_SECS / s)).max(1.0);
                    n = scaled as u64;
                }
                break;
            }
            n = n.saturating_mul(2);
        }

        // warmup
        let _ = call_secs(&f, n);

        let mut times: Vec<f64> = (0..REPS).map(|_| call_secs(&f, n)).collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let min_s = times[0];
        let med_s = times[REPS / 2];
        let ns_min = min_s * 1e9 / n as f64;
        let ns_med = med_s * 1e9 / n as f64;
        let per_sec = n as f64 / min_s;

        println!(
            "{:<20} | {:>12.2} | {:>11.2} | {:>14.0} | {}  (n={})",
            w.name, ns_min, ns_med, per_sec, w.unit, n
        );
    }
    println!();
    println!("# ns/unit is per the 'unit' column -- units differ in internal work, so compare a future");
    println!("# rustia run against THIS column per-workload, not across workloads.");
}
