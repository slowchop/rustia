# rustia — CURRENT canonical state (read this first)

> **This file is the single source of truth for "where the project actually is."** Other design docs are
> staged snapshots and may lag; where they disagree with this file, **this file wins.** (Created 2026-06-30 to
> fix version-drift flagged by 3rd-party review finding H1; updated 2026-07-25.)

## One-paragraph state
The **design phase is complete and reviewed**, and P1 implementation is now substantial in production crates.
The current workspace contains the VM/lexer/parser/compiler/public API/CLI/conformance runner, plus an
executable Luau 0.709 conformance corpus with **0 pending-oracle** and **0 pending-impl** rows. The public
embedding facade from [`75-embedded-api.md`](75-embedded-api.md) is implemented at the first useful level:
owned values with typed and nested table helpers, source-named run/compile helpers, typed host functions with
fallible `rustia::Result<T>` returns, pre-install host signature/fuel ABI metadata, retained script function
handles with freshness checks and current-generation accessors, public source-span/runtime-location error
accessors, owned-value/table serde, deterministic extension registration, hot reload, and macro sugar.
The host services bus from [`72-host-services.md`](72-host-services.md) is now implemented: facade presets
`standard()`/`deterministic()`/`live()`, a host stdout sink giving `print` its real Luau composition,
determinism-labeled wall/monotonic clock providers feeding `os.time`/`os.date`/`os.clock`, and a deterministic
logical `os.clock` fallback (executed instructions / 1_000_000) replacing the former `0.0`.
A from-scratch Rust Luau
interpreter is validated as **GO** by the earlier spikes and is now backed by the production conformance suite;
performance is continuously benchmark-gated, and the generated gate does not yet approve a blanket public speed
headline. All 8 architecture axes are decided
([`30-architecture.md`](30-architecture.md) v3), 10 conformance-verified improvements are adopted
([`55-improvements.md`](55-improvements.md)), the no-unsafe SotA performance research stack is now captured in
[`56-performance-research.md`](56-performance-research.md), and the standalone benchmark harness has the engine/workload/history
shell, generated current-results HTML report, 12 workload entries including cross-adapter `string_building` source coverage, and
the `rustia`/`boa`/`koto`/`luaur`/`mlua-lua54`/`mlua-luau`/`rhai`/`roto`/`ruau`/`rquickjs`/`wasmi`/`wasmtime` native adapters in
place.
The benchmark artifacts now also include a first-class wasm pre-lock gate source
(`../bench/wasm-gate.json`), recurring wasm target results for `rustia`, `koto`, and `rhai` under Node/V8, and
retained historical V8, JavaScriptCore-family, and SpiderMonkey portability evidence. Generated report panels
make the required production rustia wasm measurements explicit.
Its generated [`../bench/CLAIMS.md`](../bench/CLAIMS.md) gate keeps public benchmark language per-workload and
currently approves no blanket speed headline from the committed snapshot.
The public API now has release-preflight-enforced compile-checked examples for embedding and hot reload, plus a
first koto-to-rustia migration guide.
The public workspace now treats **no unsafe code** as a product feature: every workspace package inherits
`unsafe_code = "forbid"`, and the live VM is thread-affine instead of relying on an unsafe cross-thread `Send`
marker. Experimental spikes and standalone wasm FFI demos remain outside that public workspace gate.
The hardened table/atom/inline-cache performance foundation is now in-tree: VM-local byte atoms and larger
insertion-ordered hash parts use compact deterministic open-addressed `u32` side indices; loaded programs use
compact `u32` constant atoms plus a 4-byte per-PC site map; and eligible sites lazily allocate 16-byte
atom/slot/shape records that always read the live value. Mutation-safe `GETIMPORT` resolves one-to-three-component
descriptors without caching final values, and generic fixed-result `FASTCALL` guards the exact native function
handle before direct dispatch and otherwise follows the ordinary resumable call path. Non-import dot fields and
compile-time string-key reads/writes now lower to `GETTABLEKS`/`SETTABLEKS` with the same live slot
revalidation as generic index operations. `GETTABLEKS` costs fuel 2. Compiler writes keep a one-fuel key load
before an intervening RHS and then use the preloaded-key, one-fuel `SETTABLEKS`; the no-preload fully fused
bytecode mode costs fuel 2. Compound updates retain the key produced
by `GETTABLEKS` for their later `SETINDEX`, preserving the unfused three-step load/read/write fuel schedule.
`NAMECALL` fuses the constant-name load
and method lookup, not the subsequent `CALL`: its two lanes cover the receiver and the receiver's current
table-valued `__index`, revalidate both paths, and read the live callee without caching a final value, receiver, or
class. Lookup-only `NAMECALL` costs fuel 2; the mode that also preserves the prior missing-method check costs 3.
Fused costs are charged before their side effects and preloaded writes retain the original cross-RHS phase.
Their semantic instruction costs remain deterministic, but aggregate GC checkpoints can move when compiler
specialization removes dead cleanup bytecode.
Insertion order, metamethod/readonly behavior, mutation visibility, and logical fuel remain preserved.
Compiled constant pools are immutable and shared across program clones retained by different VMs. Long compiled
string atoms reference that storage without copying the bytes per VM, and an equal runtime-owned atom upgrades to
the shared bytes when the compiled literal arrives later. Ordinary loaded literals and runtime-created binary
strings of up to 13 bytes can live directly in the sealed 16-byte `Value`. Constant-key
and cache paths still atomize when identity is required, and table insertion canonicalizes a new non-nil short
key, but lookup misses, nil writes, readonly writes, and invalid-table writes do not create atoms.
The first profiled hot-loop hygiene pass now avoids entering `gc-arena` when allocation debt is already zero and
refreshes weak-table root snapshots only before a possible collection boundary or an instruction that can enter
guest code. Focused tests cover ordinary collection cadence plus nested guest execution with explicit
`collectgarbage`, while the full pending-free conformance suite preserves the public semantics.
`rustia-cli` supports file, stdin, `--eval`, and `--repl` execution with profile, fuel, source-name options,
state-preserving line evaluation, expression shorthand, and source-aware diagnostics for compile and runtime
failures.
CI wiring now includes pinned-toolchain formatting and strict Clippy gates, hosted release-preflight checks,
and explicit public-example compilation. Hosted workflows validate committed benchmark artifacts but never take
fresh timings; those come only from the explicit ephemeral Linode Linux lifecycle.
The benchmark harness prepares fresh engine/workload state for calibration and every recorded repetition, warms
that state once outside the timer, and can write execution-ordered raw samples to a separate
`rustia.bench.raw-samples.v1` JSON sidecar. `--fixed-n` bypasses calibration and uses exactly the selected work for
every warmup and timed repetition; sidecars record whether `n` was fixed or calibrated, while the stable public
history remains min/median summary JSONL. The equal-work protocol alternates ABBA/BAAB order and requires at least
three independent publication measurements of the clean final commit, with the sidecar written before each actual
summary append. The accepted exact-commit Linode v2 comparison measured 2.42% lower corrective-candidate ns/unit
on pooled `dense_tables` samples, with a conservative nominal 95% interval of -3.35% to -1.57%, excluding the
predeclared +5% material regression. Qualification spread was 0.50%, every contaminant gate passed, maximum timed
steal was 0.074% under the 0.1% ceiling, and three actual final-binary rows plus raw evidence are now in history.
Earlier diagnostic legs reduced the working-tree dense difference to about +0.90% with overlapping spreads; two later
legs were excluded after unrelated release builds saturated the host. The preceding local fresh-state ABBA
against `dac4ec2` found no material regression above 5% after hardening; stdlib/string improved materially,
closures were neutral-to-faster, and the cooled dense-table confirmation retained a borderline 1.29% slowdown.
Static bytecode and release-disassembly analysis showed that `dense_tables` executes none of the new
`GETTABLEKS`/`SETTABLEKS`/`NAMECALL` opcodes and dispatches 6,964 candidate instructions per unit versus 6,967
for the baseline. A corrective pass therefore targets cross-cutting legacy-path tolls: ordinary fuel
classification uses a base-cost lookup with validated special-cost fallbacks, numeric table access and ordering
take number-first paths, and nil tests use the exact value tag. Logical fuel, validation, numeric-key semantics,
and string canonicalization remain unchanged. The accepted result closes the controlled dense
material-regression gate for this workload and Linux protocol; it is not a blanket release speed claim.
The same dedicated Linode protocol compared `ae3beae2025feb21d0e37c70168204ae154ae125` with exact candidate
`f208c12d0ce266117743569d736346fae9a35db2` on `oop_metamethods`: candidate ns/unit was 65.9594% lower, with a
conservative nominal 95% interval of -66.3876% to -65.5482%, excluding the predeclared zero-regression threshold.
Three final candidate rows and the complete raw evidence are in history. This comparison covers the complete
candidate, including both the constant-field/method paths and its weak-table GC-scan correction; it does not
attribute the full delta to one opcode family.
An evidence-only 512-VM sampler then compared that baseline with exact candidate
`1370d6ccefba7a1956441247c3139af2a2cf31fa` over 16 accepted paired rounds and 160 fresh processes. The landed
shared/inline string representation removed 32 KiB/VM of unloaded short-constant growth, 258 KiB/VM of unloaded
long-constant growth, and 105.39 KiB/VM of loaded-short growth relative to baseline; loaded-long growth fell by
260.97 KiB/VM and retained a 7.08 KiB/VM residual. Descriptive medians show a 4 KiB larger one-time short-program
image and slower program construction in the sampled scenarios, while combined VM creation plus program
execution was faster for every scenario; those timing summaries do not carry the paired confidence intervals of
the primary memory contrasts. This is exact string-focused Linux evidence, not a complete mixed-heap RSS budget.
The subsequent exact-commit `87db17542e19ae77ec92463f8a5b8e8b8a21a666` cross-engine Linode matrix accepted
all native and wasm timed groups on their first attempt with zero timed steal. Against the preceding committed
Linux snapshot, rustia median ns/unit improved 34.97% on `dense_tables`, 27.20% on `sparse_tables`, 27.44% on
`closures_upvalues`, 22.08% on `generic_iteration`, and 22.77% on `stdlib_math_table`. The generated minimum-time
claim gate's `mlua-luau` gap therefore moved from a 10.1x geometric mean (7.1x-14.1x range) to 7.30x
(5.25x-9.53x). This is still development evidence rather than a blanket release claim. The matrix's ephemeral
VM was confirmed deleted before its evidence, native rows, and Node wasm rows were promoted.
The following exact-commit `b1cd2841fb8fc2d916047391a1d757d53a1303cd` matrix accepted all timed groups on
their first attempt and measured the focused numeric default-`table.sort` guard. Against the immediately prior
snapshot, rustia dense-table minimum/median ns/unit improved 7.42%/8.90%, and its minimum-time `mlua-luau` gap
fell from 9.20x to 8.54x. One primary-native steal tick was 0.0118%, within the frozen 0.1% allowance; all other
timed groups observed zero. The VM was confirmed deleted after 1,364 seconds and before promotion, with a
$0.108 rounded compute ceiling. Independent-host movement elsewhere left that snapshot's five-workload gap at a
7.29x geometric mean over a 5.09x-10.02x range.
The exact-commit `d4ec275e19bad0efcf8e96db4030e7380bd93a9b` matrix gates the replacement of the
recursive bespoke numeric-loop kernel with the reusable private `HotIr`. The IR is bounded and source-ordered,
uses reused unboxed numeric scratch, stages every fallible operation before its single permitted table effect,
and deopts to the already-fetched interpreter instruction without replay. Independent review found no semantic
or safety defects; explicit Off/Force parity tests cover all numeric operators, policy gates, deterministic GC,
atomic guard failures, and partial deopt. Against the immediately preceding `3732b80` Linux snapshot, native
median changes were -0.23% dense, -7.14% sparse, +5.74% closures, +5.12% generic iteration, and -0.59% stdlib,
for a +0.47% normalized geometric mean. The exact Linux result therefore preserves aggregate performance but
does not reproduce the local -29.31% dense screen; that local result remains directional only. At that snapshot,
the minimum-time gap was 3.495x to `mlua-luau` (2.688x-4.650x range) and 3.757x to Luaur
(2.722x-5.154x), while rustia beats Ruau on all five workloads. All timed groups passed on their first attempt;
one native steal tick was 0.0095%, all other groups observed zero. The VM was confirmed deleted after 1,529
seconds and before promotion, with a $0.108 rounded compute ceiling.
The prior exact-commit `a5629199643847c0da78792644ad3656887bee21` matrix gates prototype-polymorphic
closure-call fast paths plus direct immutable-upvalue captures. The compiler preserves mutable loop/backedge
capture semantics by discovering captured bindings before final lowering; the VM uses a four-lane call-site PIC
keyed by program/prototype and falls back for policy, guard, recursion, fuel, coverage, and GC boundaries. In the
committed dedicated-Linux matrix comparison, rustia `closures_upvalues` median time moved from 270.427 to
229.766 ns/call (15.036% lower latency, 1.177x throughput); every new raw sample beat every prior sample. Those
Linux snapshots used separate but identically specified dedicated instances, so the cross-host delta is
corroborating rather than a paired-host attribution.
The generated current five-workload minimum-time gaps are 3.494x to `mlua-luau` and 3.833x to Luaur; rustia
beats Ruau on four of five by minimum time. The same cross-host snapshot comparison moved rustia median latency by +2.35% on
`dense_tables`, +12.70% on `sparse_tables`, +8.93% on `generic_iteration`, and +5.74% on
`stdlib_math_table` (five-workload normalized geometric mean +2.45%). Because these are separate instances,
they are follow-up signals rather than attributed regressions; sparse and generic iteration should receive the
next stability check. All timed groups passed on their first attempt, and the VM was confirmed deleted
before the evidence and generated results were promoted, with a $0.108 rounded compute ceiling.
The current exact-commit `83bc2f370ee4149b6fa6e11eca951b41b4cccbd2` matrix adds an explicit
`mlua-luau-jit` mode without changing the interpreter-only headline. A matched JIT-off/JIT-on numeric sentinel
produced the same checksum and a 3.843x native-execution signal before any JIT row was admitted. Across the five
common workloads the reference interpreter is 3.483x rustia by geometric mean and the JIT mode is 4.283x;
closures improve from 64.67 to 28.52 ns/call under JIT, while dense tables remain essentially flat. Preparation,
first-call, combined cold-start, and warmed steady-state samples are all retained; the JIT mode's geometric-mean
cold cost is 677.26 microseconds versus 385.10 microseconds for the reference interpreter. All four timed groups
passed on their first attempt, the primary group observed one steal tick (0.00867%), and the VM was confirmed
deleted after 1,558 seconds before the 65 native rows, 15 wasm rows, and evidence were promoted.
`tools/check-workflows.js` keeps those workflow policies inside `just release-preflight`.

