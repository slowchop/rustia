# rustia — CURRENT canonical state (read this first)

> **This file is the single source of truth for "where the project actually is."** Other design docs are
> staged snapshots and may lag; where they disagree with this file, **this file wins.** (Created 2026-06-30 to
> fix version-drift flagged by 3rd-party review finding H1.)

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
([`55-improvements.md`](55-improvements.md)), and the standalone benchmark harness has the engine/workload/history
shell, static chart dashboard, 12 workload entries including cross-adapter `string_building` source coverage, and
the `rustia`/`boa`/`koto`/`mlua-lua54`/`mlua-luau`/`rhai`/`roto`/`rquickjs`/`wasmi`/`wasmtime` native adapters in
place.
Its generated [`../bench/CLAIMS.md`](../bench/CLAIMS.md) gate keeps public benchmark language per-workload and
currently approves no blanket speed headline from the committed snapshot.
The public API now has release-preflight-enforced compile-checked examples for embedding and hot reload, plus a
first koto-to-rustia migration guide.
`rustia-cli` supports file, stdin, `--eval`, and `--repl` execution with profile, fuel, source-name options,
state-preserving line evaluation, expression shorthand, and source-aware diagnostics for compile and runtime
failures.
CI wiring now includes hosted release-preflight checks, explicit public-example compilation, plus a manual
benchmark measurement workflow that targets only the generic `[self-hosted, bench]` runner label.
`tools/check-workflows.js` keeps those workflow policies inside `just release-preflight`.

## 🚀 Next actions (cold-start handoff — do in this order)
**Read order for a fresh agent:** `CLAUDE.md` (auto-loaded) → **this file** → [`30-architecture.md`](30-architecture.md) (the ADR)
→ the doc for your task. (A fresh Claude session in this repo also auto-loads the project memory.)

**Action queue:**
1. **Harden the production engine** against the LOCKED decisions ([`30-architecture.md`](30-architecture.md) §1–§4):
   keep the 16-byte tagged `Value` firewall, portable `br_table`-friendly dispatch, instruction-count fuel,
   deterministic f64/libm behavior, table-key `-0` normalization, and profile-specific GC behavior covered by
   tests while performance work proceeds.
2. **Advance the `bench/` crate into the standalone OSS benchmark project** — [`80-bench-suite.md`](80-bench-suite.md).
   The runner already exposes the requested engine matrix, five default workloads, 12 workload entries including
   source-covered `string_building`, append-only JSONL history, package metadata, split notes, static dashboard
   files, and a manual self-hosted measurement workflow. Next work is the actual standalone-project split,
   longer-run public history, and automated tracking publication.
3. **Keep the Luau 0.709 conformance corpus green**. The current repo has 684 executable fixtures covering 481
   behavior refs with no pending rows; new engine work should add focused fixtures rather than re-opening
   pending inventory.
4. **Operationalize benchmark CI** — the public workflow uses only `[self-hosted, bench]`. The owner still needs
   to register that runner before using the manual measurement workflow, and automated history publication is
   not wired yet.

**Pre-P1-LOCK gates** (finalizing the value-layout/dispatch/wasm story waits on these): see the "What's next"
section below for the full list (JSC/Firefox wasm re-measure, `call_ref` bench, GC tuning, `LuauIntegerType2`
watch).

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
- **wasm cross-engine is a PRE-LOCK GATE (still open):** only V8 (Chrome/node/deno) is measured. **JSC
  (Safari/iOS-WKWebView) + SpiderMonkey (Firefox) MUST be measured before the value-layout/wasm lock.**
  Wasm-3.0 pass done: `br_table` dispatch stays (tail-calls/`call_ref` don't reopen it — `call_ref` is a cheap
  P1 micro-bench), **wasm-GC rejected** (determinism/layout), relaxed-SIMD stays banned.
- **You CAN JIT in the browser — VALIDATED (spike `65-wasmjit`, verified).** Not native machine code, but you
  emit **wasm** at runtime and V8 compiles it. Measured: a hot kernel emitted to wasm + instantiated runs at
  **~4.6 ns/op = ~176× the interpreter** (within 1.35× of native), **~4.7 µs standup → crossover ≈ 6 calls**;
  shared-heap (imported VM memory) had no penalty. ⇒ wasm-emit JIT/AOT is now a **validated tier**, not a
  research option — the in-browser route to near-native speed. Caveat: monomorphic-f64 kernel; general Luau
  needs **type guards + deopt**, so gate on a hotness counter and use **type-directed compilation** to emit
  guard-free specialized wasm. (Corrects the earlier sloppy "JIT moot in-browser.")
