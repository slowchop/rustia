//! Observable lexer contracts from Luau 0.709 tests/Lexer.test.cpp.
//! Adapted under MIT from b968ef742741bb2b703afc3b3c53f06608c87481.
//! See conformance/upstream/luau-0.709/LICENSE.txt.

use rustia_lexer::{lex, lex_for_parser, Span, Token, TokenKind};

/// Both entry points return the same token stream for well-formed input.
fn check_tokens(source: &str, expected: &[(TokenKind, Span)]) {
    for tokens in [lex(source), lex_for_parser(source.as_bytes())] {
        let tokens = tokens.unwrap();
        let (eof, body) = tokens.split_last().expect("terminal EOF");
        assert_eq!(body.len(), expected.len(), "{source:?}");
        for (token, (kind, span)) in body.iter().zip(expected) {
            assert_eq!((&token.kind, &token.span), (kind, span), "{source:?}");
        }
        assert_eq!(eof.kind, TokenKind::Eof, "{source:?}");
        assert_eq!(
            eof.span,
            Span::new(source.len(), source.len()),
            "{source:?}"
        );
    }
}

fn check_parser_error_token(source: &str, kind: TokenKind) {
    let end = source.len();
    assert_eq!(
        lex_for_parser(source.as_bytes()).unwrap(),
        [
            Token {
                kind,
                span: Span::new(0, end)
            },
            Token {
                kind: TokenKind::Eof,
                span: Span::new(end, end)
            },
        ],
        "{source:?}"
    );
}

#[test]
fn unfinished_block_string_and_comment() {
    let source = "[[";
    let error = lex(source).unwrap_err();
    assert_eq!(error.span, Span::new(0, 2));
    assert_eq!(
        error.message,
        "Malformed string; did you forget to finish it?"
    );
    check_parser_error_token(source, TokenKind::UnfinishedString);

    let source = "--[[  ";
    let error = lex(source).unwrap_err();
    assert_eq!(error.span, Span::new(0, 6));
    assert_eq!(error.message, "unfinished comment");
    check_parser_error_token(source, TokenKind::Invalid("unfinished comment".into()));
}

#[test]
fn comments_end_at_the_matching_delimiter() {
    check_tokens("--  ", &[]);
    check_tokens(
        "--[[ function \n]] end",
        &[(TokenKind::End, Span::new(18, 21))],
    );
    for source in [
        "--[===[\n\n\n\n]===]",
        "--[[ test ]]",
        "--[=[ μέλλον ]=]",
        "--[==[ test ]==]",
    ] {
        check_tokens(source, &[]);
        // A same-line keyword distinguishes a real closer from a line comment.
        let after = format!("{source}end");
        check_tokens(
            &after,
            &[(TokenKind::End, Span::new(source.len(), after.len()))],
        );
    }
}

#[test]
fn names_and_keywords_around_a_skipped_comment() {
    check_tokens(
        "foo --[[ comment ]] bar : nil end",
        &[
            (TokenKind::Ident("foo".into()), Span::new(0, 3)),
            (TokenKind::Ident("bar".into()), Span::new(20, 23)),
            (TokenKind::Colon, Span::new(24, 25)),
            (TokenKind::Nil, Span::new(26, 29)),
            (TokenKind::End, Span::new(30, 33)),
        ],
    );
}

#[test]
fn malformed_decimal_escape_preserves_the_quoted_boundary() {
    let source = r"'\3729472897292378'";
    let error = lex(source).unwrap_err();
    assert_eq!(error.message, "decimal string escape exceeds 255");
    assert_eq!(error.span, Span::new(0, 5));

    // The parser needs both the full quote extent and raw text to choose
    // the first diagnostic for the position where the token appears.
    check_parser_error_token(
        source,
        TokenKind::MalformedStringEscape(br"\3729472897292378".to_vec()),
    );
}

#[test]
fn quoted_and_long_strings_keep_bytes_and_token_extents() {
    for (source, value) in [
        ("'test'", "test"),
        ("\"test\"", "test"),
        ("[[ test ]]", " test "),
        ("[[ test\n    ]]", " test\n    "),
        ("[[\n    test\n    ]]", "    test\n    "),
        ("[[\n    test ]]", "    test "),
        ("[=[[%s]]=]", "[%s]"),
        ("[==[ test ]==]", " test "),
        ("[==[ test\n    ]==]", " test\n    "),
        ("[==[\n    test\n    ]==]", "    test\n    "),
        ("[==[\n\n    test ]==]", "\n    test "),
    ] {
        check_tokens(
            source,
            &[(
                TokenKind::String(value.as_bytes().to_vec()),
                Span::new(0, source.len()),
            )],
        );
    }
}

#[test]
fn malformed_string_keeps_later_tokens_and_actual_eof() {
    let source = r"'\399' ~ 0xZ 1.2.3 end";
    let tokens = lex_for_parser(source.as_bytes()).unwrap();
    let expected = [
        (TokenKind::MalformedStringEscape(br"\399".to_vec()), 0, 6),
        (TokenKind::Invalid("'~'".to_owned()), 7, 8),
        (TokenKind::MalformedNumber("0xZ".to_owned()), 9, 12),
        (TokenKind::MalformedNumber("1.2.3".to_owned()), 13, 18),
        (TokenKind::End, 19, 22),
        (TokenKind::Eof, source.len(), source.len()),
    ];
    assert_eq!(tokens.len(), expected.len());
    for (token, (kind, start, end)) in tokens.iter().zip(expected) {
        assert_eq!(token.kind, kind);
        assert_eq!(token.span, Span::new(start, end));
    }
}
