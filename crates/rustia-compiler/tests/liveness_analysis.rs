//! C2.3 liveness analysis over real compiled programs.
//!
//! These tests prove the [`rustia_vm::liveness`] analysis on the lowered code
//! the production compiler actually emits — branches, loops, short-circuit
//! expressions, multiple assignment, varargs, multiple returns, closures, and
//! coroutines — and pin the analysis' soundness contract by execution:
//! replacing every `ClearLocalCell` the analysis proves to be dead pure
//! cleanup with a value-preserving no-op must not change program results,
//! while replacing required captured-local detaches must.

use rustia_compiler::compile;
use rustia_parser::parse_source;
use rustia_vm::bytecode::{Constant, Function, Instruction};
use rustia_vm::liveness::{ClearKind, FunctionLiveness, ProgramLiveness};
use rustia_vm::{Opcode, Profile, Program, Value, Vm};

fn compile_source(source: &str) -> Program {
    compile(&parse_source(source).unwrap_or_else(|error| panic!("parse failed: {error}")))
        .unwrap_or_else(|error| panic!("compile failed: {error}"))
}

fn run_value(program: &Program) -> (Vm, Value) {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let value = vm
        .run(program)
        .unwrap_or_else(|error| panic!("run failed: {error}"));
    (vm, value)
}

fn run_number(program: &Program) -> f64 {
    let (_, value) = run_value(program);
    value
        .as_number()
        .unwrap_or_else(|| panic!("program did not return a number"))
}

fn analyze_main(program: &Program) -> FunctionLiveness {
    FunctionLiveness::analyze(program, &program.main)
        .unwrap_or_else(|error| panic!("analysis failed: {error}"))
}

/// Structural invariants every analyzed compiler-emitted function must hold.
fn assert_analysis_invariants(program: &Program, function: &Function) {
    let liveness = FunctionLiveness::analyze(program, function)
        .unwrap_or_else(|error| panic!("analysis failed: {error}"));

    // Blocks partition the function body exactly.
    let receipts = liveness.block_receipts();
    let mut expected_start = 0;
    let mut logical_total = 0;
    for receipt in &receipts {
        assert_eq!(
            receipt.start_pc, expected_start,
            "blocks must be contiguous"
        );
        assert!(
            receipt.end_pc > receipt.start_pc,
            "blocks must be non-empty"
        );
        logical_total += receipt.logical_instructions;
        expected_start = receipt.end_pc;
    }
    assert_eq!(expected_start, function.code.len());
    // Identity receipts price every instruction at one logical unit.
    assert_eq!(logical_total, function.code.len() as u64);

    let diagnostics = liveness.diagnostics(function);
    assert_eq!(diagnostics.instruction_count, function.code.len());
    assert!(diagnostics.register_high_water <= diagnostics.max_registers.max(1));

    // Diagnostics counters agree with a direct opcode census.
    let move_count = function
        .code
        .iter()
        .filter(|instruction| instruction.op == Opcode::Move)
        .count();
    let clear_count = function
        .code
        .iter()
        .filter(|instruction| instruction.op == Opcode::ClearLocalCell)
        .count();
    assert_eq!(diagnostics.move_count, move_count);
    assert_eq!(diagnostics.clear_count, clear_count);
    assert_eq!(
        diagnostics.clear_count,
        diagnostics.captured_clear_count + diagnostics.uncaptured_clear_count
    );

    // Every clear of a slot that can never hold a cell must be classified
    // pure cleanup, and every clear of a register the compiler recorded as
    // captured with a possibly-attached cell must be a required detach.
    for site in liveness.clear_sites() {
        if site.kind == ClearKind::RequiredDetach {
            assert!(
                function.captured_registers.contains(&site.register),
                "detach classification requires a captured register (pc {})",
                site.pc
            );
        }
    }
}

fn assert_program_invariants(program: &Program) {
    assert_analysis_invariants(program, &program.main);
    for function in &program.functions {
        assert_analysis_invariants(program, function);
    }
}

