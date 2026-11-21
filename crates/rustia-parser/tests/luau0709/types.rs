// Adapted from Luau 0.709 tests/Parser.test.cpp:1329–2964.
// Upstream MIT license: conformance/upstream/luau-0.709/LICENSE.txt.
use crate::parse_source;
use rustia_lexer::Span;
use rustia_parser::{BinaryOp, Expr, SourceError, Stmt, TypeExpr};

fn statements(source: &str) -> Vec<Stmt> {
    parse_source(source)
        .unwrap_or_else(|e| panic!("{source:?}: {e}"))
        .statements
}

fn first_error(source: &str, start: usize, end: usize, message: &str) {
    let (span, actual) = match parse_source(source).expect_err(source) {
        SourceError::Parse(error) => (error.span, error.message),
        SourceError::Lex(error) => (error.span, error.message),
    };
    assert_eq!(
        (span, actual.as_str()),
        (Span::new(start, end), message),
        "{source:?}"
    );
}

fn returned(source: &str) -> Vec<Expr> {
    let mut stmts = statements(source);
    let Stmt::Return {
        value: Some(first),
        mut extra_values,
        ..
    } = stmts.remove(0)
    else {
        panic!("expected return")
    };
    assert!(stmts.is_empty());
    let mut values = vec![first];
    values.append(&mut extra_values);
    values
}

fn alias(source: &str) -> TypeExpr {
    let mut stmts = statements(source);
    let Stmt::TypeAlias { value, .. } = stmts.remove(0) else {
        panic!("expected type alias")
    };
    assert!(stmts.is_empty());
    value
}

fn named(ty: &TypeExpr, expected: &str) {
    assert!(
        matches!(ty, TypeExpr::Named {name, args, ..} if name == expected && args.is_empty()),
        "{ty:?}"
    );
}

#[test]
fn table_and_short_array_annotations() {
    for source in ["local n: {string}", "local n: {string: number}"] {
        let stmts = statements(source);
        let [Stmt::Local { bindings, .. }] = stmts.as_slice() else {
            panic!("expected local")
        };
        let Some(TypeExpr::Table { span }) = &bindings[0].annotation else {
            panic!("expected table type")
        };
        assert_eq!(&source[span.start..span.end], &source[9..]);
    }
    let methods = "type Vector3MT = {__add: (Vector3MT, Vector3MT) -> Vector3MT, __mul: (Vector3MT, Vector3MT|number) -> Vector3MT}";
    assert!(matches!(alias(methods), TypeExpr::Table { .. }));
}

#[test]
fn string_literal_call_and_bytes() {
    let stmts = statements("do foo 'bar' end");
    let [Stmt::Do { body, .. }] = stmts.as_slice() else {
        panic!("expected do")
    };
    let [Stmt::Expr {
        expr: Expr::Call { callee, args, .. },
        ..
    }] = body.as_slice()
    else {
        panic!("expected call")
    };
    assert!(matches!(&**callee, Expr::Name(name, _) if name == "foo"));
    assert!(matches!(args.as_slice(), [Expr::String(bytes, _)] if bytes == b"bar"));
    let cases: &[(&str, &[&[u8]])] = &[
        (
            "return [=[\nfoo\r\nbar\n\nbaz\n]=]",
            &[b"foo\nbar\n\nbaz\n"],
        ),
        (
            "return [[\r\nfoo]], [[\rfoo\r]], [[\n\rfoo\n\rbar]]",
            &[b"foo", b"\rfoo\r", b"\rfoo\n\rbar"],
        ),
        (
            r#"return "foo\n\r","foo\0324","foo\x204","foo\u{20}","foo\u{0451}""#,
            &[b"foo\n\r", b"foo 4", b"foo 4", b"foo ", b"foo\xd1\x91"],
        ),
        (
            "return \"foo\\z\n   bar\", \"foo\\\n    bar\", \"foo\\\r\nbar\"",
            &[b"foobar", b"foo\n    bar", b"foo\nbar"],
        ),
        (
            r#"return "\xAB","\u{2024}","\121","\1x","\t","\n""#,
            &[b"\xab", b"\xe2\x80\xa4", b"y", b"\x01x", b"\t", b"\n"],
        ),
    ];
    for &(source, expected) in cases {
        let bytes: Vec<_> = returned(source)
            .into_iter()
            .map(|e| match e {
                Expr::String(b, _) => b,
                _ => panic!("expected a string"),
            })
            .collect();
        assert_eq!(bytes, expected, "{source:?}");
    }
}

