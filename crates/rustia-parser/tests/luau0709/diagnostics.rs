//! Portable assertions from Luau 0.709 tests/Parser.test.cpp.
//! Rustia returns the first error and exposes no recovered syntax tree.

use crate::parse_source;
use rustia_parser::{AssignmentTarget, SourceError, Stmt, TypeExpr};

fn diagnostic(source: &str, start: usize, end: usize, message: &str) {
    let (span, actual) = match parse_source(source).unwrap_err() {
        SourceError::Lex(e) => (e.span, e.message),
        SourceError::Parse(e) => (e.span, e.message),
    };
    assert_eq!(
        (span.start, span.end, actual.as_str()),
        (start, end, message),
        "{source}"
    );
}

fn reject(source: &str) {
    assert!(parse_source(source).is_err(), "{source}");
}

fn accept(source: &str) {
    parse_source(source).unwrap_or_else(|error| panic!("{source}: {error}"));
}

#[test]
fn diagnostics_preserve_literal_spellings() {
    diagnostic(
        "return @checked 0x2",
        7,
        15,
        "Expected 'function' declaration after attribute, but got '0x2' instead",
    );
    diagnostic(
        "local 1_0 = 7",
        6,
        9,
        "Expected identifier when parsing variable name, got '1_0'",
    );
    diagnostic(
        "local 2.5e+03 = 7",
        6,
        13,
        "Expected identifier when parsing variable name, got '2.5e+03'",
    );
    diagnostic("type X = 0b0010", 8, 15, "Expected type, got '0b0010'");
    diagnostic(
        "function '\\x41'() end",
        9,
        15,
        "Expected identifier when parsing function name, got \"\\x41\"",
    );
    diagnostic(
        "local \"a\\n\\\"b\" = 1",
        6,
        14,
        "Expected identifier when parsing variable name, got \"a\\n\\\"b\"",
    );
    diagnostic(
        "local [==[\nraw\r\nline]==] = 1",
        6,
        24,
        "Expected identifier when parsing variable name, got \"\nraw\r\nline\"",
    );
    diagnostic(
        "return `prefix {(function() local 0x02 = 0 end)()}`",
        34,
        38,
        "Expected identifier when parsing variable name, got '0x02'",
    );
    diagnostic(
        "return `outer {`inner {(function() local '\\x41' = 0 end)()}`}`",
        41,
        47,
        "Expected identifier when parsing variable name, got \"\\x41\"",
    );
    diagnostic(
        "return @checked `\\x41{2}`",
        7,
        15,
        "Expected 'function' declaration after attribute, but got `\\x41{ instead",
    );
    diagnostic("return f`text`", 8, 14, "Expected <eof>, got `text`");
    diagnostic("return f`text {2}`", 8, 15, "Expected <eof>, got `text {");
    diagnostic(
        "return t:m`text`",
        7,
        10,
        "Expected '(', '{' or <string> when parsing function call, got `text`",
    );
    diagnostic(
        "return t:m `\\x41{2}`",
        7,
        11,
        "Expected '(', '{' or <string> when parsing function call, got `\\x41{",
    );
    diagnostic(
        "return (a `text`)",
        10,
        16,
        "Expected ')' (to close '(' at column 8), got `text`",
    );
    diagnostic(
        "local `\\x41{2}` = 3",
        6,
        12,
        "Expected identifier when parsing variable name, got `\\x41{",
    );
    accept("return f(`v{2}`)");
    accept("return t:m(`text`)");
}

