# rustia — CURRENT canonical state (read this first)

> **This file is the single source of truth for "where the project actually is."** Other design docs are
> staged snapshots and may lag; where they disagree with this file, **this file wins.** (Created 2026-06-30 to
> fix version-drift flagged by 3rd-party review finding H1.)

## One-paragraph state
The **design phase is complete and reviewed**, and P1 implementation has started in production crates. The
current workspace contains the initial VM/lexer/parser/compiler/API/CLI/conformance scaffold, plus the full
Luau 0.709 conformance corpus inventoried as `pending-impl`; full Luau semantics are not implemented yet. A
from-scratch Rust Luau interpreter is validated as **GO**: a walking-skeleton VM measured **~0.9–1.0 µs per
decision on one real workload** (≈ parity with reference Luau, **~4–6× faster than koto**) on native **and**
wasm-in-V8, deterministic across Linux-x86/Mac-ARM/Chrome-wasm. All 8 architecture axes are decided
([`30-architecture.md`](30-architecture.md) v3), a 709-row Luau conformance table exists in draft
([`50-conformance.md`](50-conformance.md)), and 10 conformance-verified improvements are adopted
([`55-improvements.md`](55-improvements.md)). Future benchmark workflow files may target only the generic
`[self-hosted, bench]` label; CI workflow wiring is not implemented yet.

## 🚀 Next actions (cold-start handoff — do in this order)
**Read order for a fresh agent:** `CLAUDE.md` (auto-loaded) → **this file** → [`30-architecture.md`](30-architecture.md) (the ADR)
→ the doc for your task. (A fresh Claude session in this repo also auto-loads the project memory.)

**Action queue:**
1. **P1 — build the engine core** (main path). Build to the LOCKED decisions ([`30-architecture.md`](30-architecture.md) §1–§4):
   16-byte **tagged** `Value` (sealed newtype — keep swappable until the gate below), one portable
   big-`match`→`br_table` dispatch + generic superinstructions, **gc-arena** GC on the instruction counter,
   own hand-written recursive-descent **parser** (parse types, don't typecheck), per-instruction **fuel**,
   f64 + pure-Rust **libm** + per-op NaN canon plus table-key `-0` normalization. Much of P1 (lexer/parser/compiler/VM scaffold) is NOT
   gated → can start now. Prepare the cross-arch golden + conformance + bench gates, but do not create or edit
   CI workflow/config files until the owner is told exactly what the self-hosted runner setup needs.
2. **Build the `bench/` crate** (framework + native baseline engines; wasm/JSC P1-gated) — [`80-bench-suite.md`](80-bench-suite.md).
   Fully self-hosted (iai-callgrind + criterion + github-action-benchmark). Runs in parallel with P1.
3. **Port the upstream Luau 0.709 test corpus** into this repo as rustia conformance fixtures, alongside the
   row-by-row oracle re-derivation in [`50-conformance.md`](50-conformance.md). Keep the tests public and
   provenance-clean; generated/private harness notes stay in `private/`.
4. **Wire benchmark CI** — before starting any CI configuration, notify the owner of the required setup. Then
   use only `[self-hosted, bench]` in public workflow files; keep infra details out of this public repo.

**Pre-P1-LOCK gates** (finalizing the value-layout/dispatch waits on these — build behind the sealed `Value`
meanwhile): see the "What's next" section below for the full list (JSC/Firefox wasm re-measure, `call_ref` bench,
conformance re-derive, GC tuning, `LuauIntegerType2` watch).

**Project setup facts:** dual `MIT OR Apache-2.0` (holder Gak Dev Pty Ltd); the public/private boundary +
`private/` mount are in [`CLAUDE.md`](../CLAUDE.md); no Linear.

## Canonical claims (corrected — supersede any stronger wording elsewhere)
- **Perf — state it PER-WORKLOAD, not as a blanket "~5× koto"** (review H2, now measured on an 11-workload
  reference-Luau baseline). Reference Luau is **~4.7–9× faster than koto** on tables/closures/coroutines/
  stdlib-math, but **≈ parity on generic iteration** and **koto is *faster* on pcall/xpcall** — so the honest
  public claim is "at/near reference-Luau parity, per-workload," and `generic_iteration` + `pcall` are
  **must-match** workloads. Two distinct koto multipliers: the **float-path ~5–9×** (measured) and the
  **deterministic-path ~30–40×** (the sim's *real* incumbent — host fxmul/curve, per `ai_bakeoff`). The fused-op
  sub-µs (~0.2 µs) is **bespoke/private-fork, kernel-specific**. The 11-workload suite is now the **CI
  regression gate**; rustia's own numbers come when the engine boots.
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
- **Conformance table is PARTIALLY verified** (review C2): **6 contested areas oracle-corrected** in
  `50-conformance.md` v1.1 (`table.maxn`/`getfenv`/`setfenv`/`math.map` are PRESENT — the table wrongly said
  absent; `pairs` does *not* use `__iter`; Luau has no proper-tail-call guarantee). The remaining ~703 rows are
  still model-generated and become test-backed at P1 (mechanical now that spec == oracle 0.709).

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
| [`50-conformance.md`](50-conformance.md) | per-behavior conformance table | **v1.1** — 6 rows oracle-corrected; ~703 pending P1 (C2) |
| [`55-improvements.md`](55-improvements.md) | 10 adopted improvements (+ IC/hot-cold promoted) | authoritative |
| [`70-host-bridge-contract.md`](70-host-bridge-contract.md) | the deterministic effect-queue seam to grabby's parallel/rollback sim | **authoritative** ✅ |
| [`80-bench-suite.md`](80-bench-suite.md) | continuous perf-bench suite + CI matrix design (some decisions pending) | **authoritative** ✅ |
| [`00-constraints.md`](00-constraints.md) | the contract + target matrix (§C0) | authoritative (target pinned to 0.709) |
| [`15-gapclose.md`](15-gapclose.md), [`10-landscape.md`](10-landscape.md) | earlier snapshots | **superseded** by 20/30/50/55 + this file |

## 3rd-party review (2026-06-30) — status tracker
All 8 findings assessed **VALID** and **actioned** (folded into the docs below).

| # | finding | status |
|---|---|---|
| C1 | pin the Luau target | **RESOLVED** → [`05-luau-target.md`](05-luau-target.md): Luau **0.709** (= our oracle), standalone surface, vector-3, flags-off; **f64-only verified** (`math.type==nil`); int64 is a tripwire on `FFlag::LuauIntegerType2`. |
| C2 | conformance table not trustworthy | **6 contested rows oracle-corrected** (`50-conformance.md` v1.1: `table.maxn`/`getfenv`/`setfenv`/`math.map` are PRESENT — were wrongly "absent"; `pairs`≠`__iter`; no proper-tail-calls). **~703 rows still pending full oracle re-derivation at P1.** |
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
3. **Re-derive the remaining ~703 conformance rows** against the 0.709 oracle (mechanical; spec == oracle).
4. **Tune the GC intervals** (Deterministic instruction-count interval = determinism ABI; Standard paced-debt
   multiplier) on realistic heap sizes at P1.
5. **Watch `FFlag::LuauIntegerType2`** upstream (the f64-only tripwire).

These are P1-gated; the design itself is otherwise complete. P1 (engine build) starts with the cross-engine +
conformance + benchmark gates prepared from day one; workflow/config wiring waits until the owner has been told
the required self-hosted runner setup.
