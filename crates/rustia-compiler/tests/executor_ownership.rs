//! Diagnostic: which executor actually runs each benchmark workload, and what
//! the default-off C2.3 O2 families do to that ownership.
//!
//! Run with:
//! `cargo test -p rustia-compiler --features executor-diagnostics --test executor_ownership -- --nocapture`
//!
//! This carries no timing claim. It exists because an optimization measured in
//! executed instructions only converts into time on the executor that actually
//! runs the code — and a prototype whose own slice of the attached
//! logical-unit schedule is non-identity loses its HotIR loops and its
//! leaf-call plan (`FunctionInlineCaches::new` and `recognize_leaf_call_plan`
//! gate per prototype: HotIR on the prototype containing the loop, a leaf plan
//! on the callee it replays), so an instruction win inside a hot prototype can
//! be paid for by losing a faster executor there. Until 2026-09-15 the gate
//! was program-wide and any rewrite anywhere deselected every fast path;
//! `per_prototype_gate_keeps_every_untouched_fast_path` and
//! `artificial_schedule_outside_the_hot_prototype_changes_nothing` pin the
//! per-prototype rule by counting.
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
use rustia_vm::{
    ExecutorCensus, Opcode, Profile, Program, ProgramLogicalUnits, Value, Vm, VmSettings,
};
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
    root_refresh_triggers: Vec<(Opcode, u64)>,
}

impl WorkloadCensus {
    fn root_refreshes_triggered_by(&self, opcode: Opcode) -> u64 {
        self.root_refresh_triggers
            .iter()
            .find(|(trigger, _)| *trigger == opcode)
            .map_or(0, |(_, count)| *count)
    }
}

/// Runs setup plus one `f(n)` call on a fresh VM and reads the census.
fn census_of(program: &Program) -> WorkloadCensus {
    census_with(program, VmSettings::default())
}

/// `gc_boundaries` bounds how many leaf-plan replays a deterministic
/// collection could have refused during the call, so it must use the
/// interval the VM below actually ran with, not a fresh default -- a caller
/// that passed a non-default interval would otherwise have every
/// `assert_leaf_path_serves` bound computed against the wrong lattice.
fn census_with(program: &Program, settings: VmSettings) -> WorkloadCensus {
    let interval = settings.deterministic_gc_interval.get();
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
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
        gc_boundaries: census.executed_instructions / interval,
        root_refresh_triggers: vm.gc_root_refresh_histogram(),
    }
}

/// The charged instruction count with every fast path refused:
/// `enable_opcode_census` forces the scalar dispatcher (no tape, HotIR, or
/// leaf plan), which charges each instruction its own schedule units. Every
/// other executor must charge exactly this.
fn scalar_reference_executed(program: &Program) -> u64 {
    census_with(
        program,
        VmSettings {
            enable_opcode_census: true,
            ..VmSettings::default()
        },
    )
    .census
    .executed_instructions
}

