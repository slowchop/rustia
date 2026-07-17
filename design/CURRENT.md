# rustia — CURRENT canonical state (read this first)

> **This file is the single source of truth for "where the project actually is."** Other design docs are
> staged snapshots and may lag; where they disagree with this file, **this file wins.** (Created 2026-06-30 to
> fix version-drift flagged by 3rd-party review finding H1; updated 2026-07-17.)

## One-paragraph state
The **design phase is complete and reviewed**, and P1 implementation is now substantial in production crates.
The current workspace contains the VM/lexer/parser/compiler/public API/CLI/conformance runner, plus an
executable Luau 0.709 conformance corpus with **0 pending-oracle** and **0 pending-impl** rows. The public
embedding facade from [`75-embedded-api.md`](75-embedded-api.md) is implemented at the first useful level:
owned values with typed and nested table helpers, source-named run/compile helpers, typed host functions with
fallible `rustia::Result<T>` returns, pre-install host signature/fuel ABI metadata, retained script function
handles with freshness checks and current-generation accessors, public source-span/runtime-location error
accessors, owned-value/table serde, deterministic extension registration, hot reload, and macro sugar.
A from-scratch Rust Luau
interpreter is validated as **GO** by the earlier spikes and is now backed by the production conformance suite;
performance still needs the continuous benchmark gate before public claims. All 8 architecture axes are decided
([`30-architecture.md`](30-architecture.md) v3), 10 conformance-verified improvements are adopted
([`55-improvements.md`](55-improvements.md)), the no-unsafe SotA performance research stack is now captured in
[`56-performance-research.md`](56-performance-research.md), and the standalone benchmark harness has the engine/workload/history
shell, static chart dashboard, 12 workload entries including cross-adapter `string_building` source coverage, and
the `rustia`/`boa`/`koto`/`mlua-lua54`/`mlua-luau`/`rhai`/`roto`/`rquickjs`/`wasmi`/`wasmtime` native adapters in
place.
The benchmark artifacts now also include a first-class wasm pre-lock gate source
(`../bench/wasm-gate.json`), production wasm target results for `rustia` and
`koto` on V8 plus Bun/JavaScriptCore-family runtimes, and static-site panels
that make the required production rustia wasm measurements explicit.
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
Fused costs are charged before their side effects, preloaded writes retain the original cross-RHS phase, and the
equivalent instruction-count/GC cadence is retained.
Insertion order, metamethod/readonly behavior, mutation visibility, and logical fuel remain preserved.
Compiled constant pools are immutable and shared across program clones retained by different VMs. Long compiled
string atoms reference that storage without copying the bytes per VM, and an equal runtime-owned atom upgrades to
the shared bytes when the compiled literal arrives later. Ordinary loaded literals and runtime-created binary
strings of up to 13 bytes can live directly in the sealed 16-byte `Value`. Constant-key
and cache paths still atomize when identity is required, and table insertion canonicalizes a new non-nil short
key, but lookup misses, nil writes, readonly writes, and invalid-table writes do not create atoms.
`rustia-cli` supports file, stdin, `--eval`, and `--repl` execution with profile, fuel, source-name options,
state-preserving line evaluation, expression shorthand, and source-aware diagnostics for compile and runtime
failures.
CI wiring now includes pinned-toolchain formatting and strict Clippy gates, hosted release-preflight checks,
explicit public-example compilation, plus push, nightly, and manual benchmark measurement modes that target only
the generic `[self-hosted, bench]` runner label and publish generated history to the private benchmark-data repo.
The benchmark harness prepares fresh engine/workload state for calibration and every recorded repetition, warms
that state once outside the timer, and can write execution-ordered raw samples to a separate
`rustia.bench.raw-samples.v1` JSON sidecar. `--fixed-n` bypasses calibration and uses exactly the selected work for
every warmup and timed repetition; sidecars record whether `n` was fixed or calibrated, while the stable public
history remains min/median summary JSONL. The equal-work protocol alternates ABBA/BAAB order and requires at least
three independent publication measurements of the clean final commit, with the sidecar written before each actual
summary append. No accepted exact-final public-history result has been published for the new field/string slice.
Six uncontaminated fixed-work
diagnostic legs reduced the working-tree dense difference to about +0.90% with overlapping spreads; two later
legs were excluded after unrelated release builds saturated the host. The preceding local fresh-state ABBA
against `dac4ec2` found no material regression above 5% after hardening; stdlib/string improved materially,
closures were neutral-to-faster, and the cooled dense-table confirmation retained a borderline 1.29% slowdown.
That older result remains development evidence, not a claim that the dense-table follow-up is resolved or a
blanket release speed claim.
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

## 🚀 Next actions (cold-start handoff — do in this order)
**Read order for a fresh agent:** `CLAUDE.md` (auto-loaded) → **this file** → [`02-pillars.md`](02-pillars.md)
→ [`30-architecture.md`](30-architecture.md) (the ADR)
→ the doc for your task. (A fresh Claude session in this repo also auto-loads the project memory.)

