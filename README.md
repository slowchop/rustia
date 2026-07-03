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
- Current conformance runner: **683 fixtures**, **480 refs**, **0 pending-oracle**, **0 pending-impl**.
- Public embedding API first slice: owned values, typed host functions, retained script functions, serde,
  deterministic extensions, hot reload, and macro sugar.

## Project Progress
These percentages are rough engineering estimates based on the current workspace and canonical docs as of
2026-07-03. They measure completion toward public rustia, excluding actual grabby integration work.

### Overall goals

| goal | complete | estimate confidence | current state | future plan |
|---|---:|---:|---|---|
| Whole public rustia project | 58% | 65% | Design is locked and production crates are substantial, but release, CI, and perf gates remain. | Harden P1, finish benchmark gates, then move toward full public Luau parity. |
| Usable embedded scripting crate | 64% | 70% | The `rustia` facade supports owned values, typed host functions, retained functions, serde, extensions, and hot reload. | Stabilize API semantics, diagnostics, capability policy, and docs/examples. |
| Full public Luau engine release | 48% | 58% | Luau 0.709 target and conformance corpus are strong; production engine still needs perf and parity hardening. | Complete P1/P2 runtime work, then P4 parity items such as module/loading, typechecker/LSP track, and release packaging. |
| Standalone benchmark OSS project | 64% | 80% | Harness, history, static site, five default workloads, and all 10 requested adapters are present. | Split from workspace, expand workloads, wire CI, and publish charts/history. |
| Public release readiness | 45% | 75% | License is set and docs are strong; crates are still workspace-private and CI/release polish is absent. | Prepare publishable crate metadata, examples, semver policy, and public benchmark claims. |

### Feature breakdown

