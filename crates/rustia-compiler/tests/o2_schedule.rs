//! Logical-schedule equivalence for O2-optimized programs.
//!
//! With the C2.2 executable tape merged, optimized programs carry a
//! per-instruction logical-unit schedule ([`rustia_vm::ProgramLogicalUnits`])
//! and the VM charges fuel, `executed_instructions`, and deterministic-GC
//! scheduling by those units. These tests pin the resulting contract: with any
//! O2 family enabled, a program's observable logical schedule — every fuel
//! boundary, the logical clock, and deterministic collection cadence — is
//! identical to the unoptimized compile, on both the metered scalar path and
//! the plain-mode tape.

use std::num::NonZeroU64;

use rustia_compiler::{compile_with_options, CompileOptions, ExperimentalO2Options};
use rustia_parser::parse_source;
use rustia_vm::{ErrorKind, Profile, Program, Value, Vm, VmSettings};

fn compile_o2(source: &str, o2: ExperimentalO2Options) -> Program {
    let options = CompileOptions {
        experimental_o2: o2,
        ..CompileOptions::default()
    }
    .with_source("o2-schedule", source);
    compile_with_options(&parse_source(source).expect("parse"), options).expect("compile")
}

const FAMILIES: [(&str, ExperimentalO2Options); 6] = [
    (
        "fold_constant_branches",
        ExperimentalO2Options {
            fold_constant_branches: true,
            commit_late_compound_locals: false,
            remove_dead_register_tails: false,
            elide_dead_cleanup_clears: false,
            tighten_local_slots: false,
        },
    ),
    (
        "commit_late_compound_locals",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: true,
            remove_dead_register_tails: false,
            elide_dead_cleanup_clears: false,
            tighten_local_slots: false,
        },
    ),
    (
        "remove_dead_register_tails",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: false,
            remove_dead_register_tails: true,
            elide_dead_cleanup_clears: false,
            tighten_local_slots: false,
        },
    ),
    (
        "elide_dead_cleanup_clears",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: false,
            remove_dead_register_tails: false,
            elide_dead_cleanup_clears: true,
            tighten_local_slots: false,
        },
    ),
    (
        "tighten_local_slots",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: false,
            remove_dead_register_tails: false,
            elide_dead_cleanup_clears: false,
            tighten_local_slots: true,
        },
    ),
    ("all", ExperimentalO2Options::ALL),
];

/// Schedule-sensitive corpus: loops, compound updates, branches, closures,
/// calls, coroutines, varargs, and multiple results.
const CORPUS: [&str; 6] = [
    r#"
        local total = 0
        for i = 1, 25 do
            total += i * 2
            if i % 3 == 0 then
                total -= 1
            end
        end
        return total
    "#,
    r#"
        local trace = 0
        if true then trace += 1 else trace -= 1 end
        if false then trace += 100 end
        local i = 0
        while i < 6 do
            local x = i * 3
            trace += x
            i += 1
        end
        return trace
    "#,
    r#"
        local function helper(a, b)
            local sum = a + b
            local product = a * b
            return sum, product
        end
        local total = 0
        for i = 1, 8 do
            local s, p = helper(i, i + 1)
            total += s + p
        end
        return total
    "#,
    r#"
        local fns = {}
        for i = 1, 4 do
            local v = 0
            v = i * 10
            fns[i] = function() return v end
        end
        local total = 0
        for i = 1, 4 do
            total += fns[i]()
        end
        return total
    "#,
    r#"
        local co = coroutine.create(function(a)
            local acc = a
            acc += coroutine.yield(acc)
            acc += coroutine.yield(acc)
            return acc
        end)
        local _, first = coroutine.resume(co, 1)
        local _, second = coroutine.resume(co, 10)
        local _, third = coroutine.resume(co, 100)
        return first + second + third
    "#,
    r#"
        local function tail(n)
            local acc = 0
            for i = 1, n do
                local t = { i, i * 2 }
                acc += t[1] + t[2]
            end
            return acc
        end
        return tail(9)
    "#,
];

/// Runs `program` under an exact fuel limit; `Ok` carries the result bits,
/// `Err(true)` means fuel exhaustion, `Err(false)` any other error.
fn run_with_fuel(program: &Program, fuel: u64) -> Result<u64, bool> {
    let settings = VmSettings {
        initial_fuel: Some(fuel),
        ..VmSettings::default()
    };
    match Vm::with_settings(Profile::ReproducibleStandard, settings).run(program) {
        Ok(value) => Ok(value.as_number().map(f64::to_bits).unwrap_or(0)),
        Err(error) => Err(matches!(error.kind, ErrorKind::FuelExhausted { .. })),
    }
}

