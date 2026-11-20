# rustia-parser Package

## Overview

The parser is a recursive-descent frontend that produces a span-bearing Luau AST from lexer tokens. It parses type syntax but does not provide a type checker.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents AST and parser entry points.
- `parse_source(&str)` and `parse_source_bytes(&[u8])` return `Chunk` or `SourceError`.
- AST types include `Stmt`, `Expr`, `TypeExpr`, assignment operators/targets, function parameters, and table fields.

## Invariants

- Parser recursion is capped by `MAX_PARSE_DEPTH` (512).
- AST statements and expressions carry byte spans for diagnostics and compiler mapping.
- Type aliases and annotations are represented in the AST. Type function declarations validate the body and retain the declaration name, export status and span. Static type checking is out of scope here.

## Side Effects

Parsing reads source and allocates tokens/AST/error values. It performs no external I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-parser`.