## Project pillars
The canonical decision filter is [`02-pillars.md`](02-pillars.md). These pillars define what counts as current
rustia work and what should be deferred or marked out of scope:

| # | pillar | current meaning |
|---:|---|---|
| 1 | Pinned Standalone Luau Compatibility | Luau 0.709 standalone runtime behavior: parser/compiler/VM/source-visible stdlib, vector-3, flags-off. |
| 2 | Conformance and Provenance | Compatibility claims need fixtures, upstream links, generated audits, benchmark records, or explicit defer reasons. |
| 3 | Deterministic Profiles and Replay ABI | `ReproducibleStandard` is reproducible-conformant; `DeterministicSim` is the stricter lockstep/replay subset. |
| 4 | Rollback-Safe Host Boundary | The host owns cross-tick authoritative state, typed slots, rollback capture, ordered effect queues, and commits. |
| 5 | Layered Rust Embedding API | Small typed facade first; extensions, capabilities, fuel, retained handles, serde, and future interop are layered. |
| 6 | One Safe Native + Wasm Engine | One semantic engine across native and `wasm32-unknown-unknown`, with no public unsafe-code or target-specific semantic fork. |
| 7 | Benchmark-Gated Performance Claims | Public speed claims are per workload/target and generated from committed benchmark history. |
| 8 | Public-Clean OSS Consumer Contract | Public crates stay generic, documented, packageable, semver-scoped, and free of grabby-specific API. |