**Action queue:**
1. **Harden the production engine** against the LOCKED decisions ([`30-architecture.md`](30-architecture.md) §1–§4):
   keep the 16-byte tagged `Value` firewall, portable `br_table`-friendly dispatch, instruction-count fuel,
   deterministic f64/libm behavior, table-key `-0` normalization, and profile-specific GC behavior covered by
   tests while performance work proceeds. Keep the public workspace's `unsafe_code = "forbid"` gate intact.
   For performance sequencing, use [`56-performance-research.md`](56-performance-research.md): publish the
   implemented table/atom/cache, `GETIMPORT`/`FASTCALL`, constant field/namecall, and string-representation slice
   through the fixed-work benchmark-history gate; keep the borderline dense-table cost visible until equal-work
   evidence resolves it; then pursue adaptive specialization and type-directed wasm/AOT.
2. **Advance the `bench/` crate into the standalone OSS benchmark project** — [`80-bench-suite.md`](80-bench-suite.md).
   The runner already exposes the requested engine matrix, five default workloads, 12 workload entries including
   source-covered `string_building`, append-only JSONL history, package metadata, split notes, static dashboard
   files, an explicit closed wasm pre-lock gate artifact, a wasm target bench for rustia+koto, a `call_ref`
   versus `br_table` dispatch microbench, and self-hosted push/nightly/manual publication into the private
   benchmark-data repository. Next work is the actual standalone-project split and longer-run public history.
3. **Keep the Luau 0.709 conformance corpus green**. The current repo has 692 executable fixtures covering 491
   behavior refs with no pending rows; new engine work should add focused fixtures rather than re-opening
   pending inventory.
4. **Maintain benchmark CI operations** — the workflow uses only `[self-hosted, bench]`, restores and publishes
   the private benchmark-data repository, and requires the matching runner plus deploy key to remain registered.

**Pre-P1-LOCK gates** (finalizing the value-layout/dispatch/wasm story waits on these): see the "What's next"
section below for the full list (GC tuning, benchmark publication, `LuauIntegerType2` watch).

**Project setup facts:** dual `MIT OR Apache-2.0` (holder Gak Dev Pty Ltd); the public/private boundary +
`private/` mount are in [`CLAUDE.md`](../CLAUDE.md); no Linear.

