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
- Tokenization appends one terminal EOF token.
- Numeric spellings become f64 token values; runtime compatibility remains governed by the pinned oracle and downstream semantics.

## Side Effects

Scanning is in-memory over borrowed text/bytes and allocates the returned tokens.

## Verification

From the repository root, run `cargo test --locked -p rustia-lexer`.