| area | feature | complete | estimate confidence | current state | future plan |
|---|---|---:|---:|---|---|
| Design | Constraint and target model | 95% | 90% | Core priorities, target matrix, and public/private boundary are documented. | Keep docs synced as implementation exposes new constraints. |
| Design | Architecture decisions | 92% | 85% | All eight architecture axes are decided in `design/30-architecture.md`. | Reconfirm value-layout/dispatch on JSC and SpiderMonkey before final P1 lock. |
| Design | Review cleanup | 95% | 90% | Third-party review findings are folded into canonical docs. | Treat new findings as incremental deltas, not a redesign. |
| Design | Current-state tracking | 85% | 80% | `design/CURRENT.md` is authoritative and current. | Keep README and design docs aligned after each major implementation slice. |
| Design | Research spikes | 90% | 85% | Dispatch, value layout, libm determinism, GC, snapshot, and wasm-JIT directions are backed by spikes. | Keep spike outputs as evidence, but move production claims to benchmark gates. |
| Luau target | Luau 0.709 pin | 95% | 92% | Target version, standalone surface, vector width, and flags posture are documented. | Watch upstream `FFlag::LuauIntegerType2` and update only with explicit evidence. |
| Conformance | Manifest and refs | 90% | 90% | 683 fixtures and 480 refs are executable with 0 pending rows. | Add focused fixtures for newly found gaps instead of reopening bulk inventory. |
| Conformance | Upstream corpus execution | 88% | 85% | Whole upstream Luau 0.709 conformance scripts are represented through executable fixtures. | Preserve green status while hardening engine internals. |
| Conformance | Harness and coverage notes | 86% | 85% | Runner, manifest validation, and detailed coverage notes are in-tree. | Add coverage metadata for new public-parity features as they land. |
| Frontend | Lexer | 82% | 70% | Production lexer crate exists with tests. | Expand edge-case coverage as conformance finds parser/lexer gaps. |
| Frontend | Parser | 78% | 68% | Hand-written parser handles Luau syntax and type-grammar erasure. | Continue differential and conformance-driven hardening. |
| Frontend | Type grammar parse-and-erase | 80% | 70% | Type syntax is parsed for runtime erasure; no typechecker is intended for P1. | Defer gradual typechecker/LSP work to public-parity phase. |
| Frontend | Compiler | 72% | 65% | Production compiler emits bytecode and supports conformance-specific paths. | Harden constant folding, diagnostics, superinstructions, and optimization boundaries. |
| VM/runtime | Bytecode dispatch core | 70% | 65% | Portable interpreter exists; big-match/br-table direction is locked. | Add hot/cold split, benchmark `call_ref`, and maintain fusion-invariant fuel. |
| VM/runtime | Value representation | 76% | 75% | 16-byte tagged value direction is implemented and documented. | Keep representation sealed until JSC/SpiderMonkey measurements close the gate. |
| VM/runtime | Heap and GC | 62% | 65% | `gc-arena` direction is chosen and production heap code exists. | Tune Standard pacing and DeterministicSim instruction-count collection intervals. |
| VM/runtime | Tables/metatables | 78% | 70% | Broad table, metatable, iteration, and stdlib table behavior is conformance-covered. | Continue parity hardening and performance work. |
| VM/runtime | Functions/coroutines/pcall | 76% | 70% | Functions, closures, varargs, coroutines, and protected calls have broad coverage. | Keep stackless recursion and yield paths covered as runtime changes. |
| VM/runtime | Base/math/string/bit32/vector/utf8 stdlib | 78% | 68% | Major Luau stdlib surfaces are covered by fixtures. | Fill remaining public-parity libraries and edge cases. |
| VM/runtime | Error/debug behavior | 72% | 65% | Debug/error behavior is substantially covered for conformance. | Tighten diagnostics and source-span reporting for public API use. |
| Determinism | f64/libm/canonicalization discipline | 75% | 75% | Deterministic number policy is documented and partly enforced by tests. | Keep compiler and VM on the same libm/canonicalization rules. |
| Determinism | Runtime profiles | 74% | 75% | `LuauCompat`, `ReproducibleStandard`, and `DeterministicSim` are defined; public APIs expose profiles. | Refine exact capability behavior and profile-specific GC policy. |
| Determinism | Fuel and limits | 70% | 70% | Instruction fuel exists and is part of the design contract. | Version fuel costs and keep superinstructions fuel-invariant. |
| Determinism | Cross-engine wasm proof | 30% | 80% | V8 evidence exists; JSC and SpiderMonkey are still open gates. | Build real wasm measurement harness and record golden results. |
| Embedded API | Koto-shaped facade | 70% | 75% | Builder, compile/run, globals, exports, and function calls exist. | Stabilize names, error types, examples, and migration docs. |
| Embedded API | Owned value firewall | 72% | 75% | `OwnedValue`, owned tables/keys, and conversions exist. | Harden conversion coverage and avoid leaking internal GC lifetimes. |
| Embedded API | Typed host functions | 68% | 75% | Plain Rust functions can be registered with typed args/results. | Expand arity/signature support and improve diagnostics. |
| Embedded API | Extension/linker layer | 58% | 70% | Namespaced extensions and coarse capabilities exist. | Add richer capability metadata, fuel costs, and policy validation. |
| Embedded API | Public host-bridge seam | 62% | 75% | The generic seam for deterministic host functions, typed slots, and effect ordering is documented without public grabby types. | Keep public rustia generic while strengthening capability enforcement and scratch-heap rules. |
| Embedded API | Serde bridge | 62% | 70% | Owned-value serde paths exist. | Add zero-copy live-value deserialization after the public surface stabilizes. |
| Embedded API | Retained handles | 58% | 65% | Script function handles can be retained and called. | Define invalidation/versioning behavior around hot reload. |
| Embedded API | Hot reload | 50% | 65% | Compile/install/hot-reload entry points exist. | Guarantee old-program survival, diagnostics, and handle semantics on reload failure. |
| Embedded API | Macro/codegen sugar | 35% | 65% | Macro sugar exists as an early slice, but the lower-level API should lead. | Delay expansion until the core API stops moving. |
| Benchmarks | Harness CLI and schema | 78% | 85% | `bench/` has a standalone-style CLI, JSONL history, and generated `RESULTS.md`. | Keep JSONL as the source of truth for CI and website data. |
| Benchmarks | Adapter matrix | 100% | 85% | Requested adapters are implemented; `mlua-lua54` runs as a separate binary from Luau. | Split adapters into standalone crates/binaries and keep baselines current. |
| Benchmarks | Workloads | 55% | 75% | Five default workload families are wired, with extra Luau/Koto workload seeds present. | Expand toward the 11-15 workload plan including interop, serde, fuel, and string building. |
| Benchmarks | History and static site | 70% | 80% | Append-only history, markdown results, and dependency-free dashboard are present. | Publish long-run history and richer charts after standalone split. |
| Benchmarks | Benchmark CI | 15% | 90% | CI workflow wiring is not present. | Add self-hosted `[self-hosted, bench]` workflows after runner setup is confirmed. |
| Wasm/perf | Wasm-emit JIT spike | 45% | 70% | Runtime wasm emission is validated in spikes, not production-integrated. | Add type guards/deopt and interpreter-equivalence checks before production use. |
| Wasm/perf | Native/wasm matrix | 30% | 75% | Native baselines are being recorded; full wasm engine matrix is deferred. | Measure V8, SpiderMonkey, and JSC on production rustia wasm. |
| Tooling | Workspace tests | 72% | 75% | Production crates have meaningful unit and conformance tests. | Keep expanding tests with new runtime/API work. |
| Tooling | CLI | 55% | 65% | `rustia-cli` exists as a runner surface. | Improve REPL/run ergonomics and diagnostic presentation. |
| Tooling | Live wasm demo | 35% | 70% | `demos/live-wasm` can build and serve a browser demo outside normal workspace gates. | Refresh after production wasm/perf work and make it a public example. |
| Packaging | Crate publishability | 30% | 80% | Workspace crates are `publish = false`. | Add crate metadata, examples, feature policy, and release checklist. |

## Layout
- `design/` — the design record: constraints → research → spikes → architecture (ADR) → conformance → improvements.
- `conformance/` — executable Luau fixtures and the upstream Luau 0.709 test-port plan.
- `crates/` — production P1 workspace: VM, lexer, parser, compiler, public API, CLI, and conformance runner.
- `spikes/` — throwaway prototypes (skeleton VM, libm cross-arch, wasm-JIT, gc-arena, …) with their measurements.

Built design-first with Claude Code.

## License
Licensed under either of **[Apache License, Version 2.0](LICENSE-APACHE)** or **[MIT license](LICENSE-MIT)**
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional
terms or conditions.