Important boundary: "Luau compatibility" means pinned standalone Luau runtime compatibility. It does not make
Roblox APIs, latest-Luau churn, Lua 5.4 behavior, analyzer/typechecker parity, bytecode loading, or raw Luau C
API parity release-critical work.

## Known functionality gaps (honest release boundary)

The pending-free conformance corpus proves the named 491-behavior denominator; it does **not** mean that every
production concern or host-facing facility is finished. The highest-impact gaps are:

1. **Bounded object lifetime and memory policy.** Tables, functions, upvalues, coroutine states,
   atomized/non-inline runtime strings, userdata, buffers, vectors, and loaded programs currently occupy stable
   append-only VM slots; unreachable entries are not reclaimed or reused independently of dropping the VM.
   Before public release, rustia needs a safe-Rust reachability/reuse design, explicit heap/allocation limits,
   catchable-or-host OOM policy, and representative long-lived/mixed-heap evidence. Public `Function`/`Exports`
   copies must become real roots or use an ownership/API redesign before slots can be reused: ordinary calls
   stay valid across hot reload outside an explicit reset epoch, while reset may invalidate every handle.
   GC interval tuning alone is not a substitute for reclamation.
2. **A mechanically enforced per-decision lifecycle.** The deterministic host contract says authoritative
   cross-tick state lives in host slots and the script heap is transient, but the public facade does not yet
   provide a decision/scratch reset primitive. Hosts can reconstruct a VM today; the roadmap requires an
   explicit, tested reset/reuse path that cannot silently preserve globals, upvalues, coroutines, or heap state
   across authoritative decisions.
