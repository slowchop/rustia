//! Diagnostic: which executor actually runs each benchmark workload, and what
//! the default-off C2.3 O2 families do to that ownership.
//!
//! Run with:
//! `cargo test -p rustia-compiler --features executor-diagnostics --test executor_ownership -- --nocapture`
//!
//! This carries no timing claim. It exists because an optimization measured in
//! executed instructions only converts into time on the executor that actually
//! runs the code — and attaching an optimized logical-unit schedule deselects
//! the HotIR and leaf-call plans for the whole program
//! (`Vm::load_program`/`recognize_leaf_call_plan` skip unit-carrying
//! programs), so an instruction win can be paid for by losing a faster
//! executor.
//!
//! Leaf-call plans are counted on both of their paths: `closure_fast` is the
//! `Call` opcode's inline cache (`closures_upvalues`' counter closure) and
//! `leaf_value` is the dynamic `call_value` path that generic-for iterator
//! steps take (`generic_iteration`'s `__iter` closure). Before the second
//! counter existed the report could not see the iterator replays at all, and
//! `generic_iteration`'s executor was wrongly read as constant across the O2
//! sides (design/58-vm-core2.md §C2.3, 2026-09-13).

#![cfg(feature = "executor-diagnostics")]

use rustia_compiler::{compile_with_options, CompileOptions, ExperimentalO2Options};
use rustia_parser::parse_source;
use rustia_vm::{ExecutorCensus, Profile, Program, Value, Vm, VmSettings};
use std::path::{Path, PathBuf};

const WORKLOADS: [&str; 6] = [
    "01_dense_tables",
    "02_sparse_tables",
    "04_closures_upvalues",
    "09_generic_iteration",
    "10_stdlib_math_table",
    "12_array_mutation",
];

/// Large enough for steady-state executor selection, small enough for a debug
/// test run. Override with `RUSTIA_EXECUTOR_CENSUS_N` to check that a reported
/// count scales the way its claimed mechanism says it should.
fn units() -> f64 {
    std::env::var("RUSTIA_EXECUTOR_CENSUS_N")
        .ok()
        .and_then(|raw| raw.parse::<f64>().ok())
        .unwrap_or(32.0)
}

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn compile_workload(id: &str, o2: ExperimentalO2Options) -> Option<Program> {
    let source =
        std::fs::read_to_string(repo_path(&format!("bench/workloads/luau/{id}.luau"))).ok()?;
    let chunk = parse_source(&source).unwrap_or_else(|error| panic!("parse {id}: {error}"));
    let options = CompileOptions {
        experimental_o2: o2,
        ..CompileOptions::default()
    }
    .with_source(id, &source);
    Some(
        compile_with_options(&chunk, options)
            .unwrap_or_else(|error| panic!("compile {id}: {error}")),
    )
}

struct WorkloadCensus {
    census: ExecutorCensus,
    checksum: f64,
    /// Deterministic-collection boundaries the run crossed
    /// (`executed / deterministic_gc_interval`). A boundary instruction runs
    /// scalar and never enters the tape or a leaf plan, so each boundary can
    /// cost at most one tape guard exit or one leaf replay.
    gc_boundaries: u64,
    root_refresh_triggers: Vec<(rustia_vm::Opcode, u64)>,
}

fn census_of(program: &Program) -> WorkloadCensus {
    // `Vm::new` runs on `VmSettings::default()`, so that is the interval the
    // boundary count below must use.
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let function = vm
        .run(program)
        .unwrap_or_else(|error| panic!("workload setup failed: {error}"));
    let result = vm
        .call_function(program, function, &[Value::number(units())])
        .unwrap_or_else(|error| panic!("workload call failed: {error}"));
    let checksum = result.as_number().expect("workload returns a number");
    let census = vm.executor_census();
    WorkloadCensus {
        census,
        checksum,
        gc_boundaries: census.executed_instructions
            / VmSettings::default().deterministic_gc_interval.get(),
        root_refresh_triggers: vm.gc_root_refresh_histogram(),
    }
}

fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

/// A leaf plan replays a call only when no deterministic-collection boundary
/// falls inside its fixed instruction window (the boundary instruction runs
/// scalar), so a path that serves every call of its closure shows
/// `expected` replays less at most one per boundary the run crossed.
fn assert_leaf_path_serves(id: &str, path: &str, served: u64, expected: f64, gc_boundaries: u64) {
    let served = served as f64;
    assert!(
        served <= expected && served >= expected - gc_boundaries as f64,
        "{id}: {served} leaf replays on the {path} path, expected {expected} less at most one \
         per deterministic-collection boundary ({gc_boundaries} crossed)",
    );
}

