# rustia — design index

> ⚠️ **Read [`CURRENT.md`](CURRENT.md) first — it is the single canonical state.** This index and the dated
> docs are staged snapshots and can lag; `CURRENT.md` wins on any disagreement. (Drift fixed per review H1.)
> New to Rustia? Start with the [README examples](../README.md). For the stable
> release target, use [the first-release checklist](92-first-release.md).
> This directory keeps detailed design decisions and historical research;
> a successful experiment is not automatically a shipped product feature.

**rustia** = a fast, low-memory, wasm-friendly **Luau interpreter in Rust** with a great embedding API,
built to replace **koto** in `../grabby-aliens/`. Design-first: we explore ~5 directions per decision,
research and spike, then commit. Ground truth for "is it BUILT" will always be the code + tests; these
docs are design intent.

## Status map

| doc | what it covers | status |
|---|---|---|
| [`CURRENT.md`](CURRENT.md) | **the single canonical state** — read first | **authoritative** ✅ |
| [`02-pillars.md`](02-pillars.md) | project pillars, non-goals, and the rabbit-hole decision filter | **[CANONICAL]** ✅ |
| [`00-constraints.md`](00-constraints.md) | the fixed contract every decision is scored against (from grabby + gak) | **[FOUNDATION]** ✅ |
| [`05-luau-target.md`](05-luau-target.md) | the pinned Luau target (0.709) — the conformance anchor | **[PINNED]** ✅ |
| [`10-landscape.md`](10-landscape.md) | research brief: koto internals, Rust scripting landscape, Luau spec, official Luau VM, SOTA VM/JIT/WASM techniques (+ raw JSON) | **[RESEARCH v1]** ✅ |
| [`15-gapclose.md`](15-gapclose.md) | **measured** gap-close: 4 spikes + 3 research re-runs; resolved tensions; **every gate GO**; final axes + leanings (+ raw JSON) | **[DECISIONS — measured]** ✅ |
| [`20-skeleton-raw.json`](20-skeleton-raw.json) | walking-skeleton VM spike + gc-arena micro-spike + adversarial verification (raw) | **✅** |
| [`40-spikes/`](40-spikes/) | spike receipts (libm cross-arch hashes, koto profile); spike crates in [`../spikes/`](../spikes/) | **[v1]** ✅ |
| [`25-review-raw.json`](25-review-raw.json) | 5-lens adversarial review of the ADR + P0 superinstruction measurement (raw) | **✅** |
| [`35-axes-raw.json`](35-axes-raw.json) | 5-agent exploration of the open axes (scope, snapshot, value-repr, GC, embedding API) — raw | **✅** |
| [`30-architecture.md`](30-architecture.md) | the architecture decision record + build plan — **v3 plus v3.1-v3.3 refinements, all 8 axes resolved** | **[ARCHITECTURE v3.3]** ✅ |
| [`50-conformance.md`](50-conformance.md) | historical 709-row behavior inventory; executable truth is 726 fixtures / 520 refs / 0 pending (708/502 Luau 0.709 plus 18/18 PUC 5.1.5) | **[INVENTORY + LIVE CORPUS]** ✅ |
| [`55-improvements.md`](55-improvements.md) | **10 adopted design improvements**, each verified still-conformant (dual-part table, string interning, FASTCALL, libm-enforcement, …) | **[IMPROVEMENTS v1]** ✅ |
| [`56-performance-research.md`](56-performance-research.md) | safe-Rust performance sequence, landed HotIR/closure fast paths, current Linux evidence, and measurement protocol | **[PERFORMANCE RESEARCH v2.0]** |
| [`57-memory-benchmarking.md`](57-memory-benchmarking.md) | memory cohort/reclamation/accounting/quotas, intended measurement protocol and native/Node-V8 mixed-heap evidence | **[COHORT IMPLEMENTED; PROTOCOL GAPS OPEN; MEASUREMENT PAUSED]** |
| [`58-vm-core2.md`](58-vm-core2.md) | staged safe-Rust interpreter redesign: C2.1 falsification, accepted Region-integrated receipts, executable tape, compiler liveness, register arena, direct table/call lanes | **[C2.1 FALSIFIED; C2.2 ACCEPTED: 8.17% LOWER NS/UNIT]** |
| [`65-wasmjit.md`](65-wasmjit.md) | **wasm-emit JIT / AOT-to-wasm spike** (verified) — ~176× the interpreter in V8, crossover ≈ 6 calls; the in-browser near-native path | **[SPIKE]** ✅ |
| [`70-host-bridge-contract.md`](70-host-bridge-contract.md) | typed invocation plus the deterministic effect/slot and decision-reset contract | **[CONTRACT + GENERIC SEAM IMPLEMENTED]** ✅ |
| [`72-host-services.md`](72-host-services.md) | **host services bus** — stdout sink, determinism-labeled `ServiceClock` providers, `standard()`/`deterministic()`/`live()` presets, logical `os.clock` | **[IMPLEMENTED v1]** ✅ |
| [`75-embedded-api.md`](75-embedded-api.md) | **Grabby-Compatible Embedded API** — coherent execution facade, typed schema/context transaction, serde, installation/replacement, typed Rust objects | **[PRIVATE BASELINE ACCEPTED; RELEASE GATES OPEN]** |
| [`76-koto-migration.md`](76-koto-migration.md) | public koto-to-rustia migration guide for the implemented embedding facade | **[GUIDE]** |
| [`77-hardened-leaf-environment.md`](77-hardened-leaf-environment.md) | frozen authoritative guest surface, stateless compiled entries, and fast isolated reset | **[IMPLEMENTED v1]** ✅ |
| [`78-host-functions.md`](78-host-functions.md) | shared synchronous callback definitions, live construction, persistent registration, reclamation and reset | **[IMPLEMENTED; CLIENT ROLLOUT VERIFIED]** |
| [`79-script-diagnostics.md`](79-script-diagnostics.md) | bounded source reports, typed errors, and console source mapping | **[IMPLEMENTED; CLIENT ROLLOUT VERIFIED]** |
| [`80-bench-suite.md`](80-bench-suite.md) | 12-adapter Linux/wasm benchmark product, evidence protocol, generated report, and standalone split plan | **[HARNESS IMPLEMENTED; MEASUREMENT PAUSED; SPLIT PENDING]** |
| [`81-benchmark-operations.md`](81-benchmark-operations.md) | qualified-host measurement, freshness/publication state, landscape refresh, and no-scheduled-billing policy | **[HOSTED CORRECTNESS; MEASUREMENT/PUBLICATION PAUSED]** |
| [`82-error-behaviour-fuzzing.md`](82-error-behaviour-fuzzing.md) | error recovery, bounded embedding fuzzing, and pinned-Luau differential campaigns | **[IMPLEMENTED; CLIENT ROLLOUT VERIFIED]** |
| [`83-rust-objects.md`](83-rust-objects.md) | typed Rust objects, method capabilities, staged requests, immediate mutation, lifetime and memory rules | **[IMPLEMENTED; CONTROLLED-CLIENT ROLLOUT VERIFIED]** |
| [`84-private-platform-acceptance.md`](84-private-platform-acceptance.md) | shared Linux/macOS-arm64/Node golden and clean external archived-facade contracts | **[PRIVATE CANDIDATE ACCEPTED; FULL RELEASE MATRIX OPEN]** |
| [`90-release-readiness.md`](90-release-readiness.md) | public crate metadata, publish gates, benchmark/CI gates, and release process checklist | **[CHECKLIST]** |
| [`91-semver-policy.md`](91-semver-policy.md) | public crate roles, stability tiers, release-line rules, and publish order | **[POLICY]** |
| [`92-first-release.md`](92-first-release.md) | Rustia 1.0 acceptance checklist: API, runtime gaps, platforms, documentation, and publication | **[IN PROGRESS]** |
| [`93-private-api-acceptance.md`](93-private-api-acceptance.md) | immutable private API baseline, inventory, verification and application rollout evidence | **[ACCEPTANCE RECORD]** |
| [`94-release-compatibility-audit.md`](94-release-compatibility-audit.md) | delivered compatibility/diagnostic fixes, retained benchmark evidence and application API review | **[DELIVERED; WIDER PARITY AUDIT OPEN]** |
| [`95-numeric-index-contract.md`](95-numeric-index-contract.md) | checked numeric conversion, accepted runtime changes and retained failed experiments | **[ACCEPTED EXACT RUNTIME]** |
| [`96-browser-platform-acceptance.md`](96-browser-platform-acceptance.md) | exact-runtime native/browser evidence and remaining release matrix | **[CANDIDATE CONTRACTS ACCEPTED; FULL MATRIX OPEN]** |
| [`97-validation-workflow.md`](97-validation-workflow.md) | staged full validation inventory and exact-commit push guard | **[IMPLEMENTED]** |
| [`98-public-source-readiness.md`](98-public-source-readiness.md) | cleaned history, private staging and source-visibility gates | **[PRIVATE STAGING; PUBLIC VISIBILITY NOT AUTHORIZED]** |
| [`99-benchmark-history-integrity.md`](99-benchmark-history-integrity.md) | historical graph/control provenance and unresolved repeatability | **[RETAINED EVIDENCE; SOURCE-PROFILE AUTOMATION PAUSED]** |
| [`crate-metadata.json`](crate-metadata.json) | generated cargo metadata policy audit for every workspace package | **[GENERATED]** |
| [`package-files.json`](package-files.json) | generated `cargo package --list` audit for every workspace package | **[GENERATED]** |
| [`publishability.json`](publishability.json) | generated publishability blocker audit for every workspace package | **[GENERATED]** |

