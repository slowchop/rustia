//! Luau 0.709 rejects invalid builtin arguments during execution, not compilation.
//! Oracle capture and the three conformance fixtures are recorded in design/82.
use rustia::{ErrorKind, Profile, Rustia, RustiaBuilder};

fn profiles() -> [Profile; 4] {
    [
        Profile::LuauCompat,
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::Puc51Compat,
    ]
}

fn vm(profile: Profile) -> Rustia {
    RustiaBuilder::standard().profile(profile).build().unwrap()
}

const INVALID_CALLS: &[&str] = &[
    "tostring()",
    "tonumber()",
    "type()",
    "typeof()",
    "rawget({})",
    "rawset({}, 1)",
    "rawlen()",
    "next()",
    "table.concat()",
    "table.getn()",
    "table.maxn()",
    "table.create()",
    "table.find({})",
    "table.insert()",
    "table.remove()",
    "string.sub('abc')",
    "string.len()",
    "string.byte()",
    "string.upper()",
    "string.lower()",
    "string.reverse()",
    "string.rep('abc')",
    "string.find('abc')",
    "string.split()",
    "math.unreal()",
    "table.unreal()",
    "string.unreal()",
];

#[test]
fn invalid_builtin_calls_fail_after_execution_starts() {
    for profile in profiles() {
        let mut vm = vm(profile);
        for call in INVALID_CALLS {
            for body in [
                format!("return '' .. {call}"),
                format!("return {call}"),
                format!("local value = {call}; return value"),
            ] {
                vm.insert_global("observed", 0).unwrap();
                let source = format!("observed = 1; {body}");
                let error = vm.exec(&source).unwrap_err();
                assert!(
                    matches!(error.kind(), ErrorKind::Vm(_)),
                    "{profile:?}: {source}: {error}"
                );
                assert_eq!(vm.eval::<i32>("return observed").unwrap(), 1, "{source}");
            }
        }
    }
}

#[test]
fn invalid_builtin_calls_in_dead_and_uncalled_code_do_not_reject_the_chunk() {
    for profile in profiles() {
        let mut vm = vm(profile);
        for call in INVALID_CALLS {
            for body in [
                format!("if false then local value = '' .. {call} end"),
                format!("local function unused() return '' .. {call} end"),
            ] {
                let source = format!("observed = 1; {body}; return observed");
                assert_eq!(vm.eval::<i32>(&source).unwrap(), 1, "{profile:?}: {source}");
            }
        }
    }
}

#[test]
fn protected_builtin_failures_remain_catchable() {
    for profile in profiles() {
        let mut vm = vm(profile);
        for call in INVALID_CALLS {
            let source = format!(
                "local ok, message = pcall(function() return '' .. {call} end); \
                 return not ok and type(message) == 'string'"
            );
            assert!(vm.eval::<bool>(&source).unwrap(), "{profile:?}: {source}");
        }
    }
}

// Each maximum ignored here was exercised through the pinned Luau worker.
// table.insert is deliberately absent: Luau rejects four arguments.
const EXTRA_ARGUMENT_CALLS: &[(&str, &str, &str, &str)] = &[
    ("type", "1, 99", "", "number"),
    ("typeof", "1, 99", "", "number"),
    ("tonumber", "'1', 10, 99", "", "1"),
    ("next", "{a = 1}, nil, 99", "", "a"),
    ("table.concat", "{1}, '', 1, 1, 99", "", "1"),
    ("table.getn", "{4}, 99", "", "1"),
    ("table.maxn", "{4}, 99", "", "1"),
    ("table.create", "1, 4, 99", "[1]", "4"),
    ("table.find", "{4}, 4, 1, 99", "", "1"),
    ("table.remove", "{4}, 1, 99", "", "4"),
    ("string.sub", "'abc', 1, 2, 99", "", "ab"),
    ("string.len", "'abc', 99", "", "3"),
    ("string.byte", "'abc', 1, 1, 99", "", "97"),
    ("string.upper", "'abc', 99", "", "ABC"),
    ("string.lower", "'ABC', 99", "", "abc"),
    ("string.reverse", "'abc', 99", "", "cba"),
    ("string.find", "'abc', 'a', 1, true, 99", "", "1"),
    ("string.split", "'a,b', ',', 99", "[1]", "a"),
];