#[test]
fn number_literal_values() {
    let values: Vec<_> = returned("return 1, 1.5, .5, 12_34_56, 0x1234, 0b010101")
        .into_iter()
        .map(|e| match e {
            Expr::Number(n, _) => n,
            _ => panic!("expected number"),
        })
        .collect();
    assert_eq!(values, [1.0, 1.5, 0.5, 123456.0, 4660.0, 21.0]);
}

#[test]
fn type_extents_stop_before_later_statements_and_comments() {
    let lines = [
        "type F = (string) -> string",
        "type G = string | number | boolean",
        "type H = string & number & boolean",
        "type I = number",
    ];
    let source = format!("{}\n--comment\nprint('hello')", lines.join("\n"));
    let stmts = statements(&source);
    assert_eq!(stmts.len(), 5);
    for (line, stmt) in lines.iter().zip(stmts) {
        let Stmt::TypeAlias { value, span, .. } = stmt else {
            panic!("expected alias")
        };
        assert_eq!(&source[span.start..span.end], *line);
        assert_eq!(value.span().end, span.end);
    }
}

#[test]
fn variadic_functions_and_pack_aliases() {
    let stmts = statements(
        "function foo(bar, ...: number): ...string end\nfunction f<a...>(...: a...) end",
    );
    assert!(
        matches!(stmts.as_slice(), [Stmt::Function { is_vararg:true, params: fixed, ..},Stmt::Function {is_vararg:true, params:empty,..}] if fixed.len() == 1 && empty.is_empty())
    );
    let TypeExpr::Function { params, result, .. } =
        alias("type Foo = (string, number, ...number) -> ...boolean")
    else {
        panic!("expected function")
    };
    let [first, second, TypeExpr::Variadic { inner, .. }] = params.as_slice() else {
        panic!("expected two parameters and a tail")
    };
    named(first, "string");
    named(second, "number");
    named(inner, "number");
    let TypeExpr::Variadic { inner, .. } = *result else {
        panic!("expected variadic return")
    };
    named(&inner, "boolean");
    let TypeExpr::Function { params, result, .. } = alias("type Bar = () -> (number, ...boolean)")
    else {
        panic!("expected function")
    };
    assert!(params.is_empty());
    let TypeExpr::TypePack { types, .. } = *result else {
        panic!("expected explicit return pack")
    };
    let [fixed, TypeExpr::Variadic { inner, .. }] = types.as_slice() else {
        panic!("expected fixed return and tail")
    };
    named(fixed, "number");
    named(inner, "boolean");
    let TypeExpr::Function { params, result, .. } = alias("type A = (a...) -> b...") else {
        panic!("expected function")
    };
    let [TypeExpr::Variadic { inner, .. }] = params.as_slice() else {
        panic!("expected named parameter pack")
    };
    named(inner, "a");
    let TypeExpr::Variadic { inner, .. } = *result else {
        panic!("expected named result pack")
    };
    named(&inner, "b");
}

#[test]
fn named_function_type_parameters() {
    for parameters in [
        "a: number, b: string, c: number",
        "a: number, string, c: number",
        "a: number, string, number",
    ] {
        let TypeExpr::Function { params, result, .. } =
            alias(&format!("type F = ({parameters}) -> string"))
        else {
            panic!("expected function")
        };
        let [a, b, c] = params.as_slice() else {
            panic!("expected three parameters")
        };
        named(a, "number");
        named(b, "string");
        named(c, "number");
        named(&result, "string");
    }
    let nested = alias(
        "type F = (a: number, b: string, c: number) -> (d: number, e: string, f: number) -> string",
    );
    let TypeExpr::Function { params, result, .. } = nested else {
        panic!("expected function")
    };
    assert_eq!(params.len(), 3);
    let TypeExpr::Function { params, result, .. } = *result else {
        panic!("expected nested function")
    };
    assert_eq!(params.len(), 3);
    named(&params[2], "number");
    named(&result, "string");
}

#[test]
fn generic_alias_defaults_and_pack_arguments() {
    let src = "type A<T = string> = {}\n\
               type B<T... = ...number> = {}\n\
               type C<T..., U... = T...> = {}\n\
               type D<T..., U... = ()> = {}\n\
               type E<T... = (), U... = ()> = {}\n\
               type F<T... = (string), U... = ()> = (T...) -> U...\n\
               type G<T... = ...number, U... = (string, number, boolean)> = (U...) -> T...";
    assert_eq!(statements(src).len(), 7);
    let src = "type Packed<T...> = () -> T...\n\
               type A<X...> = Packed<X...>\n\
               type B<X...> = Packed<...number>\n\
               type C<X...> = Packed<(number, X...)>";
    let stmts = statements(src);
    assert_eq!(stmts.len(), 4);
    for (stmt, expected) in stmts[1..]
        .iter()
        .zip(["X...", "...number", "(number, X...)"])
    {
        let Stmt::TypeAlias {
            value: TypeExpr::Named { name, args, .. },
            ..
        } = stmt
        else {
            panic!("expected named generic type")
        };
        assert_eq!(name, "Packed");
        assert_eq!(args.len(), 1);
        let span = args[0].span();
        assert_eq!(&src[span.start..span.end], expected);
    }
}