3. **Host-provided runtime services.** The output/time provider slice is implemented
   ([`72-host-services.md`](72-host-services.md)): `print` composes `tostring` per argument (honoring
   `__tostring`), tab-joins, and writes to a host stdout sink when one is attached; no-argument
   `os.time`/`os.date` read a wall-clock provider with a pinned-epoch fallback; and `os.clock` reads a
   monotonic-clock provider with a deterministic logical-clock fallback. Still open:
   `collectgarbage("count")` is zero and `gcinfo` is synthetic — honest internal VM memory
   accounting/telemetry remains. Typed callbacks also need
   a cheap worker-local/per-call host-context seam instead of requiring captured synchronization for mutable
   slot/effect state. Practical multi-file
   programs also need a host-owned `require` resolver, execute-once cache, and cycle detection; resolution
   remains host policy rather than a core filesystem API. Module/extension installation still needs
   transactional or isolated failure semantics.
4. **Complete public value interop.** `OwnedValue` intentionally starts small, but it cannot yet faithfully
   carry binary strings, buffers, vectors, opaque userdata identity, non-scalar/identity table keys,
   cyclic/shared table graphs, or zero-copy live values. Add the live-value serde path and identity-safe handles
   before claiming a general rich embedding boundary; full mlua-style `UserData`/`Scope` registration and async
   callbacks remain later, optional layers.
