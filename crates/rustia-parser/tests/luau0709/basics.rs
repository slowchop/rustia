// Adapted from Luau 0.709 tests/Parser.test.cpp:132–1328.
// Copyright (c) 2019–2025 Roblox Corporation. MIT license, see
// conformance/upstream/luau-0.709/LICENSE.txt.

use crate::{parse_source, parse_source_bytes};
use rustia_lexer::{lex_for_parser, Span, TokenKind};
use rustia_parser::{BinaryOp, Expr, SourceError, Stmt, TypeExpr};

fn annotation(ty: &str) -> TypeExpr {
    let chunk = parse_source(&format!("local f: {ty}")).unwrap();
    let Stmt::Local { mut bindings, .. } = chunk.statements.into_iter().next().unwrap() else {
        panic!("expected the annotated local in {ty}");
    };
    bindings.remove(0).annotation.unwrap()
}

#[test]
fn annotated_bindings_preserve_identifier_and_statement_spans() {
    let source = "local foo: string = \"Hello Types!\"\nlocal bar: SomeModule.CoolType";
    let chunk = parse_source(source).unwrap();
    let [Stmt::Local {
        bindings: first,
        init,
        ..
    }, Stmt::Local {
        bindings: second,
        span,
        ..
    }] = chunk.statements.as_slice()
    else {
        panic!("expected two local statements");
    };
    assert_eq!(first.len(), 1);
    assert_eq!(&source[first[0].span.start..first[0].span.end], "foo");
    assert!(matches!(&first[0].annotation, Some(TypeExpr::Named { name, .. }) if name == "string"));
    assert!(matches!(init.as_slice(), [Expr::String(value, _)] if value == b"Hello Types!"));
    assert_eq!(&source[second[0].span.start..second[0].span.end], "bar");
    assert!(
        matches!(&second[0].annotation, Some(TypeExpr::Named { name, .. }) if name == "SomeModule.CoolType")
    );
    assert_eq!(span.end, source.len());
}

#[test]
fn functions_keep_their_bodies_after_return_annotations() {
    for (ty, returned, arity) in [
        ("number", "55", 1),
        ("(number) -> nil", "nil", 1),
        ("(number, string)", "1, 'foo'", 2),
        ("() -> nil", "nil", 1),
    ] {
        let source = format!("function f(): {ty} return {returned} end");
        let chunk = parse_source(&source).unwrap();
        let [Stmt::Function { body, span, .. }] = chunk.statements.as_slice() else {
            panic!("{source}");
        };
        let [Stmt::Return {
            value: Some(_),
            extra_values,
            ..
        }] = body.as_slice()
        else {
            panic!("{source}");
        };
        assert_eq!(span.end, source.len());
        assert_eq!(1 + extra_values.len(), arity);
    }
}

#[test]
fn function_annotation_parameter_and_result_shapes() {
    assert!(matches!(annotation("nil"), TypeExpr::Nil(_)));
    for (ty, param_count, result_count) in [
        ("(number, string) -> nil", 2, 1),
        ("(number) -> (number, number)", 1, 2),
        ("() -> number", 0, 1),
        ("(number) -> ()", 1, 0),
    ] {
        let TypeExpr::Function { params, result, .. } = annotation(ty) else {
            panic!("{ty}");
        };
        assert_eq!(params.len(), param_count, "{ty}");
        let result_arity = match *result {
            TypeExpr::TypePack { types, .. } => types.len(),
            _ => 1,
        };
        assert_eq!(result_arity, result_count, "{ty}");
    }
}

#[test]
fn intersections_bind_outside_zero_or_multiple_results_and_inside_one_result() {
    for ty in [
        "(string) -> () & (number) -> ()",
        "(string) -> (string, number) & (number) -> (number, string)",
    ] {
        let TypeExpr::Intersection { types, .. } = annotation(ty) else {
            panic!("{ty}");
        };
        assert!(matches!(
            types.as_slice(),
            [TypeExpr::Function { .. }, TypeExpr::Function { .. }]
        ));
    }
    let TypeExpr::Function { result, .. } =
        annotation("(string) -> (string) & (number) -> (number)")
    else {
        panic!("expected a function returning an intersection");
    };
    let TypeExpr::Intersection { types, .. } = *result else {
        panic!("expected intersection result");
    };
    assert!(
        matches!(types.as_slice(), [TypeExpr::TypePack { types, .. }, TypeExpr::Function { .. }]
        if matches!(types.as_slice(), [TypeExpr::Named {name, ..}] if name == "string"))
    );
}

