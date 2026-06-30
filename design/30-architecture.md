# rustia — architecture decision record + build plan (v3, axes resolved)

> **Status: [ARCHITECTURE v3 — all axes resolved on measured evidence] (2026-06-30).** v1→v2 was a 5-lens
> adversarial review (LOCK-WITH-CHANGES); v2→v3 closed the three gated axes (B/D/H) with spikes + recovered the
> snapshot question, and folds in gak's public-release + superinstruction-policy steer. **All 8 axes now
> decided.** One residual caveat blocks a *final-final* stamp: confirm the value-layout + wasm numbers on **JSC
> (iOS/WKWebView)** and SpiderMonkey. Evidence: [`15-gapclose.md`](15-gapclose.md), [`35-axes-raw.json`](35-axes-raw.json),
> [`40-spikes/`](40-spikes/), [`25-review-raw.json`](25-review-raw.json), spike crates in [`../spikes/`](../spikes/).
>
> **v3.1 refinements (2026-06-30):** the full per-behavior conformance table is in [`50-conformance.md`](50-conformance.md)
> (709 rows + a 96-item differential-test watch-list + 80 open decisions), and **10 adopted, conformance-verified
> improvements** are in [`55-improvements.md`](55-improvements.md) — they REFINE the axes below (notably: axis D's
> IndexMap → Luau's **dual-part table** with `shift_remove` order; a **per-Program `Arc` string-intern pool +
> short-string inlining** resolving the const-pool seam; generic **FASTCALL/GETIMPORT**; a **capability+fuel
> contract on the op-extension hook**; **clippy-enforced libm-only** transcendentals; **saturating float→int**).

## 0. The bet — proven, with honest framing

A from-scratch **Rust Luau interpreter** replaces koto: deterministic (native+wasm), wasm-ready at ~parity,
and **~5× faster than koto on general scripts**. **rustia is intended to be released publicly** (open-source)
once it reaches full Luau parity; the public product is a clean, general-purpose Luau engine.

**Two perf regimes — keep them separate (the v1 blur the review corrected):**

| workload | rustia µs/decision | vs koto | what it is |
|---|---:|---:|---|
| Rust native floor (no script) | 0.149 | 35× faster | the asymptote |
| **general Luau script** (tables/strings/closures, real `call_indirect`) | **~0.9–1.0** (native + wasm) | **~5×** | the **public** headline; ≈ parity w/ reference Luau |
| IAUS scalar kernel (tagged Value) | 0.64 native / 0.67–0.71 wasm | ~7–9× | a tight numeric leaf |
| **bespoke fused op** (`OP_UTIL`) | 0.20 native / 0.24 wasm | ~25× | **private-fork only** — bakes grabby's AI math into one opcode |
| koto (today) | 4.0–5.9 | 1× | the incumbent |

- **General-script ~0.9–1.0 µs is the durable, public claim** — measured on the real heap path (agent H
  vertical slice), native + wasm at parity, canon-ON f64 golden `0xa6816bc158ca5244` bit-identical native+V8.
- **Sub-µs is kernel-specific.** It needs a *bespoke* fused op that moves the hot leaf's math into native Rust.
  That is the architectural prize — it lets grabby's per-agent AI return to script — **but it is not "general
  Luau is 25× faster," and it does not belong in public rustia.** See §2-A / §7 (public/private).

Determinism is measured on the real kernel (golden reproduced native+node+deno); f64 cross-arch bit-identity
proven (Spike C); deterministic fuel ≈ free.

## 1. Architecture overview

One opcode-handler core, **one portable engine for all targets** (native + wasm32; no per-target VM fork).

```
rustia-lexer      hand-written Luau lexer
rustia-parser     recursive-descent → AST; FULLY PARSES the Luau type grammar (generics, type packs,
                  f<a,b>(c) disambiguation) but does NOT typecheck (types runtime-erased; the gradual
                  type CHECKER is a separate, deferred, LSP-only subsystem)
rustia-compiler   AST → rustia bytecode; constant folding (SAME pinned libm + canon as the VM), builtin
                  recognition, GENERIC superinstruction fusion, register allocation; O0 mode for hot-reload
rustia-vm         16-byte tagged Value, gc-arena heap, big-match→br_table dispatch, per-instruction fuel +
                  recursion cap, stackless trampoline, coroutines; an OP-EXTENSION HOOK for host-registered ops
rustia-stdlib     math (pure-Rust libm), table, string (string.format + Lua-5.1 patterns, ASCII/C locale),
                  bit32, base globals, coroutine  [+ buffer/vector/utf8/string.pack for full public parity]
rustia            embedding API: koto-shaped Vm facade + typed FromValue/IntoValue core + ZERO-COPY serde
                  bridge; lifetime-free owned handles (the 'gc brand never leaks to the embedder)
rustia-cli        REPL / runner
```