#[test]
fn if_expression_nesting() {
    for source in [
        "return if true then 1 else 2",
        "return if true then 1 elseif true then 2 else 3",
        "return if true then 1 else if true then 2 else 3",
        "return if if true then false else true then 1 else 2",
    ] {
        let values = returned(source);
        let [Expr::IfElse {
            condition,
            else_expr,
            ..
        }] = values.as_slice()
        else {
            panic!("expected one if expression")
        };
        assert_eq!(
            matches!(**else_expr, Expr::IfElse { .. }),
            source.contains("elseif") || source.contains("else if")
        );
        assert_eq!(
            matches!(**condition, Expr::IfElse { .. }),
            source.contains("if if")
        );
    }
}

#[test]
fn union_intersection_and_group_structure() {
    let TypeExpr::Union { types, .. } = alias("type Foo = | string") else {
        panic!("expected leading union")
    };
    let [only] = types.as_slice() else {
        panic!("one union member")
    };
    named(only, "string");
    let TypeExpr::Intersection { types, .. } = alias("type Bar = & number") else {
        panic!("expected leading intersection")
    };
    let [only] = types.as_slice() else {
        panic!("one intersection member")
    };
    named(only, "number");
    let TypeExpr::Union { types, .. } =
        alias("type Value = string | (Readable & Writable) | {number}")
    else {
        panic!("expected union")
    };
    assert_eq!(types.len(), 3);
    let TypeExpr::TypePack { types: group, .. } = &types[1] else {
        panic!("expected grouped intersection")
    };
    assert!(matches!(group.as_slice(), [TypeExpr::Intersection {types,..}] if types.len()==2));
    for (source, levels) in [
        ("type Foo = (string)", 1),
        ("type Foo = ((string))", 2),
        ("type Foo = () -> (string)", 1),
    ] {
        let ty = alias(source);
        let mut inner = if let TypeExpr::Function { result, .. } = &ty {
            &**result
        } else {
            &ty
        };
        for _ in 0..levels {
            let TypeExpr::TypePack { types, span } = inner else {
                panic!("expected a parenthesized group")
            };
            assert_eq!(&source[span.start..span.start + 1], "(");
            assert_eq!(&source[span.end - 1..span.end], ")");
            let [only] = types.as_slice() else {
                panic!("one type in group")
            };
            inner = only;
        }
        named(inner, "string");
    }
}

#[test]
fn function_block_and_for_extents() {
    let src = "\n        local function foo()\n            local x = 1\n        end\n  ";
    let stmts = statements(src);
    let [Stmt::LocalFunction { body, span, .. }] = stmts.as_slice() else {
        panic!("expected local function")
    };
    assert_eq!(&src[span.start..span.end], src.trim());
    let [Stmt::Local { span, .. }] = body.as_slice() else {
        panic!("expected local")
    };
    assert_eq!(&src[span.start..span.end], "local x = 1");
    let src = "\n        do\n            local hello = \"world\"\n        end\n    ";
    let stmts = statements(src);
    let [Stmt::Do { body, span }] = stmts.as_slice() else {
        panic!("expected do")
    };
    assert_eq!(&src[span.start..span.end], src.trim());
    let [Stmt::Local { span, .. }] = body.as_slice() else {
        panic!("expected local")
    };
    assert_eq!(&src[span.start..span.end], "local hello = \"world\"");
    let stmts = statements("for value in tbl do end");
    assert!(
        matches!(stmts.as_slice(), [Stmt::ForGeneric{variables, iter,body,..}] if variables.len()==1 && variables[0].name=="value" && matches!(iter.as_slice(),[Expr::Name(name, _)] if name=="tbl") && body.is_empty())
    );
}

