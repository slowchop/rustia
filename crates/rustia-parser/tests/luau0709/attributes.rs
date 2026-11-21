//! Sources adapted from tests/Parser.test.cpp at Luau 0.709 (MIT).
use crate::parse_source;
use rustia_lexer::Span;
use rustia_parser::{AttributeKind, Expr, SourceError, Stmt};

fn check_error(marked: &str, message: &str) {
    let (before, rest) = marked.split_once('«').unwrap();
    let (bad, after) = rest.split_once('»').unwrap();
    let source = format!("{before}{bad}{after}");
    let SourceError::Parse(error) = parse_source(&source).expect_err(&source) else {
        panic!("expected a parser diagnostic for {source}");
    };
    assert_eq!(error.message, message, "{source}");
    assert_eq!(
        error.span,
        Span::new(before.len(), before.len() + bad.len()),
        "{source}"
    );
}

#[test]
fn attributes_on_global_and_local_functions() {
    let source =
        "@checked\nfunction hello(x, y) return x+y end\n    @checked\nlocal function bye() end";
    let chunk = parse_source(source).unwrap();
    let [Stmt::Function {
        attributes: global, ..
    }, Stmt::LocalFunction {
        attributes: local, ..
    }] = &*chunk.statements
    else {
        panic!("expected both declarations")
    };
    for (attrs, position) in [
        (global, source.find('@').unwrap()),
        (local, source.rfind('@').unwrap()),
    ] {
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].kind, AttributeKind::Checked);
        assert_eq!(
            attrs[0].span,
            Span::new(position, position + "@checked".len())
        );
        assert!(attrs[0].arguments.is_empty());
    }
}

#[test]
fn deprecated_attribute_on_function() {
    let source = r#"@[deprecated{ use = "greetng", reason = "Using <hello> is too causal"}]
function hello(x, y) return x + y end"#;
    let chunk = parse_source(source).unwrap();
    let Stmt::Function { attributes, .. } = &chunk.statements[0] else {
        panic!("expected function")
    };
    assert_eq!(attributes.len(), 1);
    let attr = &attributes[0];
    assert_eq!(attr.kind, AttributeKind::Deprecated);
    assert_eq!(attr.span, Span::new(2, source.find(']').unwrap()));
    assert!(matches!(attr.arguments.as_slice(), [Expr::Table {fields, ..}] if fields.len() == 2));
}

#[test]
fn attributes_on_function_expressions() {
    let source = "invoker(@checked function(x) return x+2 end)\nlocal f = @checked function(x) return x+2 end";
    let chunk = parse_source(source).unwrap();
    let [Stmt::Expr {
        expr: Expr::Call { args, .. },
        ..
    }, Stmt::Local { init, .. }] = &*chunk.statements
    else {
        panic!("expected call and binding")
    };
    for (expression, position) in [
        (&args[0], source.find('@').unwrap()),
        (&init[0], source.rfind('@').unwrap()),
    ] {
        let Expr::Function {
            attributes, span, ..
        } = expression
        else {
            panic!("expected function expression")
        };
        assert_eq!(span.start, position);
        assert_eq!(attributes.len(), 1);
        assert_eq!(attributes[0].kind, AttributeKind::Checked);
        assert_eq!(
            attributes[0].span,
            Span::new(position, position + "@checked".len())
        );
    }
}

#[test]
fn attribute_arguments_must_be_literals() {
    check_error(
        "@[deprecated{« reason = reasonString }»]\nfunction hello() end",
        "Only literals can be passed as arguments for attributes",
    );
    check_error(
        "@[checked(«(1))»] function f() end",
        "Only literals can be passed as arguments for attributes",
    );
    parse_source("@[checked({1,{true}, type='value'}), native] function f() end").unwrap();
}

#[test]
fn deprecated_attribute_checks_argument_keys_and_values() {
    for (source, message) in [
        (r#"@[«deprecated»({}, "Very deprecated")] function hello() end"#, "@deprecated can be parametrized only by 1 argument"),
        (r#"@[deprecated «"Very deprecated"»] function hello() end"#, "Unknown argument type for @deprecated"),
        (r#"@[deprecated{ «foo» = "bar" }] function hello() end"#, "Unknown argument 'foo' for @deprecated. Only string constants for 'use' and 'reason' are allowed"),
        ("@[deprecated{ use = «5» }] function hello() end", "Only constant string allowed as value for 'use'"),
        ("@[deprecated{ «'old'» }] function hello() end", "Only constants keys 'use' and 'reason' are allowed for @deprecated attribute"),
    ] { check_error(source, message); }
}

#[test]
fn attribute_list_and_name_diagnostics() {
    check_error("«@[]» function f() end", "Attribute list cannot be empty");
    check_error(
        "@[native,«]» function f() end",
        "Expected identifier when parsing attribute name, got ']'",
    );
    check_error("«@»\nfunction hello() end", "Attribute name is missing");
}

#[test]
fn attributes_require_function_statements() {
    for (source, token) in [
        ("@checked\n«if» a < 0 then a=0 end", "if"),
        ("local i=1\n@checked\n«while» a[i] do i+=1 end", "while"),
        ("@checked\n«do» local a=2 end", "do"),
        ("@checked\n«for» i=1,10 do print(i) end", "for"),
        ("@checked\n«repeat» x=2 until x==2", "repeat"),
        ("while a do if b then @checked «break» end end", "break"),
        ("function f() @checked «return» 'a' end", "return"),
    ] {
        check_error(source, &format!("Expected 'function', 'local function', 'declare function' or a function type declaration after attribute, but got '{token}' instead"));
    }
    check_error(
        "@checked\nlocal «x» = 10",
        "Expected 'function' after local declaration with attribute, but got 'x' instead",
    );
}

#[test]
fn attributes_require_function_expressions() {
    check_error(
        "invoker(function(x) return x end, «@checked» 1)",
        "Expected 'function' declaration after attribute, but got '1' instead",
    );
}

#[test]
fn duplicate_and_unknown_attributes() {
    check_error(
        "@checked\n    «@checked»\nfunction hello() end",
        "Cannot duplicate attribute '@checked'",
    );
    check_error(
        "@checked\n    «@cool_attribute»\nfunction hello() end",
        "Invalid attribute '@cool_attribute'",
    );
    check_error(
        "@native @[«native»] function f() end",
        "Cannot duplicate attribute '@native'",
    );
}

#[test]
fn type_attributes_require_definition_file_mode() {
    check_error(
        "type T = «@checked» (...number) -> number",
        "attributes are not allowed in declaration context",
    );
}

#[test]
fn missing_end_points_to_the_function_keyword_after_an_attribute() {
    for source in [
        "@checked\nfunction f()\nreturn 1",
        "@native\nlocal function f()\nreturn 1",
        "local f = @native\nfunction()\nreturn 1",
    ] {
        let SourceError::Parse(error) = parse_source(source).unwrap_err() else {
            panic!("expected parse error")
        };
        assert_eq!(
            error.message,
            "Expected 'end' (to close 'function' at line 2), got <eof>"
        );
        assert_eq!(
            error.unclosed_block.unwrap().start,
            source.find("function").unwrap()
        );
    }
}
