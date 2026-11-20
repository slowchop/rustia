# Rustia — current state

Updated: 2026-11-20 (Australia/Hobart).

This is the single authority for implemented state and active project constraints.
[GitHub Issues](https://github.com/slowchop/rustia/issues) owns development work,
priority and progress. Detailed design contracts and the pinned oracle remain the
specification; dated records explain decisions
and evidence. When an older status statement disagrees, this file wins.

## Status and permissions

Rustia is a from-scratch Rust interpreter for pinned standalone **Luau 0.709**.
The experimental interpreter became public on 20 November 2026 under MIT OR Apache-2.0.
All eight workspace packages remain at **`0.0.1`**, with **all 8 blocked by publish=false**.
Stable APIs, full Luau parity, consumer rollout and supported releases have separate acceptance gates.

- **`slowchop/rustia-historic`** retains the original private evidence. The
  operations sibling **`rustia-ops`** and private benchmark infrastructure stay private.
- [GitHub Issues](https://github.com/slowchop/rustia/issues) is the maintainer
  backlog. Issues and pull requests are publicly readable. Only repository
  collaborators can create either. [CONTRIBUTING](../CONTRIBUTING.md) describes that workflow.
- [Report a vulnerability privately](https://github.com/slowchop/rustia/security/advisories/new).

Both GitHub-hosted workflows passed on the public launch commit `e54fd4e`.
The [launch record](public-source-activation.md) links the runs and lists the platforms exercised.

## Issue #18 and #19 corrections

Metamethod calls preserve their triggering instruction location for positive `error`
levels and `debug.info`, including expanded method arguments and resumed calls. Exported type aliases and `expr :: T` assertions (including module-qualified types) parse and
have no runtime type-checking effect; assertions select one result. Two focused
fixtures were checked against the pinned Luau 0.709 oracle and through the facade
on all four profiles. The #18 follow-up resolves caller-selected error reports
against the exact caller program, including reused source names and generated
wrappers. Non-table writes use oracle-checked index messages. Wrapped coroutines
preserve their original host report across yield/resume while keeping the guest
error value unchanged. Manual string rethrows retain their explicit error site. Return annotations now distinguish single types from packs, including the pinned
one-fixed-plus-tail suffix quirk. Table types validate fields, indexers, separators,
contextual qualifiers and standalone array shorthand instead of skipping braces. Assertions now reject bare tuple/variadic packs
while retaining grouped single types and function/generic pack positions. Alias
generics accept defaults, enforce regular-before-pack/default ordering, and erase
defaults at runtime. Generic function types (`<U>(U)->U`) now parse and erase in
annotations, assertions, aliases, defaults and nested type arguments, with named
parameters and final pack tails. The pinned upstream test excerpts and runtime
checks are in `generic_function_types.luau`; negative parser/facade cases and
differential shapes cover malformed forms. Typeof annotations validate ordinary expression syntax, permit expression interpolation,
and erase without executing those expressions. The differential generator includes
21 targeted type shapes and shares its 61-shape bound with CI smoke. Broader
grammar coverage remains open;
these fixes do not establish full syntax or analyzer parity.

## Issue #21 memory recovery

Failed-call traceback state is released at the public execution boundary; saved
traceback/coroutine debug frames keep debug metadata without rooting discarded
registers. The next allocating call can reclaim failed-call garbage without a
second OOM or reset, while published globals, captured locals and external roots
remain live. Ordinary protected OOM remains catchable; transactional OOM remains
a hard abort. Host reports identify the stopped instruction without changing the
bare guest `not enough memory` message. Cross-profile regressions are in
`crates/rustia/tests/error_recovery.rs`. Full acceptance uses the repository
preflight and PR CI gates.

## Transferable consumer and facade slices

The lower-level VM has an opt-in `send` ownership mode for scheduler-owned
interpreters. It replaces local `Rc`/`RefCell` ownership with checked shared
owners and requires transferable service captures and `Send + Sync` payloads;
the default local mode and its weaker bounds remain unchanged. A VM still has
one exclusive executor at a time and may move only at an idle ownership
boundary—this does not permit concurrent calls into one interpreter.

The same slice adds compiler-supplied capture names; rooted function,
environment and capture inspection; idle capture edits; bounded raw table
inspection; retained compiled entries; owned object hooks; and a scoped host
call seam that re-enters with the unchanged fuel, depth and dispatcher. The
saved-caller driver releases bytecode frames before synchronous host dispatch
and restores their diagnostic context, roots and continuations afterward.
These are low-level `rustia-vm` contracts used by the Luvy integration. The
application facade now offers an opt-in `transferable` Cargo feature under
[issue #53](https://github.com/slowchop/rustia/issues/53) that makes
the interpreter and live handles movable between workers with stricter service
and object bounds. Execution remains exclusively borrowed, and the default
facade remains thread-affine. Luvy still uses the lower-level VM/compiler/parser
boundary because capture migration, broader owned hooks and scoped re-entry are
not yet exposed ergonomically. Borrowed scopes, broader typed hooks, and the
sync/resumable API split remain tracked in issues #56, #54, and #55 respectively.
Issue #63 provides the bounded facade slice needed by Luvy: typed
`ObjectType::length` and reset-safe generalized `ObjectType::iterator` hooks.

Synchronous native callbacks now use one explicit non-yieldable boundary in
both saved-caller and borrowed/direct execution. `tostring`, function-based
`string.gsub`, `table.foreach` and `table.foreachi` callbacks report
`coroutine.isyieldable() == false`; `coroutine.yield` raises the pinned Luau
boundary error at the attempted yield, so an inner `pcall`/`xpcall` can catch it.
The boundary belongs to the callback's coroutine: a child coroutine resumed by
that callback remains independently yieldable, and the parent boundary is
restored when the child returns. This does not make arbitrary Rust/native frames
resumable.

## Implemented contracts

| Area | Current behavior and detailed contract |
| --- | --- |
| Engine | Luau 0.709, vector-3, flags-off, f64-only; one native/Wasm engine and public-workspace `unsafe_code = "forbid"`. The default facade/handles are thread-affine; opt-in `rustia/transferable` and lower-level `rustia-vm/send` permit idle ownership transfer with one exclusive executor. [Target](05-luau-target.md), [architecture](30-architecture.md). |
| Embedding | Facade-owned settings/errors; typed owned conversions and Serde; rooted VM/epoch-safe live values, including quota-atomic host-created tables and immutable vectors ([issue #58](https://github.com/slowchop/rustia/issues/58)); composable chunks, custom environments, compiled modules, atomic installation and host-owned replacement. Lower-level embedders can retain program entries and inspect/migrate rooted captures without executing guest code. [Embedding API](75-embedded-api.md). |
| Host integration | Synchronous functions, capability declarations, borrowed invocation context, shared fuel, bounded ordered records and typed Rust objects/methods. The low-level scoped dispatcher can re-enter with the active budget/depth; host objects support optional owned field/metamethod hooks, including typed facade length and generalized iteration. [Host bridge](70-host-bridge-contract.md), [functions](78-host-functions.md), [objects](83-rust-objects.md). |
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

The executable corpus has **734 fixtures**, **524 refs**, **0 pending-oracle**
and **0 pending-impl**: 716 fixtures use Luau 0.709 and 18 use offline PUC Lua 5.1.5.
This proves the named [coverage denominator](../conformance/COVERAGE.md), not the entire language.

Retained-handle stateful fuzzing now checks replacement, rollback, collection and
reset against an independent host model in all four profiles, within the existing
input/fuel/memory bounds. This is correctness coverage, not a new remote campaign
or runtime change. [Coverage record](82-error-behaviour-fuzzing.md#retained-exports-and-functions-2026-10-30).

The source workflows run lints, workspace and external-package checks,
Rustdoc/examples, bounded fuzz/differential smoke, native Linux x86-64 and
macOS ARM64, Node/V8 and headless WebKit goldens, and cross-builds on standard
GitHub-hosted VMs. Windows
native, real Firefox and longer fuzz campaigns remain manual. Cross-builds
establish compilation for their targets.

The 2026-11-13 type-grammar candidate executed shared contracts on native Linux
ARM64, a physical iPad, an ARM64 Android emulator and local real Firefox/Safari.
A former private manual-only job ran real Firefox with checksum-pinned official
Mozilla binaries in the existing Docker correctness pool. Its 2026-11-13
immutable candidate executed all 96 browser contracts with matching goldens;
the exact head and raw job evidence are recorded separately. Physical iPad Safari
27.0.1 also executed all 96 browser contracts on the 2026-11-14 candidate, with
goldens matching fresh native references. These scoped results do not establish
final-candidate acceptance. On 2026-11-15, the user explicitly deferred native
Intel Mac execution (#39) and remote desktop Safari execution (#40) from #31.
Both remain unverified, open follow-ups and future release gates; the delivered
local Safari and physical-iPad results do not substitute for remote execution.
The former private opt-in Safari dispatch used the Apple Silicon pool
and required an already logged-in runner desktop. Two attempts failed that
prerequisite before any browser tests ran; adding the route is not execution
acceptance. No runner or browser permissions were changed. [Evidence](96-browser-platform-acceptance.md).

Install the checkout-local hook with `just install-hooks`. Prefer
`just validated-push` for a frozen final commit: it runs the full
unchanged-commit `just release-preflight` inventory once and immediately pushes
only that exact clean HEAD. Ordinary `git push` retains the full hook. The
one-shot route creates no reusable receipt and accepts only exact normal or
observed-tip leased pushes. No bypass or evidence regeneration inside validation.
[97 — Validation](97-validation-workflow.md) defines that inventory; a repaired
macOS bootstrap still requires actual native execution.

**Never run performance measurements locally.** The source correctness profile gives
correctness jobs no timing authority. Public per-commit/history automation,
landscape refresh and publication remain **paused**. A separate private maintainer
bridge admits reviewed, manually dispatched exact-cohort runtime comparisons
on `ci-bench`, plus the separately reviewed immutable #33 harness-lifetime
repeatability controls. Independently review raw data; keep outputs private. Bridge
availability closes no measurement gate and authorizes no public performance claim.
[Selected profile](public-source-profile.json), [operations](81-benchmark-operations.md).

The harness has **20 workload entries including cross-adapter `string_building` source coverage**.
The [claim gate](../bench/CLAIMS.md) **approves no blanket speed headline**; claims
remain per workload/target/protocol and tied to retained history.

The graph publication-order fix and evidence-retention changes are delivered.
Identical-binary repeatability remains **unresolved**, including the roughly 27%
closure-process outlier. Replays and the approved counter capture did not identify
its cause; prior security policy was restored and verified. The harness now destroys its
cold-start probe before preparing the steady-state workload, removing a demonstrated
live-object overlap. A destructor-order regression covers fixed-N and calibrated paths.
A separately reviewed private A/A protocol evaluates the changed harness with
counterbalanced old/fixed blocks, one timed attempt per pass and retained partial
attempt files. Its observations remain private and do not establish the historical
outlier's cause, accepted calibration or a performance claim. Shared-host capacity pressure is not proven causality. Further privileged diagnostics require separate
approval. Preserve observations and thresholds. [99 — History integrity](99-benchmark-history-integrity.md).

## Development backlog

[GitHub Issues](https://github.com/slowchop/rustia/issues) is the primary work queue.
Issue bodies, discussions and linked PRs carry acceptance criteria and progress;
this section is a navigation index, not a duplicate checklist or priority order.
The [maintainer workflow](../CONTRIBUTING.md#issue-driven-development) explains
selection, coordination, review and closure.

The [open issue list](https://github.com/slowchop/rustia/issues?q=is%3Aissue%20is%3Aopen) shows current work.
The issue descriptions and linked pull requests track delivery and acceptance.

Use the launch decision above for source visibility. Package publication, measurements
and private infrastructure actions require their own authorization.

## Reading path and retained history

For users: [README](../README.md) → crate Rustdoc → runnable examples. Practical
library guides belong in the [Rustdoc introduction](../crates/rustia/src/lib.rs),
with method details beside methods. Browser build/loading instructions live beside
its [tested demo](../demos/web-live/README.md), linked from README/Rustdoc. Callback,
failure, hardened-decision and introductory example guidance are implemented.
A hosted live editor and examples are deferred. A public `Thread` handle, async callbacks,
mlua-style `Scope` and a migration shim remain caller-driven optional layers,
not first-release gates.

The [race recording](../demos/web-live/README.md#record-the-race) runs two seeds
on native Rustia and headless WebKit/Wasm. The recorder checks every recorded
distance before rendering the MP4. The [live editor](../demos/web-live/README.md#edit-a-running-race)
keeps one VM while replacing its Luau movement rule; its recording checks
continued motion across successful and rejected edits. The WebKit CI job
retains both clips in the same artifact.

For maintainers: [AGENTS](../AGENTS.md) → this file → [pillars](02-pillars.md) → the
relevant contract in the [design index](README.md). Keep built-state facts here and
work status in GitHub Issues. Put completed chronology, run counts and detailed
results in dated records.

The [pre-compaction snapshot](CURRENT-2026-10-30-snapshot.md) preserves previous
CURRENT text verbatim; its old authority/workflow statements are historical.
The [2026-10-18 progress view](progress.html) is also a dated snapshot.
