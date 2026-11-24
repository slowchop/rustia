# Compiler Source

## Overview

[lib.rs](./lib.rs) contains the compiler API and lowering passes. [builtins.rs](./builtins.rs) contains recognized global/library names that select specialized call evaluation order. [o2.rs](./o2.rs) contains default-disabled optimization rewrites and logical receipts.

## Interface

- `compile(&Chunk) -> Result<Program>` uses default options.
- `compile_with_options` accepts source metadata and PUC constructor-order settings.
- `compile_with_options_and_receipts` also returns `ProgramReceipts` for rewritten code.
- `CompileError` carries an optional lexer span; `BuiltinEnvironment` separates public and conformance compilation modes.
- `ExperimentalO2Options::reachability_exact()` selects the documented corpus-green rewrite track; `ALL` includes dead cleanup-clear elision, which the source marks as blocked on root-liveness work.

- [compiled_call.rs](./compiled_call.rs) emits safe Rust for a numeric closure with one numeric argument and one host-supplied capture. The example exposes this experimental contract to local development tools.

## Invariants

- Source lines for calls follow the callee end or method name. Indexed stores retain the target location across RHS evaluation. Iterator callback lines identify the loop header.
- [repeat_condition.rs](./repeat_condition.rs) validates skipped bindings using the default Luau 0.709 compiler rules. A direct reference in `until` is checked lexically. A nested function contributes its surviving captures. Captured folded values reach the condition even on a jump over their declarations.
- Type function declarations emit no bytecode, constants or function prototypes for their erased bodies.
- Capture discovery is a whole-tree prepass so assignment sites account for captures and source-later global writes.
- Each compiled capture retains its lexical name at the matching upvalue-descriptor index, including transitive captures and direct immutable values. Source provenance stays in `Program::source_name` and function debug metadata. Names do not change capture execution or cell sharing.
- Capture interning consults existing descriptors after lexical local-name maps
  are restored. Repeated branch/loop reads and transitive closures reuse one
  capture slot for the same parent binding, without merging shadowed bindings.
- O2 is all-off by default. Rewrites must preserve evaluation/error order and logical schedule.
- Do not treat the experimental option fields as a stable public compiler contract.

- The interpreter compiler ignores validated function attributes. Explicit instantiation evaluates its operand once to a single result. As a method receiver or callee, that value is read before the arguments. A grouped local receiver still follows Luau’s late register read. Writes through instantiated globals still invalidate affected builtin fast paths.

## Side Effects

Compilation is in-memory and does not execute the resulting program or perform host I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-compiler`.
`tests/repeat_condition.rs` covers skipped binding identity, source spans, constant captures and literal loadstring recovery.
`tests/capture_metadata.rs` covers names, source provenance, shadowing, transitive captures, detached chunks, and execution equivalence with names removed.
It also covers branch restoration and repeated scoped reads which previously
duplicated names and could exhaust the distinct-upvalue limit.

Run `cargo test --locked -p rustia-compiler --test compiled_call` for source diagnostic and generated-code equivalence.
