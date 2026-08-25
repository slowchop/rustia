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

#![cfg(feature = "executor-diagnostics")]

use rustia_compiler::{compile_with_options, CompileOptions, ExperimentalO2Options};
use rustia_parser::parse_source;
use rustia_vm::{ExecutorCensus, Profile, Program, Value, Vm};
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
/// test run.
const N: f64 = 32.0;

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

fn census_of(program: &Program) -> (ExecutorCensus, f64) {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let function = vm
        .run(program)
        .unwrap_or_else(|error| panic!("workload setup failed: {error}"));
    let result = vm
        .call_function(program, function, &[Value::number(N)])
        .unwrap_or_else(|error| panic!("workload call failed: {error}"));
    let checksum = result.as_number().expect("workload returns a number");
    (vm.executor_census(), checksum)
}

fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

#[test]
fn report_executor_ownership() {
    println!(
        "\nworkload                 variant      executed   tape_instr    tape%  \
         hotIR_iters  tape_exits  receipts  closure_fast"
    );
    for id in WORKLOADS {
        let Some(baseline) = compile_workload(id, ExperimentalO2Options::default()) else {
            println!("{id:24} (workload source missing, skipped)");
            continue;
        };
        let (base_census, base_checksum) = census_of(&baseline);
        let optimized =
            compile_workload(id, ExperimentalO2Options::reachability_exact()).expect("source read");
        let (opt_census, opt_checksum) = census_of(&optimized);
        assert_eq!(
            base_checksum.to_bits(),
            opt_checksum.to_bits(),
            "{id}: reachability-exact O2 changed the workload checksum"
        );

        for (label, census) in [("default", base_census), ("o2-exact", opt_census)] {
            println!(
                "{id:24} {label:10} {executed:>10}   {tape_instr:>10} {tape:>7.2}%  \
                 {hot:>11}  {tape_exits:>10}  {receipts:>8}  {closure:>12}",
                executed = census.executed_instructions,
                tape_instr = census.tape_instructions,
                tape = share(census.tape_instructions, census.executed_instructions),
                hot = census.hot_ir_iterations,
                tape_exits = census.tape_guard_exits,
                receipts = census.tape_receipts,
                closure = census.closure_call_fast_entries,
            );
        }
    }
    println!();
}