#[test]
fn attributed_function_locations_include_the_attribute() {
    for src in [
        "@native\nfunction globalFunction()\nend",
        "@native\nlocal function localFunction()\nend",
        "local _ = @native function()\nend",
    ] {
        let stmts = statements(src);
        let span = match stmts.as_slice() {
            [Stmt::Function { span, .. } | Stmt::LocalFunction { span, .. }] => *span,
            [Stmt::Local { init, .. }] => init[0].span(),
            _ => panic!("expected function"),
        };
        assert_eq!(
            &src[span.start..span.end],
            &src[src.find("@native").unwrap()..]
        );
    }
}

#[test]
fn explicit_instantiation_postfix_and_call_shapes() {
    let src = "local x = f<<T, U>>()";
    let stmts = statements(src);
    let [Stmt::Local { init, .. }] = stmts.as_slice() else {
        panic!("expected local")
    };
    let [Expr::Call { callee, args, .. }] = init.as_slice() else {
        panic!("expected call")
    };
    let Expr::TypeInstantiation { expr, span } = callee.as_ref() else {
        panic!("expected an instantiated callee")
    };
    assert!(matches!(expr.as_ref(), Expr::Name(name, _) if name == "f"));
    assert_eq!(&src[span.start..span.end], "f<<T, U>>");
    assert!(args.is_empty());
    let values = returned("return f<<T, U>>, f<<T>>.x, f<<T>>[1], f<<T>><<U>>, pair()<<number>>");
    assert!(matches!(
        values.as_slice(),
        [
            Expr::TypeInstantiation { .. },
            Expr::Field { .. },
            Expr::Index { .. },
            Expr::TypeInstantiation { .. },
            Expr::TypeInstantiation { .. }
        ]
    ));
    let stmts = statements("f<<T,U>>()\nt.f<<T, U>>()\nt:f<<T, U>>()\nt['f']<<T, U>>()\nf<<>>()\nf<<(number) -> string, (T,U...), ...boolean>>()\n");
    assert_eq!(stmts.len(), 6);
    assert!(
        matches!(&stmts[2], Stmt::Expr {expr:Expr::MethodCall {method, ..}, ..} if method == "f")
    );
    assert!(stmts.iter().enumerate().all(|(i, s)| i == 2
        || matches!(
            s,
            Stmt::Expr {
                expr: Expr::Call { .. },
                ..
            }
        )));
}

#[test]
fn angle_brackets_in_types_and_comparisons() {
    let TypeExpr::Named { name, args, .. } = alias("type A = C<B<<T>() -> T>>") else {
        panic!("expected C")
    };
    assert_eq!(name, "C");
    assert!(
        matches!(args.as_slice(),[TypeExpr::Named {name,args,..}] if name=="B" && matches!(args.as_slice(),[TypeExpr::Function{..}]))
    );
    let stmts = statements("local a = b.c < d");
    let [Stmt::Local { init, .. }] = stmts.as_slice() else {
        panic!("expected local")
    };
    assert!(
        matches!(init.as_slice(), [Expr::Binary {op:BinaryOp::Lt, lhs, rhs,..}] if matches!(&**lhs, Expr::Field{field,..} if field=="c") && matches!(&**rhs, Expr::Name(n,_) if n=="d"))
    );
}

#[test]
fn table_and_call_diagnostics() {
    first_error("function stringifyTable(t)\n    local foo = (name = t)\n    return foo\nend", 49, 50, "Expected ')' (to close '(' at column 17), got '='; did you mean to use '{' when defining a table?");
    first_error(
        "function stringifyTable(t)\n    local foo = t:Parse 2\n    return foo\nend",
        43,
        51,
        "Expected '(', '{' or <string> when parsing function call, got '2'",
    );
    first_error(
        "function stringifyTable(t)\n    local foo = t:Parse\n    return foo\nend",
        43,
        50,
        "Expected function call arguments after '('",
    );
    first_error("function foo(): string, number end", 22, 23, "Expected a statement, got ','; did you forget to wrap the list of return types in parentheses?");
    first_error("function foo(): (number) -> string, string", 34, 35, "Expected a statement, got ','; did you forget to wrap the list of return types in parentheses?");
    first_error(
        "local n: {string, number}",
        16,
        17,
        "Expected '}' (to close '{' at column 10), got ','",
    );
    first_error(
        "local n: {[number]: string, number}",
        34,
        35,
        "Expected ':' when parsing table field, got '}'",
    );
    first_error(
        "local n: {x: string, number}",
        27,
        28,
        "Expected ':' when parsing table field, got '}'",
    );
    first_error(
        "local n: {x: string, nil}",
        21,
        24,
        "Expected identifier when parsing table field, got 'nil'",
    );
    first_error(
        "local n: {string | number: number}",
        25,
        26,
        "Expected '}' (to close '{' at column 10), got ':'",
    );
    first_error(
        "local n: {nil: number}",
        13,
        14,
        "Expected '}' (to close '{' at column 10), got ':'",
    );
}