#[test]
fn report_executor_ownership() {
    println!(
        "\nworkload                 variant      executed   tape_instr    tape%  \
         hotIR_iters  tape_exits  receipts  root_refresh  root_copies  closure_fast  leaf_value  \
         gc_boundaries"
    );
    for id in WORKLOADS {
        let Some(baseline) = compile_workload(id, ExperimentalO2Options::default()) else {
            println!("{id:24} (workload source missing, skipped)");
            continue;
        };
        let base = census_of(&baseline);
        let optimized =
            compile_workload(id, ExperimentalO2Options::reachability_exact()).expect("source read");
        let opt = census_of(&optimized);
        assert_eq!(
            base.checksum.to_bits(),
            opt.checksum.to_bits(),
            "{id}: reachability-exact O2 changed the workload checksum"
        );
        let base_census = base.census;
        let opt_census = opt.census;

        // Pin the claim the report is used to make: sparse_tables is the only
        // tracked workload whose index operations miss the tape's dense guard.
        // Its inner loop issues four index operations per iteration (two
        // writes, two reads) over 64 iterations per unit; the shortfall is one
        // exit per deterministic-collection boundary that lands on such an
        // operation, because a boundary instruction runs scalar and never
        // enters the tape at all.
        let expected_exits = 4.0 * 64.0 * units();
        for census in [base_census, opt_census] {
            if id == "02_sparse_tables" {
                assert!(
                    census.tape_guard_exits as f64 <= expected_exits
                        && census.tape_guard_exits as f64 >= expected_exits - units(),
                    "{id}: {} tape guard exits, expected at most {expected_exits} and no more \
                     than one shortfall per unit",
                    census.tape_guard_exits,
                );
            } else {
                assert_eq!(
                    census.tape_guard_exits, 0,
                    "{id}: tape guard exits are no longer exclusive to sparse_tables",
                );
            }
        }

        // Pin leaf-call ownership on both of its paths. generic_iteration's
        // `__iter` closure is an ArrayIterator leaf reached only through the
        // dynamic call_value path (generic-for iterator steps): 32 yielding
        // steps plus one terminating step per unit, 33 replays per unit.
        // closures_upvalues' counter closure is an UpvalueCounter leaf reached
        // only through the Call opcode's inline cache: one fast entry per
        // unit. Observed shortfalls at n = 16/32/64/128 were 1/1/3/7 replays
        // against 1/3/7/14 boundaries, each one showing up as a Len-triggered
        // root refresh from the full frame the refused call built. With
        // reachability_exact() the program-wide gate (recognize_leaf_call_plan
        // returns None for every prototype of a unit-carrying program) drops
        // both counts to zero, although the leaf prototypes themselves are
        // byte-identical on both sides.
        match id {
            "09_generic_iteration" => {
                assert_leaf_path_serves(
                    id,
                    "call_value",
                    base_census.leaf_call_value_entries,
                    33.0 * units(),
                    base.gc_boundaries,
                );
                assert_eq!(
                    base_census.closure_call_fast_entries, 0,
                    "{id}: the __iter closure is never called through the Call opcode",
                );
                assert_eq!(
                    opt_census.leaf_call_value_entries + opt_census.closure_call_fast_entries,
                    0,
                    "{id}: reachability_exact() no longer trips the program-wide leaf-plan gate",
                );
            }
            "04_closures_upvalues" => {
                assert_leaf_path_serves(
                    id,
                    "Call-opcode",
                    base_census.closure_call_fast_entries,
                    units(),
                    base.gc_boundaries,
                );
                assert_eq!(
                    base_census.leaf_call_value_entries, 0,
                    "{id}: the counter closure is never called through call_value",
                );
                assert_eq!(
                    opt_census.leaf_call_value_entries + opt_census.closure_call_fast_entries,
                    0,
                    "{id}: reachability_exact() no longer trips the program-wide leaf-plan gate",
                );
            }
            _ => {}
        }

        for (label, workload) in [("default", &base), ("o2-exact", &opt)] {
            let census = workload.census;
            println!(
                "{id:24} {label:10} {executed:>10}   {tape_instr:>10} {tape:>7.2}%  \
                 {hot:>11}  {tape_exits:>10}  {receipts:>8}  {refreshes:>12}  {copies:>11}  \
                 {closure_fast:>12}  {leaf_value:>10}  {gc_boundaries:>13}",
                executed = census.executed_instructions,
                tape_instr = census.tape_instructions,
                tape = share(census.tape_instructions, census.executed_instructions),
                hot = census.hot_ir_iterations,
                tape_exits = census.tape_guard_exits,
                receipts = census.tape_receipts,
                refreshes = census.gc_root_refreshes,
                copies = census.gc_root_values_copied,
                closure_fast = census.closure_call_fast_entries,
                leaf_value = census.leaf_call_value_entries,
                gc_boundaries = workload.gc_boundaries,
            );
        }
        let rendered = base
            .root_refresh_triggers
            .iter()
            .map(|(opcode, count)| format!("{opcode:?}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        println!(
            "{:24} triggers   tape_exit={:<8} {rendered}",
            "", base_census.gc_root_refreshes_tape_exit
        );
    }
    println!();
}
