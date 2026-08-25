//! Hit coverage for the VM Core 2 C2.5a direct existing-slot lane and the
//! C2.5b direct exact-append lane.
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
        "array_mutation",
        "bench/workloads/luau/12_array_mutation.luau",
        64,
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
        direct_append: after.direct_append - before.direct_append,
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
    let append_share = if counts.calls == 0 {
        0.0
    } else {
        (counts.direct_append as f64) * 100.0 / (counts.calls as f64)
    };
    println!(
        "c2.5a-coverage {workload}: calls={} direct={} direct_append={} generic_stored={} \
         other={} direct_share={share:.4}% direct_append_share={append_share:.4}%",
        counts.calls,
        counts.direct,
        counts.direct_append,
        counts.generic_stored,
        counts
            .calls
            .saturating_sub(counts.direct)
            .saturating_sub(counts.direct_append)
            .saturating_sub(counts.generic_stored),
    );
}

#[test]
fn tracked_workloads_report_direct_array_slot_lane_coverage() {
    for (workload, path, n) in WORKLOADS {
        report(workload, workload_counts(path, *n));
    }
}

/// The `array_mutation` workload exists precisely to exercise these lanes:
/// it builds a 256-element array once (the C2.5b direct append lane) and
/// then only rewrites filled slots (the C2.5a direct existing-slot lane),
/// so every numeric array-slot write it issues is served directly and none
/// fall to the generic `mutate_table_data` path. This is the positive
/// counterpart to the tripwire below.
#[test]
fn the_array_mutation_workload_exercises_the_direct_lane() {
    let counts = workload_counts("bench/workloads/luau/12_array_mutation.luau", 64)
        .expect("array_mutation must run");
    report("array_mutation", Some(counts));
    assert!(
        counts.direct > 0,
        "array_mutation must reach the C2.5a direct lane; got {counts:?}"
    );
    assert_eq!(
        counts.direct_append, 256,
        "only the one-time 256-slot build should take the C2.5b direct append lane; got {counts:?}"
    );
    assert_eq!(
        counts.generic_stored, 0,
        "the generic mutate_table_data path should not be reached at all; got {counts:?}"
    );
    assert_eq!(
        counts.calls - counts.direct_append,
        counts.direct,
        "every write after the build must be a direct existing-slot store; got {counts:?}"
    );
}

/// The C2.5b append lane is the mirror image: `dense_tables` reaches it on
/// every one of its numeric array-slot writes, because each unit builds a
/// fresh 256-element array from `{}` over an ascending index. That coverage
/// is what makes C2.5b measurable on a tracked workload, unlike C2.5a.
///
/// When this test fails the append lane has stopped serving a tracked
/// workload, which invalidates the gate that justified it.
#[test]
fn tracked_workloads_reach_the_direct_append_lane() {
    let counts = workload_counts("bench/workloads/luau/01_dense_tables.luau", 64)
        .expect("dense_tables must run");
    report("dense_tables", Some(counts));
    assert_eq!(
        counts.direct_append, counts.calls,
        "every dense_tables numeric array-slot write should take the C2.5b append lane; \
         got {counts:?}"
    );
    assert_eq!(
        counts.generic_stored, 0,
        "dense_tables should no longer reach the generic mutate_table_data path; got {counts:?}"
    );
    assert!(
        counts.calls > 0,
        "dense_tables must actually issue numeric array-slot writes; got {counts:?}"
    );
}

/// Tripwire pinning the measured C2.5a existing-slot hit coverage.
///
/// Every numeric array-slot write in the benchmark suite is an append or a
/// deferral: each tracked workload rebuilds its tables from `{}` and never
/// rewrites a slot it already filled, so the C2.5a *existing-slot* lane is
/// reached zero times here. That is why C2.5a needed `array_mutation` before
/// it could be measured at all.
///
/// This says nothing about the C2.5b *append* lane, which those same appends
/// do reach - see
/// [`tracked_workloads_reach_the_direct_append_lane`] for that side.
///
/// When this test fails, a tracked workload has started rewriting filled
/// slots: re-run the coverage report, then gate on a controlled host.
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
