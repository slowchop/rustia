//! Validation for the default-disabled C2.3 O2 families.
//!
//! Contract under test, per `design/58-vm-core2.md` §C2.3 and the module
//! documentation of `rustia_compiler::o2`:
//!
//! - With every flag off (the default), compilation is byte-identical to the
//!   plain path and receipts are the identity schedule.
//! - Each family, enabled alone and all together, preserves values, source
//!   evaluation order, metamethod timing, error behavior, multiple-result
//!   adjustment, vararg semantics, and captured-local close/detach behavior,
//!   while strictly reducing emitted or executed instructions on its target
//!   shapes.
//! - Logical receipts always sum to the unoptimized instruction count per
//!   function, which is what makes the fuel/deterministic-GC schedule
//!   restorable by the executable tape.

use rustia_compiler::{
    compile, compile_with_options, compile_with_options_and_receipts, CompileOptions,
    ExperimentalO2Options,
};
use rustia_parser::parse_source;
use rustia_vm::{Opcode, Profile, Program, Vm, VmSettings};

fn parse(source: &str) -> rustia_parser::Chunk {
    parse_source(source).unwrap_or_else(|error| panic!("parse failed: {error}"))
}

fn options_with(o2: ExperimentalO2Options) -> CompileOptions {
    CompileOptions {
        experimental_o2: o2,
        ..CompileOptions::default()
    }
}

fn compile_o2(source: &str, o2: ExperimentalO2Options) -> Program {
    // Compile with a source map: guest-visible error locations are then
    // line-based, which is the diagnostic surface O2 preserves. Raw PCs shift
    // whenever instructions are deleted; the remapped source map keeps every
    // surviving instruction's line, so messages and error lines are identical
    // between the optimized and unoptimized programs.
    let options = options_with(o2).with_source("o2-test", source);
    compile_with_options(&parse(source), options)
        .unwrap_or_else(|error| panic!("compile failed: {error}"))
}

fn run_string(program: &Program) -> String {
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let value = vm
        .run(program)
        .unwrap_or_else(|error| panic!("run failed: {error}"));
    match value.as_number() {
        Some(number) => format!("number:{}", number.to_bits()),
        None => {
            let text = vm
                .string_value(value)
                .unwrap_or_else(|error| panic!("string read failed: {error}"));
            match text {
                Some(text) => format!("string:{text}"),
                None => format!("kind:{:?}", value.kind()),
            }
        }
    }
}

fn total_instructions(program: &Program) -> usize {
    std::iter::once(&program.main)
        .chain(program.functions.iter())
        .map(|function| function.code.len())
        .sum()
}

fn executed_instructions(program: &Program) -> u64 {
    let settings = VmSettings {
        enable_opcode_census: true,
        ..VmSettings::default()
    };
    let mut vm = Vm::with_settings(Profile::ReproducibleStandard, settings);
    vm.run(program)
        .unwrap_or_else(|error| panic!("run failed: {error}"));
    vm.opcode_census()
        .expect("census enabled")
        .into_iter()
        .map(|(_, count)| count)
        .sum()
}

const FAMILIES: [(&str, ExperimentalO2Options); 5] = [
    (
        "fold_constant_branches",
        ExperimentalO2Options {
            fold_constant_branches: true,
            commit_late_compound_locals: false,
            remove_dead_register_tails: false,
            tighten_local_slots: false,
        },
    ),
    (
        "commit_late_compound_locals",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: true,
            remove_dead_register_tails: false,
            tighten_local_slots: false,
        },
    ),
    (
        "remove_dead_register_tails",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: false,
            remove_dead_register_tails: true,
            tighten_local_slots: false,
        },
    ),
    (
        "tighten_local_slots",
        ExperimentalO2Options {
            fold_constant_branches: false,
            commit_late_compound_locals: false,
            remove_dead_register_tails: false,
            tighten_local_slots: true,
        },
    ),
    ("all", ExperimentalO2Options::ALL),
];

