# rustia-lexer Package

## Overview

The lexer converts Luau source bytes into tokens with byte-offset spans. It is a standalone frontend crate used by the parser and compiler.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents the token model and scan entry points.
- Public functions: `lex(&str)` and `lex_bytes(&[u8])`.
- Public data: `Span`, `Token`, `TokenKind`, `LexError`, and `Result`.

## Invariants

- Span offsets are byte positions in the input.
- String tokens carry bytes, so source string data is not forced through UTF-8 conversion.
- A successful scan ends with an EOF token; lexical failures return a span-bearing error.

## Side Effects

The lexer reads borrowed input and allocates token and error data. It performs no external I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-lexer`.
