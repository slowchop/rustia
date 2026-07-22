use rustia_compiler::compile;
use rustia_parser::parse_source;
use rustia_vm::{ErrorKind, Opcode, Profile, Program, Vm, VmSettings};
use std::num::NonZeroU64;

fn compile_source(source: &str) -> Program {
    compile(&parse_source(source).unwrap_or_else(|error| panic!("parse failed: {error}")))
        .unwrap_or_else(|error| panic!("compile failed: {error}"))
}

fn run_number(source: &str) -> f64 {
    let program = compile_source(source);
    Vm::new(Profile::ReproducibleStandard)
        .run(&program)
        .unwrap_or_else(|error| panic!("run failed: {error}"))
        .as_number()
        .unwrap_or_else(|| panic!("source did not return a number: {source}"))
}

fn run_string(source: &str) -> String {
    let program = compile_source(source);
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    let value = vm
        .run(&program)
        .unwrap_or_else(|error| panic!("run failed: {error}"));
    vm.string_value(value)
        .unwrap_or_else(|error| panic!("string lookup failed: {error}"))
        .unwrap_or_else(|| panic!("source did not return a string: {source}"))
}

fn minimum_fuel(program: &Program) -> u64 {
    let succeeds = |limit| {
        let settings = VmSettings {
            initial_fuel: Some(limit),
            ..VmSettings::default()
        };
        match Vm::with_settings(Profile::ReproducibleStandard, settings).run(program) {
            Ok(_) => true,
            Err(error) if matches!(error.kind, ErrorKind::FuelExhausted { .. }) => false,
            Err(error) => panic!("unexpected runtime error: {error}"),
        }
    };

    let mut low = 0_u64;
    let mut high = 1_u64;
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
}

#[test]
fn integer_string_fast_path_keeps_shortest_round_trip_formatting() {
    let cases = [
        ("9007199254740991", "9007199254740991"),
        ("9007199254740992", "9007199254740992"),
        ("1000000000000000128", "1000000000000000100"),
        ("-1000000000000000128", "-1000000000000000100"),
        ("999999999999999872", "999999999999999900"),
    ];

    for (literal, expected) in cases {
        assert_eq!(
            run_string(&format!("return tostring({literal})")),
            expected,
            "{literal}"
        );
    }
}

#[test]
fn numeric_sort_fast_path_preserves_numeric_and_fallback_semantics() {
    assert_eq!(
        run_string(
            r#"
                local values = {math.huge, 3, -math.huge, -2, 3, 0}
                table.sort(values)
                for i = 2, #values do
                    assert(not (values[i] < values[i - 1]))
                end
                return table.concat(values, ",")
            "#,
        ),
        "-inf,-2,0,3,3,inf"
    );

    assert_eq!(
        run_number(
            r#"
                local nan = 0 / 0
                local values = {2, nan, 1}
                table.sort(values)
                local nan_count = 0
                for _, value in values do
                    if value ~= value then nan_count += 1 end
                end
                return #values * 10 + nan_count
            "#,
        ),
        31.0
    );

    assert_eq!(
        run_string(
            r#"
                local values = {1, 3, 2}
                local calls = 0
                table.sort(values, function(a, b)
                    calls += 1
                    return a > b
                end)
                assert(calls > 0)
                return table.concat(values, ",")
            "#,
        ),
        "3,2,1"
    );
}

#[test]
fn scalar_math_fastcalls_preserve_hot_calls_deopts_and_fuel() {
    let program = compile_source(
        r#"
            local abs, floor, sqrt, sin, cos =
                math.abs, math.floor, math.sqrt, math.sin, math.cos
            local result = 0
            for _ = 1, 3 do
                result += abs("-4") + floor("2.9") + sqrt("9") + sin("0") + cos("0")
            end
            return result
        "#,
    );
    assert!(
        program
            .main
            .code
            .iter()
            .filter(|instruction| instruction.op == Opcode::FastCall)
            .count()
            >= 5
    );
    assert_eq!(
        Vm::new(Profile::ReproducibleStandard)
            .run(&program)
            .unwrap()
            .as_number(),
        Some(30.0)
    );

    assert_eq!(
        run_number(
            r#"
                local floor = math.floor
                local first = floor(1.9)
                floor = function(value) return value + 40 end
                return first + floor(2)
            "#,
        ),
        43.0
    );

    let fast = compile_source(
        "local floor = math.floor local result = 0 for i = 1, 3 do result += floor(i + 0.5) end return result",
    );
    let mut ordinary = fast.clone();
    let mut replacements = 0;
    for instruction in &mut ordinary.main.code {
        if instruction.op == Opcode::FastCall {
            instruction.op = Opcode::Call;
            replacements += 1;
        }
    }
    assert_eq!(replacements, 1);
    assert_eq!(minimum_fuel(&fast), minimum_fuel(&ordinary));
}

