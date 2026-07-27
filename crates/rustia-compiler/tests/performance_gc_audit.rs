use rustia_compiler::compile;
use rustia_parser::parse_source;
use rustia_vm::{Profile, Program, Vm, VmSettings};
use std::num::NonZeroU64;

fn compile_source(source: &str) -> Program {
    compile(&parse_source(source).unwrap_or_else(|error| panic!("parse failed: {error}")))
        .unwrap_or_else(|error| panic!("compile failed: {error}"))
}

#[test]
fn function_upvalues_and_environments_remain_weak_collection_roots() {
    let upvalue_program = compile_source(
        r#"
            weak = setmetatable({}, {__mode = "v"})
            local object = {marker = 17}
            weak[2] = object
            return function()
                collectgarbage("collect")
                return weak[2] == object and object.marker == 17
            end
        "#,
    );
    let mut upvalue_vm = Vm::new(Profile::ReproducibleStandard);
    let holder = upvalue_vm.run(&upvalue_program).unwrap();
    assert_eq!(
        upvalue_vm
            .call_retained_function(holder, &[])
            .unwrap()
            .first()
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    let environment_program = compile_source(
        r#"
            weak = setmetatable({}, {__mode = "v"})
            local globals = getfenv(0)
            local environment = setmetatable({marker = 23}, {__index = globals})
            weak[3] = environment
            local function probe()
                collectgarbage("collect")
                return weak[3] ~= nil and marker == 23
            end
            setfenv(probe, environment)
            return probe
        "#,
    );
    let mut environment_vm = Vm::new(Profile::ReproducibleStandard);
    let probe = environment_vm.run(&environment_program).unwrap();
    assert_eq!(
        environment_vm
            .call_retained_function(probe, &[])
            .unwrap()
            .first()
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn leaf_shortcuts_match_interpreted_gc_countdown_across_boundaries_and_runs() {
    let program = compile_source(
        r#"
            local value, step = 0, 3
            local function counter()
                value = value + step
                return value
            end

            local function makeIterator(items)
                local index = 0
                return function()
                    index = index + 1
                    if index <= #items then return index, items[index] end
                end
            end

            local total = 0
            for _ = 1, 31 do total += counter() end
            for index, item in makeIterator({2, 4, 8, 16, 32}) do
                total += index + item
            end
            return total
        "#,
    );

    for interval in 1..=23 {
        let interval = NonZeroU64::new(interval).unwrap();
        let mut shortcut = Vm::with_settings(
            Profile::DeterministicSim,
            VmSettings {
                deterministic_gc_interval: interval,
                ..VmSettings::default()
            },
        );
        let mut interpreted = Vm::with_settings(
            Profile::DeterministicSim,
            VmSettings {
                deterministic_gc_interval: interval,
                enable_conformance_coverage: true,
                ..VmSettings::default()
            },
        );

        for run in 1..=2 {
            let shortcut_result = shortcut.run(&program).unwrap();
            let interpreted_result = interpreted.run(&program).unwrap();
            assert_eq!(
                shortcut_result.as_number(),
                interpreted_result.as_number(),
                "result mismatch at interval {interval}, run {run}",
            );
            assert_eq!(
                shortcut.deterministic_collection_count(),
                interpreted.deterministic_collection_count(),
                "collection mismatch at interval {interval}, run {run}",
            );
        }
    }
}

#[test]
fn quiescent_collection_preserves_bounded_todo_application_state() {
    for source in [
        include_str!("../../../bench/workloads/luau/19_todo_steady.luau"),
        include_str!("../../../bench/workloads/luau/20_todo_history.luau"),
    ] {
        let program = compile_source(source);
        let mut collected = Vm::new(Profile::ReproducibleStandard);
        let collected_function = collected.run(&program).unwrap();
        let collected_function = collected.retain_value(collected_function);
        let mut natural = Vm::new(Profile::ReproducibleStandard);
        let natural_function = natural.run(&program).unwrap();
        let natural_function = natural.retain_value(natural_function);

        for _ in 0..40 {
            let collected_result = collected
                .call_retained_function(
                    collected_function.value(),
                    &[rustia_vm::Value::number(128.0)],
                )
                .unwrap();
            let natural_result = natural
                .call_retained_function(
                    natural_function.value(),
                    &[rustia_vm::Value::number(128.0)],
                )
                .unwrap();
            assert_eq!(
                collected_result[0].as_number(),
                natural_result[0].as_number()
            );
        }

        collected.collect_garbage();
        let after_collection = collected
            .call_retained_function(
                collected_function.value(),
                &[rustia_vm::Value::number(128.0)],
            )
            .unwrap();
        let natural_next = natural
            .call_retained_function(natural_function.value(), &[rustia_vm::Value::number(128.0)])
            .unwrap();
        assert_eq!(after_collection[0].as_number(), natural_next[0].as_number());
    }
}