#[test]
fn alias_spans_include_keyword_and_entire_type() {
    for declaration in [
        "type A = typeof(1)",
        "type Packed1<T...> = (T...) -> (T...)",
        "type Packed2<T...> = (Packed1<T...>, T...) -> (Packed1<T...>, T...)",
        "export type Packed<T...> = (T...) -> (T...)",
    ] {
        let source = format!("\n        {declaration}\n    ");
        let chunk = parse_source(&source).unwrap();
        let [Stmt::TypeAlias { span, value, .. }] = chunk.statements.as_slice() else {
            panic!("{source}");
        };
        assert_eq!(&source[span.start..span.end], declaration);
        if declaration.starts_with("type A") {
            assert!(matches!(value, TypeExpr::Typeof { .. }));
        }
    }
}

#[test]
fn contextual_type_continue_and_export_have_distinct_statement_forms() {
    let chunk = parse_source("type('a')\ntype = nil\nlocal A = nil\ntype A = string").unwrap();
    assert!(matches!(chunk.statements.as_slice(), [
        Stmt::Expr {expr: Expr::Call {..}, ..}, Stmt::Assignment { .. }, Stmt::Local {..},
        Stmt::TypeAlias { name, value: TypeExpr::Named { name: ty, .. }, .. }
    ] if name == "A" && ty == "string"));
    let chunk = parse_source(
        "while true do continue() continue = 5 continue, continue = continue continue end",
    )
    .unwrap();
    let [Stmt::While { body, .. }] = chunk.statements.as_slice() else {
        panic!("expected while");
    };
    assert!(
        matches!(body.as_slice(), [Stmt::Expr {..}, Stmt::Assignment {..}, Stmt::Assignment {targets, ..}, Stmt::Continue {..}] if targets.len() == 2)
    );
    let chunk = parse_source(
        "export() export = 5 export, export = export export type A = number type A = number",
    )
    .unwrap();
    assert!(matches!(chunk.statements.as_slice(), [
        Stmt::Expr {..}, Stmt::Assignment {..}, Stmt::Assignment {targets, ..},
        Stmt::TypeAlias {..}, Stmt::TypeAlias {..}
    ] if targets.len() == 2));
}

#[test]
fn assertions_bind_on_each_operand() {
    let chunk = parse_source("local a = one :: any + two :: any").unwrap();
    let [Stmt::Local { init, .. }] = chunk.statements.as_slice() else {
        panic!("expected local");
    };
    let [Expr::Binary {
        op: BinaryOp::Add,
        lhs,
        rhs,
        ..
    }] = init.as_slice()
    else {
        panic!("expected addition");
    };
    for (operand, expected) in [(lhs, "one"), (rhs, "two")] {
        assert!(
            matches!(operand.as_ref(), Expr::Paren {expr,..} if matches!(expr.as_ref(), Expr::Name(name,_) if name == expected))
        );
    }
}

#[test]
fn vertical_space_separates_calls() {
    let chunk = parse_source("a()\x0bb()").unwrap();
    assert!(
        matches!(chunk.statements.as_slice(), [Stmt::Expr {span:a,..},Stmt::Expr {span:b,..}]
        if *a == Span::new(0,3) && *b == Span::new(4,7))
    );
}