fn share(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

/// Prototypes whose slice of the attached schedule is non-identity (`None`
/// is main); empty when no schedule is attached.
fn rewritten_prototypes(program: &Program) -> Vec<Option<usize>> {
    let Some(schedule) = program.logical_units() else {
        return Vec::new();
    };
    std::iter::once(None)
        .chain((0..program.functions.len()).map(Some))
        .filter(|prototype| !schedule.function_is_identity(*prototype))
        .collect()
}

/// The `UpvalueCounter` leaf shape `recognize_upvalue_counter_plan` matches:
/// `c = c + step; return c` over two upvalues, nine instructions.
fn upvalue_counter_prototype(program: &Program) -> Option<usize> {
    const SHAPE: [Opcode; 9] = [
        Opcode::GetUpvalue,
        Opcode::GetUpvalue,
        Opcode::Add,
        Opcode::ClearLocalCell,
        Opcode::SetUpvalue,
        Opcode::ClearLocalCell,
        Opcode::GetUpvalue,
        Opcode::Return,
        Opcode::Return,
    ];
    program.functions.iter().position(|function| {
        function.upvalues.len() == 2
            && function.code.len() == SHAPE.len()
            && function
                .code
                .iter()
                .zip(SHAPE)
                .all(|(instruction, opcode)| instruction.op == opcode)
    })
}

/// The counters that say which fast path served a call or a loop, as opposed
/// to how the tape happened to be windowed around collection boundaries.
fn fast_path_selection(census: &ExecutorCensus) -> [u64; 9] {
    [
        census.hot_ir_entries,
        census.hot_ir_iterations,
        census.hot_ir_guard_exits,
        census.closure_call_fast_entries,
        census.closure_call_prototype_hits,
        census.closure_call_cache_fills,
        census.closure_call_guard_exits,
        census.leaf_call_value_entries,
        census.tape_guard_exits,
    ]
}

/// A schedule of ones except `unit` on one pc of one prototype.
fn schedule_with_unit_at(
    program: &Program,
    prototype: Option<usize>,
    pc: usize,
    unit: u32,
) -> ProgramLogicalUnits {
    let mut main = vec![1u32; program.main.code.len()];
    let mut functions = program
        .functions
        .iter()
        .map(|function| vec![1u32; function.code.len()])
        .collect::<Vec<_>>();
    match prototype {
        None => main[pc] = unit,
        Some(index) => functions[index][pc] = unit,
    }
    ProgramLogicalUnits::new(main, functions).expect("units are positive")
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
        // root refresh from the full frame the refused call built. Both leaf
        // prototypes are byte-identical on both sides and keep their plans
        // under reachability_exact(): the rewrite lands in `f`, whose slice
        // is the only non-identity one, and the gate is per prototype.
        match id {
            "09_generic_iteration" => {
                for (label, side) in [("default", &base), ("o2-exact", &opt)] {
                    assert_leaf_path_serves(
                        id,
                        &format!("{label} call_value"),
                        side.census.leaf_call_value_entries,
                        33.0 * units(),
                        side.gc_boundaries,
                    );
                    assert_eq!(
                        side.census.closure_call_fast_entries, 0,
                        "{id} {label}: the __iter closure is never called through the Call opcode",
                    );
                }
            }
            "04_closures_upvalues" => {
                for (label, side) in [("default", &base), ("o2-exact", &opt)] {
                    assert_leaf_path_serves(
                        id,
                        &format!("{label} Call-opcode"),
                        side.census.closure_call_fast_entries,
                        units(),
                        side.gc_boundaries,
                    );
                    assert_eq!(
                        side.census.leaf_call_value_entries, 0,
                        "{id} {label}: the counter closure is never called through call_value",
                    );
                }
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
        let rewritten = rewritten_prototypes(&optimized)
            .iter()
            .map(|prototype| prototype.map_or("main".to_owned(), |index| format!("fn#{index}")))
            .collect::<Vec<_>>();
        println!(
            "{:24} o2-exact schedule: {} of {} prototypes non-identity [{}]",
            "",
            rewritten.len(),
            optimized.functions.len() + 1,
            rewritten.join(" "),
        );
    }
    println!();
}

/// The per-prototype gate, counted per workload: every fast path in a
/// prototype the reachability-exact track did not rewrite is served on both
/// sides, charged instructions and checksums are identical, and the one
/// workload whose rewrite lands inside its own HotIR loop (`dense_tables`)
/// loses HotIR in exactly that prototype.
#[test]
fn per_prototype_gate_keeps_every_untouched_fast_path() {
    for id in WORKLOADS {
        let Some(baseline) = compile_workload(id, ExperimentalO2Options::default()) else {
            continue;
        };
        let optimized =
            compile_workload(id, ExperimentalO2Options::reachability_exact()).expect("source read");
        assert!(
            baseline.logical_units().is_none(),
            "{id}: all-off carries no schedule"
        );
        let base = census_of(&baseline);
        let opt = census_of(&optimized);
        assert_eq!(
            base.checksum.to_bits(),
            opt.checksum.to_bits(),
            "{id}: checksum changed"
        );
        assert_eq!(
            base.census.executed_instructions, opt.census.executed_instructions,
            "{id}: charged instructions changed"
        );
        let rewritten = rewritten_prototypes(&optimized);
        let sides = [("default", &base), ("o2-exact", &opt)];

        match id {
            "04_closures_upvalues" => {
                // commit-late fires on `acc = acc + c()` in `f`; the counter
                // closure is byte-identical and keeps its UpvalueCounter plan.
                let counter =
                    upvalue_counter_prototype(&optimized).expect("counter closure is a leaf");
                assert!(!rewritten.is_empty(), "{id}: no rewrite fired");
                assert!(
                    !rewritten.contains(&Some(counter)),
                    "{id}: the counter closure's own slice is non-identity"
                );
                for (label, side) in sides {
                    assert_leaf_path_serves(
                        id,
                        &format!("{label} Call-opcode"),
                        side.census.closure_call_fast_entries,
                        units(),
                        side.gc_boundaries,
                    );
                    assert_eq!(side.census.leaf_call_value_entries, 0, "{id} {label}");
                    assert_eq!(side.census.closure_call_guard_exits, 0, "{id} {label}");
                    // Every served call either filled the caller-site cache
                    // or hit it.
                    assert_eq!(
                        side.census.closure_call_prototype_hits
                            + side.census.closure_call_cache_fills,
                        side.census.closure_call_fast_entries,
                        "{id} {label}: cache bookkeeping"
                    );
                }
                // A refused call builds a full frame whose Call refreshes
                // roots; served calls never do.
                assert_eq!(
                    base.root_refreshes_triggered_by(Opcode::Call),
                    opt.root_refreshes_triggered_by(Opcode::Call),
                    "{id}: Call-triggered root refreshes differ across the sides"
                );
            }
            "09_generic_iteration" => {
                // commit-late fires on `acc = acc + v` in `f`; the `__iter`
                // closure keeps its ArrayIterator plan and `make`'s fill loop
                // keeps HotIR (31 continuing iterations of `for i = 1, 32`).
                assert!(!rewritten.is_empty(), "{id}: no rewrite fired");
                for (label, side) in sides {
                    assert_leaf_path_serves(
                        id,
                        &format!("{label} call_value"),
                        side.census.leaf_call_value_entries,
                        33.0 * units(),
                        side.gc_boundaries,
                    );
                    assert_eq!(side.census.closure_call_fast_entries, 0, "{id} {label}");
                    assert_eq!(side.census.hot_ir_iterations, 31, "{id} {label}: make(32)");
                    // Each refused replay builds a frame whose `#items`
                    // refreshes roots; the setup's own Len adds at most one.
                    assert!(
                        side.root_refreshes_triggered_by(Opcode::Len) <= side.gc_boundaries + 1,
                        "{id} {label}: {} Len-triggered root refreshes, at most one per \
                         boundary ({}) plus one expected",
                        side.root_refreshes_triggered_by(Opcode::Len),
                        side.gc_boundaries,
                    );
                }
            }
            "12_array_mutation" => {
                // No family fires: identical programs, identical census.
                assert!(optimized.logical_units().is_none(), "{id}: a rewrite fired");
                assert!(rewritten.is_empty());
                assert_eq!(base.census, opt.census, "{id}: census differs");
                assert!(
                    base.census.hot_ir_iterations > 0,
                    "{id}: HotIR serves the loops"
                );
            }
            "01_dense_tables" => {
                // commit-late fires on `s = s + t[i]`, inside the sum loop's
                // body in `f` (the only prototype), so `f`'s own slice is
                // non-identity and both of its loops — the untouched fill
                // loop too — leave HotIR for the tape. This is the gate
                // firing where it should; restoring HotIR here needs
                // unit-aware HotIR charging, not a gate (design/58 §C2.3).
                assert_eq!(rewritten, vec![Some(0)], "{id}: rewrite lands in f only");
                assert!(
                    base.census.hot_ir_iterations > 0,
                    "{id}: HotIR serves f by default"
                );
                assert_eq!(
                    opt.census.hot_ir_iterations, 0,
                    "{id}: f's slice is non-identity, so HotIR is gated"
                );
                assert!(
                    share(
                        opt.census.tape_instructions,
                        opt.census.executed_instructions
                    ) > 99.0,
                    "{id}: the tape takes over f"
                );
            }
            "10_stdlib_math_table" => {
                // commit-late fires twice plus one dead-tail fold, but the
                // workload has no HotIR loop and no leaf closure to lose.
                assert!(
                    optimized.logical_units().is_some(),
                    "{id}: schedule attached"
                );
                for (label, side) in sides {
                    assert_eq!(side.census.hot_ir_iterations, 0, "{id} {label}");
                    assert_eq!(side.census.closure_call_fast_entries, 0, "{id} {label}");
                    assert_eq!(side.census.leaf_call_value_entries, 0, "{id} {label}");
                }
                assert_eq!(
                    base.census.native_call_inline_cache_hits,
                    opt.census.native_call_inline_cache_hits,
                    "{id}: native-call inline cache hits differ"
                );
            }
            "02_sparse_tables" => {
                // Guard exits are pinned by `report_executor_ownership`; no
                // HotIR loop or leaf closure on either side.
                for (label, side) in sides {
                    assert_eq!(side.census.hot_ir_iterations, 0, "{id} {label}");
                    assert_eq!(side.census.closure_call_fast_entries, 0, "{id} {label}");
                    assert_eq!(side.census.leaf_call_value_entries, 0, "{id} {label}");
                }
            }
            _ => unreachable!("{id} is not a tracked workload"),
        }
    }
}

/// The counterfactual that established the mechanism, inverted: attach an
/// artificial schedule to the all-off bytecode. A unit outside the hot
/// prototype (on main's first instruction, executed once during setup)
/// changes nothing but the one charged unit; a unit inside the hot prototype
/// (the counter closure's `Add`, the first pc of `f`'s fill-loop body) drops
/// exactly that prototype's fast path, and the charge stays exact either way
/// because it equals the scalar dispatcher's.
#[test]
fn artificial_schedule_outside_the_hot_prototype_changes_nothing() {
    for id in ["04_closures_upvalues", "12_array_mutation"] {
        let Some(program) = compile_workload(id, ExperimentalO2Options::default()) else {
            continue;
        };
        assert!(program.logical_units().is_none());
        let base = census_of(&program);
        assert_eq!(
            base.census.executed_instructions,
            scalar_reference_executed(&program)
        );

        let outside = program
            .clone()
            .with_logical_units(Some(schedule_with_unit_at(&program, None, 0, 2)));
        assert!(outside.logical_units().is_some());
        let out = census_of(&outside);
        assert_eq!(out.checksum.to_bits(), base.checksum.to_bits(), "{id}");
        assert_eq!(
            out.census.executed_instructions,
            base.census.executed_instructions + 1,
            "{id}: main pc 0 runs once during setup"
        );
        assert_eq!(
            out.census.executed_instructions,
            scalar_reference_executed(&outside),
            "{id}: the fast paths charge what the scalar dispatcher charges"
        );
        // The one extra unit shifts every later deterministic-collection
        // boundary by one instruction, which can move a boundary onto a tape
        // entry point (tape entries and receipts may differ by one per
        // boundary); the fast-path selection itself must not move.
        assert_eq!(
            fast_path_selection(&base.census),
            fast_path_selection(&out.census),
            "{id}: a unit outside the hot prototype changed the fast-path selection"
        );

        let (prototype, pc) = match id {
            "04_closures_upvalues" => (
                upvalue_counter_prototype(&program).expect("counter closure is a leaf"),
                2,
            ),
            _ => {
                // `f` is the only prototype; its first `ForLoop` is the fill
                // loop and `aux` is the body start.
                let fill_loop = program.functions[0]
                    .code
                    .iter()
                    .find(|instruction| instruction.op == Opcode::ForLoop)
                    .expect("f has a numeric for loop");
                (0, usize::try_from(fill_loop.aux).expect("body start"))
            }
        };
        let inside = program
            .clone()
            .with_logical_units(Some(schedule_with_unit_at(
                &program,
                Some(prototype),
                pc,
                2,
            )));
        let inn = census_of(&inside);
        assert_eq!(inn.checksum.to_bits(), base.checksum.to_bits(), "{id}");
        assert_eq!(
            inn.census.executed_instructions,
            scalar_reference_executed(&inside),
            "{id}: the gated prototype still charges what the scalar dispatcher charges"
        );
        assert!(
            inn.census.executed_instructions > base.census.executed_instructions + 1,
            "{id}: the unit sits on a pc executed more than once"
        );
        match id {
            "04_closures_upvalues" => {
                assert_eq!(inn.census.closure_call_fast_entries, 0, "{id}: plan gated");
                assert_eq!(
                    inn.census.closure_call_cache_fills, 0,
                    "{id}: nothing to cache"
                );
                assert_eq!(
                    inn.census.hot_ir_iterations, base.census.hot_ir_iterations,
                    "{id}"
                );
            }
            _ => {
                assert_eq!(inn.census.hot_ir_iterations, 0, "{id}: HotIR gated in f");
                assert_eq!(inn.census.hot_ir_entries, 0, "{id}");
                assert_eq!(
                    inn.census.closure_call_fast_entries, base.census.closure_call_fast_entries,
                    "{id}"
                );
            }
        }
    }
}