/// Sources covering every semantic-risk area the C2.3 constraints name.
const DIFFERENTIAL_CORPUS: [&str; 10] = [
    // Compound assignment on uncaptured locals, loops, branches.
    r#"
        local total = 0
        for i = 1, 12 do
            total += i * 2
            if i % 3 == 0 then
                total -= 1
            end
        end
        return total
    "#,
    // Constant branches around live code.
    r#"
        local trace = ""
        local flag = true
        if true then trace ..= "a" else trace ..= "b" end
        if false then trace ..= "c" end
        while flag do
            trace ..= "d"
            flag = false
        end
        return trace
    "#,
    // Source evaluation order via observable side effects.
    r#"
        local log = ""
        local function tap(name, value)
            log ..= name
            return value
        end
        local x = tap("1", 10) + tap("2", 20) * tap("3", 2)
        x += tap("4", 5)
        return log .. "=" .. tostring(x)
    "#,
    // Metamethod timing: __add fires exactly once per compound update.
    r#"
        local calls = 0
        local wrapped = setmetatable({ value = 7 }, {
            __add = function(lhs, rhs)
                calls += 1
                return lhs.value + rhs
            end,
        })
        local x = wrapped + 3
        x += 1
        return tostring(x) .. ":" .. tostring(calls)
    "#,
    // Errors keep occurring at the same operation and stay catchable.
    r#"
        local x = 1
        local ok, err = pcall(function()
            local y = 2
            y += nil
            return y
        end)
        return tostring(ok) .. ":" .. tostring(x) .. ":" .. tostring(err ~= nil)
    "#,
    // Captured locals: per-iteration cells with mutation after capture.
    r#"
        local fns = {}
        for i = 1, 3 do
            local v = 0
            v = i
            fns[i] = function() return v end
        end
        return fns[1]() + fns[2]() + fns[3]()
    "#,
    // Closures mutating captures through the cell.
    r#"
        local function counter(start, step)
            local value = start
            return function()
                value += step
                return value
            end
        end
        local a = counter(0, 2)
        local b = counter(100, 3)
        return a() + a() + b() + b()
    "#,
    // Multiple results and adjustment.
    r#"
        local function multi()
            return 1, 2, 3
        end
        local a, b, c, d = multi()
        local t = { multi(), multi() }
        return a + b + c + tostring(d == nil and 100 or -1) + #t
    "#,
    // Varargs.
    r##"
        local function pack(...)
            local first, second = ...
            return select("#", ...) * 100 + first + second
        end
        return pack(7, 8, 9)
    "##,
    // Coroutines with locals live across yields.
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
];

#[test]
fn default_compile_is_the_unoptimized_path_with_identity_receipts() {
    for source in DIFFERENTIAL_CORPUS {
        let chunk = parse(source);
        let plain = compile(&chunk).expect("compile");
        let with_default_options =
            compile_with_options(&chunk, CompileOptions::default()).expect("compile");
        let functions = |program: &Program| {
            std::iter::once(&program.main)
                .chain(program.functions.iter())
                .map(|function| (function.code.clone(), function.max_registers))
                .collect::<Vec<_>>()
        };
        assert_eq!(functions(&plain), functions(&with_default_options));

        let (program, receipts) =
            compile_with_options_and_receipts(&chunk, CompileOptions::default())
                .expect("compile with receipts");
        assert_eq!(functions(&plain), functions(&program));
        // Identity receipts price every instruction at one unit.
        for (pc, _) in program.main.code.iter().enumerate() {
            assert_eq!(receipts.main.unit_at(pc), 1);
        }
    }
}

#[test]
fn every_family_preserves_results_on_the_differential_corpus() {
    for source in DIFFERENTIAL_CORPUS {
        let baseline = compile_o2(source, ExperimentalO2Options::default());
        let expected = run_string(&baseline);
        for (name, family) in FAMILIES {
            let optimized = compile_o2(source, family);
            let actual = run_string(&optimized);
            assert_eq!(
                expected, actual,
                "family {name} changed the result of:\n{source}"
            );
        }
    }
}

#[test]
fn receipts_always_sum_to_the_unoptimized_schedule() {
    for source in DIFFERENTIAL_CORPUS {
        let chunk = parse(source);
        let baseline = compile(&chunk).expect("compile");
        for (name, family) in FAMILIES {
            let (optimized, receipts) =
                compile_with_options_and_receipts(&chunk, options_with(family))
                    .expect("compile with receipts");
            let functions = std::iter::once((&optimized.main, &receipts.main))
                .chain(optimized.functions.iter().zip(receipts.functions.iter()));
            let baseline_functions =
                std::iter::once(&baseline.main).chain(baseline.functions.iter());
            for ((function, receipts), original) in functions.zip(baseline_functions) {
                let receipt_total = receipts.logical_len(0, function.code.len());
                assert_eq!(
                    receipt_total,
                    original.code.len() as u64,
                    "family {name} lost logical units on:\n{source}"
                );
            }
        }
    }
}

#[test]
fn commit_late_compound_locals_lower_directly_into_the_target() {
    let source = r#"
        local total = 0
        for i = 1, 5 do
            total += i
        end
        return total
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let optimized = compile_o2(
        source,
        ExperimentalO2Options {
            commit_late_compound_locals: true,
            ..ExperimentalO2Options::default()
        },
    );
    // The read-copy, write-back, and result clear disappear: three fewer
    // instructions for the single compound update.
    assert_eq!(
        total_instructions(&optimized) + 3,
        total_instructions(&baseline)
    );
    // The add now targets the accumulator register in place.
    assert!(
        optimized
            .main
            .code
            .iter()
            .any(|instruction| instruction.op == Opcode::Add
                && instruction.a == instruction.b
                && baseline
                    .main
                    .code
                    .iter()
                    .all(|original| !(original.op == Opcode::Add
                        && original.a == instruction.a
                        && original.b == instruction.b
                        && original.c == instruction.c))),
        "expected a retargeted in-place Add"
    );
    assert_eq!(run_string(&baseline), run_string(&optimized));
}