`full_moon` is a **dev-dependency only** — a differential Luau-compat oracle the parser is fuzzed against in CI.

## 2. Decisions (all axes; directions considered → decision → evidence)

**A. Execution / dispatch — LOCKED: one portable big-`match`→`br_table` loop everywhere + GENERIC
superinstructions, plus an op-extension hook.** Considered: target-split CPS/tail-call; single portable match
[✓]; closure/threaded; fused-decode; two-tier native JIT. *Evidence:* Spike B + skeleton → match→br_table at
~parity in V8; fn-pointer/CPS strictly worse on both targets. **Superinstruction policy (gak):** generic,
app-agnostic superinstructions (numeric-for, NAMECALL, FASTCALL, GETIMPORT — the families Luau itself uses)
are **welcome in public** because they speed up *any* Luau and preserve compatibility. **Bespoke/app-specific
fused ops** (grabby's `OP_UTIL`) are **private-fork only**, registered through a public **op-extension hook**
so the fork is a thin delta, not a hard fork. JIT-to-native = optional native-only stretch tier.
**Wasm-3.0 verdict (review H3):** shipped wasm tail-calls (`return_call`, now in V8/SpiderMonkey/JSC) do **NOT**
reopen this — Spike B's threading loss was `call_indirect`'s per-opcode runtime *type + bounds check*, not
missing tail-call optimization, so `br_table` (one in-function branch, zero call overhead) still wins. The one
contender is **`call_ref`** (typed funcref, skips the runtime type-check) — scheduled as a *cheap P1 micro-bench*
vs `br_table` (expected to lose). **wasm-GC is REJECTED** for the script heap (objects aren't byte-addressable,
engine-defined layout → forfeits deterministic full-collect + host-columnar rollback AND isn't bit-identical
across engines); rustia keeps gc-arena on linear memory. **relaxed-SIMD stays banned** (FMA/rounding
nondeterminism); only fixed `f32x4` where proven bit-identical. **M3:** the **hot/cold opcode split moves to P1**
(tight hot-core br_table + `#[cold]` rare ops — de-risks the JSC/SpiderMonkey baseline tiers); the **GETTABLEKS
shape inline cache moves to P2** (depends on the dual-part table + atoms; at P1 only reserve the aux/cache word
+ a per-VM IC side-table keyed by IP — never mutate the shared `Arc<Program>`; charge the same fuel on hit/miss
to keep the fuel-ABI fusion-invariant).

**B. Value representation — LOCKED: 16-byte tagged union (Copy scalars + payload word + tag); NOT NaN-box.**
*(Overturns the v1/v2 leaning.)* Considered: NaN-box 8B; **16-byte tagged [✓]**; koto 24B [rejected];
pointer-tag; per-target hybrid. *Evidence (spike-b-value, independently re-run):* on the f64-saturated hot
path, **tagged is 0.642 µs vs NaN-box 0.954 µs (33–45% faster) native, and 35–54% faster in wasm/V8** —
NaN-boxing forces a GP↔FP register domain-cross on every arithmetic op (it also reproduced the skeleton's old
0.93 µs, proving the skeleton silently paid that tax). Tagged is layout-neutral on heap ops, and **eliminates
the NaN-box soundness hazard** (512/2048 NaN payloads aliased a tag before canon; tagged has zero aliasing).
**And agent H independently required tagged** — gc-arena's tracer must see real `Gc` pointers (NaN-box would
need hand-written `unsafe Collect`). Cost: 16B vs 8B (2× Value memory) — accepted (speed #1 > memory #2).
*Residual:* confirm on JSC/SpiderMonkey; keep `Value` a sealed newtype so it stays swappable through P1.

**C. Number model / determinism — LOCKED: f64 + pure-Rust `libm` + per-op NaN canonicalization + no-FMA;
`bit32` (and later `buffer`) as the exact-integer channel.** *Evidence:* Spike C bit-identity
(`0x4fcaf33760ef5a9a`); agent-H canon-ON f64 golden `0xa6816bc158ca5244` bit-identical native+V8. No
fixed-point primitive in the Value. **Canon split (confirmed correct by the f64 golden):** NaN-collapse to one
quiet-NaN at **every** f64-producing op; `-0.0`→`+0.0` **only at table-key normalization** (preserves Luau
`1/-0.0 = -inf` sign-of-zero). Luau int64 is FFlag-off/library-only → deferred.

**D. GC / memory — LOCKED: gc-arena 0.6 tracing GC for the script heap, collected on the deterministic
instruction counter (full collection); ALL rollback-relevant AI state stays host-columnar (off the GC heap) —
a hard constraint.** Considered: gc-arena [✓]; slotmap [✓ for host-columnar state, not the main heap];
hand-rolled mark&sweep [fallback]; Rc+cycle-collector [rejected]; region/no-GC for the kernel [✓ complement].
*Evidence (spike-d-gc):* all three flagged integration risks **retired by measurement** — write barrier is
**free** (3.18 ns/mut in Marked phase = no-barrier; `make_gray_again` is `#[cold]`), the **retain pattern**
works (a `DynamicRoot` survived **1000 forced collections**, `fetch` ~0.69 ns; deep-copy-out is a one-time
export cost), and the **`unsafe impl Send` move path is UB-clean under miri Stacked + Tree Borrows** across two
thread moves. Throughput == slotmap == plain Vec (3.18 vs 3.14 ns/op). **slotmap rejected as the main heap**
(its only edge is snapshot, which is O(heap) not O(1), and it has no cycle reclamation). Two API rules: (i) a
`VmCell` that keeps the arena + all retained `DynamicRoot`s moving together (so a `!Send` handle can't be
separated from its arena); (ii) collection trigger is the **instruction counter, never byte-debt** (object
sizes differ 32/64-bit; byte-accounting ignores `Vec` backing).

**E. Determinism enforcement — LOCKED.** Insertion-ordered maps (seedless FxHash IndexMap). Per-op NaN canon
(soundness + cross-engine). Ban FMA/fast-math/relaxed-SIMD. All transcendentals via the `libm` crate —
**including the compiler's constant folder** (hot-reload recompiles per platform → folder is on the bit-parity
path).

