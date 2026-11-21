# Lexer Source

## Overview

[lib.rs](./lib.rs) defines the public token vocabulary and a hand-written scanner for source bytes. It recognizes Luau keywords, literals, operators, and punctuation.

## Interface

- `Span { start, end }` stores byte offsets.
- `Token { kind, span }` pairs a token with its source extent.
- `TokenKind` covers keywords, identifiers, numeric and byte-string literals, operators, delimiters, and EOF.
- `lex` and `lex_bytes` return `Result<Vec<Token>>`; the crate's `Result<T>` alias uses `LexError`.

## Invariants

- Invalid bytes and malformed literal forms return structured errors with source spans.
- Tokenization appends one terminal EOF token. The parser-facing scanner retains unrecognized punctuation and Unicode as tokens for context-sensitive diagnostics. [confusables.rs](./confusables.rs) maps codepoints to the pinned Luau ASCII suggestions.
- The parser-facing scanner retains malformed quoted and unfinished string tokens with full spans. After a malformed string it keeps later tokens through EOF, including malformed numerals, so the parser can decide which error occurs first. Direct lexer calls still report the first lexical failure.
- Long-bracket strings skip one initial LF or CRLF, normalize later CRLF pairs to LF, and preserve standalone CR bytes.
- Unrecognized string escapes preserve the escaped byte. Numeric and Unicode escapes validate their contents.
- Binary and hex literals saturate at u64::MAX before nearest-even conversion to f64. Other numeric spellings also become f64 token values; runtime compatibility remains governed by the pinned oracle and downstream semantics.

## Side Effects

Scanning is in-memory over borrowed text/bytes and allocates the returned tokens.

## Verification

From the repository root, run `cargo test --locked -p rustia-lexer`.