fn minimum_fuel(program: &Program) -> u64 {
    let mut low = 0u64;
    let mut high = 1u64;
    while matches!(run_with_fuel(program, high), Err(true)) {
        high = high.checked_mul(2).expect("fuel search overflow");
    }
    while low < high {
        let middle = low + (high - low) / 2;
        if matches!(run_with_fuel(program, middle), Err(true)) {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    low
}

#[test]
fn minimum_fuel_is_identical_for_every_family() {
    for source in CORPUS {
        let baseline = compile_o2(source, ExperimentalO2Options::default());
        let expected = minimum_fuel(&baseline);
        for (name, family) in FAMILIES {
            let optimized = compile_o2(source, family);
            assert_eq!(
                minimum_fuel(&optimized),
                expected,
                "family {name} changed the fuel schedule of:\n{source}"
            );
        }
    }
}

#[test]
fn every_fuel_prefix_matches_exactly() {
    // Not just the total: every fuel boundary must land identically, so a
    // sweep over all limits below the minimum must produce the same
    // exhaustion/result outcome at every step.
    let source = CORPUS[0];
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let optimized = compile_o2(source, ExperimentalO2Options::ALL);
    let limit = minimum_fuel(&baseline) + 2;
    for fuel in 0..=limit {
        assert_eq!(
            run_with_fuel(&baseline, fuel),
            run_with_fuel(&optimized, fuel),
            "outcome diverged at fuel {fuel}"
        );
    }
}

#[test]
fn logical_clock_is_identical_on_the_plain_mode_tape() {
    // os.clock in ReproducibleStandard without providers is the deterministic
    // logical clock (executed instructions / 1e6). Plain mode (no fuel) runs
    // the executable tape, so equality here proves the tape charges the
    // restored logical schedule for optimized code.
    let source = r#"
        local total = 0
        for i = 1, 50 do
            local x = i * 2
            total += x
            if i % 5 == 0 then
                total -= 1
            end
        end
        return os.clock() * 1e6 + (total - total)
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let expected = Vm::new(Profile::ReproducibleStandard)
        .run(&baseline)
        .expect("baseline run")
        .as_number()
        .expect("clock");
    for (name, family) in FAMILIES {
        let optimized = compile_o2(source, family);
        let actual = Vm::new(Profile::ReproducibleStandard)
            .run(&optimized)
            .expect("optimized run")
            .as_number()
            .expect("clock");
        assert_eq!(
            expected.to_bits(),
            actual.to_bits(),
            "family {name} moved the logical clock"
        );
    }
}

#[test]
fn deterministic_collection_cadence_survives_interval_sweeps() {
    // Weak-table observability across deterministic collection boundaries:
    // entries are dropped via explicit strong-reference removal (never via a
    // removed cleanup clear), so every interval must observe the identical
    // sweep points with and without O2.
    let source = r#"
        local weak = setmetatable({}, { __mode = "v" })
        local strong = {}
        local observed = 0
        for i = 1, 12 do
            local entry = { value = i }
            weak[i] = entry
            strong[i] = entry
            local x = i * 3
            observed += x
        end
        for i = 1, 12 do
            strong[i] = nil
            collectgarbage()
            local alive = 0
            for _, v in pairs(weak) do
                alive += v.value - v.value + 1
            end
            observed += alive * 100
        end
        return observed
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let optimized = compile_o2(source, ExperimentalO2Options::ALL);
    // `DeterministicSim` registers no `collectgarbage`, so its leg drops the
    // explicit collection: weak entries are then swept only at deterministic
    // boundaries, which makes the result itself lattice-sensitive.
    let sim_source = source.replace("collectgarbage()", "");
    let sim_baseline = compile_o2(&sim_source, ExperimentalO2Options::default());
    let sim_optimized = compile_o2(&sim_source, ExperimentalO2Options::ALL);
    for interval in [1u64, 2, 3, 5, 7, 13, 64, 997, 10_000] {
        let settings = || VmSettings {
            deterministic_gc_interval: NonZeroU64::new(interval).expect("non-zero"),
            ..VmSettings::default()
        };
        let run = |program: &Program| {
            let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings());
            let bits = vm.run(program).expect("run").as_number().map(f64::to_bits);
            (
                bits,
                vm.executed_instructions(),
                vm.next_deterministic_collection_instruction(),
            )
        };
        assert_eq!(
            run(&baseline),
            run(&optimized),
            "collection cadence diverged at interval {interval}"
        );
        // The lattice itself, not only its weak-table consequences: the
        // collection count and the next boundary are observable, so the
        // optimized program must land on the same lattice points with the
        // same weak-table outcome.
        let run_sim = |program: &Program| {
            let mut vm = Vm::with_settings(Profile::DeterministicSim, settings());
            let bits = vm.run(program).expect("run").as_number().map(f64::to_bits);
            (
                bits,
                vm.executed_instructions(),
                vm.deterministic_collection_count(),
                vm.next_deterministic_collection_instruction(),
            )
        };
        let expected = run_sim(&sim_baseline);
        assert_eq!(
            expected,
            run_sim(&sim_optimized),
            "collection lattice diverged at interval {interval}"
        );
        assert_eq!(
            expected.2,
            expected.1 / interval,
            "baseline count off the lattice at interval {interval}"
        );
    }
}

#[test]
fn plain_and_metered_results_agree_for_optimized_programs() {
    // The tape (plain mode) and the scalar dispatcher (metered mode) must
    // agree on every optimized program.
    for source in CORPUS {
        for (name, family) in FAMILIES {
            let optimized = compile_o2(source, family);
            let plain = Vm::new(Profile::ReproducibleStandard)
                .run(&optimized)
                .expect("plain run")
                .as_number()
                .map(f64::to_bits);
            let metered = {
                let settings = VmSettings {
                    initial_fuel: Some(u64::MAX / 2),
                    ..VmSettings::default()
                };
                Vm::with_settings(Profile::ReproducibleStandard, settings)
                    .run(&optimized)
                    .expect("metered run")
                    .as_number()
                    .map(f64::to_bits)
            };
            assert_eq!(plain, metered, "family {name} diverged between modes");
        }
    }
}

#[test]
fn optimized_programs_carry_a_validated_unit_schedule() {
    let source = CORPUS[0];
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    assert!(
        baseline.logical_units().is_none(),
        "identity stays implicit"
    );
    let optimized = compile_o2(source, ExperimentalO2Options::ALL);
    let units = optimized
        .logical_units()
        .expect("optimized programs carry their schedule");
    assert!(!units.is_identity());

    // Unit-carrying programs still reset and clone cleanly.
    let cloned = optimized.clone();
    assert!(cloned.logical_units().is_some());
    let value = Vm::new(Profile::ReproducibleStandard)
        .run(&cloned)
        .expect("cloned run");
    assert_eq!(value.kind(), rustia_vm::ValueKind::Number);
    // Value equality with the baseline result.
    let expected = Vm::new(Profile::ReproducibleStandard)
        .run(&baseline)
        .expect("baseline run");
    assert_eq!(
        expected.as_number().map(f64::to_bits),
        value.as_number().map(f64::to_bits)
    );
}

/// The six tracked workloads, each a chunk returning `f(n)`. None uses `os`,
/// `require`, or `loadstring`, so every one runs under `DeterministicSim`.
const WORKLOADS: [(&str, &str); 6] = [
    (
        "01_dense_tables",
        include_str!("../../../bench/workloads/luau/01_dense_tables.luau"),
    ),
    (
        "02_sparse_tables",
        include_str!("../../../bench/workloads/luau/02_sparse_tables.luau"),
    ),
    (
        "04_closures_upvalues",
        include_str!("../../../bench/workloads/luau/04_closures_upvalues.luau"),
    ),
    (
        "09_generic_iteration",
        include_str!("../../../bench/workloads/luau/09_generic_iteration.luau"),
    ),
    (
        "10_stdlib_math_table",
        include_str!("../../../bench/workloads/luau/10_stdlib_math_table.luau"),
    ),
    (
        "12_array_mutation",
        include_str!("../../../bench/workloads/luau/12_array_mutation.luau"),
    ),
];

/// All off, every family alone, and the corpus-green track.
fn schedule_variants() -> Vec<(&'static str, ExperimentalO2Options)> {
    let mut variants = vec![("all_off", ExperimentalO2Options::default())];
    variants.extend(FAMILIES[..5].iter().copied());
    variants.push((
        "reachability_exact",
        ExperimentalO2Options::reachability_exact(),
    ));
    variants
}

/// Same emitted code, register files, and schedule: a byte-identical program
/// on identical settings is the same deterministic run.
fn same_bytecode(a: &Program, b: &Program) -> bool {
    a.main.code == b.main.code
        && a.main.max_registers == b.main.max_registers
        && a.functions.len() == b.functions.len()
        && a.functions
            .iter()
            .zip(&b.functions)
            .all(|(x, y)| x.code == y.code && x.max_registers == y.max_registers)
        && a.logical_units() == b.logical_units()
}

/// Every variant of `source` whose bytecode differs from an earlier variant's,
/// all-off first. A family that does not fire on a workload compiles to the
/// all-off program and is observed once, under the first name it appeared as.
fn distinct_variants(source: &str) -> Vec<(&'static str, Program)> {
    let mut distinct: Vec<(&'static str, Program)> = Vec::new();
    for (name, o2) in schedule_variants() {
        let program = compile_o2(source, o2);
        if !distinct
            .iter()
            .any(|(_, seen)| same_bytecode(seen, &program))
        {
            distinct.push((name, program));
        }
    }
    distinct
}

/// Standalone dead-clear elision keeps stale registers rooted through later
/// boundaries (design/58 §C2.3, blocked on C2.4), so its checksums are not
/// compared against all-off; its schedule still is.
fn compares_results(variant: &str) -> bool {
    variant != "elide_dead_cleanup_clears"
}

/// What one VM shows after a call: the result bits and the three schedule
/// fields the lattice invariant relates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScheduleObservation {
    result_bits: u64,
    executed: u64,
    count: u64,
    next: u64,
}

/// The invariant `Vm::maybe_collect_with_roots` restores after every charge:
/// the next boundary is the first lattice point above the charged clock, and
/// under `DeterministicSim` the count is the number of lattice points reached
/// (`ReproducibleStandard` keeps the count at zero by contract).
fn assert_on_collection_lattice(vm: &Vm, profile: Profile, interval: u64, context: &str) {
    let executed = vm.executed_instructions();
    assert_eq!(
        vm.next_deterministic_collection_instruction(),
        (executed / interval + 1) * interval,
        "{context}: next boundary is off the lattice"
    );
    let expected_count = if profile.is_deterministic_sim() {
        executed / interval
    } else {
        0
    };
    assert_eq!(
        vm.deterministic_collection_count(),
        expected_count,
        "{context}: collection count is off the lattice"
    );
}

/// Runs the workload's setup chunk and then `calls` consecutive `f(n)` calls
/// on one VM, checking the lattice invariant after setup and after every
/// call. `live_bytes` receives the heap's live bytes after each call.
fn observe_schedule(
    program: &Program,
    profile: Profile,
    interval: u64,
    n: f64,
    calls: usize,
    live_bytes: &mut Vec<usize>,
    context: &str,
) -> Vec<ScheduleObservation> {
    let settings = VmSettings {
        deterministic_gc_interval: NonZeroU64::new(interval).expect("non-zero"),
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(profile, settings);
    let function = vm
        .run(program)
        .unwrap_or_else(|error| panic!("{context}: setup failed: {error}"));
    let function = vm
        .retain_value(function)
        .unwrap_or_else(|error| panic!("{context}: retain failed: {error}"));
    assert_on_collection_lattice(&vm, profile, interval, &format!("{context}, setup"));
    (1..=calls)
        .map(|call| {
            let callee = function
                .value(&vm)
                .unwrap_or_else(|error| panic!("{context}: retained value: {error}"));
            let result = vm
                .call_function(program, callee, &[Value::number(n)])
                .unwrap_or_else(|error| panic!("{context}: call {call} failed: {error}"));
            assert_on_collection_lattice(
                &vm,
                profile,
                interval,
                &format!("{context}, call {call}"),
            );
            live_bytes.push(vm.memory_stats().live_bytes);
            ScheduleObservation {
                result_bits: result.as_number().map(f64::to_bits).unwrap_or(0),
                executed: vm.executed_instructions(),
                count: vm.deterministic_collection_count(),
                next: vm.next_deterministic_collection_instruction(),
            }
        })
        .collect()
}

#[test]
fn deterministic_collection_count_tracks_the_unit_lattice_across_calls() {
    // design/58 §C2.3: at every instruction edge, on every program, across
    // consecutive calls on one VM, `deterministic_collection_count ==
    // executed / interval` and the next boundary sits on the same lattice.
    // The unoptimized program satisfies this trivially; a unit-carrying
    // program only does so if a multi-unit charge that steps over several
    // boundaries fires each of them. Beyond the invariant, every family must
    // reproduce the all-off program's charged clock (the receipt contract)
    // and, for the corpus-green families, its results.
    const INTERVALS: [u64; 6] = [1, 7, 13, 97, 997, 10_000];
    let mut crossed_at = [false; INTERVALS.len()];
    for (workload, source) in WORKLOADS {
        let compiled = distinct_variants(source);
        assert_eq!(compiled[0].0, "all_off");
        if matches!(workload, "04_closures_upvalues" | "09_generic_iteration") {
            // Non-vacuity: commit-late fires on both, leaving multi-unit
            // survivors whose charges straddle boundaries at small intervals.
            let track = compile_o2(source, ExperimentalO2Options::reachability_exact());
            assert!(
                track.logical_units().is_some(),
                "{workload}: reachability_exact attached no schedule"
            );
            assert!(
                compiled.len() > 1,
                "{workload}: no family changed the bytecode"
            );
        }
        for (slot, interval) in INTERVALS.into_iter().enumerate() {
            // Interval 1 collects at every instruction; keep those legs short
            // and count instructions rather than time.
            let legs: &[(Profile, f64)] = match interval {
                1 => &[
                    (Profile::DeterministicSim, 2.0),
                    (Profile::ReproducibleStandard, 1.0),
                ],
                97 | 10_000 => &[
                    (Profile::DeterministicSim, 8.0),
                    (Profile::ReproducibleStandard, 8.0),
                ],
                7 | 13 => &[(Profile::DeterministicSim, 2.0)],
                _ => &[(Profile::DeterministicSim, 8.0)],
            };
            for &(profile, n) in legs {
                let context = |variant: &str| {
                    format!("{workload}, {variant}, interval {interval}, n {n}, {profile:?}")
                };
                let baseline = observe_schedule(
                    &compiled[0].1,
                    profile,
                    interval,
                    n,
                    2,
                    &mut Vec::new(),
                    &context("all_off"),
                );
                if profile.is_deterministic_sim() {
                    crossed_at[slot] |= baseline[1].count > 0;
                    // Every workload crosses a boundary at the small
                    // intervals; the large ones are covered by the long
                    // workloads (asserted once per interval below).
                    assert!(
                        interval > 97 || baseline[1].count > 0,
                        "{}: no boundary crossed, the sweep is vacuous",
                        context("all_off")
                    );
                }
                for (variant, program) in &compiled[1..] {
                    let observed = observe_schedule(
                        program,
                        profile,
                        interval,
                        n,
                        2,
                        &mut Vec::new(),
                        &context(variant),
                    );
                    for (call, (seen, expected)) in observed.iter().zip(&baseline).enumerate() {
                        let call = call + 1;
                        assert_eq!(
                            seen.executed,
                            expected.executed,
                            "{}: call {call} charged a different clock",
                            context(variant)
                        );
                        assert_eq!(
                            (seen.count, seen.next),
                            (expected.count, expected.next),
                            "{}: call {call} collected on a different schedule",
                            context(variant)
                        );
                        if compares_results(variant) {
                            assert_eq!(
                                seen.result_bits,
                                expected.result_bits,
                                "{}: call {call} changed the result",
                                context(variant)
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(
        crossed_at.iter().all(|crossed| *crossed),
        "some interval was never crossed by any workload: {crossed_at:?}"
    );
}

#[test]
fn reproducible_standard_heap_stays_bounded_across_calls_for_unit_carrying_programs() {
    // A skipped boundary collection under `ReproducibleStandard` shows up as
    // a heap that grows call over call (the refused candidate leaked
    // ~39 KB per `generic_iteration` call and one 256-slot table per
    // `dense_tables` unit). The slack only has to absorb the garbage of at
    // most one interval, which a regression clears by more than 2x at the
    // second call.
    const SLACK: usize = 16 * 1024;
    const CALLS: usize = 4;
    const N: f64 = 128.0;
    for (workload, source) in WORKLOADS {
        let compiled = distinct_variants(source);
        for interval in [97u64, 10_000] {
            let mut baseline_live = Vec::new();
            let mut variant_live = Vec::new();
            for (variant, program) in &compiled {
                let context = format!("{workload}, {variant}, interval {interval}");
                let mut live = Vec::new();
                let seen = observe_schedule(
                    program,
                    Profile::ReproducibleStandard,
                    interval,
                    N,
                    CALLS,
                    &mut live,
                    &context,
                );
                assert!(seen.iter().all(|seen| seen.count == 0), "{context}");
                for call in 1..CALLS {
                    assert!(
                        live[call] <= live[0] + SLACK,
                        "{context}: live bytes grew across calls: {live:?}"
                    );
                }
                if baseline_live.is_empty() {
                    baseline_live = live;
                } else {
                    variant_live.push((*variant, live));
                }
            }
            for (variant, live) in &variant_live {
                for call in 0..CALLS {
                    assert!(
                        live[call] <= baseline_live[call] + SLACK,
                        "{workload}, {variant}, interval {interval}: live bytes exceed \
                         all-off after call {}: {live:?} versus {baseline_live:?}",
                        call + 1
                    );
                }
            }
        }
    }
}