#[test]
fn numeric_values_match_decimal_hex_and_binary_literals() {
    for (literals, expected) in [
        ("1, .5, 1.5, 1e-5, 1.5e-5, 12_345.1_25", &[1.0, 0.5, 1.5, 1e-5, 1.5e-5, 12345.125][..]),
        ("0xab, 0XAB05, 0xff_ff, 0xffffffffffffffff", &[171., 43781., 65535., u64::MAX as f64][..]),
        ("0b1, 0b0, 0b101010, 0b1111111111111111111111111111111111111111111111111111111111111111",
        &[1., 0., 42., u64::MAX as f64][..]),
    ] {
        let chunk = parse_source(&format!("return {literals}")).unwrap();
        let [Stmt::Return { value, extra_values, .. }] = chunk.statements.as_slice() else { panic!("{literals}"); };
        let actual: Vec<_> = value.iter().chain(extra_values).map(|expr| match expr {
            Expr::Number(value,_) => *value, _ => panic!("expected number")
        }).collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn unicode_in_strings_and_invalid_tokens_have_byte_spans() {
    let chunk = parse_source("local snowman = '☃'").unwrap();
    let [Stmt::Local { init, .. }] = chunk.statements.as_slice() else {
        panic!("expected local");
    };
    assert!(matches!(init.as_slice(), [Expr::String(bytes, _)] if bytes == "☃".as_bytes()));
    let source = [b"\xff\xfe".as_slice(), "☃․".as_bytes()].concat();
    let tokens = lex_for_parser(&source).unwrap();
    let expected = [
        (0, 1, "invalid UTF-8 sequence"),
        (1, 2, "invalid UTF-8 sequence"),
        (2, 5, "Unicode character U+2603"),
        (5, 8, "Unicode character U+2024 (did you mean '.'?)"),
    ];
    assert_eq!(tokens.len(), expected.len() + 1);
    for (token, (start, end, text)) in tokens.iter().zip(expected) {
        assert_eq!(token.span, Span::new(start, end));
        assert!(matches!(&token.kind, TokenKind::Invalid(desc) if desc == text));
    }
    assert!(matches!(tokens.last().unwrap().kind, TokenKind::Eof));
}

fn check_error(source: &[u8], start: usize, end: usize, expected: &str) {
    let (span, message) = match parse_source_bytes(source).unwrap_err() {
        SourceError::Lex(error) => (error.span, error.message),
        SourceError::Parse(error) => (error.span, error.message),
    };
    let source = String::from_utf8_lossy(source);
    assert_eq!(message, expected, "{source}");
    assert_eq!(span, Span::new(start, end), "{source}");
}

#[test]
fn invalid_utf8_expression_diagnostics() {
    for source in [b"local pi = \xff!", b"local pi = \xe2!"] {
        check_error(
            source,
            11,
            12,
            "Expected identifier when parsing expression, got invalid UTF-8 sequence",
        );
    }
}

#[test]
fn malformed_type_diagnostics() {
    for (source, start, end, message) in [
        (
            "\nlocal a: (number, number) -> (string\n",
            38,
            38,
            "Expected ')' (to close '(' at line 2), got <eof>",
        ),
        (
            "\nlocal a: (number, number) -> (\n            string\n",
            51,
            51,
            "Expected ')' (to close '(' at line 2), got <eof>",
        ),
        (
            "\nlocal a: (number, number)\n",
            27,
            27,
            "Expected '->' when parsing function type, got <eof>",
        ),
        (
            "\nlocal a: (number, number\n",
            26,
            26,
            "Expected ')' (to close '(' at line 2), got <eof>",
        ),
        (
            "\nlocal a: {foo: string,\n",
            24,
            24,
            "Expected identifier when parsing table field, got <eof>",
        ),
        (
            "\nlocal a: {foo: string\n",
            23,
            23,
            "Expected '}' (to close '{' at line 2), got <eof>",
        ),
        (
            "\nlocal a: { [string]: number, [number]: string }\n",
            30,
            46,
            "Cannot have more than one table indexer",
        ),
        (
            "\ntype T = <a>foo\n",
            13,
            16,
            "Expected '(' when parsing function parameters, got 'foo'",
        ),
        (
            "type A = number & string | boolean",
            9,
            34,
            "Mixing union and intersection types is not allowed; consider wrapping in parentheses.",
        ),
        (
            "type F = ((string, number))",
            26,
            27,
            "Expected '->' when parsing function type, got ')'",
        ),
        (
            "type F = () -> ((string, number))",
            32,
            33,
            "Expected '->' when parsing function type, got ')'",
        ),
        (
            "type 5 = number",
            5,
            6,
            "Expected identifier when parsing type name, got '5'",
        ),
        (
            "type A",
            6,
            6,
            "Expected '=' when parsing type alias, got <eof>",
        ),
        ("type A<", 7, 7, "Expected identifier, got <eof>"),
        (
            "type A<B",
            8,
            8,
            "Expected '>' (to close '<' at column 7), got <eof>",
        ),
        (
            "local a: Foo.=",
            13,
            14,
            "Expected identifier when parsing field name, got '='",
        ),
    ] {
        check_error(source.as_bytes(), start, end, message);
    }
}

#[test]
fn malformed_number_diagnostics() {
    for (source, start, end, message) in [
        ("return 0b123", 7, 12, "Malformed number"),
        ("return 123x", 7, 11, "Malformed number"),
        ("return 0xg", 7, 10, "Malformed number"),
        ("return 0x0x123", 7, 14, "Malformed number"),
        (
            "return 0xffffffffffffffffffffllllllg",
            7,
            36,
            "Malformed number",
        ),
        (
            "return 0x0xffffffffffffffffffffffffffff",
            7,
            39,
            "Malformed number",
        ),
    ] {
        check_error(source.as_bytes(), start, end, message);
    }
}

#[test]
fn statement_diagnostics() {
    for (source, start, end, message) in [
        (
            "   -",
            3,
            4,
            "Expected identifier when parsing expression, got '-'",
        ),
        ("return 0 print(5)", 9, 14, "Expected <eof>, got 'print'"),
        (
            "while true do break print(5) end",
            20,
            25,
            "Expected 'end' (to close 'do' at column 12), got 'print'",
        ),
        (
            "while true do continue print(5) end",
            23,
            28,
            "Expected 'end' (to close 'do' at column 12), got 'print'",
        ),
        (
            "export function a() end",
            0,
            6,
            "Incomplete statement: expected assignment or a function call",
        ),
        (
            "fiddlesticks",
            0,
            12,
            "Incomplete statement: expected assignment or a function call",
        ),
        (
            "a() += 5",
            4,
            6,
            "Expected identifier when parsing expression, got '+='",
        ),
        (
            "(a) += 5",
            0,
            3,
            "Assigned expression must be a variable or a field",
        ),
        (
            "a, b += 5",
            5,
            7,
            "Expected '=' when parsing assignment, got '+='",
        ),
    ] {
        check_error(source.as_bytes(), start, end, message);
    }
}

#[test]
fn unicode_diagnostics() {
    for (source, start, end, message) in [
        ("local ☃ = 10", 6, 9, "Expected identifier when parsing variable name, got Unicode character U+2603"),
        ("local pi = 3․13", 12, 15, "Expected identifier when parsing expression, got Unicode character U+2024 (did you mean '.'?)"),
    ] {
        check_error(source.as_bytes(),start,end,message);
    }
}

#[test]
fn interpolation_diagnostics() {
    for (source, start, end, message) in [
        (
            "_ = `{{oops}}`",
            4,
            5,
            "Double braces are not permitted within interpolated strings; did you mean '\\{'?",
        ),
        (
            "_ = `{nice} {{oops}}`",
            4,
            6,
            "Double braces are not permitted within interpolated strings; did you mean '\\{'?",
        ),
        (
            "_ = `{a`",
            7,
            8,
            "Malformed interpolated string; did you forget to add a '}'?",
        ),
        (
            "_ = `{abcdefg`",
            13,
            14,
            "Malformed interpolated string; did you forget to add a '}'?",
        ),
        (
            "_ =       `{a`",
            13,
            14,
            "Malformed interpolated string; did you forget to add a '}'?",
        ),
        (
            "_ = { `{a` }",
            9,
            12,
            "Malformed interpolated string; did you forget to add a '}'?",
        ),
        (
            "_ = { `x {\"y\"} {z` }",
            17,
            20,
            "Malformed interpolated string; did you forget to add a '}'?",
        ),
        (
            "local a: `what` = `???`",
            9,
            15,
            "Interpolated string literals cannot be used as types",
        ),
        (
            "local b: `what {\"the\"}` = `???`",
            9,
            16,
            "Interpolated string literals cannot be used as types",
        ),
        (
            "local c: `what {\"the\"} heck` = `???`",
            9,
            16,
            "Interpolated string literals cannot be used as types",
        ),
        (
            "_ = print `{42}`",
            10,
            12,
            "Expected identifier when parsing expression, got `{",
        ),
        (
            "print(`{}`)",
            6,
            8,
            "Malformed interpolated string, expected expression inside '{}'",
        ),
        (
            "print(`{}{1}`)",
            6,
            8,
            "Malformed interpolated string, expected expression inside '{}'",
        ),
        (
            "local a = `???\\xQQ {1}`",
            10,
            20,
            "Interpolated string literal contains malformed escape sequence",
        ),
        (
            "local a = `??? {42 !!}`",
            10,
            16,
            "Malformed interpolated string, got '!'",
        ),
    ] {
        check_error(source.as_bytes(), start, end, message);
    }
}

#[test]
fn missing_block_terminator_hints() {
    for (source, start, end, message) in [
        (r#"-- i am line 1
function BottomUpTree(item, depth)
  if depth > 0 then
    local i = item + item
    depth = depth - 1
    local left, right = BottomUpTree(i-1, depth), BottomUpTree(i, depth)
    return { item, left, right }
  else
    return { item }
end

function ItemCheck(tree)
  if tree[2] then
    return tree[1] + ItemCheck(tree[2]) - ItemCheck(tree[3])
  else
    return tree[1]
  end
end
        "#, 404, 404, "Expected 'end' (to close 'function' at line 2), got <eof>; did you forget to close 'else' at line 8?"),
        (r#"-- i am line 1
function ItemCheck(tree)
  if tree[2] then return tree[1] + ItemCheck(tree[2]) - ItemCheck(tree[3]) else return tree[1]
end

function BottomUpTree(item, depth)
  if depth > 0 then
    local i = item + item
    depth = depth - 1
    local left, right = BottomUpTree(i-1, depth), BottomUpTree(i, depth)
    return { item, left, right }
  else
    return { item }
  end
end
        "#, 394, 394, "Expected 'end' (to close 'function' at line 2), got <eof>; did you forget to close 'else' at line 3?"),
        ("-- i am line 1\nrepeat\n  print(1)\n  repeat\n    print(2)\n  print(3)\nuntil false\n        ", 86, 86, "Expected 'until' (to close 'repeat' at line 2), got <eof>; did you forget to close 'repeat' at line 4?"),
        (r#"-- i am line 1
local function BottomUpTree(item, depth)
  if depth > 0 then
    local i = item + item
    depth = depth - 1
    local left, right = BottomUpTree(i-1, depth), BottomUpTree(i, depth)
    return { item, left, right }
  else
    return { item }
end

local function ItemCheck(tree)
  if tree[2] then
    return tree[1] + ItemCheck(tree[2]) - ItemCheck(tree[3])
  else
    return tree[1]
  end
end
        "#, 416, 416, "Expected 'end' (to close 'function' at line 2), got <eof>; did you forget to close 'else' at line 8?"),
        (r#"-- i am line 1
local function ItemCheck(tree)
  if tree[2] then
    return tree[1] + ItemCheck(tree[2]) - ItemCheck(tree[3])
  else
    return tree[1]
      end
end

local function BottomUpTree(item, depth)
  if depth > 0 then
    local i = item + item
    depth = depth - 1
    local left, right = BottomUpTree(i-1, depth), BottomUpTree(i, depth)
    return { item, left, right }
  else
    return { item }
  end
        "#, 422, 422, "Expected 'end' (to close 'function' at line 10), got <eof>"),
        (r#"-- i am line 1
function stringifyTable(t)
    local entries = {}
    for k, v in pairs(t) do
        -- if we find a nested table, convert that recursively
        if type(v) == "table" then
            v = stringifyTable(v)
        else
            v = tostring(v)
        k = tostring(k)

        -- add another entry to our stringified table
        entries[#entries + 1] = ("s = s"):format(k, v)
    end

    -- the memory location of the table
    local id = tostring(t):sub(8)

    return ("{s}@s"):format(table.concat(entries, ", "), id)
end
        "#, 557, 557, "Expected 'end' (to close 'function' at line 2), got <eof>; did you forget to close 'else' at line 8?"),
    ] {
        check_error(source.as_bytes(),start,end,message);
    }
}

#[test]
fn table_annotations_allow_named_fields_indexers_and_both_separators() {
    for ty in [
        "{x: number, y: string}",
        "{[string]: number, f: () -> nil}",
        "{x: number, y: string, }",
        "{x: number; y: string; }",
    ] {
        let source = format!("local t: {ty}");
        let TypeExpr::Table { span } = annotation(ty) else {
            panic!("{ty}");
        };
        assert_eq!(&source[span.start..span.end], ty);
    }
}

#[test]
fn parenthesized_intersection_can_be_a_union_member() {
    let TypeExpr::Union { types, .. } = annotation("(number & string) | boolean") else {
        panic!("expected union");
    };
    assert!(
        matches!(types.as_slice(), [TypeExpr::TypePack { types, .. },TypeExpr::Named {name,..}]
        if name == "boolean" && matches!(types.as_slice(), [TypeExpr::Intersection {..}]))
    );
}
