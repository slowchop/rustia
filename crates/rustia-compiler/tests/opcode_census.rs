//! C2.3 static and dynamic opcode census for the five tracked workloads.
//!
//! Generates `design/58-vm-core2-c23-census.json`: per workload, the static
//! opcode counts of the compiled program, the dynamic per-opcode execution
//! counts of one `f(n)` call on the exact scalar interpreter
//! ([`rustia_vm::vm::VmSettings::enable_opcode_census`]), and the liveness
//! diagnostics (`Move`/`ClearLocalCell` structure, removable-cleanup and
//! lowering candidates). The committed report is compared byte-for-byte so it
//! cannot drift from the compiler; regenerate with
//! `RUSTIA_UPDATE_C23_CENSUS=1 cargo test -p rustia-compiler --test
//! opcode_census`.
//!
//! The census also pins two semantic contracts on real workload code: census
//! mode changes no result and no fuel schedule, and every clear the liveness
//! analysis proves to be dead pure cleanup is execution-equivalent to a
//! value-preserving no-op.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use rustia_compiler::{compile_with_options, CompileOptions};
use rustia_parser::parse_source;
use rustia_vm::bytecode::{Constant, Function, Instruction};
use rustia_vm::liveness::{ClearKind, FunctionLiveness, ProgramLiveness};
use rustia_vm::{Opcode, Profile, Program, Value, Vm, VmSettings};

/// The five tracked benchmark workloads from `bench/workloads/luau/`.
const WORKLOADS: [&str; 5] = [
    "01_dense_tables",
    "02_sparse_tables",
    "04_closures_upvalues",
    "09_generic_iteration",
    "10_stdlib_math_table",
];

/// Fixed dynamic work per workload. Small on purpose: the census is a count
/// report, never a timing, and counts scale linearly in `n` anyway.
const DYNAMIC_N: f64 = 8.0;

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

fn compile_workload(id: &str) -> Program {
    let source = std::fs::read_to_string(repo_path(&format!("bench/workloads/luau/{id}.luau")))
        .unwrap_or_else(|error| panic!("read workload {id}: {error}"));
    let chunk =
        parse_source(&source).unwrap_or_else(|error| panic!("parse workload {id}: {error}"));
    compile_with_options(&chunk, CompileOptions::default().with_source(id, &source))
        .unwrap_or_else(|error| panic!("compile workload {id}: {error}"))
}

fn workload_functions(program: &Program) -> impl Iterator<Item = &Function> {
    std::iter::once(&program.main).chain(program.functions.iter())
}

fn static_opcode_counts(program: &Program) -> BTreeMap<String, u64> {
    let mut counts = BTreeMap::new();
    for function in workload_functions(program) {
        for instruction in &function.code {
            *counts
                .entry(format!("{:?}", instruction.op))
                .or_insert(0u64) += 1;
        }
    }
    counts
}

fn run_workload_call(program: &Program, settings: VmSettings, n: f64) -> (Vm, f64) {
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    let function = vm
        .run(program)
        .unwrap_or_else(|error| panic!("workload setup failed: {error}"));
    assert_eq!(
        function.kind(),
        rustia_vm::ValueKind::Function,
        "workload must return f"
    );
    // Count only the workload call, not module setup.
    vm.reset_opcode_census();
    let result = vm
        .call_function(program, function, &[Value::number(n)])
        .unwrap_or_else(|error| panic!("workload call failed: {error}"));
    let number = result
        .as_number()
        .unwrap_or_else(|| panic!("workload checksum must be a number"));
    (vm, number)
}

fn dynamic_opcode_counts(program: &Program) -> (BTreeMap<String, u64>, f64) {
    let settings = VmSettings {
        enable_opcode_census: true,
        ..VmSettings::default()
    };
    let (vm, checksum) = run_workload_call(program, settings, DYNAMIC_N);
    let census = vm
        .opcode_census()
        .expect("census mode records opcode counts");
    let counts = census
        .into_iter()
        .map(|(opcode, count)| (format!("{opcode:?}"), count))
        .collect();
    (counts, checksum)
}