**F. Parser / frontend — LOCKED: own hand-written recursive-descent → rustia bytecode; full_moon as a dev-only
oracle.** Fully parses the Luau type grammar (real work) but does not typecheck. Floored `%` (Lua/Luau is
floored; Rust truncates), comparison-based `math.min`/`max`. Not consuming Luau LBC v7.

**G. Steppability / fuel — LOCKED: stackless trampoline + per-instruction deterministic fuel + recursion cap;
coroutines as resumable contexts.** Fuel charged at the **logical/source-op level** (fusion-invariant) or via a
**versioned cost table** — changing the superinstruction set is a fuel-ABI break. Fuel exhaustion is a
**distinct error**, not silent `nil`.

**H. Embedding API — LOCKED: koto-SHAPED facade over a typed `FromValue`/`IntoValue` core + a ZERO-COPY serde
bridge; defer mlua `UserData`/`Scope`.** Considered: koto-shim [✓ outer]; mlua-style [partial — take the typed
core]; both-layered [✓]; trait-objects [defer]; serde-centric [✓ as a sub-facility]. *Evidence (agent-H
vertical slice):* the host-fn ABI `Box<dyn Fn(&[OwnedValue]) -> Result<OwnedValue>>` is both the real
`call_indirect` target *and* the `'gc` firewall, and covers **100% of grabby's host fns**. **Zero-copy serde**
(serde directly over live in-arena values inside `mutate`, behind `vm.deserialize_exports::<T>()`) is **30%
faster** than the literal `exports().clone()`-then-serde (1.01 vs 1.47 µs; the snapshot deep-copy is the cost,
not serde at 0.065 µs). **`exports()` = "the table the chunk returns"** (Luau has no `export{…}`). **0 lines of
embedder-side unsafe** for the heap/serde path. **Batched decide-N demoted to optional P4** (FFI is ~4%;
contradicts the per-agent `Selector` shape).

## 3. Determinism: two profiles, not a fork (gak, 2026-06-30)