#[test]
fn unfinished_comment_and_literal_diagnostics() {
    first_error(
        "--[[unfinished work",
        0,
        19,
        "Expected identifier when parsing expression, got unfinished comment",
    );
    first_error(
        "--!strict\n--[[unfinished work",
        10,
        29,
        "Expected identifier when parsing expression, got unfinished comment",
    );
    first_error(
        "local x = 1 --[[unfinished work",
        12,
        31,
        "Expected identifier when parsing expression, got unfinished comment",
    );
    first_error(
        "return \"\\u{\"",
        7,
        12,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"\\u{FO}\"",
        7,
        15,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"\\u{123456789}\"",
        7,
        22,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"\\359\"",
        7,
        13,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"\\xFO\"",
        7,
        13,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"\\xF\"",
        7,
        12,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"\\x\"",
        7,
        11,
        "String literal contains malformed escape sequence",
    );
    first_error(
        "return \"",
        7,
        8,
        "Malformed string; did you forget to finish it?",
    );
    first_error(
        "return \"\\",
        7,
        9,
        "Malformed string; did you forget to finish it?",
    );
    first_error(
        "return \"\r\r",
        7,
        8,
        "Malformed string; did you forget to finish it?",
    );
}

#[test]
fn assignment_and_annotation_diagnostics() {
    first_error(
        "local a,b; (2), b = b, a",
        11,
        14,
        "Assigned expression must be a variable or a field",
    );
    first_error(
        "local a,b; a, (3) = b, a",
        14,
        17,
        "Assigned expression must be a variable or a field",
    );
    first_error("local a : 2 = 2", 9, 11, "Expected type, got '2'");
    first_error("local x:", 8, 8, "Expected type, got <eof>");
    first_error("local x:=42", 8, 9, "Expected type, got '='");
    first_error("function func():end", 16, 19, "Expected type, got 'end'");
}

#[test]
fn generic_and_pack_diagnostics() {
    first_error(
        "function foo(): (...number, string) end",
        26,
        27,
        "Expected ')' (to close '(' at column 17), got ','",
    );
    first_error(
        "type Foo = (...number, string) -> (...string, number)",
        21,
        22,
        "Expected ')' (to close '(' at column 12), got ','",
    );
    first_error(
        "type MyFunc = (a: number, b: string, c: number) -> (d: number, e: string, f: number)",
        84,
        84,
        "Expected '->' when parsing function type, got <eof>",
    );
    first_error(
        "type MyFunc = (number) -> (d: number) <a, b, c> -> number",
        38,
        39,
        "Expected '->' when parsing function type, got '<'",
    );
    first_error(
        "local a: <T>(number -> string",
        20,
        22,
        "Expected ')' (to close '(' at column 13), got '->'",
    );
    first_error(
        "type Y<T = number, U> = {}",
        20,
        21,
        "Expected default type after type name",
    );
    first_error(
        "type Y<T... = ...number, U...> = {}",
        29,
        30,
        "Expected default type pack after type pack name",
    );
    first_error(
        "type Y<T... = (string) -> number> = {}",
        14,
        32,
        "Expected type pack after '=', got type",
    );
    first_error(
        "type Y<T...> = {a: T..., b: number}",
        20,
        23,
        "Unexpected '...' after type name; type pack is not allowed in this context",
    );
    first_error(
        "type Y<T...> = {a: (number | string)...",
        36,
        39,
        "Unexpected '...' after type annotation",
    );
    first_error(
        "type A = (b: number)",
        20,
        20,
        "Expected '->' when parsing function type, got <eof>",
    );
    first_error(
        "type P<T...> = () -> T... type B = P<(x: number, y: string)>",
        59,
        60,
        "Expected '->' when parsing function type, got '>'",
    );
    first_error(
        "type F<T... = (a: string)> = (T...) -> ()",
        25,
        26,
        "Expected '->' when parsing function type, got '>'",
    );
}

#[test]
fn explicit_operand_diagnostics() {
    first_error("return f<<T,>>()", 12, 13, "Expected type, got '>'");
    first_error(
        "f<<T>> = 1",
        0,
        6,
        "Assigned expression must be a variable or a field",
    );
    first_error(
        "f<<T>>",
        0,
        6,
        "Incomplete statement: expected assignment or a function call",
    );
    first_error(
        "return f<<(number,string) | nil>>()",
        26,
        27,
        "Expected '>' (to close '<' at column 10), got '|'",
    );
}
