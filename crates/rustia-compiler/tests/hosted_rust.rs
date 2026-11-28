use rustia_compiler::hosted_rust::{emit, Options};
use std::collections::BTreeSet;

#[test]
fn hosted_numeric_loops_and_interpolation_use_adapter_values() {
    use rustia_compiler::hosted_rust::emit_module;

    let source = b"return function(v)
        for i = v, v, v do return `prefix:{v}:suffix` end
    end";
    let hosted = emit(
        "strings.luau",
        source,
        "entry",
        Options {
            captures: Some(&BTreeSet::new()),
            supports_builtin: |_| false,
        },
    )
    .unwrap();
    assert_eq!(hosted.matches("host.number(&").count(), 3);
    let evaluated_step = hosted.find("let step_value = ").unwrap();
    let first_coercion = hosted.find("host.number(&start_value)?").unwrap();
    assert!(evaluated_step < first_coercion);
    assert_eq!(
        hosted.matches("host.interpolation_value(value)?").count(),
        1
    );
    assert_eq!(
        hosted.matches("host.binary(\"..\", text, part)?").count(),
        2
    );

    let standalone = emit_module("strings.luau", source, "entry", "adapter", None).unwrap();
    assert_eq!(standalone.matches(".number()?").count(), 3);
    assert!(standalone.contains("text.extend(adapter::string_bytes"));
}

#[test]
fn callback_captures_and_builtin_order_follow_lexical_scope() {
    let captures = BTreeSet::from(["count"]);
    let rust = emit(
        "step.luau",
        b"return function()
    count += tonumber('2')
    local tonumber = tonumber
    count += tonumber(3)
end",
        "step",
        Options {
            captures: Some(&captures),
            supports_builtin: |name| name == "tonumber",
        },
    )
    .unwrap();
    assert!(rust.contains("host.capture(\"count\")?"));
    assert!(rust.contains("host.set_capture(\"count\", value)?"));
    assert!(rust.contains("let arguments = vec![Value::String(String::from(\"2\"))]; let function = host.global(\"tonumber\")?;"));
    assert!(
        rust.lines().any(|line| {
            line.contains("let function = l") && line.contains(".read_local(); let arguments")
        }),
        "{rust}"
    );
}

#[test]
fn callback_rejection_reports_the_original_source_location() {
    let captures = BTreeSet::new();
    let error = emit(
        "nested/step.luau",
        b"return function()\n    return absent()\nend",
        "step",
        Options {
            captures: Some(&captures),
            supports_builtin: |_| false,
        },
    )
    .unwrap_err();
    assert_eq!(error.file, "nested/step.luau");
    assert_eq!((error.line, error.column), (2, 12));
    assert!(error.message.contains("unsupported live global"));
}

#[test]
fn standalone_policy_checks_host_calls_imports_globals_and_builtin_fields() {
    use rustia_compiler::hosted_rust::{emit_module, Builtin, ClosedHost};
    let host = ClosedHost {
        namespace: "app",
        builtin: |root, member| match (root, member) {
            ("math", None) => Builtin::Namespace,
            ("math", Some("abs")) => Builtin::Value,
            _ => Builtin::Missing,
        },
        check_call: &|name, literal| {
            if name == "open" && literal == Some(b"input") {
                Ok(())
            } else {
                Err("host declaration denied".into())
            }
        },
    };
    let emit = |source: &[u8]| emit_module("mod.luau", source, "entry", "adapter", Some(&host));
    let rust =
        emit(b"local dep = require('./dep')\nreturn app.open('input'), math.abs(-1)").unwrap();
    assert!(rust.contains("adapter::Host"));
    assert!(rust.contains("host.require(\"mod.luau\", \"./dep\")"));
    let typed =
        emit(b"local dep = require<<any>>('./dep')\nreturn app.open<<any>>('input'), math.abs(-1)")
            .unwrap();
    assert_eq!(typed, rust);
    let shadowed =
        emit(b"local require = function(x) return x end\nreturn require<<any>>('local')").unwrap();
    assert!(!shadowed.contains("host.require("));
    let local_namespace =
        emit(b"local app = { open = function(x) return x end }\nreturn app.open<<any>>('local')")
            .unwrap();
    assert!(!local_namespace.contains("host.global(\"app\")"));

    for source in [
        &b"return app.open('other')"[..],
        b"return app.open<<any>>('other')",
        b"return require<<any>>(load())",
        b"return require(load())",
        b"return math['unknown']",
        b"return math",
        b"other = 4",
        b"return app",
    ] {
        assert!(emit(source).is_err());
    }
    let open = emit_module(
        "mod.luau",
        b"other = 4\nreturn require(other)",
        "entry",
        "adapter",
        None,
    );
    assert!(open.is_ok());
    assert!(emit_module("mod.luau", b"return 1", "entry", "bad::adapter", None).is_err());
}

#[test]
fn hosted_loop_bodies_start_with_a_checkpoint() {
    use rustia_compiler::hosted_rust::emit_module;

    let source = b"return function()
        while true do continue end
        repeat until true
        for i = 1, 2 do end
        for key in {} do end
    end";
    let hosted = emit(
        "loops.luau",
        source,
        "entry",
        Options {
            captures: Some(&BTreeSet::new()),
            supports_builtin: |_| false,
        },
    )
    .unwrap();
    let checkpoint = "host.loop_checkpoint()?;";
    assert_eq!(hosted.matches(checkpoint).count(), 4);
    assert!(hosted.contains("host.loop_checkpoint()?;\ncontinue;"));
    assert!(hosted.contains("loop {\nhost.loop_checkpoint()?;\nif"));
    let standalone = emit_module("loops.luau", source, "entry", "adapter", None).unwrap();
    assert!(!standalone.contains(checkpoint));
}
