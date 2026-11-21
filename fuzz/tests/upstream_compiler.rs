//! Compare guest-visible results of the pinned compiler cases, including return arity.
#![cfg(feature = "oracle")]

use rustia::{RustiaBuilder, ServiceClock, ServiceEntropy, ValueType};

#[path = "compiler0709/basics.rs"]
mod basics;
#[path = "compiler0709/calls.rs"]
mod calls;
#[path = "compiler0709/optimizations.rs"]
mod optimizations;

#[derive(Debug, PartialEq)]
enum Scalar {
    Nil,
    Boolean(bool),
    Number(f64),
    NegativeZero,
    Nan,
    Bytes(Vec<u8>),
}

fn number(n: f64) -> Scalar {
    if n.is_nan() {
        Scalar::Nan
    } else if n == 0.0 && n.is_sign_negative() {
        Scalar::NegativeZero
    } else {
        Scalar::Number(n)
    }
}

#[track_caller]
fn rustia_results(source: &str) -> Vec<Scalar> {
    // loadstring uses the public source compiler and returns a callable chunk.
    // Calling it as a chunk preserves module scope, hot comments and all results.
    let mut rustia = RustiaBuilder::live()
        .profile(rustia::Profile::LuauCompat)
        .wall_clock(ServiceClock::fixed(0.0))
        .monotonic_clock(ServiceClock::fixed(0.0))
        .entropy(ServiceEntropy::fixed(0))
        .build()
        .unwrap();
    rustia
        .insert_global("compiler_case_source", source)
        .unwrap();
    let loader = "local source = compiler_case_source\ncompiler_case_source = nil\nreturn assert(loadstring(source, 'compiler-case'))";
    let function = rustia
        .load(loader)
        .eval_live()
        .unwrap_or_else(|e| panic!("Rustia compile: {e}\n{source}"))
        .as_function(&rustia)
        .unwrap()
        .expect("loadstring returned a function");
    let values = function
        .call_values(&mut rustia, &[])
        .unwrap_or_else(|e| panic!("Rustia run: {e}\n{source}"));
    values
        .into_iter()
        .map(|v| match v.kind(&rustia).unwrap() {
            ValueType::Nil => Scalar::Nil,
            ValueType::Boolean => Scalar::Boolean(v.as_boolean(&rustia).unwrap().unwrap()),
            ValueType::Number => number(v.as_number(&rustia).unwrap().unwrap()),
            ValueType::String => Scalar::Bytes(v.string_bytes(&mut rustia).unwrap().unwrap()),
            kind => panic!("return scalar observations for {kind:?}\n{source}"),
        })
        .collect()
}

#[track_caller]
fn luau_results(source: &str, level: u8) -> Vec<Scalar> {
    let lua = mlua::Lua::new();
    lua.set_compiler(mlua::Compiler::new().set_optimization_level(level));
    let function = lua
        .load(source)
        .set_name("=compiler-case")
        .into_function()
        .unwrap_or_else(|e| panic!("Luau O{level} compile: {e}\n{source}"));
    let values: mlua::MultiValue = function
        .call(())
        .unwrap_or_else(|e| panic!("Luau O{level} run: {e}\n{source}"));
    values
        .into_iter()
        .map(|v| match v {
            mlua::Value::Nil => Scalar::Nil,
            mlua::Value::Boolean(b) => Scalar::Boolean(b),
            mlua::Value::Integer(i) => number(i as f64),
            mlua::Value::Number(n) => number(n),
            mlua::Value::String(s) => Scalar::Bytes(s.as_bytes().to_vec()),
            other => panic!(
                "return scalar observations for {}\n{source}",
                other.type_name()
            ),
        })
        .collect()
}

#[track_caller]
fn agrees(source: &str) {
    let actual = rustia_results(source);
    assert!(!actual.is_empty(), "missing guest observations\n{source}");
    for level in [1, 2] {
        assert_eq!(
            actual,
            luau_results(source, level),
            "Luau O{level}\n{source}"
        );
    }
}

#[track_caller]
fn rejects(source: &str) {
    for level in [1, 2] {
        assert!(
            mlua::Compiler::new()
                .set_optimization_level(level)
                .compile(source)
                .is_err(),
            "Luau O{level} accepted\n{source}"
        );
    }
    assert!(
        rustia::Rustia::compile_module("compiler-case", source).is_err(),
        "Rustia accepted\n{source}"
    );
}

#[test]
fn return_values_preserve_arity_types_and_bytes() {
    let source = r#"return nil, false, 0, -1/math.huge, 0/0, "a\0\255", nil"#;
    let expected = [
        Scalar::Nil,
        Scalar::Boolean(false),
        Scalar::Number(0.0),
        Scalar::NegativeZero,
        Scalar::Nan,
        Scalar::Bytes(vec![b'a', 0, 255]),
        Scalar::Nil,
    ];
    assert_eq!(rustia_results(source), expected);
    for level in [1, 2] {
        assert_eq!(luau_results(source, level), expected);
    }
}

#[test]
fn equivalent_closure_identity_depends_on_the_engine() {
    // Luau documents closure caching as optional. A saved reference stays
    // identical to itself in both engines; repeated evaluations can differ.
    let source = r#"
local root = {}
local function factory() return function() return root end end
local function independent(x) return function() x += 1 return x end end
local a, b = factory(), factory()
local c, d = independent(0), independent(0)
return a == a, a() == b(), a == b, ({[a]=true})[b] == true, c == d, c() == d()
"#;
    let expected = |shared| [true, true, shared, shared, false, true].map(Scalar::Boolean);
    assert_eq!(rustia_results(source), expected(false));
    for level in [1, 2] {
        assert_eq!(luau_results(source, level), expected(true), "Luau O{level}");
    }
}

#[test]
fn skipped_local_error_points_to_the_condition_and_names_the_jump_line() {
    let source =
        "repeat\n    if take then\n        continue\n    end\n    local value = take\nuntil value";
    let message = "Local value used in the repeat..until condition is undefined because continue statement on line 3 jumps over it";
    let error = rustia::Rustia::compile_module("compiler-case", source)
        .err()
        .expect("Rustia accepted the skipped local");
    let rustia::ErrorKind::Compile(detail) = error.kind() else {
        panic!("expected a compile diagnostic: {error}");
    };
    assert_eq!(detail.message, message);
    let span = detail.span.as_ref().expect("condition span");
    assert_eq!(&source[span.start..span.end], "value");
    assert_eq!(error.source_location().unwrap().line, 6);
    for level in [1, 2] {
        let error = mlua::Compiler::new()
            .set_optimization_level(level)
            .compile(source)
            .unwrap_err();
        let mlua::Error::SyntaxError {
            message: diagnostic,
            ..
        } = error
        else {
            panic!("expected a Luau compile diagnostic: {error}");
        };
        assert_eq!(diagnostic, format!("6: {message}"));
    }
}
