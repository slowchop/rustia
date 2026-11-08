# Rustia — current state

Updated: 2026-11-01 (Australia/Hobart).

This is the single authority for current project status. Detailed design contracts
and the pinned oracle remain the specification; dated records explain decisions
and evidence. When an older status statement disagrees, this file wins.

## Status and permissions

Rustia is a from-scratch Rust interpreter for pinned standalone **Luau 0.709**.
Development remains private and at 0.x. Source access, exact consumer acceptance
and supported releases have separate gates; full parity and API stability are not promised.

- Work in **`slowchop/rustia`**, the cleaned source repository. The original
  **`slowchop/rustia-historic`** remains a separate private evidence repository.
- **Both repositories stay private.** All eight workspace packages remain at
  **`0.0.1`**, **all 8 blocked by publish=false**. Public source visibility,
  package publication and supported releases require separate explicit decisions.
- Outside contribution, issue-reporting and support policies are **undecided**.
  Do not invite submissions, open reporting channels or promise maintainer review.
- Main protections and auto-merge settings still await separate approval.
  Check applicable CI and independent review before merging; use merge commits.
- Keep private infrastructure and secrets in the separate ignored `private/`
  sibling repository. The public/private boundary is in [AGENTS](../AGENTS.md).

[98 — Source readiness](98-public-source-readiness.md) records the audit, history
projection and visibility gates. Historic PR/CI links remain private evidence.

## Issue #18 and #19 corrections

Metamethod calls preserve their triggering instruction location for positive `error`
levels and `debug.info`. Exported type aliases and `expr :: T` assertions parse and
have no runtime type-checking effect; assertions select one result. Two focused
fixtures were checked against the pinned Luau 0.709 oracle and through the facade
on all four profiles. Related cross-chunk diagnostic snippets, nil writes and
coroutine report attribution from #18 remain separate follow-up work. The existing
balanced-token table-type parser remains more permissive than the full type-grammar
strictness intended by design/50; this change reuses that grammar.

## Implemented contracts

| Area | Current behavior and detailed contract |
| --- | --- |
| Engine | Luau 0.709, vector-3, flags-off, f64-only; one native/Wasm engine, public workspace `unsafe_code = "forbid"`, thread-affine live VM/handles. [Target](05-luau-target.md), [architecture](30-architecture.md). |
| Embedding | Facade-owned settings/errors; typed owned conversions and Serde; rooted VM/epoch-safe live values; composable chunks, custom environments, compiled modules, atomic installation and host-owned replacement. [Embedding API](75-embedded-api.md). |
| Host integration | Synchronous functions, capability declarations, borrowed invocation context, shared fuel, bounded ordered records and typed Rust objects/methods. [Host bridge](70-host-bridge-contract.md), [functions](78-host-functions.md), [objects](83-rust-objects.md). |
| Services and modules | Explicit output/clock/entropy providers and native/WASI/browser adapters. `require` needs a host resolver; core has no ambient filesystem. Module ABI v3 supports yieldable loads and coroutine-owned loading state. Each installation has a fresh dependency-cache cohort; older rooted functions retain theirs. [Services](72-host-services.md), [embedding API](75-embedded-api.md). |
| Lifetime and limits | Ordinary objects/programs are reclaimed and generation-safely reused; rooted public values stay alive. Maintained logical-live quota accounting, telemetry, atomic OOM recovery and decision reset are implemented. Mixed-heap native/Node-V8 correctness checks cover bounded reuse. Rust payload/capture allocations and process RSS are outside that quota. [Architecture](30-architecture.md), [memory protocol](57-memory-benchmarking.md). |
| Diagnostics and CLI | Structured compile/runtime errors, bounded source reports; file/stdin/eval/REPL and version output. [Diagnostics](79-script-diagnostics.md), [error/fuzzing contract](82-error-behaviour-fuzzing.md). |
| Hardened decisions | An allowlisted layer on DeterministicSim, with mandatory memory limit, direct compiled-entry invocation and reset after success or failure. This narrows the same engine. [Hardened leaf contract](77-hardened-leaf-environment.md). |

### Environment selection

- **`standard()` / ReproducibleStandard:** reproducible defaults, including
  insertion-order iteration, pinned RNG and deterministic clock fallbacks; explicit
  nondeterministic providers/capabilities are permitted.