/// Duplicated from `tests/liveness_analysis.rs`: replaces every clear the
/// analysis proves to be dead pure cleanup with a value-preserving
/// `Move t, t` of identical fuel cost.
fn patch_dead_pure_cleanup(program: &Program) -> (Program, usize) {
    let mut replaced = 0;
    let mut patch = |function: &Function| -> Function {
        let liveness = FunctionLiveness::analyze(program, function)
            .unwrap_or_else(|error| panic!("analysis failed: {error}"));
        let mut function = function.clone();
        for site in liveness.clear_sites() {
            if site.kind == ClearKind::PureCleanup && site.value_dead_after {
                function.code[site.pc] =
                    Instruction::new(Opcode::Move, site.register, site.register, 0);
                replaced += 1;
            }
        }
        function
    };
    let main = patch(&program.main);
    let functions: Vec<Function> = program.functions.iter().map(&mut patch).collect();
    let constants: Vec<Constant> = program.constants().to_vec();
    let patched = Program::with_functions(main, constants, functions);
    (patched, replaced)
}

fn append_count_map(json: &mut String, indent: &str, counts: &BTreeMap<String, u64>) {
    json.push_str("{\n");
    let mut first = true;
    for (name, count) in counts {
        if !first {
            json.push_str(",\n");
        }
        first = false;
        let _ = write!(json, "{indent}  \"{name}\": {count}");
    }
    let _ = write!(json, "\n{indent}}}");
}

fn render_census() -> String {
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str(
        "  \"comment\": \"C2.3 static/dynamic opcode census over the five tracked workloads. \
         Generated by crates/rustia-compiler/tests/opcode_census.rs; regenerate with \
         RUSTIA_UPDATE_C23_CENSUS=1 cargo test -p rustia-compiler --test opcode_census. \
         Dynamic counts are exact scalar executions of one f(n) call; this file carries \
         no timing claims.\",\n",
    );
    let _ = writeln!(json, "  \"dynamic_n\": {},", DYNAMIC_N as u64);
    json.push_str("  \"workloads\": {\n");

    for (index, id) in WORKLOADS.iter().enumerate() {
        let program = compile_workload(id);
        let static_counts = static_opcode_counts(&program);
        let static_total: u64 = static_counts.values().sum();
        let (dynamic_counts, _) = dynamic_opcode_counts(&program);
        let dynamic_total: u64 = dynamic_counts.values().sum();
        let register_work = dynamic_counts.get("Move").copied().unwrap_or(0)
            + dynamic_counts.get("ClearLocalCell").copied().unwrap_or(0);
        let register_share = if dynamic_total == 0 {
            0.0
        } else {
            (register_work as f64 / dynamic_total as f64) * 100.0
        };

        let liveness = ProgramLiveness::analyze(&program)
            .unwrap_or_else(|error| panic!("analyze workload {id}: {error}"));
        let diagnostics = liveness.total_diagnostics(&program);

        let _ = writeln!(json, "    \"{id}\": {{");
        json.push_str("      \"static_opcodes\": ");
        append_count_map(&mut json, "      ", &static_counts);
        let _ = writeln!(json, ",\n      \"static_total\": {static_total},");
        json.push_str("      \"dynamic_opcodes\": ");
        append_count_map(&mut json, "      ", &dynamic_counts);
        let _ = writeln!(json, ",\n      \"dynamic_total\": {dynamic_total},");
        let _ = writeln!(
            json,
            "      \"dynamic_move_clear_share_percent\": {register_share:.2},"
        );
        json.push_str("      \"liveness_diagnostics\": {\n");
        let _ = writeln!(
            json,
            "        \"instruction_count\": {},",
            diagnostics.instruction_count
        );
        let _ = writeln!(json, "        \"move_count\": {},", diagnostics.move_count);
        let _ = writeln!(
            json,
            "        \"retargetable_move_count\": {},",
            diagnostics.retargetable_move_count
        );
        let _ = writeln!(
            json,
            "        \"clear_count\": {},",
            diagnostics.clear_count
        );
        let _ = writeln!(
            json,
            "        \"captured_clear_count\": {},",
            diagnostics.captured_clear_count
        );
        let _ = writeln!(
            json,
            "        \"uncaptured_clear_count\": {},",
            diagnostics.uncaptured_clear_count
        );
        let _ = writeln!(
            json,
            "        \"removable_clear_candidates\": {},",
            diagnostics.removable_clear_candidates
        );
        let _ = writeln!(
            json,
            "        \"target_directed_candidates\": {},",
            diagnostics.target_directed_candidates
        );
        let _ = writeln!(
            json,
            "        \"constant_operand_candidates\": {},",
            diagnostics.constant_operand_candidates
        );
        let _ = writeln!(
            json,
            "        \"branch_fold_candidates\": {},",
            diagnostics.branch_fold_candidates
        );
        let _ = writeln!(
            json,
            "        \"register_high_water\": {},",
            diagnostics.register_high_water
        );
        let _ = writeln!(
            json,
            "        \"max_registers\": {}",
            diagnostics.max_registers
        );
        json.push_str("      }\n");
        json.push_str("    }");
        json.push_str(if index + 1 < WORKLOADS.len() {
            ",\n"
        } else {
            "\n"
        });
    }

    json.push_str("  }\n}\n");
    json
}