#[test]
fn constant_branches_fold_to_unconditional_flow() {
    let source = r#"
        local trace = ""
        if true then trace ..= "t" else trace ..= "e" end
        if false then trace ..= "x" end
        return trace
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let optimized = compile_o2(
        source,
        ExperimentalO2Options {
            fold_constant_branches: true,
            ..ExperimentalO2Options::default()
        },
    );
    assert!(total_instructions(&optimized) < total_instructions(&baseline));
    let conditional_jumps = |program: &Program| {
        program
            .main
            .code
            .iter()
            .filter(|instruction| matches!(instruction.op, Opcode::JumpIf | Opcode::JumpIfNot))
            .count()
    };
    assert!(conditional_jumps(&optimized) < conditional_jumps(&baseline));
    assert_eq!(run_string(&baseline), run_string(&optimized));
}

#[test]
fn dead_register_tails_disappear_and_execution_shrinks() {
    let source = r#"
        local acc = 0
        local t = { 1, 2, 3, 4 }
        for i = 1, 4 do
            local x = t[i] + i
            acc = acc + x
        end
        return acc
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let optimized = compile_o2(
        source,
        ExperimentalO2Options {
            remove_dead_register_tails: true,
            ..ExperimentalO2Options::default()
        },
    );
    assert!(total_instructions(&optimized) < total_instructions(&baseline));
    assert_eq!(run_string(&baseline), run_string(&optimized));
    assert!(
        executed_instructions(&optimized) < executed_instructions(&baseline),
        "the dynamic instruction count must shrink"
    );
}

#[test]
fn tighten_local_slots_never_grows_and_preserves_results() {
    for source in DIFFERENTIAL_CORPUS {
        let baseline = compile_o2(source, ExperimentalO2Options::default());
        let optimized = compile_o2(
            source,
            ExperimentalO2Options {
                tighten_local_slots: true,
                ..ExperimentalO2Options::default()
            },
        );
        for (function, original) in std::iter::once(&optimized.main)
            .chain(optimized.functions.iter())
            .zip(std::iter::once(&baseline.main).chain(baseline.functions.iter()))
        {
            assert!(function.max_registers <= original.max_registers);
            assert!(function.max_registers >= 1);
        }
        assert_eq!(run_string(&baseline), run_string(&optimized));
    }
}

#[test]
fn captured_local_detaches_survive_every_family() {
    // The mutable per-iteration capture: any lost detach makes iterations
    // share one cell and changes the sum.
    let source = r#"
        local fns = {}
        for i = 1, 3 do
            local v = 0
            v = i
            fns[i] = function() return v end
        end
        return fns[1]() + fns[2]() + fns[3]()
    "#;
    for (name, family) in FAMILIES {
        let optimized = compile_o2(source, family);
        assert_eq!(
            run_string(&optimized),
            format!("number:{}", 6f64.to_bits()),
            "family {name} broke captured-local detach behavior"
        );
    }
}

#[test]
fn erroring_compound_updates_keep_their_error_site_and_message() {
    let source = r#"
        local ok, err = pcall(function()
            local y = 2
            y += {}
            return y
        end)
        return tostring(ok) .. ":" .. tostring(err)
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let expected = run_string(&baseline);
    assert!(expected.starts_with("string:false:"));
    for (name, family) in FAMILIES {
        let optimized = compile_o2(source, family);
        assert_eq!(
            run_string(&optimized),
            expected,
            "family {name} changed error behavior"
        );
    }
}

#[test]
fn full_o2_reduces_static_and_dynamic_instructions_broadly() {
    // A mixed program with hot accumulator shapes: all families together
    // must strictly shrink both the emitted and the executed instruction
    // stream while preserving the checksum.
    let source = r#"
        local acc = 0
        local t = {}
        for i = 1, 32 do
            t[i] = i * i
        end
        for i = 1, 32 do
            local x = t[i] + i
            acc += x
            if i % 4 == 0 then
                acc -= 1
            end
        end
        for _, v in ipairs(t) do
            acc += v
        end
        return acc
    "#;
    let baseline = compile_o2(source, ExperimentalO2Options::default());
    let optimized = compile_o2(source, ExperimentalO2Options::ALL);
    assert!(total_instructions(&optimized) < total_instructions(&baseline));
    assert_eq!(run_string(&baseline), run_string(&optimized));
    let baseline_dynamic = executed_instructions(&baseline);
    let optimized_dynamic = executed_instructions(&optimized);
    assert!(
        optimized_dynamic < baseline_dynamic,
        "expected fewer executed instructions ({optimized_dynamic} vs {baseline_dynamic})"
    );
}
