//! Capture names describe lexical slots without changing executable captures.
use rustia_compiler::{compile_with_options, BuiltinEnvironment, CompileOptions};
use rustia_parser::parse_source;
use rustia_vm::{bytecode::UpvalueDescriptor, Function, Profile, Program, Vm};

fn compile(source: &str) -> Program {
    compile_with_options(
        &parse_source(source).unwrap(),
        CompileOptions::default().with_source("captures.luau", source),
    )
    .unwrap()
}

fn named<'a>(program: &'a Program, name: &str) -> &'a Function {
    program
        .functions
        .iter()
        .find(|function| function.debug_name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing function {name}"))
}

fn assert_result(program: &Program, expected: f64) {
    for profile in [
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::LuauCompat,
        Profile::Puc51Compat,
    ] {
        let mut vm = Vm::new(profile);
        assert_eq!(vm.run(program).unwrap().as_number(), Some(expected));
    }
    for function in std::iter::once(&program.main).chain(&program.functions) {
        assert_eq!(function.upvalues.len(), function.upvalue_names.len());
    }
}

#[test]
fn names_cover_mutable_cells_and_direct_immutable_captures() {
    let program = compile(
        "local function make(start, step)\n local count = start\n local function advance() count += step return count end\n return advance\nend\nlocal f = make(1, 2) f() return f()",
    );
    let advance = named(&program, "advance");
    assert_eq!(advance.upvalue_names, ["count", "step"]);
    assert!(matches!(
        advance.upvalues.as_slice(),
        [
            UpvalueDescriptor::ParentLocal(_),
            UpvalueDescriptor::ParentLocalValue(_)
        ]
    ));
    assert_eq!(program.source_name.as_deref(), Some("captures.luau"));
    assert_eq!(advance.debug_line_defined, Some(3));
    assert_result(&program, 5.0);
}

#[test]
fn transitive_captures_keep_names_even_without_parent_reads() {
    let program = compile(
        "local count, step = 1, 2\nlocal function outer()\n local function advance() count += step return count end\n return advance\nend\nlocal f = outer() f() return f()",
    );
    let outer = named(&program, "outer");
    let advance = named(&program, "advance");
    assert_eq!(outer.upvalue_names, ["count", "step"]);
    assert_eq!(advance.upvalue_names, ["count", "step"]);
    assert!(matches!(
        outer.upvalues.as_slice(),
        [
            UpvalueDescriptor::ParentLocal(_),
            UpvalueDescriptor::ParentLocalValue(_)
        ]
    ));
    assert!(matches!(
        advance.upvalues.as_slice(),
        [
            UpvalueDescriptor::ParentUpvalue(0),
            UpvalueDescriptor::ParentUpvalueValue(1)
        ]
    ));
    assert_result(&program, 5.0);
}

#[test]
fn shadowed_names_remain_attached_to_distinct_lexical_slots() {
    let program = compile(
        "local value = 11\nlocal function first() return value end\nlocal second\ndo\n local value = 29\n local function second_value() return value end second = second_value\nend\nreturn first() * 100 + second()",
    );
    let first = named(&program, "first");
    let second = named(&program, "second_value");
    assert_eq!(first.upvalue_names, ["value"]);
    assert_eq!(second.upvalue_names, ["value"]);
    assert_ne!(first.upvalues, second.upvalues);
    assert_result(&program, 1129.0);
}

#[test]
fn capture_order_follows_descriptor_order_after_source_reordering() {
    for (expression, names) in [
        ("left + right", ["left", "right"]),
        ("right + left", ["right", "left"]),
    ] {
        let source = format!(
            "local left, right = 4, 9 local function sum() return {expression} end return sum()"
        );
        let program = compile(&source);
        let sum = named(&program, "sum");
        assert_eq!(sum.upvalue_names, names);
        for (index, name) in names.into_iter().enumerate() {
            assert_eq!(sum.upvalue_name(index), Some(name));
        }
        assert_result(&program, 13.0);
    }
}

#[test]
fn detached_chunk_compilation_restores_parent_capture_names() {
    let source = "local x = 4 local function before() return x end local loaded = (loadstring('local y = 8 return function() return y end')) local function after() return x end return before() + after() + loaded()()";
    let program = compile_with_options(
        &parse_source(source).unwrap(),
        CompileOptions {
            builtins: BuiltinEnvironment::LuauConformance,
            ..CompileOptions::default()
        }
        .with_source("detached.luau", source),
    )
    .unwrap();
    assert_eq!(named(&program, "before").upvalue_names, ["x"]);
    assert_eq!(named(&program, "after").upvalue_names, ["x"]);
    assert!(program
        .functions
        .iter()
        .any(|function| function.upvalue_names == ["y"]));
    assert_result(&program, 16.0);
}

#[test]
fn stripping_capture_names_keeps_execution_and_instruction_accounting() {
    let program = compile(
        "local count = 2 local function make(step) return function() count += step return count end end local a, b = make(3), make(7) a() b() return a()",
    );
    let mut stripped = program.clone();
    stripped.main.upvalue_names.clear();
    for function in &mut stripped.functions {
        function.upvalue_names.clear();
    }
    for profile in [
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::LuauCompat,
        Profile::Puc51Compat,
    ] {
        let mut named_vm = Vm::new(profile);
        let mut stripped_vm = Vm::new(profile);
        assert_eq!(named_vm.run(&program).unwrap().as_number(), Some(15.0));
        assert_eq!(stripped_vm.run(&stripped).unwrap().as_number(), Some(15.0));
        assert_eq!(
            named_vm.executed_instructions(),
            stripped_vm.executed_instructions()
        );
    }
}

#[test]
fn branch_restoration_reuses_the_same_lexical_capture() {
    let program = compile("local state={} local function update(run) if run then state.helper=function() return 3 end end return state end update(true) return update(false).helper()");
    assert_eq!(named(&program, "update").upvalue_names, ["state"]);
    assert_result(&program, 3.0);
}

#[test]
fn transitive_branch_capture_and_later_reads_share_one_descriptor() {
    let program = compile("local count=1 local function outer(run) if run then local function bump() count+=2 return count end bump() end local function peek() return count end return peek end local a=outer(true) local b=outer(false) return a()*10+b()");
    assert_eq!(named(&program, "outer").upvalue_names, ["count"]);
    assert_eq!(named(&program, "bump").upvalue_names, ["count"]);
    assert_eq!(named(&program, "peek").upvalue_names, ["count"]);
    assert_result(&program, 33.0);
}

#[test]
fn repeated_scoped_reads_do_not_exhaust_the_distinct_upvalue_limit() {
    let mut source = String::from("local count=0 local function bump(run) ");
    for _ in 0..300 {
        source.push_str("if run then count+=1 end ");
    }
    source.push_str("return count end return bump(true)");
    let program = compile(&source);
    assert_eq!(named(&program, "bump").upvalue_names, ["count"]);
    assert_result(&program, 300.0);
}
