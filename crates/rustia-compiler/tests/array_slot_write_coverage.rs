//! Hit coverage for the VM Core 2 C2.5a direct existing dense-array slot lane.
//!
//! This runs the benchmark workloads through the same
//! parse/compile/`Vm::call_function` sequence as the `rustia-engine` adapter in
//! `bench/src/main.rs` and reports which lane served every numeric array-slot
//! write. It exists so a candidate can be screened for hit coverage before any
//! billable controlled timing, which is why it is compiled only under the
//! default-off `table-write-diagnostics` feature: ordinary and benchmark builds
//! carry no hot-path instrumentation.
//!
//! Run it with:
//!
//! ```text
//! cargo test -p rustia-compiler --features table-write-diagnostics \
//!     --test array_slot_write_coverage -- --nocapture
//! ```
#![cfg(feature = "table-write-diagnostics")]

use rustia_compiler::{compile_with_options, CompileOptions};
use rustia_parser::parse_source;
use rustia_vm::{NumericArraySlotWriteCounts, Profile, Value, ValueKind, Vm};
use std::fs;
use std::path::PathBuf;

/// Every Luau workload in the benchmark suite, with a small fixed unit count.
/// The first two entries are this slice's primary targets; the next three
/// complete the tracked five-workload regression screen.
const WORKLOADS: &[(&str, &str, u64)] = &[
    (
        "dense_tables",
        "bench/workloads/luau/01_dense_tables.luau",
        64,
    ),
    (
        "stdlib_math_table",
        "bench/workloads/luau/10_stdlib_math_table.luau",
        4_000,
    ),
    (
        "sparse_tables",
        "bench/workloads/luau/02_sparse_tables.luau",
        64,
    ),
    (
        "closures_upvalues",
        "bench/workloads/luau/04_closures_upvalues.luau",
        4_000,
    ),
    (
        "generic_iteration",
        "bench/workloads/luau/09_generic_iteration.luau",
        4_000,
    ),
    (
        "oop_metamethods",
        "bench/workloads/luau/03_oop_metamethods.luau",
        4_000,
    ),
    (
        "string_patterns",
        "bench/workloads/luau/05_string_patterns.luau",
        64,
    ),
    (
        "string_format",
        "bench/workloads/luau/06_string_format.luau",
        64,
    ),
    ("coroutines", "bench/workloads/luau/07_coroutines.luau", 64),
    (
        "pcall_xpcall",
        "bench/workloads/luau/08_pcall_xpcall.luau",
        64,
    ),
    (
        "serialization",
        "bench/workloads/luau/11_serialization.luau",
        64,
    ),
    (
        "string_building",
        "bench/workloads/luau/15_string_building.luau",
        64,
    ),
    (
        "memory_burst",
        "bench/workloads/luau/16_memory_burst.luau",
        8,
    ),
    (
        "memory_churn",
        "bench/workloads/luau/17_memory_churn.luau",
        8,
    ),
    (
        "memory_retained_control",
        "bench/workloads/luau/18_memory_retained_control.luau",
        8,
    ),
    ("todo_steady", "bench/workloads/luau/19_todo_steady.luau", 8),
    (
        "todo_history",
        "bench/workloads/luau/20_todo_history.luau",
        8,
    ),
    (
        "todo_retained_control",
        "bench/workloads/luau/21_todo_retained_control.luau",
        8,
    ),
];

/// The workload sources live in the benchmark harness, outside this crate's
/// packaged files, so they are resolved from the workspace root at run time.
fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// Runs one workload the way the benchmark harness does. `None` means the
/// workload could not run on this build (an unrelated unsupported stdlib
/// feature), which is reported rather than treated as zero coverage.
fn workload_counts(source_path: &str, n: u64) -> Option<NumericArraySlotWriteCounts> {
    let path = repository_root().join(source_path);
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let chunk = parse_source(&source).expect("workload parses");
    let program = compile_with_options(
        &chunk,
        CompileOptions::default().with_source(source_path, &source),
    )
    .expect("workload compiles");

    // Same sequence as the benchmark harness: run the chunk to obtain the
    // workload function, then call it once with the fixed unit count.
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let function = vm.run(&program).expect("workload initializes");
    assert_eq!(
        function.kind(),
        ValueKind::Function,
        "a workload must return a function"
    );
    let before = vm.numeric_array_slot_write_counts();
    vm.call_function(&program, function, &[Value::number(n as f64)])
        .ok()?;
    let after = vm.numeric_array_slot_write_counts();
    Some(NumericArraySlotWriteCounts {
        calls: after.calls - before.calls,
        direct: after.direct - before.direct,
        generic_stored: after.generic_stored - before.generic_stored,
    })
}

fn report(workload: &str, counts: Option<NumericArraySlotWriteCounts>) {
    let Some(counts) = counts else {
        println!("c2.5a-coverage {workload}: not runnable on this build, skipped");
        return;
    };
    let share = if counts.calls == 0 {
        0.0
    } else {
        (counts.direct as f64) * 100.0 / (counts.calls as f64)
    };
    println!(
        "c2.5a-coverage {workload}: calls={} direct={} generic_stored={} \
         other={} direct_share={share:.4}%",
        counts.calls,
        counts.direct,
        counts.generic_stored,
        counts
            .calls
            .saturating_sub(counts.direct)
            .saturating_sub(counts.generic_stored),
    );
}

#[test]
fn tracked_workloads_report_direct_array_slot_lane_coverage() {
    for (workload, path, n) in WORKLOADS {
        report(workload, workload_counts(path, *n));
    }
}

/// Tripwire pinning the measured C2.5a hit coverage.
///
/// Every numeric array-slot write in the benchmark suite is currently either
/// an append/growth or a deferral: each tracked workload rebuilds its tables
/// from `{}` and never rewrites a slot it already filled, so the direct lane
/// is reached zero times. That makes the slice unmeasurable by the controlled
/// A/B protocol, which is why it is retained on correctness grounds rather
/// than promoted with a timing claim.
///
/// When this test fails, a tracked workload has started reaching the lane and
/// the performance bet reopens: re-run the coverage report, then gate the
/// slice on a controlled host.
#[test]
fn tracked_workloads_do_not_yet_reach_the_direct_lane() {
    for (workload, path, n) in WORKLOADS.iter().take(5) {
        let counts = workload_counts(path, *n);
        report(workload, counts);
        let Some(counts) = counts else {
            continue;
        };
        assert_eq!(
            counts.direct, 0,
            "{workload} now reaches the C2.5a direct lane ({counts:?}); \
             the slice has become measurable and needs a controlled A/B"
        );
    }
}
