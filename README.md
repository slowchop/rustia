# rustia

A fast, low-memory, **deterministic** Luau interpreter written in Rust — built for a deterministic game runtime,
and intended for public release as a standalone Luau 0.709-compatible Rust engine once its conformance,
benchmark, API, and release gates are satisfied.

> **Status: P1 engine implementation in progress.** The locked design is implemented through a substantial
> production VM/compiler/API/conformance slice; `spikes/` are throwaway prototypes that validated the design.
> Read **[`design/CURRENT.md`](design/CURRENT.md)** for the canonical state, then
> [`design/README.md`](design/README.md) for the full doc map.

## Highlights (spikes validated the direction; production gates are now in-tree)
- Performance claims are benchmark-gated per workload and target; no global speed headline is published until
  the committed claim gate approves it. A wasm-emission spike validates a hot monomorphic f64-kernel tier, but
  general Luau speed claims remain gated on type guards, fallback behavior, and benchmark history.
- `DeterministicSim` is bit-reproducible by construction on the tested matrix; `ReproducibleStandard` remains
  Luau-conformant while making reproducible choices for unspecified behavior.
- One portable engine: big-`match`→`br_table` interpreter + a wasm-emit JIT/AOT tier; **gc-arena** GC,
  **16-byte tagged** values, own hand-written Luau parser, and thread-affine safe-Rust VM execution.
- **No unsafe code in the public workspace:** all workspace crates, including the benchmark harness, inherit
  `unsafe_code = "forbid"`; research spikes and standalone wasm FFI demos are kept outside that product gate.
- Pinned to **Luau 0.709** (standalone luau.org surface, not Roblox).
- Current conformance runner: **692 fixtures**, **491 refs**, **0 pending-oracle**, **0 pending-impl**.
- Public embedding API first slice: typed `run<T>()`, owned values with
  `Option<T>`/`Vec<T>`/`BTreeMap<String, T>`/table conversions plus typed and nested table helpers, source-named
  run/compile helpers, typed/capability-tagged host functions with fallible `rustia::Result<T>` returns and up to
  eight arguments/results, retained script functions, pre-install host signature/fuel ABI metadata,
  retained-handle freshness checks and current-generation accessors, public source-span/runtime-location error
  accessors, owned-value/table serde, deterministic extensions, hot reload, and macro sugar.

## Project Pillars
The canonical decision filter is [`design/02-pillars.md`](design/02-pillars.md). The short version:

- **Pinned Standalone Luau Compatibility** — Luau 0.709 standalone runtime behavior, not Roblox/latest-Luau drift.
- **Conformance and Provenance** — claims need fixtures, upstream links, audits, benchmarks, or explicit defers.
- **Deterministic Profiles and Replay ABI** — reproducible public behavior plus a stricter lockstep profile.
- **Rollback-Safe Host Boundary** — hosts own authoritative state, rollback capture, and ordered effect commits.
- **Layered Rust Embedding API** — small typed facade first; advanced interop stays layered.
- **One Safe Native + Wasm Engine** — one semantic engine across native and `wasm32-unknown-unknown`.
- **Benchmark-Gated Performance Claims** — no blanket speed headline without generated claim approval.
- **Public-Clean OSS Consumer Contract** — generic crates, docs, semver, CI, packaging, and no grabby-specific API.

## Project Progress
The detailed progress view is generated from [`design/progress.json`](design/progress.json) into
[`design/progress.html`](design/progress.html). These are rough engineering estimates as of 2026-07-04 for the
public rustia project; actual grabby integration work is explicitly out of scope.

| goal | complete | confidence |
|---|---:|---:|
| Whole public rustia project | 80% | 65% |
| Usable embedded scripting crate | 83% | 70% |
| Full public standalone Luau engine release | 48% | 58% |
| Standalone benchmark OSS project | 80% | 80% |
| Public release readiness | 73% | 75% |

- Current conformance runner: **692 fixtures**, **491 refs**, **0 pending**.
- The benchmark harness has all 10 requested adapters and a non-default `string_building` workload source set
  across them; Lua 5.4 runs separately from Luau because `mlua-sys` cannot link both in one binary.
- Current benchmark claim posture is generated at [`bench/CLAIMS.md`](bench/CLAIMS.md); it approves no blanket
  speed headline from the current snapshot.
- Major remaining gates are engine hardening, long-run benchmark history, public benchmark publication, and release
  packaging. Semver policy is tracked in [`design/91-semver-policy.md`](design/91-semver-policy.md), release
  notes in [`CHANGELOG.md`](CHANGELOG.md), koto migration guidance in
  [`design/76-koto-migration.md`](design/76-koto-migration.md), crate metadata audit in
  [`design/crate-metadata.json`](design/crate-metadata.json), and package file-list review in
  [`design/package-files.json`](design/package-files.json). Current publish blockers are generated at
  [`design/publishability.json`](design/publishability.json).
- `just release-preflight` validates the current release-note, package-file, generated leaf package dry-run,
  progress-dashboard, project-status, benchmark-result, crate-metadata, workflow-policy, example-compile, and
  workspace-test gates without running fresh benchmark measurements; `.github/workflows/ci.yml` mirrors that gate
  on hosted Linux CI.

## Layout
- `design/` — the design record: constraints → research → spikes → architecture (ADR) → conformance → improvements.
- `conformance/` — executable Luau fixtures and the upstream Luau 0.709 test-port plan.
- `crates/` — production P1 workspace: VM, lexer, parser, compiler, public API, CLI, and conformance runner.
- `spikes/` — throwaway prototypes (skeleton VM, libm cross-arch, wasm-JIT, gc-arena, …) with their measurements.

## Examples
- `cargo run -p rustia --example embedding` — globals, fallible typed host functions, deterministic extensions, retained script functions, and serde.
- `cargo run -p rustia --example hot_reload` — compile/install hot reload plus retained-handle survival and freshness checks.

## CLI
- `cargo run -p rustia-cli -- path/to/script.luau`
- `cargo run -p rustia-cli -- --profile deterministic-sim --fuel 100000 --eval "return 40 + 2"`
- `cargo run -p rustia-cli -- --source-name stdin.luau -`
- `cargo run -p rustia-cli -- --repl`

Compile and runtime failures include source-name-aware line snippets and caret markers where rustia has source
span or runtime-line evidence.

Built design-first with Claude Code.

## License
Licensed under either of **[Apache License, Version 2.0](LICENSE-APACHE)** or **[MIT license](LICENSE-MIT)**
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional
terms or conditions.