- **`deterministic()` / DeterministicSim:** rejects nondeterministic providers and
  forbidden host capabilities; deterministic output/clock/entropy providers are
  supported. Its collection schedule uses executed instruction counts.
- **`live()`:** first-class live clocks, entropy, output and runtime compilation;
  deliberately makes no replay guarantee. It shares the advanced embedding API.
- **LuauCompat** selects Luau-style hash iteration with the Standard service
  surface. **Puc51Compat** separately targets tested PUC Lua 5.1.5 table/order,
  sorting and formatting behavior with reproducible host-service rules.

Standard modes use paced/incremental collection; HardenedLeaf narrows DeterministicSim.
Ordinary builder defaults have no fuel or accounted-memory limit: set budgets explicitly.
[Services](72-host-services.md) and [architecture](30-architecture.md) define these contracts.

### Failure and reuse boundaries

Ordinary execution keeps completed guest writes after an error. Installation
adds a guest-state savepoint and publishes candidate exports only on success.
Typed invocation publishes staged records only on outer success; it does not
roll back guest writes by itself. A failed callback discards its own staged
suffix; earlier completed records can survive a later error caught by `pcall`.
Hardened entry execution automatically resets decision-local guest state.

Reset invalidates guest handles and preserves configured host definitions,
services and settings. Neither reset nor installation undoes host I/O, callback
captures, `HostMut` changes or Rust payload mutation. Reset drops guest object
instances; it does not restore payload contents. A contained host panic poisons
execution until successful reset; containment requires an unwinding build.
Ordinary script errors do not imply poisoning. [Rustdoc guide](../crates/rustia/src/lib.rs).

### Optimization boundary

Caches, shared constants, HotIR and plain executable tape are implemented. Fuel,
logical clocks and deterministic collection must preserve the logical-unit schedule;
memory-limited/instrumented paths retain required scalar checks. Experimental O2
stays **default-off**; dead-clear elision is blocked on liveness-aware root scans.
Corpus-green rewrites still need per-workload measurement. [VM Core 2](58-vm-core2.md)
records these gates. Wasm JIT/AOT spikes are research, not a shipped execution tier.

## Validation and benchmark scope

The executable corpus has **728 fixtures**, **520 refs**, **0 pending-oracle**
and **0 pending-impl**: 710 fixtures use Luau 0.709 and 18 use offline PUC Lua 5.1.5.
This proves the named [coverage denominator](../conformance/COVERAGE.md), not the entire language.

