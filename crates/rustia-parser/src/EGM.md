# Parser Source

## Overview

[lib.rs](./lib.rs) defines the AST, structured parse/source errors, and the recursive-descent parser used by the compiler.

## Interface

- `parse_source` and `parse_source_bytes` are the public parse entry points.
- `Chunk` contains statements. `Stmt`, `Expr`, and `TypeExpr` represent program syntax, including type grammar.
- `ParseError` contains parse failure details; `SourceError` distinguishes lexer and parser failures.

## Invariants

- Generic function types parse an erased binder list before the parameter list, require an arrow, accept named/unnamed fixed parameters and one final pack tail, and include the binder in their source span. Function declaration and function type binders share regular-before-pack ordering and reject defaults; no type checker or new runtime representation is introduced.

- Alias bodies and generic defaults require single types and are parsed then erased, with regular parameters before packs and a default required after the first default. Pack defaults have explicit fixed/tail grammar. Assertions and regular defaults require a single type; grouped singles, function types and named generic pack arguments retain their own contexts. Table types validate named/string properties, one indexer, contextual read/write qualifiers and standalone array shorthand; nested types use the shared single-type grammar. Broader syntax parity remains a separate acceptance gate.

- `typeof(expr)` uses the ordinary expression parser with its normal depth/span checks, then discards the expression AST. Interpolation is legal inside the expression; invalid expressions reject before execution.

- Contextual `export type` uses the alias grammar; `expr :: T` parses the type and lowers to a single-result parenthesized expression without runtime checks. Named types accept the module-qualified `Module.Type` form and optional generic arguments.

- Value-function return annotations distinguish a single type from a return pack. Suffixes require exactly one fixed entry; the pinned Luau 0.709 one-fixed-plus-tail suffix quirk is retained. Bare tails cannot take suffixes and nested bare packs reject. Suffixes consume the already-parsed node without backtracking; a test-only visit counter enforces one primary visit per nested type node. Leading union/intersection operators are supported; mixed unparenthesized operators reject.

- Recursive parser paths share a 512-frame budget across statements, types, expressions and function signatures, alongside local depth checks. typeof expression mode validates function-body loop/vararg context before erasure; signatures keep the enclosing context and ordinary invalid functions retain their compiler error phase.
- AST source spans are byte offsets inherited from lexer tokens.
- Type syntax is parsed and retained as syntax; this module does not type-check it.

## Side Effects

The parser performs in-memory tokenization and AST construction and returns errors as values.

## Verification

From the repository root, run `cargo test --locked -p rustia-parser`.