5. **Semantic-tail and diagnostics hardening.** Continue differential parser/compiler testing, keep valid
   runtime errors catchable at runtime rather than prematurely rejecting them at compile time, and finish
   public stack traces/source diagnostics. Module/extension installation and runtime-failing hot reload also need
   documented transactional or isolated failure semantics; generation counters alone do not roll back partial
   global/heap changes. New gaps require focused Luau-0.709 oracle fixtures.

## 🚀 Next actions (cold-start handoff — do in this order)
**Read order for a fresh agent:** `CLAUDE.md` (auto-loaded) → **this file** → [`02-pillars.md`](02-pillars.md)
→ [`30-architecture.md`](30-architecture.md) (the ADR)
→ the doc for your task. (A fresh Claude session in this repo also auto-loads the project memory.)

**Action queue:**
1. **Design and implement bounded VM object lifetimes.** Add reachability-based reclamation/reuse for ordinary
   VM objects, coroutine state, and loaded programs; true host-handle roots/epoch-safe reuse; heap/allocation
   quotas; an explicit OOM contract; and a deterministic
   per-decision reset/reuse API. Prove it with long-lived churn, cyclic graphs, hot reload, retained handles,
   weak tables, native/wasm parity, and representative mixed-heap memory evidence before tuning final intervals.
2. **Finish the host-facing runtime boundary.** Add honest internal
   memory accounting/telemetry, a worker-local/per-call host context, a host-owned module resolver/cache,
   complete binary/rich-value interop, zero-copy live serde, and transactional install/hot-reload/stack
   diagnostics. Keep these layered and public-clean; do not leak grabby-specific types or a raw C API.
3. **Harden the production engine** against the LOCKED decisions ([`30-architecture.md`](30-architecture.md) §1–§4):
   keep the 16-byte tagged `Value` firewall, portable `br_table`-friendly dispatch, instruction-count fuel,
   deterministic f64/libm behavior, table-key `-0` normalization, profile-specific GC behavior, and the public
   workspace's `unsafe_code = "forbid"` gate. Add differential/fuzz coverage for newly discovered compiler and
   runtime-error-phase edges, and close the full native/browser correctness-build-golden matrix without adding
   developer-machine timings to benchmark history.
4. **Continue measured performance work** using [`56-performance-research.md`](56-performance-research.md).
   The dense-table, constant field/namecall, shared-string, HotIR, and closure-call slices now have accepted
   Linux evidence. Next extend the same verified private HotIR to additional effect/control-flow shapes, pursue
   cached-length and iterator/table refinements, add profile-led superinstructions/O2, and only then productionize
   type-directed wasm/AOT with guards, deopt, and interpreter-equivalence tests.
5. **Advance the `bench/` crate into the standalone OSS benchmark project** — [`80-bench-suite.md`](80-bench-suite.md).
   The runner already exposes the requested engine matrix, five default workloads, 12 workload entries including
   source-covered `string_building`, a coherent Linux-only JSONL result snapshot, package metadata, split notes,
   a standalone HTML report, an explicit closed wasm pre-lock gate artifact, a wasm target bench for rustia+koto+rhai, a `call_ref`
   versus `br_table` dispatch microbench, and an explicit dedicated-Linode native-plus-wasm matrix lifecycle.
   Next work is the actual standalone-project split, the interop/serde/fuel-limit workload tranche, regression
   alerts, and longer-run public history.
6. **Stabilize and publish the public crates.** Settle the consumer API and semver surface, close the generated
   release gates, flip `publish = false` in dependency order, and publish honest per-workload documentation.
7. **Keep the Luau 0.709 conformance corpus green**. The current repo has 692 executable fixtures covering 491
   behavior refs with no pending rows; new engine work should add focused fixtures rather than re-opening
   pending inventory.
8. **Maintain benchmark operations** — hosted CI remains validation-only; the operator-started Linode lifecycle
   is the sole active timing source and must keep its Linux-only completeness and cleanup gates green.

**Pre-release gates:** see the "What's next" section below. Object reclamation/limits and the deterministic
decision-reset contract now precede final GC tuning; benchmark publication and the `LuauIntegerType2` watch
remain ongoing gates.

**Project setup facts:** dual `MIT OR Apache-2.0` (holder Gak Dev Pty Ltd); the public/private boundary +
`private/` mount are in [`CLAUDE.md`](../CLAUDE.md); no Linear.

