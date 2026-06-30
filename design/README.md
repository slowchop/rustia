# rustia — design index

> ⚠️ **Read [`CURRENT.md`](CURRENT.md) first — it is the single canonical state.** This index and the dated
> docs are staged snapshots and can lag; `CURRENT.md` wins on any disagreement. (Drift fixed per review H1.)
> **Status note:** the walking-skeleton number IS measured (see [`30-architecture.md`](30-architecture.md) /
> [`20-skeleton-raw.json`](20-skeleton-raw.json)); the "~5× koto" headline is measured on **one** workload only
> and the broad benchmark + cross-engine (JSC/Firefox) gates are pending — see `CURRENT.md`.

**rustia** = a fast, low-memory, wasm-friendly **Luau interpreter in Rust** with a great embedding API,
built to replace **koto** in `../grabby-aliens/`. Design-first: we explore ~5 directions per decision,
research and spike, then commit. Ground truth for "is it BUILT" will always be the code + tests; these
docs are design intent.

## Status map

| doc | what it covers | status |
|---|---|---|
| [`CURRENT.md`](CURRENT.md) | **the single canonical state** — read first | **authoritative** ✅ |
| [`00-constraints.md`](00-constraints.md) | the fixed contract every decision is scored against (from grabby + gak) | **[FOUNDATION]** ✅ |
| [`05-luau-target.md`](05-luau-target.md) | the pinned Luau target (0.709) — the conformance anchor | **[PINNED]** ✅ |
| [`10-landscape.md`](10-landscape.md) | research brief: koto internals, Rust scripting landscape, Luau spec, official Luau VM, SOTA VM/JIT/WASM techniques (+ raw JSON) | **[RESEARCH v1]** ✅ |
| [`15-gapclose.md`](15-gapclose.md) | **measured** gap-close: 4 spikes + 3 research re-runs; resolved tensions; **every gate GO**; final axes + leanings (+ raw JSON) | **[DECISIONS — measured]** ✅ |
| [`20-skeleton-raw.json`](20-skeleton-raw.json) | walking-skeleton VM spike + gc-arena micro-spike + adversarial verification (raw) | **✅** |
| [`40-spikes/`](40-spikes/) | spike receipts (libm cross-arch hashes, koto profile); spike crates in [`../spikes/`](../spikes/) | **[v1]** ✅ |
| [`25-review-raw.json`](25-review-raw.json) | 5-lens adversarial review of the ADR + P0 superinstruction measurement (raw) | **✅** |
| [`35-axes-raw.json`](35-axes-raw.json) | 5-agent exploration of the open axes (scope, snapshot, value-repr, GC, embedding API) — raw | **✅** |
| [`30-architecture.md`](30-architecture.md) | the architecture decision record + build plan — **v3, all 8 axes resolved on measured evidence** | **[ARCHITECTURE v3]** ✅ |
| [`50-conformance.md`](50-conformance.md) | **per-behavior Luau conformance table** — 709 rows, specified/unspecified→rustia's choice, + the 96-row differential-test watch-list + 80 open decisions (+ raw) | **[CONFORMANCE v1]** ✅ |
| [`55-improvements.md`](55-improvements.md) | **10 adopted design improvements**, each verified still-conformant (dual-part table, string interning, FASTCALL, libm-enforcement, …) | **[IMPROVEMENTS v1]** ✅ |
| [`65-wasmjit.md`](65-wasmjit.md) | **wasm-emit JIT / AOT-to-wasm spike** (verified) — ~176× the interpreter in V8, crossover ≈ 6 calls; the in-browser near-native path | **[SPIKE]** ✅ |

**TL;DR of where we are:** research + measurement say **GO**, now validated by a working VM. A from-scratch
Rust Luau interpreter measured **~0.93 µs/decision native / ~0.99 µs wasm** = **4–6× faster than koto**, with
the deterministic fuel meter ≈ free, full-VM wasm tax ~1.05×, and the **golden hash reproduced bit-identically
across native + node + deno**. Reference Luau is 6–8× koto; f64+libm is bit-identical cross-arch; only `Send`
is needed; embedding Luau-C++-to-wasm is dead; gc-arena is viable (Send via the standard wrapper). The
architecture is decided (`30-architecture.md`); next: adversarial review + measure the one *projected* number
(the fused superinstruction, target ≤0.5 µs).

## The decision axes (each gets ~5 directions explored)

1. **Execution strategy** — tree-walk · register bytecode VM · tiered interp+JIT · wasm-emitting baseline JIT · copy-and-patch · AOT-to-wasm.
2. **Value representation** — NaN-boxing (64-bit) · NaN-boxing (wasm32 / 32-bit ptr) · tagged enum · pointer-tagging · hybrid.
3. **GC / memory model** — Rc/Arc + cycle collector · gc-arena (piccolo-style tracing) · generational incremental · arena/region · hybrid. (Must be Send+Sync; deterministic.)
4. **WASM execution & tiering** — interpreter-only · nested-wasm JIT · copy-and-patch-to-wasm · AOT script→wasm · superinstructions-only. (Only *native-machine-code* JIT can't run inside the wasm sandbox; **wasm-emitting JIT/AOT — generate wasm at runtime, let the host engine compile it — IS viable** and is the in-browser speed lever for hot code.)
5. **Parser / frontend** — reuse `full_moon` · hand-written recursive descent · port the official Luau parser · generator (pest/lalrpop).
6. **Rust embedding API** — mlua-style (`UserData`/`FromLua`/`IntoLua`) · rquickjs-style · native-trait-based · serde-bridge-centric · macro-derived.
7. **Number / determinism strategy (T1)** — how to reconcile Luau's f64-only numbers with grabby's integer/Q16.16 deterministic boundary (buffers · fixed-point userdata · integer mode · provably-deterministic f64 subset).

## How we decide

See the scoring rubric at the bottom of [`00-constraints.md`](00-constraints.md): native speed, wasm32
speed, memory, determinism, Luau compat, Send+Sync, embedding/DX, implementation risk — speed and
determinism dominate.
