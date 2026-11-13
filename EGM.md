# rustia Blueprint

## System Overview

Rustia implements a portable Luau interpreter in Rust. Source moves through the lexer,
parser, and compiler into a bytecode VM; the `rustia` facade supplies application APIs
and the CLI supplies file/stdin/REPL execution. Selectable environment contracts share
one engine. The workspace is private and unpublished, with private self-hosted correctness CI under
the explicit source profile. Public benchmark automation and publication remain paused.
A separate private maintainer bridge supports only manually dispatched, reviewed
exact-cohort runtime comparisons; its outputs stay private and do not authorize public
performance claims. Local performance measurements remain prohibited. The pinned
conformance corpus does not establish full Luau parity or release readiness.

Read [AGENTS.md](./AGENTS.md), [CURRENT](./design/CURRENT.md), the
[pillars](./design/02-pillars.md), and the relevant design documents before changes.
These maps describe implementation boundaries and navigation. They do not supersede
the canonical state, design contracts, or pinned Luau 0.709 oracle.

## Interface

The [workspace manifest](./Cargo.toml) contains seven runtime/tooling crates and the
benchmark harness. Follow these maps from the root toward the file being edited:

| Boundary | Map | Responsibility |
| --- | --- | --- |
| Runtime and embedding | [crates](./crates/EGM.md) | Lexer, AST/parser, compiler, VM, facade, CLI, conformance runner. |
| Benchmark sources and artifacts | [bench](./bench/EGM.md) | Workloads, engine adapters, memory protocols, analysis, and report generation. |
| Repository tooling | [tools](./tools/EGM.md) | Validation, release/package audits, pinned oracle tooling, and controlled benchmark operations. |
| Fuzzing | [fuzz](./fuzz/EGM.md) | Separate workspace for bounded frontend/VM, stateful embedding, and differential fuzzing. |
| Browser examples | [demos](./demos/EGM.md) | Bare-Wasm standard demo and wasm-bindgen recipe using JavaScript services. |
| Historical experiments | [spikes](./spikes/EGM.md) | Isolated architecture experiments, outside production workspace guarantees. |
| Design support code | [design](./design/EGM.md) | Progress renderer and navigation to authoritative specifications. |

The latest [drift report](./2026-10-29_DRIFT_REPORT.md) records this static design/source audit,
applied corrections and memory-protocol gaps at the audit date. The prospective v2 fixes and
pending measurement acceptance are tracked in [memory benchmarking](./design/57-memory-benchmarking.md).
The drift report is not release acceptance.

The test-only [conformance corpus](./conformance/README.md) is indexed by
[MANIFEST.tsv](./conformance/MANIFEST.tsv) and [REFS.tsv](./conformance/REFS.tsv).
Tests, fixture-only trees, vendored code, generated output, ignored directories, and
symlinks have no separate maps. Hidden configuration is outside the bootstrap tree. Self-hosted
CI workflows under [.github/workflows](./.github/workflows/) are operational correctness
entry points; the macOS arm64 runtime job uses its account-provisioned Rustup to install the
pinned toolchain and target without system package writes, then verifies the native host
before running shared correctness suites. A private manual-only real Firefox job uses the existing Linux Docker correctness pool with checksum-pinned official browser/driver archives inside its disposable container. The [Justfile](./Justfile) remains the local
validation entry point. The ignored private sibling is a separate repository and is outside
this map.
The hosted Linux workspace job additionally runs the default-off executor accounting
diagnostic. Its fixed workload path counts are CI-only and carry no timing claim.

## Invariants

- Correctness workflows use ordinary self-hosted groups only, with private-repository
  and same-repository PR admission. Benchmark access remains in the separate private
  bridge; public visibility requires a new isolated execution decision.

- `standard()` has reproducible defaults and permits explicit nondeterministic providers;
  `deterministic()` enforces its replay contract; `live()` intentionally provides live
  services. Hardened leaves narrow the deterministic guest surface.
- Public workspace crates inherit `unsafe_code = "forbid"`. Deterministic numeric,
  iteration, fuel, and collection rules belong to the selected contract and versioned ABI.
- The application facade owns its settings/errors and retained handles. VM internals
  remain separate lower-level APIs; handles must respect VM ownership and decision epochs.
- Host-owned authoritative state and external effects are not silently rolled back with
  guest state. Typed invocation journals and guest installation savepoints have distinct
  boundaries.
- Performance evidence comes only from `ci-bench`; local correctness/static checks are
  not measurements. Do not run benchmark binaries or timing/profiling scripts locally.
- GitHub Issues owns source-development work status and coordination; AGENTS.md and CONTRIBUTING.md define the issue-to-PR workflow. CURRENT links to the backlog and remains authoritative for built state and constraints. Issue/PR templates live under `.github/`.
- Keep affected maps and parent links current alongside code. Preserve human-facing
  READMEs and report disagreements with authoritative documents explicitly.

## Side Effects

The engine allocates and mutates VM-owned state and invokes explicitly configured host
services. CLI and conformance tooling read files and emit output. Repository tools may
write generated artifacts, launch processes, operate remote benchmark infrastructure,
or publish results when specifically invoked and authorized. Reading a map grants no
authority for those operations.

## Verification

Run commands from the repository root. `cargo test --locked --workspace` checks workspace
correctness; `cargo fmt --all -- --check` checks Rust formatting. Follow each component's
map for narrower checks. Documentation-only map changes require section, source-reference,
link, scope, and diff validation; they do not require benchmark execution.