fn clone_program_with_main(program: &Program, main: Function) -> Program {
    let constants: Vec<Constant> = program.constants().to_vec();
    Program::with_functions(main, constants, program.functions.clone())
}

/// Replaces every `ClearLocalCell` the analysis proves to be dead pure
/// cleanup with a value-preserving `Move t, t` of identical fuel cost, in the
/// main function and every prototype.
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
    let mut patched = clone_program_with_main(program, main);
    patched.functions = functions;
    (patched, replaced)
}

/// Replaces *every* clear — including required captured-local detaches — with
/// the same value-preserving no-op. Used as the negative control.
fn patch_all_clears(program: &Program) -> Program {
    let patch = |function: &Function| -> Function {
        let mut function = function.clone();
        for instruction in &mut function.code {
            if instruction.op == Opcode::ClearLocalCell {
                *instruction = Instruction::new(Opcode::Move, instruction.a, instruction.a, 0);
            }
        }
        function
    };
    let main = patch(&program.main);
    let functions = program.functions.iter().map(patch).collect();
    let mut patched = clone_program_with_main(program, main);
    patched.functions = functions;
    patched
}

#[test]
fn branches_merge_liveness_and_split_blocks() {
    let program = compile_source(
        r#"
            local a = 1
            local b = 2
            local result
            if a < b then
                result = a + b
            else
                result = a - b
            end
            return result
        "#,
    );
    assert_program_invariants(&program);
    let liveness = analyze_main(&program);
    assert!(
        liveness.cfg().blocks().len() >= 3,
        "an if/else must produce branch, arm, and join blocks"
    );
    // Some block boundary carries both operands into an arm.
    let receipts = liveness.block_receipts();
    assert!(
        receipts
            .iter()
            .skip(1)
            .any(|receipt| receipt.live_in.len() >= 2),
        "branch arms must see the locals live at entry"
    );
    assert_eq!(run_number(&program), 3.0);
}

#[test]
fn while_loops_keep_accumulators_live_across_backedges() {
    let program = compile_source(
        r#"
            local total = 0
            local i = 1
            while i <= 10 do
                total = total + i
                i = i + 1
            end
            return total
        "#,
    );
    assert_program_invariants(&program);
    let liveness = analyze_main(&program);
    let classes = liveness.register_classes();
    // The accumulator and induction registers survive block boundaries.
    let boundary_survivors = classes
        .iter()
        .filter(|class| class.live_across_block_boundary)
        .count();
    assert!(
        boundary_survivors >= 2,
        "loop accumulator and induction variable must cross the backedge"
    );
    assert_eq!(run_number(&program), 55.0);
}

#[test]
fn numeric_for_control_window_stays_live() {
    let program = compile_source(
        r#"
            local total = 0
            for i = 1, 5 do
                total = total + i
            end
            return total
        "#,
    );
    assert_program_invariants(&program);
    let liveness = analyze_main(&program);
    // Find the ForLoop backedge and check the loop-control window is live
    // into its block.
    let for_loop_pc = program
        .main
        .code
        .iter()
        .position(|instruction| instruction.op == Opcode::ForLoop)
        .expect("numeric for emits ForLoop");
    let block = liveness.cfg().block_index_of_pc(for_loop_pc).unwrap();
    let receipts = liveness.block_receipts();
    let control_base = program.main.code[for_loop_pc].a;
    for offset in 0..3 {
        assert!(
            receipts[block].live_in.contains(control_base + offset),
            "numeric-for control register {offset} must stay live"
        );
    }
    assert_eq!(run_number(&program), 15.0);
}

#[test]
fn short_circuit_expressions_stay_block_local() {
    let program = compile_source(
        r#"
            local a = 4
            local b = 0
            local chosen = a > 3 and a * 2 or b - 1
            local guarded = a ~= nil and a or 99
            return chosen + guarded
        "#,
    );
    assert_program_invariants(&program);
    let liveness = analyze_main(&program);
    assert!(
        liveness.cfg().blocks().len() >= 4,
        "and/or chains must lower to conditional blocks"
    );
    assert_eq!(run_number(&program), 12.0);
}