#[test]
fn native_next_and_inext_iterator_fast_paths_preserve_loop_cells() {
    let source = r#"
        local array = {10, 20, 30}
        local inext, state, control = ipairs(array)
        local closures = {}
        local sum = 0
        for index, value, extra in inext, state, control do
            closures[index] = function() return index * 100 + value, extra end
            sum += index + value
        end
        for index = 1, 3 do
            local packed, extra = closures[index]()
            assert(packed == index * 100 + array[index])
            assert(extra == nil)
        end

        local next_fn, table_state, table_control = pairs({a = 7, b = 11})
        for _, value in next_fn, table_state, table_control do
            sum += value
        end
        return sum
    "#;
    let program = compile_source(source);
    assert!(
        program
            .main
            .code
            .iter()
            .filter(|instruction| instruction.op == Opcode::GenericCallLoop)
            .count()
            >= 2
    );
    assert_eq!(
        Vm::new(Profile::ReproducibleStandard)
            .run(&program)
            .unwrap()
            .as_number(),
        Some(84.0)
    );

    let invalid = compile_source(
        "local function iterate() for key in next, 42, nil do end end return pcall(iterate)",
    );
    let mut vm = Vm::new(Profile::ReproducibleStandard);
    assert_eq!(vm.run(&invalid).unwrap().as_bool(), Some(false));
}

#[test]
fn inline_and_spilled_register_and_upvalue_storage_are_equivalent() {
    let mut source = String::from("local ");
    for index in 1..=40 {
        if index > 1 {
            source.push_str(", ");
        }
        source.push_str(&format!("v{index}"));
    }
    source.push_str(" = ");
    for index in 1..=40 {
        if index > 1 {
            source.push_str(", ");
        }
        source.push_str(&index.to_string());
    }
    source.push_str(" local function total() return ");
    for index in 1..=40 {
        if index > 1 {
            source.push_str(" + ");
        }
        source.push_str(&format!("v{index}"));
    }
    source.push_str(" end return total()");
    assert_eq!(run_number(&source), 820.0);

    assert_eq!(
        run_number(
            r#"
                local function worker()
                    local a01, a02, a03, a04, a05, a06, a07, a08, a09, a10 = 1,2,3,4,5,6,7,8,9,10
                    local a11, a12, a13, a14, a15, a16, a17, a18, a19, a20 = 11,12,13,14,15,16,17,18,19,20
                    local a21, a22, a23, a24, a25, a26, a27, a28, a29, a30 = 21,22,23,24,25,26,27,28,29,30
                    local a31, a32, a33, a34, a35, a36, a37, a38, a39, a40 = 31,32,33,34,35,36,37,38,39,40
                    coroutine.yield(a40)
                    return a01 + a10 + a20 + a30 + a40
                end
                local thread = coroutine.create(worker)
                local ok, yielded = coroutine.resume(thread)
                assert(ok and yielded == 40)
                local ok2, result = coroutine.resume(thread)
                assert(ok2)
                return result
            "#,
        ),
        101.0
    );
}

#[test]
fn leaf_counter_plan_falls_back_before_mutation_for_coercion_and_metamethods() {
    assert_eq!(
        run_number(
            r#"
                local function makeCounter()
                    local value, step = 1, 2
                    local function counter()
                        value = value + step
                        return value
                    end
                    local function reset(nextValue, nextStep)
                        value, step = nextValue, nextStep
                    end
                    return counter, reset
                end

                local counter, reset = makeCounter()
                assert(counter() == 3)
                reset("4", "3")
                assert(counter() == 7)

                local addable = setmetatable({value = 10}, {
                    __add = function(lhs, rhs) return lhs.value + rhs end,
                })
                reset(addable, 5)
                assert(counter() == 15)
                return counter()
            "#,
        ),
        20.0
    );
}

