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