Determinism work splits in two, and only the second part needs a flag:
- **Group A — free & fully-conformant (ALWAYS ON):** pure-Rust libm, IEEE-exact/no-FMA, NaN canon,
  insertion-ordered tables, fixed seeded PCG32 `math.random`, Schubfach `tostring`, instruction-fuel limit.
  The Luau spec leaves transcendentals/NaN-bits/iteration-order unspecified, and PCG32+Schubfach are *what
  Luau already does* — so these are a legit (better, reproducible) Luau implementation, removing nothing.
- **Group B — genuinely removes a real Luau feature (PROFILE-GATED):** weak tables (`__mode`),
  `collectgarbage()`, time-based `os.*`. (GC-on-instruction-count is script-invisible → effectively free.)

⇒ **TWO built profiles from one codebase, surfaced under THREE names** (review M1):
- **`ReproducibleStandard` (the public default):** full, conformant Luau (Group A on + Group B available),
  already cross-platform-reproducible for any script that doesn't touch Group B. *Honest framing: this is
  reproducible-conformant, NOT byte-identical to reference Luau on unspecified behavior (iteration order,
  transcendental ULPs) — by deliberate, spec-legal choice.*
- **`DeterministicSim` (grabby/sim) — a real build, not a knob:** ReproducibleStandard **minus** Group B
  (capability-removed, raising `ErrorKind::CapabilityRemoved`, never silent `nil`) + frozen base tables
  (enabling compile-time FASTCALL fold) + GC pinned to the instruction counter. A sandboxed *restriction* —
  anything that runs here runs identically under Standard. **Bit-identical everywhere, always.**
- **`LuauCompat` — a posture, not a third engine:** a set of *knobs over Standard* (e.g. reference-style NaN
  sign, unpinned/hash iteration order, system-libm) for users needing maximal byte-parity with reference Luau.
  True parity on unspecified behavior is unachievable without surrendering the determinism wins, so this is
  config + a test posture, **demoted from a full mode**.
Mechanism: a per-VM runtime setting; PLUS an optional **compile-time feature** to hard-strip Group B for a
minimal locked-down sandbox/wasm build.

**GC policy per profile (review M2): two policies, one gc-arena collector.**
- **ReproducibleStandard:** **incremental / paced** collection (gc-arena pacing, allocation-debt driven) →
  bounded pause times for console/mod/browser-frame VMs; GC timing is script-invisible-but-not-bit-reproducible
  (fine for Standard; matches Luau's own incremental mark-&-sweep; the write barrier is measured free at
  3.18 ns/mut). Expose a pacing multiplier.
- **DeterministicSim:** **full-collect-to-completion**, triggered ONLY by the instruction counter (never
  byte-debt — object sizes differ 32/64-bit), no observable partial-incremental state. **Version the collection
  interval as part of the determinism ABI.** Both policies produce identical script *results*; only pause
  shape/timing differs. Tune both on realistic heap sizes at P1.

## 3b. Determinism + snapshot spec (operational — applies to the Deterministic profile)

1. Bit-identical script result on x86_64-linux / aarch64-mac / wasm32 — pinned cross-arch golden that
   **exercises the f64 path** (`0/0`, `inf-inf`, `sqrt(-1)`, `0*inf`, `-0.0`, large/small), not just integers.
2. Maps iterate in **insertion order** (seedless FxHash IndexMap).
3. **f64 / Value soundness:** canonicalize NaN at **every** f64→`Value` ingress (arith, `FORNLOOP`, host-fn
   return, serde-in, coercion, literal, constant fold). `-0.0` canon only at table keys. No FMA/fast-math/
   relaxed-SIMD. *(Tagged Value removes the NaN-aliasing memory-soundness angle; canon remains required for
   cross-engine determinism — wasm NaN payloads are nondeterministic.)*
4. **Deterministic fuel** (instruction-count) + recursion cap; fusion-invariant or versioned.
5. **GC determinism:** collection driven **only by the instruction counter** (never byte-debt); **full
   collection to completion** (no observable partial-incremental state); **weak tables (`__mode`) +
   `collectgarbage` forbidden in the deterministic profile** (Luau has no `__gc` — one hazard already gone).
6. **Snapshot / rollback (the gc-arena "no cheap snapshot" answer):** rollback state never enters the GC heap;
   it lives **host-columnar** (`Vec<AgentState>`, `clone_from` with buffer reuse). Measured (spike-snap, 4000
   agents × 400 B, 24-deep ring): **snapshot 17 µs / restore 18 µs — PASS (≤1 ms), ring = 36.6 MiB.** Slotmap
   29/29 µs; persistent `im`/`rpds` give O(1) ~ns snapshots but tax every mutation 1.7–4× (tick 86–194 µs vs
   49 µs) — so host-columnar `Vec` wins for grabby's mutate-heavy/snapshot-light cadence.
