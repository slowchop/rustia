# rustia-lexer Package

## Overview

The lexer converts Luau source bytes into tokens with byte-offset spans. It is a standalone frontend crate used by the parser and compiler.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents the token model and scan entry points.
- `lex(&str)` and `lex_bytes(&[u8])` return decoded tokens with byte spans. The parser uses `lex_for_parser(&[u8])` to retain otherwise invalid tokens for syntax diagnostics.
- Public data: `Span`, `Token`, `TokenKind`, `LexError`, and `Result`.

## Invariants

- Span offsets are byte positions in the input.
- String tokens carry bytes, so source string data is not forced through UTF-8 conversion.
- Well-formed comments are skipped. An unfinished block comment produces a span-bearing error for standalone callers and an invalid token for parser diagnostics. A successful scan ends with one EOF token.

## Side Effects

The lexer reads borrowed input and allocates token and error data. It performs no external I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-lexer`. The [pinned lexer ports](./tests/luau_0709.rs) check token boundaries, decoded bytes and comment handling.