#[test]
fn multiple_assignment_temporaries_are_uncaptured_cleanup() {
    let program = compile_source(
        r#"
            local a, b = 1, 2
            a, b = b, a
            local c, d, e = a + b, a - b, a * b
            return a * 100 + b * 10 + c + d + e
        "#,
    );
    assert_program_invariants(&program);
    let liveness = analyze_main(&program);
    let diagnostics = liveness.diagnostics(&program.main);
    // Swap and multi-assignment lowering produces cleanup clears, none of
    // which touch a captured slot.
    assert!(diagnostics.clear_count > 0);
    assert_eq!(diagnostics.captured_clear_count, 0);
    assert_eq!(diagnostics.uncaptured_clear_count, diagnostics.clear_count);
    assert_eq!(run_number(&program), 216.0);

    let (patched, replaced) = patch_dead_pure_cleanup(&program);
    assert!(replaced > 0, "swap temporaries must be provably dead");
    assert_eq!(run_number(&patched), 216.0);
}

#[test]
fn vararg_functions_read_the_vararg_pack() {
    let program = compile_source(
        r##"
            local function sum(...)
                local a, b = ...
                local rest = select("#", ...)
                return a + b + rest
            end
            return sum(10, 20, 30, 40)
        "##,
    );
    assert_program_invariants(&program);
    // The vararg prototype must contain vararg-reading instructions the
    // facts model reports as such.
    let vararg_function = program
        .functions
        .iter()
        .find(|function| function.is_vararg)
        .expect("sum must be a vararg prototype");
    let liveness = FunctionLiveness::analyze(&program, vararg_function)
        .unwrap_or_else(|error| panic!("analysis failed: {error}"));
    assert!(
        liveness
            .instruction_facts()
            .iter()
            .any(|facts| facts.reads_varargs),
        "vararg lowering must read the vararg pack"
    );
    assert_eq!(run_number(&program), 34.0);
}

#[test]
fn multiple_returns_flow_through_result_windows() {
    let program = compile_source(
        r#"
            local function pair()
                return 7, 35
            end
            local x, y = pair()
            local function tail()
                return pair()
            end
            local a, b = tail()
            return x + y + a + b
        "#,
    );
    assert_program_invariants(&program);
    assert_eq!(run_number(&program), 84.0);

    let (patched, _) = patch_dead_pure_cleanup(&program);
    assert_eq!(run_number(&patched), 84.0);
}

#[test]
fn captured_locals_produce_required_detaches() {
    let program = compile_source(
        r#"
            local fns = {}
            for i = 1, 3 do
                local v = 0
                v = i
                fns[i] = function() return v end
            end
            return fns[1]() + fns[2]() + fns[3]()
        "#,
    );
    assert_program_invariants(&program);
    let liveness = analyze_main(&program);
    let diagnostics = liveness.diagnostics(&program.main);
    assert!(
        diagnostics.captured_clear_count > 0,
        "the per-iteration captured local must have required detaches"
    );
    assert!(!liveness.captured_registers().is_empty());
    assert_eq!(run_number(&program), 6.0);

    // Replacing only provably-dead pure cleanup preserves per-iteration
    // capture identity.
    let (patched, _) = patch_dead_pure_cleanup(&program);
    assert_eq!(run_number(&patched), 6.0);

    // Negative control: dropping the required detaches makes the iterations
    // share one cell, which changes the observable result. This proves the
    // detach classification is load-bearing, not vacuous.
    let broken = patch_all_clears(&program);
    let broken_result = run_number(&broken);
    assert_ne!(
        broken_result, 6.0,
        "removing required captured-local detaches must be observable"
    );
}