7. Exact-integer work via `bit32`/`buffer` or host fns — never lossy f64 round-trips. RNG/hashing via grabby's
   recipe (ChaCha8, FNV-1a, integer ε-greedy).
8. **JIT determinism contract (the wasm-emit JIT is a new determinism surface).** On the authoritative
   (`DeterministicSim`) path the JIT is allowed ONLY under a **JIT≡interpreter equivalence guarantee**: the
   interpreter is the deterministic ORACLE, and a JIT'd function is trusted on the golden only if proven
   bit-identical to interpreting it (a differential check). It emits under the **same f64 discipline as the
   interpreter — scalar or strict *fixed-reduction-order* SIMD only; NO autovectorized reductions, NO FMA, NO
   relaxed-SIMD; canonical NaN/`-0`.** The hotness trigger runs off **deterministic state** (call/instruction
   count, never wall-clock) so the JIT *decision* is lockstep-identical, and codegen is deterministic (same
   bytecode → same bytes). Given equivalence, JIT timing/presence is determinism-IRRELEVANT (a peer interpreting
   vs a peer that has JIT'd the same function get identical bits). Anything not provably equivalent (e.g.
   autovectorized reductions chased for speed) is fenced to `ReproducibleStandard`/view-side, off the golden.
   *(Measurement honesty: our f64 bit-identity results — Spike C, the interpreter golden, the shipped
   runtime-gen JIT — were all SCALAR; the only SIMD that may have appeared was LLVM autovectorizing the
   static-AOT speed reference. SIMD determinism is NOT proven; it is handled by the discipline above + the oracle.)*
9. **Cross-tick authoritative state MUST be host-columnar, never the GC heap / Lua globals / upvalues** (the
   rollback invariant, from the parallelism agent). Any script state that persists across ticks and affects the
   sim lives in host-provided **typed slots in the captured columnar arrays**; the GC heap stays
   transient/reconstructable, out of the 24-deep rollback ring. A script stashing persistent decision state in a
   global/upvalue = an un-captured authoritative-state leak → replay desync. `DeterministicSim` enforces this.