Retained-handle stateful fuzzing now checks replacement, rollback, collection and
reset against an independent host model in all four profiles, within the existing
input/fuel/memory bounds. This is correctness coverage, not a new remote campaign
or runtime change. [Coverage record](82-error-behaviour-fuzzing.md#retained-exports-and-functions-2026-10-30).

Private self-hosted CI covers lints, workspace/external-package contracts, Rustdoc/examples,
bounded fuzz/differential smoke, native Linux x86-64 and macOS ARM64, Node/V8 and
headless WebKit goldens, plus cross-builds. Builds are not runtime acceptance.
Ordinary correctness jobs use generic Linux/macOS/Windows self-hosted groups,
with private-repository and same-repository PR guards. Public visibility requires
a separate isolation review; there is no hosted fallback or benchmark-runner access.
Dated Linux ARM64, Windows and real Firefox/Safari passes do not close Intel Mac,
mobile, final-candidate or remote-browser gates. [Evidence](96-browser-platform-acceptance.md).

Install the checkout-local hook with `just install-hooks`. Every push must pass
the full unchanged-commit `just release-preflight` inventory through the normal
hook. No bypass or evidence regeneration inside validation. [97 — Validation](97-validation-workflow.md)
defines that inventory; a repaired macOS bootstrap still requires actual native execution.

**Never run performance measurements locally.** The source correctness profile gives
correctness jobs no timing authority. Public per-commit/history automation,
landscape refresh and publication remain **paused**. A separate private maintainer
bridge admits only reviewed, manually dispatched exact-cohort runtime comparisons
on `ci-bench`. Independently review raw data; keep outputs private. Bridge
availability closes no measurement gate and authorizes no public performance claim.
[Selected profile](public-source-profile.json), [operations](81-benchmark-operations.md).

The harness has **20 workload entries including cross-adapter `string_building` source coverage**.
The [claim gate](../bench/CLAIMS.md) **approves no blanket speed headline**; claims
remain per workload/target/protocol and tied to retained history.

The graph publication-order fix and evidence-retention changes are delivered.
Identical-binary repeatability remains **unresolved**, including the roughly 27%
closure-process outlier. Replays and the approved counter capture did not identify
its cause; prior security policy was restored and verified. Shared-host capacity
pressure is not proven causality. Further privileged diagnostics require separate
approval. Preserve observations and thresholds. [99 — History integrity](99-benchmark-history-integrity.md).

## Open work

One action list; completing work does not authorize source or package publication.

1. **Finish source-access preparation:** retain the privacy audit and original
   history, recheck the exact candidate/settings, and resolve the visibility gates
   in [98](98-public-source-readiness.md). Public visibility needs explicit approval.
2. **Finish documentation and stable-API review:** extend conversion guidance and
   meaningful facade reference coverage; audit 1.0 classifications, extensibility
   and consumer compatibility before freezing. Settings/errors, live exports and
   typed objects are implemented. Preserve the simple facade and preset freedom.
   [Release/API review](92-first-release.md), [documentation review](100-documentation-review.md).
3. **Harden semantics and platforms:** add focused oracle fixtures and longer
   coverage-guided/differential campaigns for compiler/runtime error-phase gaps;
   close actual native/browser/mobile execution and golden requirements in
   [00](00-constraints.md), [90](90-release-readiness.md) and [96](96-browser-platform-acceptance.md).
4. **Finish memory policy and evidence:** tune GC intervals against the maintained
   mixed-heap correctness gate; deterministic intervals are replay-ABI choices.
   Protocol v2 implements paired control-adjusted retention, anonymous-memory area,
   separate warmup processes and 100 ms natural idle before quiescent sampling.
   Add external allocator/RSS history and obtain fresh Linux memory evidence through
   an authorized memory lane; the private bridge currently supports runtime comparisons
   only. Historical values/thresholds remain unchanged. [57](57-memory-benchmarking.md),
   [drift review](../2026-10-29_DRIFT_REPORT.md).
5. **Resolve benchmark repeatability and extend trusted evidence:** use the private
   bridge within scope; preserve raw samples/acceptance rules and one history for
   result/claim/report views. Broader baselines, realistic mixed heaps and public
   automation remain pending. Benchmark extraction, interop/Serde/fuel workloads
   and alerts remain deferred. [80](80-bench-suite.md), [99](99-benchmark-history-integrity.md).
6. **Complete supported-release gates before a separate release decision:** verify
   package/API/CLI acceptance and exact consumer rollouts, then follow generated
   dependency order only after publication is authorized. The future 1.0 promise
   covers embedding API and CLI; lower-level engine APIs remain separately 0.x.
   [Release plan](92-first-release.md), [acceptance](93-private-api-acceptance.md),
   [versioning](91-semver-policy.md).
7. **Maintain the pinned target:** watch `FFlag::LuauIntegerType2`, add focused
   fixtures, and retain first-release exclusions: Roblox APIs, analyzer/typechecker,
   bytecode loading and raw-C-API parity. [Target](05-luau-target.md), [pillars](02-pillars.md).

## Reading path and retained history

For users: [README](../README.md) → crate Rustdoc → runnable examples. Practical
library guides belong in the [Rustdoc introduction](../crates/rustia/src/lib.rs),
with method details beside methods. Browser build/loading instructions live beside
its [tested demo](../demos/web-live/README.md), linked from README/Rustdoc. Callback,
failure, hardened-decision and introductory example guidance are implemented.
No documentation website is planned. A public `Thread` handle, async callbacks,
mlua-style `Scope` and a migration shim remain caller-driven optional layers,
not first-release gates.

For maintainers: [AGENTS](../AGENTS.md) → this file → [pillars](02-pillars.md) → the
relevant contract in the [design index](README.md). Keep one live status and action
list. Put completed chronology, run counts and detailed results in dated records.

The [pre-compaction snapshot](CURRENT-2026-10-30-snapshot.md) preserves previous
CURRENT text verbatim; its old authority/workflow statements are historical.
The [2026-10-18 progress view](progress.html) is also a dated snapshot.