**TL;DR of where we are:** the architecture is decided and a substantial safe-Rust Luau 0.709 engine exists:
726 fixtures / 520 refs / 0 pending (708/502 Luau 0.709 plus 18/18 PUC 5.1.5), a typed embedding facade with host-pluggable output/clock services and
canonical-identity module resolution,
block-local invocation contexts, capability views, shared fuel, and bounded record transactions,
CLI, deterministic profiles, reusable
private HotIR, and a retained 12-adapter dedicated-Linux benchmark pipeline. This source profile runs hosted correctness CI;
measurement and publication are paused pending a trusted private bridge. For committed historical measurements,
read the generated [benchmark results](../bench/RESULTS.md) and [claim checks](../bench/CLAIMS.md).
The private API baseline includes facade-owned settings, typed Rust objects,
yieldable module loading and source/stack diagnostics. Before public release, complete the broader semantic-tail
audit, full native/mobile/browser runtime matrix, API freeze, GC interval tuning and packaging gates.
Object/program reclamation,
quotas/OOM, maintained live-memory accounting, script counters, epoch-safe decision reset, and the
per-invocation host seam are implemented. A maintained 96-epoch/3,072-wave mixed-heap soak now runs on native
and Node/V8 wasm with an exact reset plateau and stable warmed wasm linear-memory pages; this is correctness
evidence, not a claim of allocator or process-RSS parity.