#[test]
fn committed_census_report_matches_the_compiler() {
    let census_path = repo_path("design/58-vm-core2-c23-census.json");
    let rendered = render_census();
    if std::env::var_os("RUSTIA_UPDATE_C23_CENSUS").is_some() {
        std::fs::write(&census_path, &rendered)
            .unwrap_or_else(|error| panic!("write census report: {error}"));
        return;
    }
    let committed = std::fs::read_to_string(&census_path).unwrap_or_else(|error| {
        panic!(
            "read committed census report ({error}); generate it with \
             RUSTIA_UPDATE_C23_CENSUS=1 cargo test -p rustia-compiler --test opcode_census"
        )
    });
    assert_eq!(
        committed, rendered,
        "the committed opcode census drifted from the compiler; regenerate with \
         RUSTIA_UPDATE_C23_CENSUS=1 cargo test -p rustia-compiler --test opcode_census"
    );
}

#[test]
fn census_mode_preserves_workload_results() {
    for id in WORKLOADS {
        let program = compile_workload(id);
        let (_, default_result) = run_workload_call(&program, VmSettings::default(), DYNAMIC_N);
        let census_settings = VmSettings {
            enable_opcode_census: true,
            ..VmSettings::default()
        };
        let (vm, census_result) = run_workload_call(&program, census_settings, DYNAMIC_N);
        assert_eq!(
            default_result.to_bits(),
            census_result.to_bits(),
            "census mode changed the {id} checksum"
        );
        let census = vm.opcode_census().expect("census counts recorded");
        assert!(
            !census.is_empty(),
            "the {id} call must execute instructions"
        );
        // The census is invisible unless enabled.
        let (default_vm, _) = run_workload_call(&program, VmSettings::default(), DYNAMIC_N);
        assert!(default_vm.opcode_census().is_none());
    }
}

#[test]
fn census_mode_preserves_the_fuel_schedule() {
    // The census must not change logical fuel: the minimum fuel that
    // completes a mixed program is identical with and without it.
    let source = r#"
        local total = 0
        for i = 1, 20 do
            local t = { i, i + 1 }
            total = total + t[1] * t[2]
            if i % 3 == 0 then
                total = total - i
            end
        end
        return total
    "#;
    let chunk = parse_source(source).expect("parse");
    let program = compile_with_options(&chunk, CompileOptions::default()).expect("compile");

    let minimum_fuel = |census: bool| -> u64 {
        let succeeds = |limit: u64| {
            let settings = VmSettings {
                initial_fuel: Some(limit),
                enable_opcode_census: census,
                ..VmSettings::default()
            };
            match Vm::with_settings(Profile::ReproducibleStandard, settings).run(&program) {
                Ok(_) => true,
                Err(error) if matches!(error.kind, rustia_vm::ErrorKind::FuelExhausted { .. }) => {
                    false
                }
                Err(error) => panic!("unexpected runtime error: {error}"),
            }
        };
        let mut low = 0u64;
        let mut high = 1u64;
        while !succeeds(high) {
            high = high.checked_mul(2).expect("fuel search overflow");
        }
        while low < high {
            let middle = low + (high - low) / 2;
            if succeeds(middle) {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low
    };

    assert_eq!(minimum_fuel(false), minimum_fuel(true));
}

#[test]
fn dead_pure_cleanup_is_execution_equivalent_on_workloads() {
    for id in WORKLOADS {
        let program = compile_workload(id);
        let (_, expected) = run_workload_call(&program, VmSettings::default(), DYNAMIC_N);
        let (patched, replaced) = patch_dead_pure_cleanup(&program);
        assert!(
            replaced > 0,
            "workload {id} must contain provably dead cleanup"
        );
        let (_, patched_result) = run_workload_call(&patched, VmSettings::default(), DYNAMIC_N);
        assert_eq!(
            expected.to_bits(),
            patched_result.to_bits(),
            "dead-cleanup patch changed the {id} checksum"
        );
    }
}