- **Luau target PINNED** (review C1): **Luau 0.709** (= our mlua oracle), standalone (not-Roblox) surface,
  vector width 3, flags-off — see [`05-luau-target.md`](05-luau-target.md). **f64-only is VERIFIED correct**
  (`math.type==nil`; integers are default-OFF even at HEAD 0.727 behind `FFlag::LuauIntegerType2`).
- **Conformance corpus is executable and pending-free** (review C2 follow-through): the runner reports
  **684 fixtures**, **481 refs**, **684 Luau 0.709 oracle-backed rows**, **0 pending-oracle**, **0 pending-impl**,
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
| [`30-architecture.md`](30-architecture.md) | the ADR (8 axes, decided) v3 + v3.1 refinements | authoritative for design |
| [`05-luau-target.md`](05-luau-target.md) | the pinned-target spec (Luau 0.709) | **authoritative** ✅ |
| [`50-conformance.md`](50-conformance.md), [`../conformance/COVERAGE.md`](../conformance/COVERAGE.md) | per-behavior conformance table + executable coverage notes | **current runner:** 684 fixtures / 481 refs / 0 pending |
| [`55-improvements.md`](55-improvements.md) | 10 adopted improvements (+ IC/hot-cold promoted) | authoritative |
| [`70-host-bridge-contract.md`](70-host-bridge-contract.md) | the deterministic effect-queue seam to grabby's parallel/rollback sim | **authoritative** ✅ |
| [`75-embedded-api.md`](75-embedded-api.md) | Grabby-Compatible Embedded API: public facade + deterministic extension/linker plan | **implemented first slice** ✅ |
| [`76-koto-migration.md`](76-koto-migration.md) | public migration guide from koto-shaped embeddings to the implemented rustia facade | **first slice** |
| [`80-bench-suite.md`](80-bench-suite.md) | continuous perf-bench suite + CI matrix design (some decisions pending) | **runner/adapters/history/site/package metadata/manual benchmark workflow implemented; split/published tracking pending** |
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
| C2 | conformance table not trustworthy | **RESOLVED for current corpus** — runner reports 684 Luau 0.709 oracle-backed fixtures, 481 refs, and 0 pending rows. Keep adding targeted oracle fixtures for new bugs/features. |
| H1 | version drift | **FIXED** (this file + banners). |
| H2 | "~5× koto" too narrow | **RESOLVED** — drop the blanket number; state **per-workload** (below). |
| H3 | wasm gate + Wasm 3.0 | **Wasm-3.0 assessed** (br_table stays; wasm-GC rejected; relaxed-SIMD banned); **JSC/SpiderMonkey measurement STILL the open pre-lock gate.** |
| M1 | three named modes | **RESOLVED** — LuauCompat(knobs)/ReproducibleStandard(default)/DeterministicSim(real build). |
| M2 | Standard incremental GC | **RESOLVED** — paced/incremental for Standard, full-collect-on-instruction-count for DeterministicSim. |
| M3 | promote inline caches + hot/cold | **RESOLVED** — hot/cold→P1, GETTABLEKS IC→P2. |

## What's next (the genuine remaining gates before P1 lock)
The review is actioned (all 8 folded in above + into the docs). The honest residual open items:
1. **JSC (Safari/iOS) + SpiderMonkey (Firefox) wasm measurement** — the real H3 pre-lock gate (needs a real
   luau-on-wasm harness; `alt-luau-wasm` is not one). Final value-layout/dispatch lock is blocked on this.
2. **`call_ref` vs `br_table` micro-bench** on V8/JSC/SpiderMonkey (cheap; expected to confirm br_table).
3. **Tune the GC intervals** (Deterministic instruction-count interval = determinism ABI; Standard paced-debt
   multiplier) on realistic heap sizes at P1.
4. **Record production benchmark baselines** for rustia and each completed external adapter, then make
   `RESULTS.md` and the future static website consume the same JSONL history.
5. **Watch `FFlag::LuauIntegerType2`** upstream (the f64-only tripwire).

These are P1-gated; the design itself is otherwise complete. P1 (engine build) starts with the cross-engine +
conformance + benchmark gates prepared from day one; workflow/config wiring waits until the owner has been told
the required self-hosted runner setup.
