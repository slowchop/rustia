# rustia Blueprint

## System Overview

Rustia implements a portable Luau interpreter in Rust. Source moves through the lexer,
parser, and compiler into a bytecode VM; the `rustia` facade supplies application APIs
and the CLI supplies file/stdin/REPL execution. Selectable environment contracts share
one engine. The experimental source release uses standard GitHub-hosted correctness
CI and keeps all eight workspace crates unpublished. Public benchmark automation
and publication remain paused.

A separate private maintainer bridge supports only manually dispatched, reviewed
exact-cohort runtime comparisons and the separately reviewed #33 harness-lifetime
repeatability controls; its outputs stay private and do not authorize public
performance claims. Local performance measurements remain prohibited. The pinned
conformance corpus does not establish full Luau parity or release readiness.

Read [AGENTS.md](./AGENTS.md), [CURRENT](./design/CURRENT.md), the
[pillars](./design/02-pillars.md), and the relevant design documents before changes.
These maps describe implementation boundaries and navigation. They do not supersede
the canonical state, design contracts, or pinned Luau 0.709 oracle.

## Interface

The [hosted CI inventory](.github/public-ci/EGM.md) covers required correctness
and differential fuzz checks. Both workflows ran on the public visibility event.
The [launch decision](design/public-source-activation.md) describes the release.
The [3 July 2026 status page](status-report.html) archives the early implementation plan. [SECURITY.md](SECURITY.md) identifies
the private vulnerability reporting route.

The [workspace manifest](./Cargo.toml) contains seven runtime/tooling crates and the
benchmark harness. Follow these maps from the root toward the file being edited:

| Boundary | Map | Responsibility |
| --- | --- | --- |
| Runtime and embedding | [crates](./crates/EGM.md) | Lexer, AST/parser, compiler, VM, facade, CLI, conformance runner. |
| Benchmark sources and artifacts | [bench](./bench/EGM.md) | Workloads, engine adapters, memory protocols, analysis, and report generation. |
| Repository tooling | [tools](./tools/EGM.md) | Validation, release/package audits, pinned oracle tooling, and controlled benchmark operations. |
| Fuzzing | [fuzz](./fuzz/EGM.md) | Separate workspace for bounded frontend/VM, stateful embedding, and differential fuzzing. |
| Browser examples | [demos](./demos/EGM.md) | Bare-Wasm standard demo, wasm-bindgen services recipe, and a recorded native/WebKit race replay. |
| Static website | [website](./website/EGM.md) | Browser homepage, playground and static build. |
| Historical experiments | [spikes](./spikes/EGM.md) | Isolated architecture experiments, outside production workspace guarantees. |
| Design support code | [design](./design/EGM.md) | Progress renderer and navigation to authoritative specifications. |

The latest [drift report](./2026-11-21_DRIFT_REPORT.md) records the design/source review, including
parser coverage, collection scheduling and the scope of platform and benchmark evidence.

The test-only [conformance corpus](./conformance/README.md) is indexed by
[MANIFEST.tsv](./conformance/MANIFEST.tsv) and [REFS.tsv](./conformance/REFS.tsv).
Tests, fixture-only trees, vendored code, generated output, ignored directories, and
symlinks have no separate maps. [Source workflows](./.github/workflows/) run on
standard hosted VMs. Native Windows and real Firefox are manual checks. See the
[platform record](design/96-browser-platform-acceptance.md) for the earlier
private Safari attempts. The [Justfile](./Justfile) remains the local validation
entry point. The ignored private sibling is a separate repository.

The dispatch-only [Quick](./.github/workflows/quick.yml) and
[Bulk](./.github/workflows/bulk.yml) trust fixtures are loaded from protected
`main`. They project the controller's typed inputs into stable run titles and
verify the exact requested checkout before candidate commands. Quick runs the
canonical early partition from protected policy; Bulk retains the synthetic
candidate handshake. Both remain inactive controller prerequisites.

The hosted Linux workspace job additionally runs the default-off executor accounting
diagnostic. Its fixed workload path counts are CI-only and carry no timing claim.

## Invariants

- Correctness workflows have read-only repository permissions and use standard
  hosted runners. Private runner groups and benchmark infrastructure stay isolated.

- `standard()` has reproducible defaults and permits explicit nondeterministic providers;
  `deterministic()` enforces its replay contract; `live()` intentionally provides live
  services. Hardened leaves narrow the deterministic guest surface.
- Public workspace crates inherit `unsafe_code = "forbid"`. Deterministic numeric,
  iteration, fuel, and collection rules belong to the selected contract and versioned ABI.
- The application facade owns its settings/errors and retained handles. VM internals
  remain separate lower-level APIs; handles must respect VM ownership and decision epochs.
  The facade is thread-affine by default; its opt-in `transferable` feature selects
  the lower-level VM's `send` mode for idle exclusive ownership transfer, not concurrent execution.
- Host-owned authoritative state and external effects are not silently rolled back with
  guest state. Typed invocation journals and guest installation savepoints have distinct
  boundaries.
- Performance evidence comes only from `ci-bench`; local correctness/static checks are
  not measurements. Do not run benchmark binaries or timing/profiling scripts locally.
- GitHub Issues is the maintainer backlog. Anyone can read issues and PRs. Only collaborators can create them; AGENTS.md and CONTRIBUTING.md define the issue-to-PR workflow. CURRENT links to the backlog and remains authoritative for built state and constraints. Issue/PR templates live under `.github/`.
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