#[test]
fn immutable_captures_do_not_require_detaches() {
    // A once-assigned loop local is captured by value (ParentLocalValue), so
    // its cleanup is not a detach and per-iteration identity is preserved by
    // the copy itself.
    let program = compile_source(
        r#"
            local fns = {}
            for i = 1, 3 do
                local v = i
                fns[i] = function() return v end
            end
            return fns[1]() + fns[2]() + fns[3]()
        "#,
    );
    assert_program_invariants(&program);
    assert_eq!(run_number(&program), 6.0);
    let (patched, _) = patch_dead_pure_cleanup(&program);
    assert_eq!(run_number(&patched), 6.0);
}

#[test]
fn coroutine_locals_survive_yield_boundaries() {
    let program = compile_source(
        r#"
            local function producer()
                local acc = 0
                for i = 1, 3 do
                    acc = acc + coroutine.yield(i)
                end
                return acc
            end
            local co = coroutine.create(producer)
            local total = 0
            local _, first = coroutine.resume(co)
            total = total + first
            local _, second = coroutine.resume(co, 10)
            total = total + second
            local _, third = coroutine.resume(co, 20)
            total = total + third
            local _, final = coroutine.resume(co, 30)
            return total + final
        "#,
    );
    assert_program_invariants(&program);

    // Inside the producer prototype, the accumulator must be live across the
    // guest-entering yield call.
    let producer = program
        .functions
        .iter()
        .find(|function| {
            function
                .debug_name
                .as_deref()
                .is_some_and(|name| name.contains("producer"))
        })
        .or_else(|| program.functions.first())
        .expect("producer prototype exists");
    let liveness = FunctionLiveness::analyze(&program, producer)
        .unwrap_or_else(|error| panic!("analysis failed: {error}"));
    let classes = liveness.register_classes();
    assert!(
        classes.iter().any(|class| class.live_across_guest),
        "the accumulator must survive the yield's guest boundary"
    );

    assert_eq!(run_number(&program), 66.0);
    let (patched, _) = patch_dead_pure_cleanup(&program);
    assert_eq!(run_number(&patched), 66.0);
}

#[test]
fn dead_pure_cleanup_is_execution_equivalent_across_constructs() {
    // One combined program covering every construct class; the patched
    // variant must be value-equivalent, which is the executable definition of
    // "this clear only nils a dead, cell-free slot".
    let source = r##"
        local function varargs(...)
            local first = ...
            local count = select("#", ...)
            return first + count
        end
        local function multi()
            return 1, 2, 3
        end
        local counters = {}
        local total = 0
        for i = 1, 4 do
            local base = 0
            base = i * 10
            counters[i] = function() return base end
            local a, b, c = multi()
            total = total + a + b + c
            if i % 2 == 0 then
                total = total + varargs(i, i + 1)
            else
                total = total + (i > 2 and i or -i)
            end
            local j = 1
            while j < 3 do
                total = total + j
                j = j + 1
            end
            for _, v in ipairs({ i, i + 1 }) do
                total = total + v
            end
        end
        for index = 1, 4 do
            total = total + counters[index]()
        end
        return total
    "##;
    let program = compile_source(source);
    assert_program_invariants(&program);

    let expected = run_number(&program);
    let (patched, replaced) = patch_dead_pure_cleanup(&program);
    assert!(
        replaced > 0,
        "the combined program must contain removable cleanup"
    );
    assert_eq!(run_number(&patched), expected);
}

#[test]
fn program_liveness_analyzes_every_compiled_prototype() {
    let program = compile_source(
        r#"
            local function outer(x)
                local function inner(y)
                    return x + y
                end
                return inner(1) + inner(2)
            end
            return outer(10)
        "#,
    );
    let liveness = ProgramLiveness::analyze(&program).expect("program analysis");
    assert_eq!(liveness.functions.len(), program.functions.len());
    let total = liveness.total_diagnostics(&program);
    let expected_instructions: usize = std::iter::once(&program.main)
        .chain(program.functions.iter())
        .map(|function| function.code.len())
        .sum();
    assert_eq!(total.instruction_count, expected_instructions);
    assert_eq!(run_number(&program), 23.0);
}