#[test]
fn multiple_parse_errors() {
    reject(
        r#"
local a = 3 * (
return a +
"#,
    );
}

#[test]
fn statement_error_recovery_expected() {
    reject(
        r#"
function a(a, b) return a + b end
some
a(2, 5)
"#,
    );
}

#[test]
fn statement_error_recovery_unexpected() {
    reject("+");
}

#[test]
fn extra_token_in_consume() {
    diagnostic(
        r#"
function test + (a, f) return a + f end
return test(2, 3)
"#,
        15,
        16,
        "Expected '(' when parsing function, got '+'",
    );
}

#[test]
fn extra_token_in_consume_match() {
    diagnostic(
        r#"
function test(a, f+) return a + f end
return test(2, 3)
"#,
        19,
        20,
        "Expected ')' (to close '(' at column 14), got '+'",
    );
}

#[test]
fn extra_token_in_consume_match_end() {
    diagnostic(
        r#"
if true then
    return 12
then
end
"#,
        28,
        32,
        "Expected 'end' (to close 'then' at line 2), got 'then'",
    );
}

#[test]
fn extra_table_indexer_recovery() {
    reject(
        r#"
local a : { [string] : number, [number] : string, count: number }
"#,
    );
}

#[test]
fn recovery_of_parenthesized_expressions() {
    reject("function foo(a, b. c) return a + b end");
    accept("function foo(a, b) return a + b end");
    reject("function foo(a, b: { a: number, b: number. c:number }) return a + b end");
    accept("function foo(a, b: { a: number, b: number }) return a + b end");
    reject("function foo(a, b): (number -> number return a + b end");
    accept("function foo(a, b): (number) -> number return a + b end");
    reject("function foo(a, b): (number, number -> number return a + b end");
    accept("function foo(a, b): (number) -> number return a + b end");
    reject("function foo(a, b): (number; number) -> number return a + b end");
    accept("function foo(a, b): (number) -> number return a + b end");
    reject("function foo(a, b): (number, number return a + b end");
    accept("function foo(a, b): (number, number) end");
    reject("local function foo(a, b): (number, number return a + b end");
    accept("local function foo(a, b): (number, number) end");
    reject("type F = (number, number -> number");
    accept("type F = (number, number) -> number");
    reject("function foo(a, b: { a: number, b: number) return a + b end");
    accept("function foo(a, b: { a: number, b: number }) return a + b end");
    reject("function foo(a, b: { [number: number}) return a + b end");
    accept("function foo(a, b: { [number]: number}) return a + b end");
    reject("local n: (string | number = 2");
    accept("local n: (string | number) = 2");
    reject(
        r#"
function foo(a, b
    return a + b
end
"#,
    );
    accept("function foo(a, b) return a + b end");
}

#[test]
fn incomplete_method_call() {
    reject(
        r#"
        function howdy()
            return game:
        end
    "#,
    );
}

#[test]
fn incomplete_method_call_2() {
    reject(
        r#"
        local game = { GetService=function(s) return 'hello' end }

        function a()
            game:a
        end
    "#,
    );
}

#[test]
fn incomplete_method_call_still_yields_an_ast_expr_index_name() {
    reject(
        r#"
        game:
    "#,
    );
}

#[test]
fn recover_confusables() {
    diagnostic(
        "local a = 4 != 10",
        12,
        14,
        "Unexpected '!='; did you mean '~='?",
    );
    diagnostic(
        "local a = true && false",
        15,
        17,
        "Unexpected '&&'; did you mean 'and'?",
    );
    diagnostic(
        "local a = false || true",
        16,
        18,
        "Unexpected '||'; did you mean 'or'?",
    );
    diagnostic(
        "local a = !false",
        10,
        11,
        "Unexpected '!'; did you mean 'not'?",
    );
    diagnostic(
        "local a = 4 ! = 10",
        12,
        13,
        "Expected identifier when parsing expression, got '!'",
    );
    diagnostic(
        "local a = true & & false",
        15,
        16,
        "Expected identifier when parsing expression, got '&'",
    );
    diagnostic(
        "local a = false | | true",
        16,
        17,
        "Expected identifier when parsing expression, got '|'",
    );
}

#[test]
fn capture_comments() {
    accept("\n        --!strict\n\n        local a = 5 -- comment one\n        local b = 8 -- comment two\n        --[[\n            Multi line comment\n        ]]\n        local c = 'see'\n    ");
}

#[test]
fn capture_broken_comment_at_the_start_of_the_file() {
    diagnostic(
        "--[[`",
        0,
        5,
        "Expected identifier when parsing expression, got unfinished comment",
    );
    reject(
        r#"
        --[[
    "#,
    );
}

#[test]
fn capture_broken_comment() {
    reject(
        r#"
        local a = "test"

        --[[broken!
    "#,
    );
}

#[test]
fn empty_function_type_error_recovery() {
    diagnostic(
        r#"
type Fn = (
    any,
    string | number | ()
) -> any
"#,
        44,
        46,
        "Expected '->' after '()' when parsing function type; did you mean 'nil'?",
    );
    diagnostic(
        "type Fn = (any, string | number | (number, number)) -> any",
        50,
        51,
        "Expected '->' when parsing function type, got ')'",
    );
    diagnostic(
        "type Fn = (any, string | number | <a>()) -> any",
        39,
        40,
        "Expected '->' when parsing function type, got ')'",
    );
    diagnostic(
        "type Fn = (any, string | number | <a...>()) -> any",
        42,
        43,
        "Expected '->' when parsing function type, got ')'",
    );
}

#[test]
fn generic_type_list_recovery() {
    diagnostic(
        r#"
local function foo<T..., U>(a: U, ...: T...): (U, ...T) return a, ... end
return foo(1, 2 -- to check for a second error after recovery
"#,
        27,
        28,
        "Generic types come before generic type packs",
    );
}

#[test]
fn recover_index_name_keyword() {
    reject(
        r#"
local b
local a = b.do
    "#,
    );
    reject(
        r#"
local b
local a = b.
do end
    "#,
    );
}

#[test]
fn recover_self_call_keyword() {
    reject(
        r#"
local b
local a = b:do
    "#,
    );
    reject(
        r#"
local b
local a = b:
do end
    "#,
    );
}

#[test]
fn recover_type_index_name_keyword() {
    reject(
        r#"
local A
local b : A.do
    "#,
    );
    reject(
        r#"
local A
local b : A.do
do end
    "#,
    );
}

#[test]
fn recover_expected_type_pack() {
    reject(
        r#"
type Y<T..., U = T...> = (T...) -> U...
    "#,
    );
}

#[test]
fn recover_unexpected_type_pack() {
    reject("type X<T...> = { a: T..., b: number }");
    reject("type Y<T> = { a: T..., b: number }");
    reject("type Z<T> = { a: string | T..., b: number }");
}

#[test]
fn recover_function_return_type_annotations() {
    diagnostic(
        "type F = (number): Custom<boolean, number, string>",
        17,
        18,
        "Return types in function type annotations are written after '->' instead of ':'",
    );
    diagnostic(
        "type G = Packed<(number): (string, number, boolean)>",
        24,
        25,
        "Return types in function type annotations are written after '->' instead of ':'",
    );
    diagnostic(
        "local function f(x: number) -> Custom<string, boolean, number> end",
        28,
        30,
        "Function return type annotations are written after ':' instead of '->'",
    );
}

#[test]
fn error_message_for_using_function_as_type_annotation() {
    diagnostic(r#"
        type Foo = function
    "#, 20, 28, "Using 'function' as a type annotation is not supported, consider replacing with a function type annotation e.g. '(...any) -> ...any'");
}

#[test]
fn get_a_nice_error_when_there_is_an_extra_comma_at_the_end_of_a_function_argument_list() {
    diagnostic(
        r#"
        foo(a, b, c,)
    "#,
        21,
        22,
        "Expected expression after ',' but got ')' instead",
    );
}

#[test]
fn get_a_nice_error_when_there_is_an_extra_comma_at_the_end_of_a_function_parameter_list() {
    diagnostic(
        r#"
        export type VisitFn = (
            any,
            Array<TAnyNode | Array<TAnyNode>>, -- extra comma here
        ) -> any
    "#,
        125,
        126,
        "Expected type after ',' but got ')' instead",
    );
}

#[test]
fn get_a_nice_error_when_there_is_an_extra_comma_at_the_end_of_a_generic_parameter_list() {
    diagnostic(
        r#"
        export type VisitFn = <A, B,>(a: A, b: B) -> ()
    "#,
        37,
        38,
        "Expected type after ',' but got '>' instead",
    );
}

#[test]
fn get_a_nice_error_when_there_is_no_comma_between_table_members() {
    diagnostic(
        r#"
        local t = {
            first = 1
            second = 2,
            third = 3,
            fouth = 4,
        }
    "#,
        55,
        61,
        "Expected ',' after table constructor element",
    );
}

#[test]
fn get_a_nice_error_when_there_is_no_comma_after_last_table_member() {
    diagnostic(
        r#"
        local t = {
            first = 1

        local ok = true
        local good = ok == true
    "#,
        52,
        57,
        "Expected '}' (to close '{' at line 2), got 'local'",
    );
}

#[test]
fn missing_default_type_pack_argument_after_variadic_type_parameter() {
    diagnostic(
        r#"
        type Foo<T... = > = nil
    "#,
        24,
        26,
        "Expected type, got '>'",
    );
}

#[test]
fn table_type_keys_cant_contain_nul() {
    diagnostic(
        r#"
        type Foo = { ["\0"]: number }
    "#,
        22,
        23,
        "String literal contains malformed escape sequence or \\0",
    );
}

#[test]
fn invalid_escape_literals_get_reported_but_parsing_continues() {
    diagnostic(
        r#"
        local foo = "\xQQ"
        print(foo)
    "#,
        21,
        27,
        "String literal contains malformed escape sequence",
    );
}

#[test]
fn unfinished_string_literals_get_reported_but_parsing_continues() {
    diagnostic(
        "local x = \"ab\n `x`",
        10,
        13,
        "Malformed string; did you forget to finish it?",
    );
    diagnostic(
        "return [=[`",
        7,
        11,
        "Malformed string; did you forget to finish it?",
    );
    accept("return \"a\\\r\nb\", `x`");
    accept("return \"a\\z\n b\", `x`");
    diagnostic(
        r#"
        local foo = "hi
        print(foo)
    "#,
        21,
        24,
        "Malformed string; did you forget to finish it?",
    );
}

#[test]
fn unfinished_string_literal_types_get_reported_but_parsing_continues() {
    diagnostic(
        r#"
        type Foo = "hi
        print(foo)
    "#,
        20,
        23,
        "Malformed string; did you forget to finish it?",
    );
}

#[test]
fn do_block_with_no_end() {
    reject(
        r#"
        do
    "#,
    );
}

#[test]
fn parse_interpolated_string_with_lookahead_involved() {
    accept(
        r#"
        local x = `{ {y} }`
    "#,
    );
}

#[test]
fn parse_interpolated_string_with_lookahead_involved2() {
    accept(
        r#"
        local x = `{ { y{} } }`
    "#,
    );
}

#[test]
fn parse_checked_outside_decl_fails() {
    diagnostic(
        r#"
    local @checked = 3
"#,
        11,
        19,
        "Expected identifier when parsing variable name, got 'checked'",
    );
}

#[test]
fn parse_checked_in_and_out_of_decl_fails() {
    diagnostic(
        r#"
    local @checked = 3
    @checked declare function abs(n: number): number
"#,
        11,
        19,
        "Expected identifier when parsing variable name, got 'checked'",
    );
}

#[test]
fn parse_checked_as_function_name_fails() {
    diagnostic(
        r#"
    @checked function(x: number) : number
    end
"#,
        22,
        23,
        "Expected identifier when parsing function name, got '('",
    );
}

#[test]
fn cannot_use_at_as_variable_name() {
    diagnostic(
        r#"
    local @blah = 3
"#,
        11,
        16,
        "Expected identifier when parsing variable name, got 'blah'",
    );
}

#[test]
fn read_write_table_properties() {
    accept("\n        type A = {read x: number}\n        type B = {write x: number}\n        type C = {read x: number, write x: number}\n        type D = {read: () -> string}\n        type E = {write: (string) -> ()}\n        type F = {read read: () -> string}\n        type G = {read write: (string) -> ()}\n\n        type H = {read [\"A\"]: number}\n        type I = {write [\"A\"]: string}\n\n        type J = {read [number]: number}\n        type K = {write [number]: string}\n    ");
}

#[test]
fn can_parse_leading_bar_unions_successfully() {
    accept("type A = | \"Hello\" | \"World\"");
}

#[test]
fn can_parse_leading_ampersand_intersections_successfully() {
    accept("type A = & { string } & { number }");
}

#[test]
fn mixed_leading_intersection_and_union_not_allowed() {
    diagnostic(
        "type A = & number | string | boolean",
        9,
        36,
        "Mixing union and intersection types is not allowed; consider wrapping in parentheses.",
    );
    diagnostic(
        "type A = | number & string & boolean",
        9,
        36,
        "Mixing union and intersection types is not allowed; consider wrapping in parentheses.",
    );
}

#[test]
fn grouped_function_type() {
    let chunk = parse_source(
        r#"
        type X<T> = T
        local x: X<(() -> ())?>
    "#,
    )
    .unwrap();
    let [Stmt::TypeAlias { .. }, Stmt::Local { bindings, init, .. }] = chunk.statements.as_slice()
    else {
        panic!("expected alias and local")
    };
    assert!(init.is_empty());
    let [binding] = bindings.as_slice() else {
        panic!("expected one binding")
    };
    assert_eq!(binding.name, "x");
    let Some(TypeExpr::Named { name, args, .. }) = &binding.annotation else {
        panic!("expected generic annotation")
    };
    assert_eq!(name, "X");
    let [TypeExpr::Optional { inner, .. }] = args.as_slice() else {
        panic!("expected optional parameter")
    };
    let TypeExpr::TypePack { types, .. } = &**inner else {
        panic!("expected grouped function")
    };
    assert!(matches!(types.as_slice(), [TypeExpr::Function { .. }]));
}

#[test]
fn complex_union_in_generic_ty() {
    let chunk = parse_source(
        r#"
        type X<T> = T
        local x: X<
            | number
            | boolean
            | string
        >
    "#,
    )
    .unwrap();
    let [Stmt::TypeAlias { .. }, Stmt::Local { bindings, init, .. }] = chunk.statements.as_slice()
    else {
        panic!("expected alias and local")
    };
    assert!(init.is_empty());
    let [binding] = bindings.as_slice() else {
        panic!("expected one binding")
    };
    assert_eq!(binding.name, "x");
    let Some(TypeExpr::Named { name, args, .. }) = &binding.annotation else {
        panic!("expected generic annotation")
    };
    assert_eq!(name, "X");
    let [TypeExpr::Union { types, .. }] = args.as_slice() else {
        panic!("expected union parameter")
    };
    let names: Vec<_> = types
        .iter()
        .map(|ty| match ty {
            TypeExpr::Named { name, .. } => name.as_str(),
            _ => panic!("expected type name"),
        })
        .collect();
    assert_eq!(names, ["number", "boolean", "string"]);
}

#[test]
fn function_name_has_correct_start_location() {
    let source = r#"
        function simple()
        end

        function T:complex()
        end
    "#;
    let chunk = parse_source(source).unwrap();
    let [Stmt::Function {
        target: AssignmentTarget::Local { span: simple, .. },
        ..
    }, Stmt::Function {
        target: AssignmentTarget::Field { span: method, .. },
        ..
    }] = chunk.statements.as_slice()
    else {
        panic!("expected plain function and method")
    };
    assert_eq!(&source[simple.start..simple.end], "simple");
    assert_eq!(&source[method.start..method.end], "T:complex");
}

#[test]
fn stat_end_includes_semicolon_position() {
    let source = r#"
        local x = 1
        local y = 2;
        local z = 3  ;
    "#;
    let chunk = parse_source(source).unwrap();
    let ends: Vec<_> = chunk
        .statements
        .iter()
        .map(|stmt| match stmt {
            Stmt::Local { span, .. } => span.end,
            _ => panic!("expected local"),
        })
        .collect();
    assert_eq!(
        ends,
        ["1", "2;", "3  ;"].map(|s| source.find(s).unwrap() + s.len())
    );
}

#[test]
fn parsing_string_union_indexers() {
    accept("type foo = { [\"bar\" | \"baz\"]: number }");
}

#[test]
fn parsing_incomplete_string_interpolation_missing_curly_at_eof() {
    diagnostic(
        "print(`{e.x} {e.a",
        16,
        17,
        "Malformed interpolated string; did you forget to add a '}'?",
    );
}

#[test]
fn parsing_incomplete_string_interpolation_missing_backtick_at_eof() {
    diagnostic(
        "print(`{e.x} {e.a}",
        17,
        18,
        "Malformed interpolated string; did you forget to add a '`'?",
    );
}

#[test]
fn parsing_incomplete_string_interpolation_missing_curly_with_backtick_at_eof() {
    diagnostic(
        "print(`{e.x} {e.a`",
        17,
        18,
        "Malformed interpolated string; did you forget to add a '}'?",
    );
}

#[test]
fn parsing_incomplete_string_interpolation_missing_curly_broken_string() {
    diagnostic(
        r#"print(`{e.x} {e.a
"#,
        16,
        17,
        "Malformed interpolated string; did you forget to add a '}'?",
    );
}

#[test]
fn parsing_incomplete_string_interpolation_missing_backtick_broken_string() {
    diagnostic(
        r#"print(`{e.x} {e.a}
"#,
        17,
        18,
        "Malformed interpolated string; did you forget to add a '`'?",
    );
}

#[test]
fn parsing_incomplete_string_interpolation_missing_curly_with_backtick_broken_string() {
    diagnostic(
        r#"print(`{e.x} {e.a`
"#,
        17,
        18,
        "Malformed interpolated string; did you forget to add a '}'?",
    );
}

#[test]
fn parse_type_name() {
    let chunk = parse_source("type F = <A>(A, string, boolean?) -> number").unwrap();
    let [Stmt::TypeAlias {
        value: TypeExpr::Function { params, result, .. },
        ..
    }] = chunk.statements.as_slice()
    else {
        panic!("expected function type")
    };
    assert_eq!(params.len(), 3);
    assert!(
        matches!(&**result, TypeExpr::Named { name, args, .. } if name == "number" && args.is_empty())
    );
}

#[test]
fn explicit_type_instantiation_errors() {
    diagnostic(
        "local a = x:a<<T>>",
        10,
        18,
        "Expected '(', '{' or <string> when parsing function call, got <eof>",
    );
}

#[test]
fn malformed_strings_report_the_first_error_in_context() {
    // Luau 0.709 Lexer.test.cpp's broken_string_works and testBrokenEscapeTolerant,
    // and their value, type, attribute, call and earlier-error contexts.
    for (source, start, end, expected, lexical) in [
        (
            r#"type T = {["\xQ"]: typeof(0xZ)}"#,
            26,
            29,
            "Malformed number",
            true,
        ),
        (
            r#"type T = {["\xQ"]: 0xZ }"#,
            18,
            22,
            "Expected type, got '0xZ'",
            false,
        ),
        (
            r#"type T = {["\xQ"]: "\xR"}"#,
            19,
            24,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            r#"type T = {["\xQ"]: }"#,
            18,
            20,
            "Expected type, got '}'",
            false,
        ),
        (
            r#"type T = {["\xQ"] number}"#,
            18,
            24,
            "Expected ':' when parsing table field, got 'number'",
            false,
        ),
        (
            "'\\3729472897292378'",
            0,
            19,
            "Expected identifier when parsing expression, got \"\\3729472897292378\"",
            false,
        ),
        (
            "[[",
            0,
            2,
            "Expected identifier when parsing expression, got malformed string",
            false,
        ),
        (
            "local = '\\3729472897292378'",
            6,
            7,
            "Expected identifier when parsing variable name, got '='",
            false,
        ),
        (
            "local = [[",
            6,
            7,
            "Expected identifier when parsing variable name, got '='",
            false,
        ),
        (
            "local '\\399'",
            6,
            12,
            "Expected identifier when parsing variable name, got \"\\399\"",
            false,
        ),
        (
            "if then return '\\399' end",
            3,
            7,
            "Expected identifier when parsing expression, got 'then'",
            false,
        ),
        (
            "'\\399' ~ 0xZ 0xZ end",
            0,
            6,
            "Expected identifier when parsing expression, got \"\\399\"",
            false,
        ),
        (
            "f:method [[",
            0,
            9,
            "Expected '(', '{' or <string> when parsing function call, got malformed string",
            false,
        ),
        (
            "fn [[",
            0,
            2,
            "Incomplete statement: expected assignment or a function call",
            false,
        ),
        (
            "return '\\3729472897292378'",
            7,
            26,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            "return [[",
            7,
            9,
            "Malformed string; did you forget to finish it?",
            true,
        ),
        (
            "return '\\399' 0xZ",
            7,
            13,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            "f '\\399'",
            2,
            8,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            "local x: '\\399'",
            9,
            15,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            "local x: [[",
            9,
            11,
            "Malformed string; did you forget to finish it?",
            true,
        ),
        (
            "local t = { ['\\399'] = 1 }",
            13,
            19,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            "type T = {['\\399']: string}",
            10,
            11,
            "String literal contains malformed escape sequence or \\0",
            false,
        ),
        (
            "@[deprecated '\\399'] function f() end",
            13,
            19,
            "String literal contains malformed escape sequence",
            true,
        ),
        (
            "return `p {'\\399'} end`",
            11,
            17,
            "String literal contains malformed escape sequence",
            true,
        ),
    ] {
        let (span, message, actual_lexical) = match parse_source(source).unwrap_err() {
            SourceError::Lex(e) => (e.span, e.message, true),
            SourceError::Parse(e) => (e.span, e.message, false),
        };
        assert_eq!(
            (span.start, span.end, message.as_str(), actual_lexical),
            (start, end, expected, lexical),
            "{source:?}"
        );
    }
}

#[test]
fn ordinary_strings_and_interpolation_report_the_first_error() {
    for (source, start, end, message) in [
        (
            r#"type T = {["\0"]: string}; local s = `\xR`"#,
            10,
            11,
            r"String literal contains malformed escape sequence or \0",
        ),
        (
            r#"type T = {["\xQ"]: string}; local s = `\xR`"#,
            10,
            11,
            r"String literal contains malformed escape sequence or \0",
        ),
        (
            r#"return `ok`; "no end"#,
            13,
            20,
            r#"Expected <eof>, got malformed string"#,
        ),
        (
            r#"return 1; "no end -- `tick`"#,
            10,
            27,
            r#"Expected <eof>, got malformed string"#,
        ),
        (
            r#"return "\xQP", `\xQ`"#,
            7,
            13,
            r#"String literal contains malformed escape sequence"#,
        ),
        (
            r#"return `ok`; [[unterminated"#,
            13,
            27,
            r#"Expected <eof>, got malformed string"#,
        ),
        (
            r#"local x = `ok`; return "\xQP", `\xQ`"#,
            23,
            29,
            r#"String literal contains malformed escape sequence"#,
        ),
        (
            r#"local a = `ok`; "\xQP"; print(`\xQ`)"#,
            16,
            22,
            r#"Expected identifier when parsing expression, got "\xQP""#,
        ),
        (
            r#"type T = {["\xQ"]: `\xR`}"#,
            19,
            24,
            r#"Interpolated string literal contains malformed escape sequence"#,
        ),
        (
            r#"if x "unfinished"#,
            5,
            16,
            r#"Expected 'then' when parsing if statement, got malformed string"#,
        ),
        (
            r#"return `{"\xQP"} \xQ`"#,
            9,
            15,
            r#"String literal contains malformed escape sequence"#,
        ),
        (
            r#"return `foo {'\372'} {`\xQ`}`"#,
            13,
            19,
            r#"String literal contains malformed escape sequence"#,
        ),
        (
            r#"if x then elseif y "unfinished"#,
            19,
            30,
            r#"Expected 'then' when parsing if statement, got malformed string"#,
        ),
        (
            r#"if true end"#,
            8,
            11,
            r#"Expected 'then' when parsing if statement, got 'end'"#,
        ),
        (
            r#"local = "\xQ"; return `\xR`"#,
            6,
            7,
            r#"Expected identifier when parsing variable name, got '='"#,
        ),
    ] {
        diagnostic(source, start, end, message);
    }
}
