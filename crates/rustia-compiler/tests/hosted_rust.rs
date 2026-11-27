use rustia_compiler::hosted_rust::{emit, Options};
use std::collections::BTreeSet;

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
    for source in [
        &b"return app.open('other')"[..],
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
