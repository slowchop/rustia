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
use rustia_vm::{ErrorKind, Profile, Program, Vm, VmSettings};

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
    for interval in [1u64, 2, 3, 5, 7, 13, 64, 997, 10_000] {
        let run = |program: &Program| {
            let settings = VmSettings {
                deterministic_gc_interval: NonZeroU64::new(interval).expect("non-zero"),
                ..VmSettings::default()
            };
            Vm::with_settings(Profile::ReproducibleStandard, settings)
                .run(program)
                .expect("run")
                .as_number()
                .map(f64::to_bits)
        };
        assert_eq!(
            run(&baseline),
            run(&optimized),
            "collection cadence diverged at interval {interval}"
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