## The decision axes (each gets ~5 directions explored)

1. **Execution strategy** — tree-walk · register bytecode VM · tiered interp+JIT · wasm-emitting baseline JIT · copy-and-patch · AOT-to-wasm.
2. **Value representation** — NaN-boxing (64-bit) · NaN-boxing (wasm32 / 32-bit ptr) · tagged enum · pointer-tagging · hybrid.
3. **GC / memory model** — Rc/Arc + cycle collector · gc-arena (piccolo-style tracing) · generational incremental · arena/region · hybrid. (Live VMs are thread-affine; immutable compiled artifacts/config may be shared. Determinism remains mandatory.)
4. **WASM execution & tiering** — interpreter-only · nested-wasm JIT · copy-and-patch-to-wasm · AOT script→wasm · superinstructions-only. (Only *native-machine-code* JIT can't run inside the wasm sandbox; **wasm-emitting JIT/AOT — generate wasm at runtime, let the host engine compile it — IS viable** and is the in-browser speed lever for hot code.)
5. **Parser / frontend** — reuse `full_moon` · hand-written recursive descent · port the official Luau parser · generator (pest/lalrpop).
6. **Rust embedding API** — mlua-style (`UserData`/`FromLua`/`IntoLua`) · rquickjs-style · native-trait-based · serde-bridge-centric · macro-derived.
7. **Number / determinism strategy (T1)** — how to reconcile Luau's f64-only numbers with grabby's integer/Q16.16 deterministic boundary (buffers · fixed-point userdata · integer mode · provably-deterministic f64 subset).

## How we decide

See the scoring rubric at the bottom of [`00-constraints.md`](00-constraints.md): native speed, wasm32
speed, memory, determinism, Luau compat, Send+Sync, embedding/DX, implementation risk — speed and
determinism dominate.