#[test]
fn leaf_iterator_plan_falls_back_before_mutation_for_len_and_index_metamethods() {
    assert_eq!(
        run_number(
            r#"
                local function makeIterator(items)
                    local index = 0
                    return function()
                        index = index + 1
                        if index <= #items then return index, items[index] end
                    end
                end

                local plain = makeIterator({10, 20})
                local p1, v1 = plain()
                local p2, v2 = plain()
                assert(p1 == 1 and v1 == 10 and p2 == 2 and v2 == 20)
                assert(plain() == nil)

                local calls = 0
                local intercepted = setmetatable({10}, {
                    __len = function()
                        calls = calls + 1
                        return 3
                    end,
                    __index = function(_, index)
                        calls = calls + 10
                        return index * 100
                    end,
                })
                local iter = makeIterator(intercepted)
                local i1, x1 = iter()
                local i2, x2 = iter()
                local i3, x3 = iter()
                assert(i1 == 1 and x1 == 10)
                assert(i2 == 2 and x2 == 200)
                assert(i3 == 3 and x3 == 300)
                assert(iter() == nil)
                return calls
            "#,
        ),
        24.0
    );
}

#[test]
fn leaf_plans_preserve_fuel_and_deterministic_collection_boundaries() {
    let program = compile_source(
        r#"
            local value, step = 0, 3
            local function counter()
                value = value + step
                return value
            end
            local total = 0
            for _ = 1, 80 do total = total + counter() end
            return total
        "#,
    );

    let minimum = minimum_fuel(&program);
    let finite = VmSettings {
        initial_fuel: Some(minimum),
        ..VmSettings::default()
    };
    assert_eq!(
        Vm::with_settings(Profile::ReproducibleStandard, finite)
            .run(&program)
            .unwrap()
            .as_number(),
        Some(9720.0)
    );

    let interval = NonZeroU64::new(7).unwrap();
    let mut fast = Vm::with_settings(
        Profile::DeterministicSim,
        VmSettings {
            deterministic_gc_interval: interval,
            ..VmSettings::default()
        },
    );
    let fast_result = fast.run(&program).unwrap();
    let mut interpreted = Vm::with_settings(
        Profile::DeterministicSim,
        VmSettings {
            deterministic_gc_interval: interval,
            enable_conformance_coverage: true,
            ..VmSettings::default()
        },
    );
    let interpreted_result = interpreted.run(&program).unwrap();
    assert_eq!(fast_result.as_number(), interpreted_result.as_number());
    assert_eq!(
        fast.deterministic_collection_count(),
        interpreted.deterministic_collection_count()
    );

    let one_call = compile_source(
        "local value, step = 0, 1 local function counter() value += step return value end return counter()",
    );
    let recursion_error = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            recursion_limit: 0,
            ..VmSettings::default()
        },
    )
    .run(&one_call)
    .unwrap_err();
    assert!(matches!(
        recursion_error.kind,
        ErrorKind::RecursionLimit { limit: 0 }
    ));
}

#[test]
fn numeric_loop_kernels_do_not_elide_overwritten_index_metamethod_reads() {
    let program = compile_source(
        r#"
            local hits = 0
            local mt = {
                __index = function(_, index)
                    hits = hits + 1
                    return index
                end,
            }
            local function scan(n, values)
                for i = 1, n do
                    local dead = values[i]
                    dead = 0
                end
            end
            scan(4, setmetatable({}, mt))
            return hits
        "#,
    );
    let mut fast = Vm::new(Profile::ReproducibleStandard);
    assert_eq!(fast.run(&program).unwrap().as_number(), Some(4.0));
    let mut fuelled = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            initial_fuel: Some(1_000_000),
            ..VmSettings::default()
        },
    );
    assert_eq!(fuelled.run(&program).unwrap().as_number(), Some(4.0));
}

#[test]
fn numeric_loop_kernels_do_not_elide_overwritten_arithmetic_errors() {
    let program = compile_source(
        r#"
            local function scan(n, values)
                for i = 1, n do
                    local dead = values[i] + 1
                    dead = 0
                end
                return true
            end
            return pcall(scan, 2, {1, {}})
        "#,
    );
    let mut fast = Vm::new(Profile::ReproducibleStandard);
    assert_eq!(fast.run(&program).unwrap().as_bool(), Some(false));
    let mut fuelled = Vm::with_settings(
        Profile::ReproducibleStandard,
        VmSettings {
            initial_fuel: Some(1_000_000),
            ..VmSettings::default()
        },
    );
    assert_eq!(fuelled.run(&program).unwrap().as_bool(), Some(false));
}