## Canonical claims (corrected — supersede any stronger wording elsewhere)
- **Perf — state it PER-WORKLOAD, not as a blanket "~5× koto"** (review H2, measured on an 11-workload
  reference-Luau baseline). Reference Luau is **~4.7–9× faster than koto** on tables/closures/coroutines/
  stdlib-math, but **≈ parity on generic iteration** and **koto is *faster* on pcall/xpcall** — so the honest
  public claim is "at/near reference-Luau parity, per-workload," and `generic_iteration` + `pcall` are
  **must-match** workloads. Two distinct koto multipliers: the **float-path ~5–9×** (measured) and the
  **deterministic-path ~30–40×** (the sim's *real* incumbent — host fxmul/curve, per `ai_bakeoff`). The fused-op
  sub-µs (~0.2 µs) is **bespoke/private-fork, kernel-specific**. The committed benchmark crate now has five
  default measured workloads plus a source-covered #15 `string_building` workload; rustia's public numbers still
  need the continuous benchmark gate before claims.
- **wasm cross-engine PRE-LOCK GATE is CLOSED:** production rustia wasm is now measured on V8 (node/deno),
  Bun's JavaScriptCore-family runtime, and SpiderMonkey, and the `call_ref` versus `br_table` dispatch
  microbench is measured on those same families. `br_table` remains the portable dispatch baseline. A Safari
  Technology Preview / `jsc` shell run is still desirable before final public wording for Safari/iOS.
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
  literal/runtime strings are landed. A local fresh-state ABBA for the preceding hardening found no credible
  material regression above 5%, while retaining a borderline +1.29% dense-table result. A fixed-work working-tree
  diagnostic for the newer field/string slice measured about +0.90% across six uncontaminated legs, but exact-final
  evidence and public history are still pending, so neither result is blanket speed evidence. Next come
  equal-work validation and publication, hot-loop hygiene, adaptive quickening, superinstructions/O2, and
  type-directed wasm/AOT. Native copy-and-patch/JIT work remains a research reference unless it can satisfy the
  public `unsafe_code = "forbid"` gate.
- **Luau target PINNED** (review C1): **Luau 0.709** (= our mlua oracle), standalone (not-Roblox) surface,
  vector width 3, flags-off — see [`05-luau-target.md`](05-luau-target.md). **f64-only is VERIFIED correct**
  (`math.type==nil`; integers are default-OFF even at HEAD 0.727 behind `FFlag::LuauIntegerType2`).
- **Conformance corpus is executable and pending-free** (review C2 follow-through): the runner reports
  **692 fixtures**, **491 refs**, **692 Luau 0.709 oracle-backed rows**, **0 pending-oracle**, **0 pending-impl**,
  and **0 rustia-only**. The old warning about ~703 model-generated rows is obsolete; future work is incremental
  coverage for newly discovered gaps, not a bulk oracle re-derivation pass.

## Determinism model (sharpened per review M1 — three named modes)
- **LuauCompat** — match reference Luau as closely as possible, *including* its choices on unspecified behavior
  (for users who need maximal compatibility). May be partly config knobs rather than a full separate build.
- **ReproducibleStandard** (the recommended public default) — fully spec-conformant Luau + our reproducible,
  cross-platform-identical choices for unspecified behavior (insertion-order iteration, pinned PCG32, Schubfach
  tostring, libm). *This is "reproducible-conformant," NOT byte-identical-to-reference-Luau* — the honest
  framing the review demanded.
- **DeterministicSim** (grabby) — ReproducibleStandard **minus** the env-dependent features (weak tables,
  `collectgarbage`, time-based `os.*`) + GC pinned to the instruction counter. A sandboxed subset, bit-identical
  everywhere. (M2: the Standard modes get an **incremental/paced** GC policy; only DeterministicSim uses
  full-collect-on-instruction-count.)

## Canonical doc map (status)
| doc | role | status |
|---|---|---|
| **CURRENT.md** (this) | the live canonical state | authoritative |
| [`02-pillars.md`](02-pillars.md) | project pillars, non-goals, and the rabbit-hole decision filter | **canonical** |
| [`30-architecture.md`](30-architecture.md) | the ADR (8 axes, decided) v3 + v3.1 refinements | authoritative for design |
| [`05-luau-target.md`](05-luau-target.md) | the pinned-target spec (Luau 0.709) | **authoritative** ✅ |
| [`50-conformance.md`](50-conformance.md), [`../conformance/COVERAGE.md`](../conformance/COVERAGE.md) | per-behavior conformance table + executable coverage notes | **current runner:** 692 fixtures / 491 refs / 0 pending |
| [`55-improvements.md`](55-improvements.md) | 10 adopted improvements (+ IC/hot-cold promoted) | authoritative |
| [`56-performance-research.md`](56-performance-research.md) | no-unsafe SotA performance stack, top-10 ordered bets, C++ Luau estimates | current research note |
| [`70-host-bridge-contract.md`](70-host-bridge-contract.md) | the deterministic effect-queue seam to grabby's parallel/rollback sim | **authoritative** ✅ |
| [`75-embedded-api.md`](75-embedded-api.md) | Grabby-Compatible Embedded API: public facade + deterministic extension/linker plan | **implemented first slice** ✅ |
| [`76-koto-migration.md`](76-koto-migration.md) | public migration guide from koto-shaped embeddings to the implemented rustia facade | **first slice** |
| [`80-bench-suite.md`](80-bench-suite.md) | continuous perf-bench suite + CI matrix design | **runner/adapters/history/site/package metadata and push/nightly/manual data-repo publication implemented; standalone split pending** |
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
| M1 | three named modes | **RESOLVED** — LuauCompat(knobs)/ReproducibleStandard(default)/DeterministicSim(real build). |
| M2 | Standard incremental GC | **RESOLVED** — paced/incremental for Standard, full-collect-on-instruction-count for DeterministicSim. |
| M3 | promote inline caches + hot/cold | **RESOLVED** — compact site-scoped live-slot caches plus `GETIMPORT`/`FASTCALL` and dedicated `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` landed; hot/cold remains P1. |

## What's next (the genuine remaining gates before P1 lock)
The review is actioned (all 8 folded in above + into the docs). The honest residual open items:
1. **Tune the GC intervals** (Deterministic instruction-count interval = determinism ABI; Standard paced-debt
   multiplier) on realistic heap sizes at P1.
2. **Record production benchmark baselines** for rustia and each completed external adapter. For the current
   performance slice, use one recorded fixed `n`, retain separate baseline/candidate raw sidecars, and alternate
   ABBA/BAAB run order so drift is visible. Publish only actual harness-produced summary records from at least
   three clean-final-commit legs and withhold an interpretation unless their retained evidence agrees; do not
   treat either the existing +1.29% dense result or the working-tree +0.90% diagnostic as resolved before that
   evidence. Keep `RESULTS.md`, claims, and the static benchmark view consuming the same JSONL history.
3. **Watch `FFlag::LuauIntegerType2`** upstream (the f64-only tripwire).

These are P1-gated; the design itself is otherwise complete. P1 (engine build) starts with the cross-engine +
conformance + benchmark gates prepared from day one; the self-hosted runner and private data-repo deploy key are
external operational prerequisites for fresh CI measurements.
