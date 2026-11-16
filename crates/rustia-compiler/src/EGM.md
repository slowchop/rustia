# Compiler Source

## Overview

[lib.rs](./lib.rs) contains the compiler API and lowering passes. [builtins.rs](./builtins.rs) contains recognized global/library names that select specialized call evaluation order. [o2.rs](./o2.rs) contains default-disabled optimization rewrites and logical receipts.

## Interface

- `compile(&Chunk) -> Result<Program>` uses default options.
- `compile_with_options` accepts source metadata and PUC constructor-order settings.
- `compile_with_options_and_receipts` also returns `ProgramReceipts` for rewritten code.
- `CompileError` carries an optional lexer span; `BuiltinEnvironment` separates public and conformance compilation modes.
- `ExperimentalO2Options::reachability_exact()` selects the documented corpus-green rewrite track; `ALL` includes dead cleanup-clear elision, which the source marks as blocked on root-liveness work.

## Invariants

- Capture discovery is a whole-tree prepass so assignment sites account for captures and source-later global writes.
- Each compiled capture retains its lexical name at the matching upvalue-descriptor index, including transitive captures and direct immutable values. Source provenance stays in `Program::source_name` and function debug metadata. Names do not change capture execution or cell sharing.
- Capture interning consults existing descriptors after lexical local-name maps
  are restored. Repeated branch/loop reads and transitive closures reuse one
  capture slot for the same parent binding, without merging shadowed bindings.
- O2 is all-off by default. Rewrites must preserve evaluation/error order and logical schedule.
- Do not treat the experimental option fields as a stable public compiler contract.

## Side Effects

Compilation is in-memory and does not execute the resulting program or perform host I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-compiler`.
`tests/capture_metadata.rs` covers names, source provenance, shadowing, transitive captures, detached chunks, and execution equivalence with names removed.
It also covers branch restoration and repeated scoped reads which previously
duplicated names and could exhaust the distinct-upvalue limit.
