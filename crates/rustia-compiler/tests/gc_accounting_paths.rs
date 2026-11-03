//! Correctness-only census for the real acceptance workload call paths.
//!
//! Run in private self-hosted CI with:
//! `cargo test --locked --release -p rustia-compiler --features executor-diagnostics --test gc_accounting_paths -- --nocapture`
//!
//! This reports counters, never timings. Each row separates the source's
//! `Vm::run` setup, a full-N warmup, and the following full-N call on the same
//! standard VM. This matches the warmed sample's adapter lifecycle; the separate
//! cold metric is excluded. It does not recover counters from prior benchmark
//! binaries or timed legs. Fixed work and checksums make missing sources,
//! incomplete calls, or changed workload behavior visible.

#![cfg(feature = "executor-diagnostics")]

use rustia_compiler::compile_with_options;
use rustia_parser::parse_source;
use rustia_vm::{ExecutorCensus, Profile, Program, Value, ValueKind, Vm, VmSettings};

struct Case {
    workload: &'static str,
    source: &'static str,
    n: u64,
    expected_checksum: f64,
}

const CASES: [Case; 6] = [
    Case {
        workload: "closures_upvalues",
        source: include_str!("../../../bench/workloads/luau/04_closures_upvalues.luau"),
        n: 32,
        expected_checksum: 504.0,
    },
    Case {
        workload: "closures_upvalues",
        source: include_str!("../../../bench/workloads/luau/04_closures_upvalues.luau"),
        n: 512,
        expected_checksum: 77_184.0,
    },
    Case {
        workload: "closures_upvalues",
        source: include_str!("../../../bench/workloads/luau/04_closures_upvalues.luau"),
        n: 632_142,
        expected_checksum: 112_392_753_633.0,
    },
    Case {
        workload: "generic_iteration",
        source: include_str!("../../../bench/workloads/luau/09_generic_iteration.luau"),
        n: 32,
        expected_checksum: 732_640.0,
    },
    Case {
        workload: "generic_iteration",
        source: include_str!("../../../bench/workloads/luau/09_generic_iteration.luau"),
        n: 512,
        expected_checksum: 11_722_240.0,
    },
    Case {
        workload: "generic_iteration",
        source: include_str!("../../../bench/workloads/luau/09_generic_iteration.luau"),
        n: 8_272,
        expected_checksum: 189_387_440.0,
    },
];

fn compile(source: &str, workload: &str) -> Program {
    let chunk = parse_source(source).unwrap_or_else(|error| panic!("parse {workload}: {error}"));
    compile_with_options(
        &chunk,
        rustia_compiler::CompileOptions::default().with_source(workload, source),
    )
    .unwrap_or_else(|error| panic!("compile {workload}: {error}"))
}

fn counter_delta(after: ExecutorCensus, before: ExecutorCensus) -> ExecutorCensus {
    let delta = |later: u64, earlier: u64| {
        later
            .checked_sub(earlier)
            .expect("diagnostic counter decreased between snapshots")
    };
    ExecutorCensus {
        executed_instructions: delta(after.executed_instructions, before.executed_instructions),
        gc_root_refreshes: delta(after.gc_root_refreshes, before.gc_root_refreshes),
        gc_root_refreshes_clean_accounted: delta(
            after.gc_root_refreshes_clean_accounted,
            before.gc_root_refreshes_clean_accounted,
        ),
        gc_root_refreshes_dirty_skip: delta(
            after.gc_root_refreshes_dirty_skip,
            before.gc_root_refreshes_dirty_skip,
        ),
        gc_root_refresh_attempts_no_frame: delta(
            after.gc_root_refresh_attempts_no_frame,
            before.gc_root_refresh_attempts_no_frame,
        ),
        gc_root_values_copied: delta(after.gc_root_values_copied, before.gc_root_values_copied),
        gc_root_refreshes_tape_exit: delta(
            after.gc_root_refreshes_tape_exit,
            before.gc_root_refreshes_tape_exit,
        ),
        closure_call_fast_entries: delta(
            after.closure_call_fast_entries,
            before.closure_call_fast_entries,
        ),
        leaf_call_value_entries: delta(
            after.leaf_call_value_entries,
            before.leaf_call_value_entries,
        ),
        ..ExecutorCensus::default()
    }
}

fn assert_refresh_partition(workload: &str, phase: &str, census: ExecutorCensus) {
    assert_eq!(
        census.gc_root_refreshes_clean_accounted
            + census.gc_root_refreshes_dirty_skip
            + census.gc_root_refresh_attempts_no_frame,
        census.gc_root_refreshes,
        "root-refresh partition mismatch for {workload} {phase}"
    );
}

fn report(workload: &str, n: u64, phase: &str, checksum: &str, census: ExecutorCensus) {
    assert_refresh_partition(workload, phase, census);
    println!(
        "GC_ACCOUNTING_PATH_CENSUS workload={workload} n={n} phase={phase} profile=ReproducibleStandard memory_tracking=false memory_limit_bytes=none deterministic_gc_interval=10000 checksum={checksum} executed_instructions={} root_refreshes={} clean_accounted={} dirty_skip={} no_frame={} root_values_copied={} tape_exit_refreshes={} closure_fast_entries={} leaf_value_entries={}",
        census.executed_instructions,
        census.gc_root_refreshes,
        census.gc_root_refreshes_clean_accounted,
        census.gc_root_refreshes_dirty_skip,
        census.gc_root_refresh_attempts_no_frame,
        census.gc_root_values_copied,
        census.gc_root_refreshes_tape_exit,
        census.closure_call_fast_entries,
        census.leaf_call_value_entries,
    );
}

#[test]
fn reports_default_standard_gc_accounting_paths_for_acceptance_workloads() {
    for case in CASES {
        let program = compile(case.source, case.workload);
        let settings = VmSettings::default();
        assert!(!settings.enable_memory_tracking);
        assert_eq!(settings.memory_limit_bytes, None);
        assert_eq!(settings.deterministic_gc_interval.get(), 10_000);

        let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
        let function = vm
            .run(&program)
            .unwrap_or_else(|error| panic!("setup {}: {error}", case.workload));
        assert_eq!(
            function.kind(),
            ValueKind::Function,
            "{} setup",
            case.workload
        );
        let setup_census = vm.executor_census();
        report(case.workload, case.n, "setup", "function", setup_census);

        let mut before_call = setup_census;
        for phase in ["warmup", "post_warmup_call"] {
            let result = vm
                .call_function(&program, function, &[Value::number(case.n as f64)])
                .unwrap_or_else(|error| panic!("{phase} {} n={}: {error}", case.workload, case.n));
            let checksum = result
                .as_number()
                .unwrap_or_else(|| panic!("{} n={} returned a non-number", case.workload, case.n));
            assert_eq!(
                checksum, case.expected_checksum,
                "{} n={} {phase}",
                case.workload, case.n
            );
            let after_call = vm.executor_census();
            let call_census = counter_delta(after_call, before_call);
            report(
                case.workload,
                case.n,
                phase,
                &format!("{checksum:.0}"),
                call_census,
            );
            before_call = after_call;
        }
    }
}