10. **Effect-merge / tick-phase contract (the host-bridge):** scripted leaves run in the parallel-solve phase,
    read ONLY the frozen previous-tick snapshot (never another lane's this-tick writes), and emit into a per-lane
    typed effect-queue; the host commits effects single-threaded, sorted by **(emitter SimId, intra-agent
    emit-seq)** — deterministic regardless of lane/worker/timing. No `&World`/`&mut` sharing.

## 4. Build plan

- **P0 / P0.5 — DONE ✅.** Fused-op (~0.20 µs, private-fork validation) + the vertical slice (general-script
  ~0.9–1.0 µs, canon-ON golden, real `call_indirect`, gc-arena heap, retain+serde) measured. Axes B/D/H closed.
- **P1 — engine core.** Lexer + recursive-descent parser (full type-grammar parse, no typecheck) + compiler
  (constant-fold w/ pinned libm+canon, generic-superinstruction fusion, regalloc, O0 mode); `rustia-vm` with
  the **16-byte tagged Value**, gc-arena heap + write barriers + the `VmCell` Send unit + the op-extension hook,
  big-match dispatch, fuel + recursion cap, stackless trampoline. Host-columnar rollback region. Cross-arch
  golden (incl. f64) in CI across the FULL target matrix (§C0): native macOS-arm64/**x86_64-Intel**/Windows/
  Linux-x86_64/arm64, and wasm in **V8 (Chrome) + SpiderMonkey (Firefox) + JSC (Safari/iOS)**. Parser fuzzed vs
  full_moon.
- **P2 — minimum-viable Luau** (grabby's first private use): ~70 opcodes; full grammar + Luau niceties; libs
  math/table/string(+format+patterns)/bit32/coroutine + base globals; metatable OOP; floored `%`. Byte-compat:
  Schubfach `tostring` (high), `string.format` (high), ASCII/C-locale pattern classes (high), PCG32 (low).
- **P3 — drop into grabby (private fork).** koto-shaped shim + typed core + zero-copy serde. Wire behind the
  `Selector` trait + the `ai_bakeoff` Criterion harness as a continuous perf/golden gate. Rewrite the `.koto`
  scenario/console scripts → `.luau`. Register grabby's bespoke fused AI op via the op-extension hook.
- **P4 — full public parity + release.** The deferred public-parity libs come IN: **`buffer`, `vector` (+lib),
  `utf8`, `string.pack`**, the rest of `math`/`string`/`table`, `require`/module system, and the **gradual type
  *checker*** (the big LSP-only subsystem). Differential conformance vs full_moon / reference Luau. Per-VM RSS
  budget + measurement (priority #2). Inline caches (speed-only). License + clean public codebase (no
  grabby-specific code). *Optional:* native-only JIT (cranelift first). **VALIDATED tier — wasm-emit
  JIT/AOT-to-wasm (the in-browser speed answer, spike `65-wasmjit`):** emitting a hot kernel to wasm at runtime
  (via `wasm-encoder`) and instantiating it in V8 runs at **~4.6 ns/op = ~176× the interpreter** (within 1.35×
  of native LLVM), for a **~4.7 µs one-time standup → crossover ≈ 6 calls**; generated modules can import the
  VM's linear memory (no bounds-check penalty). Gate emission behind a **hotness counter** (`calls × interp_cost
  > ~5 µs`). Caveat: measured on a **monomorphic f64** kernel; dynamically-typed Luau needs **type guards +
  deopt/fallback** — which is exactly where **type-directed compilation** pays (Luau's types let us emit
  guard-free specialized wasm). So the in-browser high-speed path = **type-directed AOT-to-wasm for hot,
  proven-monomorphic kernels.** Use sync `new WebAssembly.Module` for the tiny (<1.1 KB) modules.

## 5. Public vs private (the release model)

- **Public rustia** = a clean, general-purpose, fully-Luau-compatible engine. Generic superinstructions for
  speed. An **op-extension hook** + a sealed embedding API. No grabby-specific code. License TBD (P4).
- **gak's private fork** = registers bespoke fused ops (the AI-kernel `OP_UTIL`, ~25× koto) through the hook —
  a thin delta tracking upstream, never a hard fork.
- **Parity bar for public release = full Luau** (the §2-F deferred items return at P4); grabby's *first* private
  use runs on the P2 minimum-viable subset.

## 6. Risk register (post-resolution)

| risk | sev | status / mitigation |
|---|---|---|
| Value-layout + wasm numbers are **V8-only / arm-Mac+Linux-x86 only**; **Firefox (SpiderMonkey)**, **Safari/iOS (JSC)**, **Intel Mac**, Windows, ARM-Linux untested | **high** | the one open gate (§C0 matrix) — re-measure at P1 before the final stamp; `Value` sealed/swappable until then |
| General-script perf (~5× koto) is now **measured** (agent-H slice) but on one workload | med | broaden workloads at P1/P3; keep the Criterion gate |
| `unsafe impl Send` soundness is an argument + one miri pass | med | `VmCell` encapsulation; keep miri/loom in CI; the move path is already UB-clean under both borrow models |
| GC full-collect cost scales O(live-objects) (~10 ns/obj; ~200 µs at 20k objects) | med | tune the instruction-count collection interval at P1 on realistic heap sizes; confirm bit-reproducible native vs wasm |
| Byte-compat tail (Schubfach, patterns, sort-tie, PCG32) | med | discrete P2 tasks, each gated by a differential test |
| Per-VM RSS unmeasured (priority #2); 16B Value doubles value memory; GcBox +16 B/obj; grows-only wasm memory | med | measure at P1; set a per-VM budget; wasm binary already 128 KB |
| Const-pool / `Gc`-string seam (shared `Arc<Program>` can't hold per-arena `Gc` constants) | med | resolve the interning policy + equality/hashing when real string constants land (P1) |
| Public-release scope (full Luau parity) is a long tail | low | P4; grabby ships privately on the P2 subset first |

## 7. Open sub-decisions (deferred, not blocking)
- Final string-interning / const-pool representation (Arc<str> host leaves vs per-VM materialization).
- Exact generic-superinstruction set beyond numeric-for — driven by P4 profiling of real scripts.
- Whether production AI runs inside the rollback sim (host-columnar) or as a separate non-rolled-back layer.
- Whether any transcendental runs in the 50 Hz per-tick path (vs one-shot scenario gen).
- Native JIT: cranelift vs copy-and-patch, if pursued at all.
- License choice for the public release.
