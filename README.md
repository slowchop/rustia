# rustia

A fast, low-memory, **deterministic** Luau interpreter written in Rust — built to replace **koto** as the
scripting layer in [grabby-aliens](https://github.com/slowchop/grabby-next), and intended for public release
at full Luau parity.

> **Status: P1 engine implementation in progress.** The locked design is implemented through a substantial
> production VM/compiler/API/conformance slice; `spikes/` are throwaway prototypes that validated the design.
> Read **[`design/CURRENT.md`](design/CURRENT.md)** for the canonical state, then
> [`design/README.md`](design/README.md) for the full doc map.

## Highlights (spikes validated the direction; production gates are now in-tree)
- Designed to run at/near reference-Luau parity on measured workloads, with koto comparisons tracked
  per-workload; **near-native on hot numeric paths** via a runtime wasm-emit JIT (~176× the interpreter in V8,
  crossover ≈ 6 calls).
- **Bit-for-bit deterministic** across x86-64-linux / arm64-mac / wasm32 (f64 + pure-Rust libm + per-op
  NaN canonicalization, table-key `-0` normalization, and no-FMA), with two profiles: `ReproducibleStandard` (public) and
  `DeterministicSim` (lockstep multiplayer).
- One portable engine: big-`match`→`br_table` interpreter + a wasm-emit JIT/AOT tier; **gc-arena** GC,
  **16-byte tagged** values, own hand-written Luau parser, **Send-not-Sync** N-VM threading.
- Pinned to **Luau 0.709** (standalone luau.org surface, not Roblox).
- Current conformance runner: **684 fixtures**, **481 refs**, **0 pending-oracle**, **0 pending-impl**.
- Public embedding API first slice: typed `run<T>()`, owned values with `Option<T>`/`Vec<T>`/`BTreeMap<String, T>`/table
  conversions, typed/capability-tagged host functions with up to eight arguments/results, retained script
  functions, pre-install host signature metadata, serde, deterministic extensions, hot reload, and macro sugar.

## Project Progress
The detailed progress view is generated from [`design/progress.json`](design/progress.json) into
[`design/progress.html`](design/progress.html). These are rough engineering estimates as of 2026-07-03 for the
public rustia project; actual grabby integration work is explicitly out of scope.

| goal | complete | confidence |
|---|---:|---:|
| Whole public rustia project | 65% | 65% |
| Usable embedded scripting crate | 73% | 70% |
| Full public Luau engine release | 48% | 58% |
| Standalone benchmark OSS project | 72% | 80% |
| Public release readiness | 62% | 75% |

- Current conformance runner: **684 fixtures**, **481 refs**, **0 pending**.
- The benchmark harness has all 10 requested adapters; Lua 5.4 runs separately from Luau because `mlua-sys`
  cannot link both in one binary.
- Current benchmark claim posture is generated at [`bench/CLAIMS.md`](bench/CLAIMS.md); it approves no blanket
  speed headline from the current snapshot.
- Major remaining gates are engine hardening, long-run benchmark history, JSC/SpiderMonkey wasm proof, and release
  packaging. Semver policy is tracked in [`design/91-semver-policy.md`](design/91-semver-policy.md), release
  notes in [`CHANGELOG.md`](CHANGELOG.md), crate metadata audit in
  [`design/crate-metadata.json`](design/crate-metadata.json), and package file-list review in
  [`design/package-files.json`](design/package-files.json). Current publish blockers are generated at
  [`design/publishability.json`](design/publishability.json).
- `just release-preflight` validates the current release-note, package-file, leaf package dry-run,
  progress-dashboard, benchmark-result, crate-metadata, workflow-policy, and workspace-test gates without running
  fresh benchmark measurements; `.github/workflows/ci.yml` mirrors that gate on hosted Linux CI.

## Layout
- `design/` — the design record: constraints → research → spikes → architecture (ADR) → conformance → improvements.
- `conformance/` — executable Luau fixtures and the upstream Luau 0.709 test-port plan.
- `crates/` — production P1 workspace: VM, lexer, parser, compiler, public API, CLI, and conformance runner.
- `spikes/` — throwaway prototypes (skeleton VM, libm cross-arch, wasm-JIT, gc-arena, …) with their measurements.

## Examples
- `cargo run -p rustia --example embedding` — globals, typed host functions, deterministic extensions, retained script functions, and serde.
- `cargo run -p rustia --example hot_reload` — compile/install hot reload and retained-handle behavior across reload failures.

## CLI
- `cargo run -p rustia-cli -- path/to/script.luau`
- `cargo run -p rustia-cli -- --profile deterministic-sim --fuel 100000 --eval "return 40 + 2"`
- `cargo run -p rustia-cli -- --source-name stdin.luau -`

Built design-first with Claude Code.

## License
Licensed under either of **[Apache License, Version 2.0](LICENSE-APACHE)** or **[MIT license](LICENSE-MIT)**
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional
terms or conditions.
