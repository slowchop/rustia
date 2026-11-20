# rustia — gap-close: measured evidence & resolved decisions

> ⚠️ **SUPERSEDED snapshot.** Written before the walking-skeleton spike. Its "rustia's own µs/decision is
> unmeasured" / "next step" lines are STALE — that number was since measured (~0.9–1.0 µs general / ~0.2 µs
> fused). Canonical state: [`CURRENT.md`](CURRENT.md); design: [`30-architecture.md`](30-architecture.md).

> **Status: [DECISIONS — measured] (2026-06-30).** Closes the gating gaps the research critic raised, with
> **measured** data from 4 spikes + 3 research re-runs. Receipts in [`40-spikes/`](40-spikes/) and
> [`15-gapclose-raw.json`](15-gapclose-raw.json); spike crates in [`../spikes/`](../spikes/). **Verdict:
> every gate reads GO for a from-scratch Rust Luau interpreter.** Two spikes (C, D) hit a structured-output
> formatting failure but DID the work; their numbers below are verified against the on-disk receipts.

## Measured numbers (aarch64-mac, single-threaded, release opt-level 3 — this machine)

### Perf — Luau vs koto on grabby's REAL integer-AI kernel (Spike A)
| path | µs/decision | vs native | agents in ≤5 ms |
|---|---:|---:|---:|
| Rust native floor (Q16.16) | 0.149 | 1.0× | ~33,500 |
| **Luau realistic** (argmax in script, rest Rust — koto's `select()` shape) | **0.73** | ~4.9× | **~6,850** |
| Luau VM-internal (select+SARSA+contagion all scripted; conservative bound) | 1.45 | ~9.7× | ~3,450 |
| koto-float (zero host calls, all scripted) | 4.4 | ~29× | ~1,140 |
| koto-int (host fxmul/curve) | 5.9 | ~40× | ~847 |
| empty mlua Luau call (FFI entry only) | 0.030 /call | — | — |

⇒ Reference Luau is **6–8× faster than koto** on the *real* workload (even the fully-scripted bound is
**3–4×**). The "~4× / ≤1.25 µs MVP" target **holds on the real kernel**, not just a string-sort. This is the
**bar rustia must hit** — it does not inherit it (see remaining unknowns). FFI entry is ~4% of cost; VM
execution dominates (same as koto), so a batched decide-N API helps but isn't the main lever.

### WASM dispatch tax — the go/no-go gate (Spike B), ns/dispatch best-of-5
| dispatch style | native | node V8 | deno V8 | **wasm tax** |
|---|---:|---:|---:|---:|
| **`match`→`br_table`** (recommended) | 1.217 | 1.028 | 1.338 | **0.85–1.10× (parity)** |
| fn-pointer / call-threaded | 2.234 | 2.995 | 3.361 | 1.34–1.51× |

The portable match loop had similar times on native and V8 for this spike. Calling each handler
through a function pointer was slower on both tested targets. In the [spike implementation](../spikes/spike-b-dispatch/src/lib.rs),
each handler returns to the dispatch loop. Tail-call dispatch remains a separate, unmeasured design.

### libm cross-arch bit-identity — gates T1 (Spike C), FNV-1a over a 46-value sweep
| target | combined_canon | combined_raw |
|---|---|---|
| aarch64-mac | `0x4fcaf33760ef5a9a` | 0x0c87e93abd96c30a |
| x86_64-linux (docker) | **`0x4fcaf33760ef5a9a`** | 0xfbb250793a309b2f |
| wasm32 (node) | **`0x4fcaf33760ef5a9a`** | 0xcd6949aece2e598e |

⇒ **f64 + the pure-Rust `libm` crate + NaN/signed-zero canonicalization + no-FMA is bit-identical across the
full target triple.** Per-function transcendental hashes (sin/cos/exp/ln/pow/tan/atan2) match exactly; the
*only* raw divergence is NaN sign/payload (`sqrt(-1)` = `0x7ff8…` mac/wasm vs `0xfff8…` x86_64) and `-0.0`,
both fixed by canonicalization. **T1 is solved without a fixed-point primitive in the core Value.**

### koto cost decomposition — ranks the design levers (Spike D), samply 2 kHz, 24,587 samples
| cost center | self % | rustia lever |
|---|---:|---|
| bytecode decode (`InstructionReader::next`, Iterator-based) | **29.3%** | tight `match`→`br_table` + **superinstructions** |
| `KValue` drop + clone (Arc refcount) | **~20%** | Copy scalars + **traced-heap Gc** (no Arc on hot path) |
| dispatch body (`execute_instructions`) | 9.1% | — |
| range iterator (`run_iterator_next`+`pop_front`) | ~8% | dedicated `FORNPREP`/`FORNLOOP` numeric-for |
| host call (`call_native_function`) | 4.7% | native int ops in-VM remove host-call tax |
| map/global lookup (indexmap) | ~3.3% | import/precomputed-global slots |

## Resolved tensions (with evidence)

- **T1 (f64 vs fixed-point) — RESOLVED.** f64-internal, **no fixed-point primitive in the core Value**:
  f64 + pure-Rust `libm` + canonicalize NaN/`-0.0` + no-FMA. Offer Luau **`buffer`** as the exact-integer
  channel; keep grabby's Q16.16 multiply as a host fn. Luau's int64 (`LUA_TINTEGER`) is FFlag-OFF /
  library-only / not wired to operators — a deferrable opt-in, not a compat requirement. *Evidence: Spike C
  bit-identity; Luau arith opcodes fast-path on f64+vector only; grabby's script-visible ints are <2⁵³
  (exact in f64).*
- **Thread model — UPDATED: live rustia VMs are thread-affine safe Rust.** grabby never shares a `&VM` across threads (sim App
  is `!Send` Bevy `NonSend`, pinned per-tick; console VM is `NonSend`; real threads run *separate* worlds via
  mpsc). `Sync` only for shared **immutable** artifacts (bytecode, LUTs, interner) behind `Arc`. *This
  dissolves the gc-arena objection — you move whole arenas, never share `Gc` across threads.*
- **gc-arena — CONDITIONALLY VIABLE, now the leading GC direction.** Adopt its architecture (incremental
  tracing + mutate-XOR-collect, which *forces* the stackless steppable VM we need); collect at deterministic
  fuel boundaries; keep per-agent AI/rollback state OUT of the GC heap in host columnar arrays. *Evidence:
  safe-only thread-affine VM policy removes the unsafe Send marker; Spike D's ~20% Arc tax favors tracing over Rc/Arc; Luau has **no
  `__gc` finalizers** — one fewer determinism hazard.* **Gated on a 1-day micro-spike** (verify `Arena: Send`
  + serde bridge + steppable ergonomics under the branded `'gc` lifetime).
- **Embed Luau-C++-to-wasm — KILLED, validates from-scratch.** Reproduced locally: mlua+Luau builds/runs
  natively but `cargo build --target wasm32-unknown-unknown` **fails on the first C++ file** (`Ast.h:6:10:
  fatal error: 'iterator' file not found` — wasm32-unknown-unknown ships no C++ stdlib/libc). The only
  working path (emscripten/wasi) produces a different-ABI module that can't compose with grabby's
  wasm-bindgen Bevy module. Keep Luau C++ as a *technique/compat reference* only.

## Minimum-viable Luau scope (for grabby; full Roblox compat deferred)

- **~70 of Luau's ~90 opcodes** — rustia owns its own compiler+bytecode (enables superinstructions; drops ~8
  Roblox-specific opcodes + the class system; lowers FASTCALL* to ordinary CALL; ignores typeinfo/native-codegen bytecode).
