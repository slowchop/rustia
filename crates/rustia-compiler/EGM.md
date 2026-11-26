# rustia-compiler Package

## Overview

The compiler converts the parser's Luau AST into `rustia-vm` bytecode. It is a lower-level engine package; application users normally compile through the `rustia` facade.

Package metadata: [Cargo.toml](./Cargo.toml).

## Interface

- [`src/EGM.md`](./src/EGM.md) documents compile entry points and lowering modules.
- Depends on the lexer, parser, and VM crates. Source and graph SHA-256 identities use a private compiler implementation so digest support does not expand the public facade inventory. The opt-in generated-call example emits Rust for a bounded, numeric closure.
- The opt-in captured-host-call driver emits Rust for a bounded callback using host-owned handles.
- The experimental source-bundle boundary accepts canonical in-memory module keys and bytes, preserves the live compiler/host SHA-256 identity over sorted paths and exact source bytes, and derives a separate deterministic dependency-first graph fingerprint for top-level literal `require` declarations. File discovery and project-path policy remain embedder-owned.

## Invariants

- The compiler discovers captured locals and syntactic global writes before final lowering.
- Default-disabled O2 passes do not alter the default bytecode schedule.
- Logical receipt metadata lets optimized code retain the baseline logical instruction/fuel and deterministic-collection schedule.
- Luau type syntax is parsed as runtime-erased metadata; this compiler does not type-check programs.
- Static module graphs parse reachable sources only, reject missing modules and cycles with the importing source span, and permit an explicit resolver when an embedder's path policy differs from Rustia's default relative resolver.
- Stateless function-graph admission rejects persistent module state, mutable captures, host effects, indirect calls and nested closures before a backend is selected. The embedder still owns fallback and activation.

## Side Effects

Compilation allocates AST/lowering/bytecode data and returns structured compile errors; it does not execute guest code or call host services.

## Verification

From the repository root, run `cargo test --locked -p rustia-compiler`.
`tests/source_bundle.rs` covers canonical ordering and identity, nested and shared imports, missing modules, cycles, explicit resolver policy, source locations, and unreachable invalid sources.
`tests/stateless_helper.rs` covers dependency-first stateless admission, host resolver policy, state/effect rejection, builtin shadowing, and indirect calls.
The feature-gated `tests/gc_accounting_paths.rs` test uses the actual closure and
generic-iteration workloads to report setup, warmup and following-call root-accounting/executor
counters at fixed work sizes. It asserts exact checksums and emits stable rows,
but performs no timing; run it only through its explicit release-mode
`executor-diagnostics` feature command in the hosted Linux workspace CI job.
