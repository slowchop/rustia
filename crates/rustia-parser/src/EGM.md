# Parser Source

## Overview

[lib.rs](./lib.rs) defines the AST, structured parse/source errors, and the recursive-descent parser used by the compiler. [diagnostics.rs](./diagnostics.rs) formats unexpected tokens and matched-delimiter locations. Source-based parsing borrows the original bytes so diagnostics preserve numeric spellings and raw string contents, including inside interpolation. The token-only `Parser::new` uses the decoded token values.

[attributes.rs](attributes.rs) parses and validates function annotations. [diagnostics.rs](diagnostics.rs) formats Luau token and delimiter errors.

## Interface

- `parse_source` and `parse_source_bytes` are the public parse entry points.
- `Chunk` contains statements. `Stmt`, `Expr`, and `TypeExpr` represent program syntax, including type grammar.
- `ParseError` contains parse failure details; `SourceError` distinguishes lexer and parser failures.

## Invariants

- Generic function types parse an erased binder list before the parameter list, require an arrow, accept named/unnamed fixed parameters and one final pack tail, and include the binder in their source span. Function declaration and function type binders share regular-before-pack ordering and reject defaults; no type checker or new runtime representation is introduced.

- Alias bodies and generic defaults require single types and are parsed then erased, with regular parameters before packs and a default required after the first default. Pack defaults have explicit fixed/tail grammar. Assertions and regular defaults require a single type; grouped singles, function types and named generic pack arguments retain their own contexts. Table types validate named/string properties, one indexer, contextual read/write qualifiers and standalone array shorthand; nested types use the shared single-type grammar. Broader syntax parity remains a separate acceptance gate.

- `typeof(expr)` uses the ordinary expression parser with its normal depth/span checks, then discards the expression AST. Interpolation is legal inside the expression; invalid expressions reject before execution.

- An optionally exported `type function` declaration validates its ordinary function signature and body.
  The parser tracks lexical locals in chunks containing this syntax. A type function can reference
  globals and its own locals. A nested type function establishes a new capture boundary. The AST
  retains the declaration name, export status and span.
- Local variables, fixed parameters and numeric or generic for-loop binders share the optional
  single-type annotation grammar in value and type functions. Vararg annotations also accept a
  bare named type pack such as `A...`. Loop variables enter scope after all header expressions.
- Contextual `export type` also supports ordinary type aliases; `expr :: T` parses the type and lowers to a single-result parenthesized expression without runtime checks. Named types accept the module-qualified `Module.Type` form and optional generic arguments.

- Value-function return annotations distinguish a single type from a return pack. Suffixes require exactly one fixed entry; the pinned Luau 0.709 one-fixed-plus-tail suffix quirk is retained. Bare tails cannot take suffixes and nested bare packs reject. Suffixes consume the already-parsed node without backtracking; a test-only visit counter enforces one primary visit per nested type node. Leading union/intersection operators are supported; mixed unparenthesized operators reject.

- Recursive parser paths share a 512-frame budget across statements, types, expressions and function signatures, alongside local depth checks. Erased `typeof` expressions and type function declarations validate function-body loop/vararg context before erasure; signatures keep the enclosing context and ordinary invalid functions retain their compiler error phase.
- Explicit type instantiation uses the same type-or-pack argument grammar as named types,
  including empty lists. An `Expr::TypeInstantiation` retains its operand and source extent and yields one
  value. It can be indexed, called or used as a value. An instantiated method requires
  call arguments.
- Function and method calls reject an opening argument parenthesis on a later source line.
  For explicit type arguments, regular calls compare the opening parenthesis to the closing
  type delimiter. Method calls compare it to the method name. String and table arguments may
  start on a later line.
- Unexpected bad string tokens produce grammar errors using the raw spelling or unfinished-string label. Reading a malformed literal in a value or type position preserves its lexical error phase. A quoted table-type property checks the colon and value type before reporting a bad property name. Interpolation lowering preserves the ordinary string tokens it has already read so an error in a later section can be diagnosed in order.
- AST source spans are byte offsets inherited from lexer tokens. Statement spans include an optional trailing semicolon.
- Type syntax is parsed and retained as syntax; this module does not type-check it.

## Side Effects

The parser performs in-memory tokenization and AST construction and returns errors as values.

## Verification

From the repository root, run `cargo test --locked -p rustia-parser`. The [upstream inventory](../../../conformance/upstream/luau-0.709/parser-cases.tsv) maps pinned tests to Rustia coverage.
