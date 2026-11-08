# Parser Source

## Overview

[lib.rs](./lib.rs) defines the AST, structured parse/source errors, and the recursive-descent parser used by the compiler.

## Interface

- `parse_source` and `parse_source_bytes` are the public parse entry points.
- `Chunk` contains statements. `Stmt`, `Expr`, and `TypeExpr` represent program syntax, including type grammar.
- `ParseError` contains parse failure details; `SourceError` distinguishes lexer and parser failures.

## Invariants

- Contextual `export type` uses the alias grammar; `expr :: T` parses the type and lowers to a single-result parenthesized expression without runtime checks. Named types accept the module-qualified `Module.Type` form and optional generic arguments.

- Recursive parser paths enforce a maximum depth of 512.
- AST source spans are byte offsets inherited from lexer tokens.
- Type syntax is parsed and retained as syntax; this module does not type-check it.

## Side Effects

The parser performs in-memory tokenization and AST construction and returns errors as values.

## Verification

From the repository root, run `cargo test --locked -p rustia-parser`.
