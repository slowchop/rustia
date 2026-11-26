# Crates Blueprint

## System Overview

`crates/` groups the interpreter's public application facade, lower-level engine crates, command-line runner, and conformance tooling. The implementation is a deterministic-by-contract Luau interpreter with multiple selectable profiles. The code and [`design/CURRENT.md`](../design/CURRENT.md) describe the built state; this tree does not establish full Luau parity or supported-release readiness.

## Interface

The workspace packages and their boundaries are documented in their package maps.

## Subdirectories & System Boundaries

- [`rustia`](./rustia/EGM.md): application-facing builder, execution, conversion, and embedding API.
- [`rustia-cli`](./rustia-cli/EGM.md): `rustia` command-line runner and REPL.
- [`rustia-compiler`](./rustia-compiler/EGM.md): AST-to-bytecode compilation and experimental numeric Rust generation.
- [`rustia-conformance`](./rustia-conformance/EGM.md): pinned-corpus runner and reporting library.
- [`rustia-lexer`](./rustia-lexer/EGM.md): tokenization and byte spans.
- [`rustia-parser`](./rustia-parser/EGM.md): recursive-descent parser and AST.
- [`rustia-vm`](./rustia-vm/EGM.md): bytecode runtime and engine-level primitives.

## Invariants

- The public `rustia` facade is the application boundary; compiler and VM controls are lower-level 0.x contracts. Its opt-in `send` feature selects `rustia-vm/send`, making the facade and live handles movable at idle ownership boundaries while every call still requires exclusive access. Retained-entry construction, capture inspection and scoped re-entry remain consumer-proven lower-level seams, not facade-wide guarantees.
- [`design/CURRENT.md`](../design/CURRENT.md) is authoritative when staged design snapshots differ from the current implementation.
- Deterministic execution is an explicitly selected contract. `standard()` and `live()` retain the general embedding surface, with policies and defaults selected by profile.
- Luau 0.709 is the pinned compatibility oracle. Passing corpus rows do not imply full language parity.
- Do not run performance benchmarks on a developer machine. Only the private `ci-bench` service provides benchmark evidence.

## Side Effects

The libraries execute guest code and configured host callbacks; the CLI reads scripts/modules and writes output; the conformance runner reads corpus files and executes fixtures. Service callbacks can perform external I/O that guest-state rollback cannot undo.

## Verification

From the repository root, run `cargo test --locked --workspace` for workspace correctness. This is not benchmark evidence.
