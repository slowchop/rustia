//! Source-level ports of the interpolation cases in the MIT-licensed Luau 0.709 lexer tests.
//! <https://github.com/luau-lang/luau/blob/b968ef742741bb2b703afc3b3c53f06608c87481/tests/Lexer.test.cpp#L141-L247>
//! Rustia exposes decoded parts and expression spans through the parser.

use crate::parse_source;
use rustia_lexer::Span;
use rustia_parser::{Expr, InterpolatedStringPart, SourceError, Stmt};

fn check_interpolation(
    literal: &str,
    expected_parts: &[InterpolatedStringPart],
    expected_strings: &[(&str, Span)],
    expected_sections: &[&str],
) {
    // In expression position, the original standalone lexer input is unchanged.
    let source = format!("return {literal}");
    let chunk = parse_source(&source).unwrap();
    let [Stmt::Return {
        value:
            Some(Expr::InterpolatedString {
                parts,
                expressions,
                span,
            }),
        ..
    }] = chunk.statements.as_slice()
    else {
        panic!("expected one interpolated return value");
    };
    assert_eq!(span.start, "return ".len());
    assert_eq!(&source[span.start..span.end], literal);
    assert_eq!(parts, expected_parts);
    assert_eq!(expressions.len(), expected_strings.len());

    // Each quoted expression ends at the '}' that opens the next text
    // section. Check the original boundaries as well as the decoded text.
    let mut sections = Vec::new();
    let mut previous_end = span.start;
    for (expr, &(expected_value, expected_span)) in expressions.iter().zip(expected_strings) {
        let Expr::Paren { expr, .. } = expr else {
            panic!("expected the embedded expression wrapper");
        };
        let Expr::String(value, quoted) = expr.as_ref() else {
            panic!("expected a quoted expression")
        };
        assert_eq!(value, expected_value.as_bytes());
        assert_eq!(
            Span::new(quoted.start - span.start, quoted.end - span.start),
            expected_span
        );
        sections.push(&source[previous_end..quoted.start]);
        previous_end = quoted.end;
    }
    sections.push(&source[previous_end..span.end]);
    assert_eq!(sections, expected_sections);
}

#[test]
fn string_interpolation_basic() {
    use InterpolatedStringPart::{Expr, Text};
    check_interpolation(
        r#"`foo {"bar"}`"#,
        &[Text(b"foo ".to_vec()), Expr(0)],
        &[("bar", Span::new(6, 11))],
        &["`foo {", "}`"],
    );
}

#[test]
fn string_interpolation_full() {
    use InterpolatedStringPart::{Expr, Text};
    check_interpolation(
        r#"`foo {"bar"} {"baz"} end`"#,
        &[
            Text(b"foo ".to_vec()),
            Expr(0),
            Text(b" ".to_vec()),
            Expr(1),
            Text(b" end".to_vec()),
        ],
        &[("bar", Span::new(6, 11)), ("baz", Span::new(14, 19))],
        &["`foo {", "} {", "} end`"],
    );
}

fn first_error(source: &str, expected_span: Span, expected_message: &str) {
    let (span, message) = match parse_source(source).unwrap_err() {
        SourceError::Lex(e) => (e.span, e.message),
        SourceError::Parse(e) => (e.span, e.message),
    };
    assert_eq!(
        (span, message.as_str()),
        (expected_span, expected_message),
        "{source}"
    );
}

const DOUBLE_BRACE: &str =
    r"Double braces are not permitted within interpolated strings; did you mean '\{'?";

#[test]
fn string_interpolation_double_brace() {
    first_error("return `foo{{bad}}bar`", Span::new(7, 11), DOUBLE_BRACE);
}

#[test]
fn string_interpolation_double_but_unmatched_brace() {
    first_error("return `{{oops}`, 1", Span::new(7, 8), DOUBLE_BRACE);
}

#[test]
fn string_interpolation_unmatched_brace() {
    let source = r#"return {
        `hello {"world"}
    } -- this might be incorrectly parsed as a string"#;
    first_error(
        source,
        Span::new(32, 33),
        "Malformed interpolated string; did you forget to add a '`'?",
    );
}

#[test]
fn string_interpolation_with_unicode_escape() {
    check_interpolation(
        r"`\u{1F41B}`",
        &[InterpolatedStringPart::Text(
            "\u{1f41b}".as_bytes().to_vec(),
        )],
        &[],
        &[r"`\u{1F41B}`"],
    );
}