#[test]
fn ignored_extra_arguments_agree_across_call_shapes() {
    for profile in profiles() {
        let mut vm = vm(profile);
        for &(callee, args, suffix, expected) in EXTRA_ARGUMENT_CALLS {
            let expression = format!("{callee}({args}){suffix}");
            let mut sources = vec![
                format!("return '' .. {expression}"),
                format!("local value = {expression}; return '' .. value"),
                format!("local function tail() return {expression} end; return '' .. tail()"),
                format!("local target = {callee}; return '' .. target({args}){suffix}"),
                format!(
                    "local ok, value = pcall(function() return {expression} end); \
                     assert(ok, value); return '' .. value"
                ),
            ];
            if matches!(
                callee,
                "type"
                    | "typeof"
                    | "next"
                    | "table.concat"
                    | "string.sub"
                    | "string.upper"
                    | "string.lower"
                    | "string.reverse"
                    | "string.split"
            ) {
                sources.push(format!("return {expression}"));
            }
            for source in sources {
                assert_eq!(
                    vm.eval::<String>(&source).unwrap(),
                    expected,
                    "{profile:?}: {source}"
                );
            }
        }
    }
}

#[test]
fn ignored_extra_arguments_still_run_once_in_order_and_can_throw() {
    for profile in profiles() {
        let mut vm = vm(profile);
        for &(callee, args, suffix, expected) in EXTRA_ARGUMENT_CALLS {
            let source = format!(
                "observed = 0; local function extra() observed = observed + 1; return 42 end; \
                 return '' .. {callee}({args}, extra()){suffix} .. ':' .. observed"
            );
            assert_eq!(
                vm.eval::<String>(&source).unwrap(),
                format!("{expected}:1"),
                "{profile:?}: {source}"
            );
        }
        assert_eq!(
            vm.eval::<String>(
                "observed = 0; local function extra(n) observed = observed * 10 + n; return n end; \
                 return type(extra(1), extra(2), extra(3)) .. ':' .. observed"
            ).unwrap(),
            "number:123"
        );
        let error = vm.exec(
            "observed = 0; local function extra() observed = observed + 1; error('extra', 0) end; \
             return type(1, extra())"
        ).unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Vm(_)));
        assert_eq!(vm.eval::<i32>("return observed").unwrap(), 1);
        let error = vm
            .exec("observed = 1; return table.insert({}, 1, 4, 99)")
            .unwrap_err();
        assert!(matches!(error.kind(), ErrorKind::Vm(_)));
        assert_eq!(vm.eval::<i32>("return observed").unwrap(), 1);
    }
}

#[test]
fn iterator_extra_arguments_preserve_results_and_side_effects() {
    for profile in profiles() {
        let mut vm = vm(profile);
        for iterator in ["ipairs", "pairs"] {
            let source = format!(
                "observed = 0; local function extra() observed = observed + 1; return 99 end; \
                 local result = ''; for k, v in {iterator}({{4}}, extra()) do \
                 result = result .. k .. ':' .. v end; return result .. ':' .. observed"
            );
            assert_eq!(
                vm.eval::<String>(&source).unwrap(),
                "1:4:1",
                "{profile:?}: {source}"
            );
        }
    }
}

#[test]
fn builtin_output_work_checks_fuel_before_allocation_across_call_shapes() {
    for profile in profiles() {
        for source in [
            "return #string.rep('x', 600000)",
            "local repeat_string = string.rep; return #repeat_string('x', 600000)",
            "return #(string.rep)('x', 600000)",
        ] {
            for (budget, expect_fuel) in [(2_048, true), (1_000_000, false)] {
                let mut vm = RustiaBuilder::standard()
                    .profile(profile)
                    .fuel(budget)
                    .memory_limit_bytes(512 * 1024)
                    .build()
                    .unwrap();
                let error = vm.load(source).exec().unwrap_err();
                let kind = error.as_vm_error().unwrap().kind();
                if expect_fuel {
                    assert!(
                        matches!(kind, rustia::VmErrorKind::FuelExhausted { .. }),
                        "{profile:?}: {source}: {error}"
                    );
                } else {
                    assert!(
                        matches!(kind, rustia::VmErrorKind::OutOfMemory),
                        "{profile:?}: {source}: {error}"
                    );
                }
                vm.reset_decision().unwrap();
                assert_eq!(vm.load("return 42").eval::<i64>().unwrap(), 42);
            }
        }
    }
}