- **Syntax (parse+run):** full expression/statement grammar + the cheap Luau niceties scenarios/console use
  (`continue`, compound assignment, `//`, `0x/0b/1_000` literals, string interpolation, generalized iteration,
  if-expressions). **The parser must accept-and-ignore the full Luau *type* grammar** (annotations, generics,
  type functions, attributes) — types are runtime-erased, so the gradual **type *checker* is a separable,
  deferrable subsystem** (needed only for LSP diagnostics, not to run grabby).
- **Libs:** `math` (pure-Rust libm transcendentals), `table`, `string` (incl. Schubfach `tostring` byte-parity +
  the Lua-5.1 pattern engine), `buffer`, `bit32`, base globals, `coroutine`. Schubfach/pattern/PCG32/sort-tie
  byte-parity are discrete porting-risk items for full byte-compat.

## Final design axes + data-justified leanings (ready for 5-direction exploration)

| # | axis | leaning (from data) |
|---|---|---|
| 1 | **Execution/dispatch** | One portable big-`match`→`br_table` core **everywhere** + superinstruction fusion. (Spike B parity; Spike D's 29% decode cost.) JIT = native-only stretch. |
| 2 | **Value representation** | NaN-boxed **or** compact tagged union with **Copy scalars + traced `Gc` heap pointer** — no Arc on the hot path. (Spike D's 20% Arc tax.) *Empirical native-vs-wasm comparison in the skeleton spike.* |
| 3 | **Number/determinism** | f64 + pure-Rust libm + NaN/`-0.0` canon + no-FMA; `buffer` as exact-int channel. (Spike C.) |
| 4 | **GC/memory** | Tracing GC, collect at deterministic fuel boundaries (gc-arena architecture); AI state host-side columnar. Live VMs stay thread-affine safe Rust. Gated on gc-arena micro-spike. |
| 5 | **WASM/JIT tiering** | Interpreter-primary one-core everywhere; native-only JIT stretch; AOT-script→wasm a research option; embed-Luau killed. |
| 6 | **Parser/frontend** | Own hand-written recursive-descent → rustia's own bytecode (enables superinstructions); `full_moon` as a differential **compat oracle**. |
| 7 | **Steppability/fuel** | Stackless trampoline VM + **per-instruction** deterministic fuel + recursion cap; coroutines as resumable contexts. |
| 8 | **Embedding API** | Layer: mlua-style ergonomic core + koto-shaped compat shim + serde bridge; **mandatory batched decide-N API**; inner tight math native. Hot-reload = re-eval. |

## Remaining unknowns (what the next spikes must close)

1. **rustia's OWN µs/decision is unmeasured** — Spike A measured *reference* Luau (the bar), not rustia. The
   single biggest open number. ⇒ **walking-skeleton spike.**
2. The determinism-SAFE (Q16.16/no-FMA/canonicalized, golden-reproducing) path perf — Spike A used f64 as a
   perf proxy.
3. Full-VM wasm tax (Spike B was dispatch+ALU core only; a real VM adds memory bounds checks, heap/string
   loads, GC, `call_indirect`); and on x86/mobile-ARM V8 + JSC/SpiderMonkey (untested).
4. gc-arena `Arena: Send` + serde + steppable ergonomics — indirect evidence only.
5. Per-VM RSS (priority #2 has no number; VM count = thread count, not agent count).
6. Whether the production AI even runs *inside* the rollback sim (rides the 24-deep ring, ≤1 ms restore) or
   as a separate non-rolled-back layer; and whether any transcendental runs in the 50 Hz per-tick path at all.

## Next step (recommended)

Build the **rustia walking-skeleton VM spike** baking in the leanings (big-`match`→`br_table` + a few
superinstructions; NaN-boxed/Copy-scalar value + traced heap; f64+libm+canon number core with dedicated
numeric-for; per-instruction fuel), port `ai_bakeoff`'s `KotoSelector` → `RustiaSelector`, and measure
**rustia's own µs/decision on aarch64-mac native AND wasm32-in-V8** against the 0.73 µs reference-Luau bar +
the golden cross-arch hash (real bit-determinism, not just the libm sweep). In parallel: the **gc-arena
micro-spike**. Then run the formal 5-direction exploration of the genuinely-open axes (value-repr, GC,
parser, embedding) using these leanings as priors.
