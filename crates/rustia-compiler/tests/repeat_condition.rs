//! CompileError locations and captures from Luau 0.709 Compiler.test.cpp::LoopContinueUntil.
use rustia_compiler::{compile_with_options, BuiltinEnvironment, CompileError, CompileOptions};
use rustia_lexer::Span;
use rustia_parser::parse_source;
use rustia_vm::{Profile, Program, Vm, VmSettings};

fn compile(source: &str) -> Result<Program, CompileError> {
    compile_with_options(
        &parse_source(source).unwrap(),
        CompileOptions::default().with_source("repeat.luau", source),
    )
}

#[test]
fn points_to_condition_and_names_earliest_compiled_continue() {
    let condition = "(function() return rr end)() < 0.5";
    let source = format!(
        "repeat\r\n  local r = input\r\n  if false then continue end\r\n  if r then\r\n    continue\r\n  end\r\n  if other then continue end\r\n  local rr = r + 0.3\r\nuntil {condition}\r\n"
    );
    let offset = source.find(condition).unwrap();
    let error = compile(&source).unwrap_err();
    assert_eq!(
        error.span,
        Some(Span::new(offset, offset + condition.len()))
    );
    assert_eq!(error.message, "Local rr used in the repeat..until condition is undefined because continue statement on line 5 jumps over it");
}

#[test]
fn condition_reads_binding_identity_in_source_order() {
    for (source, name) in [
        ("local x = 1 repeat do continue end local x, y = a, b until y + x", "y"),
        ("repeat do continue end local x = nil until true or x", "x"),
        ("repeat do continue end local function x() end until x()", "x"),
        ("repeat do continue end local x = source until (function() return function() return x end end)()", "x"),
        ("repeat do continue end local x = false until (function() x = true; return true end)()", "x"),
        ("repeat do return end do continue end local x = 1 until x", "x"),
        ("repeat do break end do continue end local x = 1 until x", "x"),
        ("repeat do continue end local s = 'a' < 'z' until (function() return s end)()", "s"),
        ("repeat do continue end local s = math.abs(-1) until (function() return s end)()", "s"),
        ("local flag = false repeat if flag then continue end local x = 2 until x flag = true", "x"),
    ] {
        let message = compile(source).unwrap_err().message;
        assert!(message.starts_with(&format!("Local {name} used ")), "{source}: {message}");
    }
    for source in [
        "repeat local x = input do continue end local y = 1 until x",
        "repeat while read do continue end local x = 2 until x",
        "repeat repeat continue until true local x = 2 until x",
        "repeat do continue end local x = item until (function(x) return x end)(true)",
        "repeat do continue end local x = item until (function() local x = true; return x end)()",
        "repeat do continue end local x = item until (function() if false then return x end return true end)()",
        "local flag = false do local flag; flag = true end repeat if flag then continue end local x = 2 until x",
        "repeat if flag and false then continue end local x = 1 until x",
        "repeat if ((flag or true)) then local q=1 else continue end local x=1 until x",
        "repeat do (function() while reading do continue end end)() end local x=1 until x",
        "repeat if 1 == 2 then continue end local x = 1 until x",
    ] {
        compile(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    }
}

#[test]
fn closure_reads_folded_skipped_values_on_the_continue_edge() {
    // The turn limit makes any wrong value observable without relying on fuel.
    let source = r#"
        local turns, out = 0, 0
        local base = 20
        repeat
            turns += 1
            if turns == 1 then continue end
            local answer = base + 2
        until (function() out = answer; return answer == 22 end)() or turns >= 2
        return 100 * turns + out
    "#;
    let program = compile(source).unwrap();
    let value = Vm::new(Profile::ReproducibleStandard)
        .run(&program)
        .unwrap();
    assert_eq!(value.as_number(), Some(122.));
}

#[test]
fn skipped_constant_string_keeps_its_bytes_in_the_closure() {
    let source = r#"
        local turns, out = 0, ""
        local prefix = "a\0"
        repeat
            turns += 1
            if turns == 1 then continue end
            local label = `{prefix}b` .. "c"
        until (function() out = label; return true end)()
        return if out == "a\0bc" then turns else 0
    "#;
    let program = compile(source).unwrap();
    let value = Vm::new(Profile::ReproducibleStandard)
        .run(&program)
        .unwrap();
    assert_eq!(value.as_number(), Some(1.));
}

#[test]
fn invalid_literal_loadstring_remains_a_guest_error() {
    fn dynamic(source: &[u8], _: Option<&[u8]>, _: Profile) -> Result<Program, String> {
        compile(std::str::from_utf8(source).unwrap()).map_err(|e| e.message)
    }
    let source = r#"
        local a = (loadstring("repeat do continue end local x = 1 until x"))
        local b, message = loadstring("repeat do continue end local x = 1 until x")
        local expect = "Local x used in the repeat..until condition is undefined because continue statement on line 1 jumps over it"
        return if a == nil and b == nil and message == expect then 1 else 0
    "#;
    let options = CompileOptions {
        builtins: BuiltinEnvironment::LuauConformance,
        ..Default::default()
    };
    let program = compile_with_options(&parse_source(source).unwrap(), options).unwrap();
    let settings = VmSettings {
        loadstring_compiler: Some(dynamic),
        ..Default::default()
    };
    let value = Vm::with_settings(Profile::ReproducibleStandard, settings)
        .run(&program)
        .unwrap();
    assert_eq!(value.as_number(), Some(1.));
}
