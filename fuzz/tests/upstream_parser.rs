//! Run the same source examples and Rustia assertions against the pinned compiler.
#![cfg(feature = "oracle")]

use rustia_parser::{Chunk, SourceError};
use std::sync::Once;

#[path = "../../crates/rustia-parser/tests/luau0709/mod.rs"]
mod luau0709;

fn parse_source(source: &str) -> Result<Chunk, SourceError> {
    parse_source_bytes(source.as_bytes())
}

fn reference_compiles(source: &[u8]) -> mlua::Result<Vec<u8>> {
    // This test binary has its own process. Select the same single parser extension
    // Rustia supports before any test calls the upstream compiler.
    static CONFIGURE: Once = Once::new();
    CONFIGURE
        .call_once(|| mlua::Lua::set_fflag("LuauExplicitTypeInstantiationSyntax", true).unwrap());
    mlua::Compiler::new().compile(source)
}

fn parse_source_bytes(source: &[u8]) -> Result<Chunk, SourceError> {
    let reference = reference_compiles(source);
    let actual = rustia_parser::parse_source_bytes(source);
    assert_eq!(
        actual.is_ok(),
        reference.is_ok(),
        "upstream: {:?}, Rustia: {:?}, source: {:?}",
        reference.err(),
        actual.as_ref().err(),
        String::from_utf8_lossy(source)
    );
    actual
}

#[test]
fn previously_covered_syntax_agrees_with_pinned_luau() {
    let declarations = include_str!(
        "../../conformance/fixtures/upstream-luau-0.709/type_function_declarations.luau"
    );
    for source in [
        "print(5)",
        "local x: number = 3",
        "for index: number = 1, 2 do end",
        "return 4 :: number",
        "return 1 -- final line",
        "local n = 1; n += 2",
        "local function f(): (number, T...)? end",
        declarations,
    ] {
        parse_source(source).unwrap();
    }
    for source in ["return f\n(1)", "local x; type function f() return x end"] {
        assert!(parse_source(source).is_err(), "{source}");
    }
}

#[test]
fn compiler_scope_checks_agree_with_pinned_luau() {
    for (source, expected) in [
        ("local function f() return ... end", false),
        ("local function f(...) return ... end", true),
        ("break", false),
        ("continue", false),
        ("while true do local function f() break end end", false),
        ("while true do break end", true),
    ] {
        let chunk = rustia_parser::parse_source(source).unwrap();
        assert_eq!(
            rustia_compiler::compile(&chunk).is_ok(),
            expected,
            "{source}"
        );
        assert_eq!(
            reference_compiles(source.as_bytes()).is_ok(),
            expected,
            "{source}"
        );
    }
}