## Canonical claims (corrected — supersede any stronger wording elsewhere)
- **Perf — state it PER-WORKLOAD, not as a blanket "~5× koto"** (review H2). The historical 11-workload seed
  found reference Luau **~4.7–9× faster than koto** on tables/closures/coroutines/stdlib-math, approximately at
  parity on generic iteration, and slower than koto on pcall/xpcall; those results set must-match workloads, not
  a rustia claim. In the current five-workload Linux snapshot, rustia trails `mlua-luau` by 2.796x-4.880x by
  minimum time with a 3.494x geometric mean, and the generated gate approves no blanket speed headline. Two distinct historical
  koto multipliers remain useful context: the **float-path ~5–9×** (measured) and the
  **deterministic-path ~30–40×** (the sim's *real* incumbent — host fxmul/curve, per `ai_bakeoff`). The fused-op
  sub-µs (~0.2 µs) is **bespoke/private-fork, kernel-specific**. The committed benchmark crate now has five
  default measured workloads plus a source-covered #15 `string_building` workload; rustia's public numbers must
  earn approval from the existing generated gate before claims.
- **wasm cross-engine PRE-LOCK GATE is CLOSED:** production rustia wasm is now measured on V8 (node/deno),
  Bun's JavaScriptCore-family runtime, and SpiderMonkey, and the `call_ref` versus `br_table` dispatch
  microbench is measured on those same families. `br_table` remains the portable dispatch baseline. Real
  Safari/iOS JSC correctness/golden coverage remains an open public-release gate; active timings stay on Linux.
  Wasm-3.0 pass done: `br_table` dispatch stays (tail-calls/`call_ref` don't reopen it — `call_ref` is a cheap
  P1 micro-bench), **wasm-GC rejected** (determinism/layout), relaxed-SIMD stays banned.
- **You CAN JIT in the browser — VALIDATED (spike `65-wasmjit`, verified).** Not native machine code, but you
  emit **wasm** at runtime and V8 compiles it. Measured: a hot kernel emitted to wasm + instantiated runs at
  **~4.6 ns/op = ~176× the interpreter** (within 1.35× of native), **~4.7 µs standup → crossover ≈ 6 calls**;
  shared-heap (imported VM memory) had no penalty. ⇒ wasm-emit JIT/AOT is now a **validated tier**, not a
  research option — the in-browser route to near-native speed. Caveat: monomorphic-f64 kernel; general Luau
  needs **type guards + deopt**, so gate on a hotness counter and use **type-directed compilation** to emit
  guard-free specialized wasm. (Corrects the earlier sloppy "JIT moot in-browser.")
- **SotA performance plan is no-unsafe:** [`56-performance-research.md`](56-performance-research.md) ranks the
  current speed work. Compact deterministic table/atom indices, compact site-scoped live-slot caches,
  mutation-safe `GETIMPORT`, guarded generic fixed-result `FASTCALL`, constant-key
  `GETTABLEKS`/`SETTABLEKS`, two-lane live `NAMECALL` lookup, shared compiled constant storage, and 13-byte inline
  literal/runtime strings are landed. Exact dedicated-Linode evidence now measures 2.42% lower ns/unit for the
  corrective dense candidate, with a conservative 95% interval of -3.35% to -1.57%, and 65.959% lower ns/unit
  for the complete OOP candidate on `oop_metamethods`, with a conservative interval of -66.39% to -65.55%; the latter also includes the
  weak-table GC-scan correction, and both comparisons retain
  three final-candidate history rows. A separate exact 512-VM sampler confirms the shared/inline representation's
  per-VM constant-string savings across 16 paired rounds while exposing the small one-time program-image and
  construction tradeoffs. The first accepted exact cross-engine matrix reduced rustia's median ns/unit by
  22.08%-34.97% across all five tracked workloads versus the previous committed Linux snapshot. The following
  numeric-sort matrix improved dense median time another 8.90% and reduced that workload's `mlua-luau` gap from
  9.20x to 8.54x. The landed private HotIR then retained native aggregate performance on its exact Linux gate
  (+0.47% normalized geometric mean versus the preceding snapshot). The subsequent prototype-polymorphic
  closure-call/direct-immutable-upvalue slice reduced `closures_upvalues` Linux median latency by 15.036%
  (270.427 to 229.766 ns/call) and leaves the current generated minimum-time geometric-mean gaps at 3.494x
  against `mlua-luau` and 3.833x against Luaur; rustia beats Ruau on four of five tracked workloads by minimum
  time. The local -29.31% dense result did not reproduce on Linux and remains directional only. These are workload- and
  protocol-specific results, not blanket speed or RSS claims. Next come bounded object reclamation/quotas,
  broader long-run history, Linux-specific profiling of HotIR, verified multi-effect/control-flow HotIR growth,
  superinstructions/O2, and type-directed wasm/AOT. Native copy-and-patch/JIT
  work remains a research reference unless it can satisfy the public
  `unsafe_code = "forbid"` gate.
- **Luau target PINNED** (review C1): **Luau 0.709** (= our mlua oracle), standalone (not-Roblox) surface,
  vector width 3, flags-off — see [`05-luau-target.md`](05-luau-target.md). **f64-only is VERIFIED correct**
  (`math.type==nil`; integers are default-OFF even at HEAD 0.727 behind `FFlag::LuauIntegerType2`).
- **Conformance corpus is executable and pending-free** (review C2 follow-through): the runner reports
  **692 fixtures**, **491 refs**, all 692 fixtures backed by the Luau 0.709 oracle, **0 pending-oracle**,
  **0 pending-impl**, and **0 rustia-only**. That named executable denominator is authoritative; it does not
  claim a one-to-one audit of every row in the older 709-row design inventory. Future work is focused oracle
  coverage and reconciliation for newly discovered gaps, not treating old model prose as implementation truth.

## Determinism model (one engine, three concrete runtime profiles)
- **LuauCompat** — a real `Profile` variant that currently selects Luau-style hash iteration and exposes the
  Standard service/capability surface. Further reference-oriented knobs require explicit implementation/tests.
- **ReproducibleStandard** (the recommended public default) — targets the full pinned standalone Luau surface
  and is pending-free on the current 491-ref denominator, with reproducible choices for unspecified behavior
  (insertion-order iteration, pinned PCG32, Schubfach tostring, libm). Its `os` table is always registered;
  without providers, `os.time`/`os.date` read the pinned epoch and `os.clock()` returns the deterministic
  logical clock (executed instructions / 1_000_000) instead of the former `0.0`
  ([`72-host-services.md`](72-host-services.md)). It is not an unbounded full-conformance or
  byte-identical-to-reference claim; semantic-tail and full-target gates remain.
- **DeterministicSim** (grabby) — ReproducibleStandard minus the env-dependent globals `collectgarbage`,
  `gcinfo`, and `os.*`, plus any optional host-supplied `loadstring`; it rejects external-mutable-state and
  nondeterministic-effect host registrations. Deterministic output/clock providers are admitted: a stdout
  sink may be attached on any profile, the `os` table (all four members) is registered iff at least one
  deterministic clock provider is plugged, and a nondeterministic provider is rejected by the facade
  `build()` and silently dropped by the infallible VM constructors. Weak tables remain supported and are
  swept on the deterministic
  instruction-count schedule. A sandboxed subset designed for bit-identical replay and verified on the current
  tested matrix. (M2: the Standard modes use an
  **incremental/paced** collection schedule; only DeterministicSim
  uses the full-collect-on-instruction-count trigger.) Scheduling and weak sweeping are implemented; complete
  ordinary-object/program reclamation, reuse, accounting, limits, and tuning remain open.

## Canonical doc map (status)
| doc | role | status |
|---|---|---|
| **CURRENT.md** (this) | the live canonical state | authoritative |
| [`02-pillars.md`](02-pillars.md) | project pillars, non-goals, and the rabbit-hole decision filter | **canonical** |
| [`30-architecture.md`](30-architecture.md) | the ADR (8 axes, decided) v3 plus v3.1-v3.3 refinements/corrections | authoritative for design |
| [`05-luau-target.md`](05-luau-target.md) | the pinned-target spec (Luau 0.709) | **authoritative** ✅ |
| [`50-conformance.md`](50-conformance.md), [`../conformance/COVERAGE.md`](../conformance/COVERAGE.md) | per-behavior conformance table + executable coverage notes | **current runner:** 692 fixtures / 491 refs / 0 pending |
| [`55-improvements.md`](55-improvements.md) | 10 adopted improvements (+ IC/hot-cold promoted) | authoritative |
| [`56-performance-research.md`](56-performance-research.md) | no-unsafe SotA performance stack, top-10 ordered bets, C++ Luau estimates | current research note |
| [`70-host-bridge-contract.md`](70-host-bridge-contract.md) | the deterministic effect-queue seam to grabby's parallel/rollback sim | **authoritative** ✅ |
| [`72-host-services.md`](72-host-services.md) | host services bus: stdout sink, determinism-labeled clock providers, facade presets, logical `os.clock` | **implemented** ✅ |
| [`75-embedded-api.md`](75-embedded-api.md) | Grabby-Compatible Embedded API: public facade + deterministic extension/linker plan | **implemented first slice** ✅ |
| [`76-koto-migration.md`](76-koto-migration.md) | public migration guide from koto-shaped embeddings to the implemented rustia facade | **first slice** |
| [`80-bench-suite.md`](80-bench-suite.md) | continuous perf-bench suite + CI matrix design | **runner/adapters/history/target-first report/package metadata and dedicated Linode lifecycle implemented; standalone split pending** |
| [`90-release-readiness.md`](90-release-readiness.md) | public crate metadata, publish gates, benchmark/CI gates, and release process checklist | **metadata/checklist/changelog/preflight/CI gates started; publish=false remains** |
| [`91-semver-policy.md`](91-semver-policy.md) | public crate roles, stability tiers, release-line rules, and publish order | **policy started; 0.0.x remains unstable** |
| [`crate-metadata.json`](crate-metadata.json) | generated cargo metadata policy audit for all workspace packages | **current audit: 8 packages / metadata policy ok / publish=false** |
| [`package-files.json`](package-files.json) | generated package file-list review for all workspace packages | **current audit: 8 packages / 0 forbidden paths** |
| [`publishability.json`](publishability.json) | generated publishability blocker review for all workspace packages | **current audit: all 8 blocked by publish=false; internal dependency order recorded** |
| [`00-constraints.md`](00-constraints.md) | the contract + target matrix (§C0) | authoritative (target pinned to 0.709) |
| [`15-gapclose.md`](15-gapclose.md), [`10-landscape.md`](10-landscape.md) | earlier snapshots | **superseded** by 20/30/50/55 + this file |

## 3rd-party review (2026-06-30) — status tracker
All 8 findings assessed **VALID** and **actioned** (folded into the docs below).

| # | finding | status |
|---|---|---|
| C1 | pin the Luau target | **RESOLVED** → [`05-luau-target.md`](05-luau-target.md): Luau **0.709** (= our oracle), standalone surface, vector-3, flags-off; **f64-only verified** (`math.type==nil`); int64 is a tripwire on `FFlag::LuauIntegerType2`. |
| C2 | conformance table not trustworthy | **RESOLVED for current corpus** — runner reports 692 Luau 0.709 oracle-backed fixtures, 491 refs, and 0 pending rows. Keep adding targeted oracle fixtures for new bugs/features. |
| H1 | version drift | **FIXED** (this file + banners). |
| H2 | "~5× koto" too narrow | **RESOLVED** — drop the blanket number; state **per-workload** (below). |
| H3 | wasm gate + Wasm 3.0 | **RESOLVED** — Wasm-3.0 assessed (br_table stays; wasm-GC rejected; relaxed-SIMD banned); V8 + Bun/JSC-family + SpiderMonkey production wasm and `call_ref` dispatch measured. |
| M1 | three named modes | **RESOLVED** — one engine with concrete LuauCompat/ReproducibleStandard(default)/DeterministicSim runtime profiles. |
| M2 | Standard incremental GC | **RESOLVED as a scheduling/profile decision** — paced/incremental for Standard, full-collect-on-instruction-count for DeterministicSim; complete ordinary-object reclamation/accounting/tuning remains open. |
| M3 | promote inline caches + hot/cold | **RESOLVED** — compact site-scoped live-slot caches plus `GETIMPORT`/`FASTCALL` and dedicated `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` landed; hot/cold remains P1. |

## What's next (the genuine remaining public-release gates)
The review is actioned (all 8 folded in above + into the docs). The honest residual open items, in order:
1. **Implement and validate bounded object reclamation, quotas, OOM behavior, and per-decision reset/reuse.**
   Coroutine/program stores, true public-handle rooting, weak tables, cyclic graphs, hot reload, and native/wasm
   behavior must remain correct; ordinary retained calls survive hot reload outside an explicit reset epoch.
2. **Complete the host service and rich-value boundary:** honest
   internal VM memory telemetry and quota enforcement, a worker-local/per-call host context, a host-owned
   deterministic module resolver/cache, binary/rich/cyclic values, live-value serde, transactional
   extension/module/hot-reload failure behavior, and stronger traceback diagnostics.
3. **Tune the GC intervals only after reclamation exists** (Deterministic instruction-count interval =
   determinism ABI; Standard paced-debt multiplier) on realistic long-lived and mixed heaps.
4. **Harden compiler/runtime error-phase parity** with focused oracle fixtures plus differential/fuzz testing.
5. **Close the full correctness/build/golden target matrix.** Linux-only timings remain intentional, but public
   release still requires the native and browser targets in [`00-constraints.md`](00-constraints.md) to build
   and reproduce the declared goldens; real Firefox/Safari and non-Linux native coverage is still incomplete.
6. **Record broader production benchmark baselines** for rustia and each completed external adapter. The exact
   dense and OOP fixed-work gates, plus the string-focused 512-VM evidence gate, are committed and resolved for
   their declared Linux protocols. Continue using recorded fixed work, paired order, retained raw samples, and
   actual harness-produced final rows; extend the evidence to longer-run public history and realistic mixed heaps
   without generalizing these workload-specific results. Keep `RESULTS.md`, claims, and the static benchmark view
   consuming the same JSONL history.
7. **Stabilize and publish the crates** in the generated dependency order after release preflight is green.
8. **Watch `FFlag::LuauIntegerType2`** upstream (the f64-only tripwire).

These span the remaining P1 engine hardening and P4 product/release work. The architecture decisions are
complete, but implementation is not release-complete. The cross-engine, conformance, and benchmark gates are
already active; Linode credentials and an explicit operator start remain the external operational prerequisites
for fresh performance measurements.
