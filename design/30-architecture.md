# rustia — architecture decision record + build plan (v3, axes resolved)

> **Status: [ARCHITECTURE v3 — all axes resolved on measured evidence] (2026-06-30).** v1→v2 was a 5-lens
> adversarial review (LOCK-WITH-CHANGES); v2→v3 closed the three gated axes (B/D/H) with spikes + recovered the
> snapshot question, and folds in gak's public-release + superinstruction-policy steer. **All 8 axes now
> decided.** Production wasm and dispatch are now measured across V8, Bun's JavaScriptCore-family runtime, and
> SpiderMonkey; real Safari/iOS JSC plus the remaining native correctness/build/golden matrix are still-open
> public-release gates. Timings remain dedicated-Linux-only.
> Evidence: [`15-gapclose.md`](15-gapclose.md), [`35-axes-raw.json`](35-axes-raw.json),
> [`40-spikes/`](40-spikes/), [`25-review-raw.json`](25-review-raw.json), spike crates in [`../spikes/`](../spikes/).
>
> **v3.1 refinements (2026-06-30 historical snapshot):** the per-behavior conformance table was then 709 design
> rows; the live executable state is now tracked in [`CURRENT.md`](CURRENT.md) at 694 fixtures / 493 refs /
> 0 pending. **10 adopted, conformance-verified
> improvements** are in [`55-improvements.md`](55-improvements.md) — they REFINE the axes below (notably: axis D's
> flat map → Luau's **dual-part table** with stable logical-deletion order; a **per-Program `Arc` string-intern pool +
> short-string inlining** resolving the const-pool seam; generic **FASTCALL/GETIMPORT**; a **capability+fuel
> contract on the op-extension hook**; **clippy-enforced libm-only** transcendentals; **saturating float→int**).
>
> **v3.2 implementation update (2026-07-20):** production tables retain tombstones for ordinary nil deletion;
> VM-local byte atoms and larger hash parts use compact deterministic open-addressed `u32` side indices. Loaded
> programs store compact `u32` constant atoms and a 4-byte per-PC site map, with lazily allocated 16-byte
> shape/key/live-slot records at eligible sites. Mutation-safe `GETIMPORT` and guarded generic fixed-result
> `FASTCALL` are implemented with ordinary semantic fallbacks and logical fuel preserved. Immutable compiled
> constant storage is shared across VMs, long compiled atoms borrow those bytes, and binary strings up to 13
> bytes can live inside `Value`. Dedicated `GETTABLEKS`, `SETTABLEKS`, and two-lane live `NAMECALL` lookup are
> implemented; full Luau `__namecall` context remains a separate conformance item. Exact dedicated-Linux runs
> now retain accepted dense/OOP throughput evidence plus a separate paired 512-VM string-memory corpus.
>
> **v3.3 implementation update (2026-07-27):** the collection scheduling/profile decision remains locked.
> Complete object/program reachability and generation-safe reuse, aggregate logical-live quotas with atomic OOM,
> epoch-safe decision-reset ABI v1, maintained live-memory accounting, and the graph-preserving rooted
> `Value`/`Table`/`Function` handle spine are implemented. Broader mixed-heap native/Wasm pressure evidence,
> live-object constructors/chunk loading, and live serde remain release gates.

## 0. The bet — proven, with honest framing

A from-scratch **Rust Luau interpreter** replaces koto with one deterministic native+wasm engine.
**rustia is intended to be released publicly** as a pinned standalone Luau-compatible Rust engine once its
conformance, benchmark, API, and release gates are satisfied. Performance language is generated per workload
from committed benchmark history; the current gate approves no blanket speed headline.

**Historical spike regimes — useful architecture evidence, not current public claims:**

| workload | rustia µs/decision | vs koto | what it is |
|---|---:|---:|---|
| Rust native floor (no script) | 0.149 | 35× faster | the asymptote |
| **general Luau script** (tables/strings/closures, real `call_indirect`) | **~0.9–1.0** (native + wasm) | **~5×** | an earlier vertical-slice result; not a blanket release claim |
| IAUS scalar kernel (tagged Value) | 0.64 native / 0.67–0.71 wasm | ~7–9× | a tight numeric leaf |
| **bespoke fused op** (`OP_UTIL`) | 0.20 native / 0.24 wasm | ~25× | **private-fork only** — bakes grabby's AI math into one opcode |
| koto (today) | 4.0–5.9 | 1× | the incumbent |

- **General-script ~0.9–1.0 µs is historical workload evidence** — measured on the earlier real-heap vertical
  slice, native + wasm at parity, canon-ON f64 golden `0xa6816bc158ca5244` bit-identical native+V8. Current
  production claims still require committed benchmark history and generated approval.
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
rustia-vm         16-byte tagged Value, VM heap + profile-specific gc-arena scheduling, big-match→br_table
                  dispatch, per-instruction fuel + recursion cap, stackless trampoline, coroutines, and the
                  math/table/string/bit32/base/coroutine/buffer/vector/utf8/string.pack standard-library surface
rustia            embedding API: koto-shaped Vm facade + typed FromRustia/IntoRustia core + owned-value serde;
                  VM/epoch-rooted live Value/Table/Function handles preserve graph identity without leaking the
                  'gc brand; constructors, chunk loading, and live serde remain planned advanced ergonomics
rustia-cli        REPL / runner
```

The reference Luau 0.709 frontend/runtime corpus is the current differential oracle. Additional parser fuzzing
against an independent frontend remains planned; `full_moon` is not currently a workspace dependency.

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
contender was **`call_ref`** (typed funcref, skips the runtime type-check); the production microbench now
confirms `br_table` across the measured engine families. **wasm-GC is REJECTED** for the script heap (objects aren't byte-addressable,
engine-defined layout → forfeits deterministic full-collect + host-columnar rollback AND isn't bit-identical
across engines); rustia keeps gc-arena on linear memory. **relaxed-SIMD stays banned** (FMA/rounding
nondeterminism); only fixed `f32x4` where proven bit-identical. **M3:** the **hot/cold opcode split moves to P1**
(tight hot-core br_table + `#[cold]` rare ops — de-risks the JSC/SpiderMonkey baseline tiers). Per-loaded-program
cache metadata uses a 4-byte site-map entry per PC and allocates 16-byte shape/key/live-slot records only when
eligible table/import sites fill. `GETIMPORT` reads live values through up to three guarded slots; generic
fixed-result `FASTCALL` guards an exact native function handle and falls back to ordinary resumable calls.
Neither mutates shared `Program` data or changes logical fuel. Dedicated
`GETTABLEKS`/`SETTABLEKS`/`NAMECALL` specialization is now implemented through the same per-VM live-slot
cache model; full Luau `__namecall` context remains separate conformance work.

**B. Value representation — LOCKED: 16-byte tagged union (Copy scalars + payload word + tag); NOT NaN-box.**
*(Overturns the v1/v2 leaning.)* Considered: NaN-box 8B; **16-byte tagged [✓]**; koto 24B [rejected];
pointer-tag; per-target hybrid. *Evidence (spike-b-value, independently re-run):* on the f64-saturated hot
path, **tagged is 0.642 µs vs NaN-box 0.954 µs (33–45% faster) native, and 35–54% faster in wasm/V8** —
NaN-boxing forces a GP↔FP register domain-cross on every arithmetic op (it also reproduced the skeleton's old
0.93 µs, proving the skeleton silently paid that tax). Tagged is layout-neutral on heap ops, and **eliminates
the NaN-box soundness hazard** (512/2048 NaN payloads aliased a tag before canon; tagged has zero aliasing).
**And agent H independently required tagged** — gc-arena's tracer must see real `Gc` pointers (NaN-box would
need hand-written `unsafe Collect`). Cost: 16B vs 8B (2× Value memory) — accepted (speed #1 > memory #2).
*Residual:* complete the native matrix and confirm in a real Safari/iOS JSC environment; keep `Value` a sealed
newtype so it stays swappable through P1.

**C. Number model / determinism — LOCKED: f64 + pure-Rust `libm` + per-op NaN canonicalization + no-FMA;
`bit32` and `buffer` as the exact-integer channels.** *Evidence:* Spike C bit-identity
(`0x4fcaf33760ef5a9a`); agent-H canon-ON f64 golden `0xa6816bc158ca5244` bit-identical native+V8. No
fixed-point primitive in the Value. **Canon split (confirmed correct by the f64 golden):** NaN-collapse to one
quiet-NaN at **every** f64-producing op; `-0.0`→`+0.0` **only at table-key normalization** (preserves Luau
`1/-0.0 = -inf` sign-of-zero). Luau int64 is FFlag-off/library-only → deferred.

**D. GC / memory — LOCKED target: one safe tracing/reclaiming VM heap using gc-arena 0.6 where appropriate,
with allocation-debt pacing for Standard/LuauCompat and deterministic instruction-count full-cycle boundaries
for DeterministicSim; ALL rollback-relevant AI state stays host-columnar (off the script heap) — a hard
constraint.** Considered: gc-arena [✓ component]; slotmap [✓ for host-columnar state, not the main heap];
hand-rolled mark&sweep [fallback]; Rc+cycle-collector [rejected]; region/no-GC for the kernel [✓ complement].
*Evidence (spike-d-gc):* the key GC integration risks **retired by measurement** — write barrier is
**free** (3.18 ns/mut in Marked phase = no-barrier; `make_gray_again` is `#[cold]`), and the **retain pattern**
works (a `DynamicRoot` survived **1000 forced collections**, `fetch` ~0.69 ns; deep-copy-out is a one-time
export cost). The old cross-thread live-VM move path was measured UB-clean, but production now avoids that
marker entirely: a live `VmCell` is thread-affine while it owns its gc-arena component and side stores. Throughput == slotmap == plain Vec
(3.18 vs 3.14 ns/op). **slotmap rejected as the main heap** (its only edge is snapshot, which is O(heap) not
O(1), and it has no cycle reclamation). Two API rules: (i) hosts parallelize with worker-local VMs and shared
immutable compiled artifacts/configuration, not by moving live VMs; (ii) DeterministicSim collection uses the
instruction counter, never byte-debt, while Standard/LuauCompat may use allocation debt.

**Production implementation gap (2026-07-27):** ordinary reachability and generation-safe reuse cover tables,
functions, upvalues, atomized runtime strings, userdata, buffers, and vectors; retained host values are explicit
roots. Host-requested quiescence also prunes unreachable coroutine state and releases spare heap/interner/stack
capacity. Loaded programs use generation-safe reusable handles and host-requested quiescence drops obsolete
program bytecode, constants, inline caches, and coverage rows after the final reachable function disappears.
Script-triggered and quota-forced collection now perform the same program-reachability cleanup while treating
active top-level calls and suspended coroutine frames as program roots.
`Vm::memory_stats()` now exposes VM-owned live/reserved bytes and optional instruction-boundary peaks. Mark
scratch uses compact generation arrays instead of general-purpose tree nodes. Heap/program totals are
maintained and VM-control totals are cached with local structural refresh, so quota admission and the
script-visible counters avoid a quadratic whole-VM census. `VmSettings::memory_limit_bytes` now enforces a
deterministic logical-live quota: pressure forces one
collection, preflights large string/table/buffer allocations, stages growing table writes before commit, and
returns catchable `OutOfMemory`. Exact reserved-byte telemetry remains an on-demand census, and opt-in peak
tracking deliberately samples it at instruction boundaries. Exact-commit Linode cohort
`safe-memory-588b541` retained 99 native samples after confirmed deletion and passed the wasm32 512 KiB
quota/recovery probe. Rustia tied the 256 KiB best shared-workload peak and retained a 14.4 KiB shared mean,
below the frozen mlua/Luau/JIT reference's 24.8 KiB. Decision-reset ABI v1 now discards every guest allocation
and retained root, restores deterministic state, invalidates handles by epoch, and remains exactly flat after
warm-up across 600 focused native decisions. A Node/V8 wasm32 probe verifies the same lifecycle and exact
plateau across 300 more decisions. The remaining axis-D work is broader mixed-heap native/Wasm pressure
coverage before interval tuning is meaningful.

**E. Determinism enforcement — LOCKED.** Dense `Vec` array parts plus insertion-ordered `Vec` hash entries;
larger hash parts use a stable custom-mixed open-addressed `Vec<u32>` side index at no more than 50% load, and
logical nil deletion retains tombstones. The index stores authoritative-entry indices rather than duplicate
keys, is released below the 16-entry threshold or on clear, and never defines iteration order. VM-local string
interning uses the same compact deterministic open-addressing principle and compares bytes in authoritative
storage. Per-op NaN canon
(soundness + cross-engine). Ban FMA/fast-math/relaxed-SIMD. All transcendentals via the `libm` crate —
**including the compiler's constant folder** (hot-reload recompiles per platform → folder is on the bit-parity
path).

**F. Parser / frontend — LOCKED: own hand-written recursive-descent → rustia bytecode; pinned Luau 0.709
reference corpus as the current oracle, with independent frontend fuzzing planned.** Fully parses the Luau type grammar (real work) but does not typecheck. Floored `%` (Lua/Luau is
floored; Rust truncates), comparison-based `math.min`/`max`. Not consuming Luau LBC v7.

**G. Steppability / fuel — LOCKED: stackless trampoline + per-instruction deterministic fuel + recursion cap;
coroutines as resumable contexts.** Fuel charged at the **logical/source-op level** (fusion-invariant) or via a
**versioned cost table** — changing the superinstruction set is a fuel-ABI break. Fuel exhaustion is a
**distinct error**, not silent `nil`. `GETIMPORT` therefore costs `2n - 1` fuel for an `n`-component path
(1/3/5 for one/two/three components), matching the unfused global-plus-index lowering's total cost on cache hit
or miss. The fused instruction reserves that total atomically before its first lookup, so insufficient fuel
cannot expose a partially resolved import path.
`FASTCALL` costs the same one base fuel as `CALL`; native variable-work fuel is charged exactly once, while cache
misses and non-native/reassigned callees use the ordinary call path with the same yield, error, and continuation
behavior.

**H. Embedding API — LOCKED direction: koto-SHAPED facade over a typed `FromRustia`/`IntoRustia` core, with
lifetime-safe per-invocation host contexts, transactional records, an owned bridge first, and a future live
ZERO-COPY serde layer; defer mlua `UserData`/`Scope`.** Considered:
koto-shim [✓ outer]; mlua-style [partial — take the typed core]; both-layered [✓]; trait-objects [defer];
serde-centric [✓ as a sub-facility]. *Historical vertical-slice evidence:* the spike used
`Box<dyn Fn(&[OwnedValue]) -> Result<OwnedValue>>` as its `call_indirect` target/GC firewall and covered the
then-audited grabby host functions. Production now separates a lifetime-independent `HostSchema` from a fresh
borrowed `HostContext` on every `Rustia::invoke`; capability-specific views expose input, slots, transactional
effects/writes, or profile-gated external mutation without storing context pointers in the VM. The public typed
wrappers snapshot/convert guest values and load owned returns. **Zero-copy serde**
(serde directly over live in-arena values inside `mutate`, behind `vm.deserialize_exports::<T>()`) is **30%
faster** than the literal `exports().clone()`-then-serde (1.01 vs 1.47 µs; the snapshot deep-copy is the cost,
not serde at 0.065 µs). **`exports()` = "the table the chunk returns"** (Luau has no `export{…}`). **0 lines of
embedder-side unsafe** for the spike heap/serde path. Production currently exposes owned-value/table serde;
live-value zero-copy serde remains a release-roadmap item. Authoritative action leaves use the generic
`GuestSurface::HardenedLeaf` layer plus a borrowed `CompiledEntry`: the top-level decision body runs directly
without retained exports or loaded-program installation, while inputs/effects remain in the typed invocation
transaction. `invoke_entry_into` couples successful lane publication with an unconditional decision reset, so
the same lifecycle also recovers from failed actions and contained host panics. **Batched decide-N demoted to
optional P4** (FFI is ~4%; contradicts the per-agent `Selector` shape).

## 3. Determinism: one engine, three runtime profiles (implementation-corrected 2026-07-23)

Determinism work splits in two, and only the second part needs a flag:
- **Group A — spec-legal reproducibility foundations (ALWAYS ON):** pure-Rust libm, IEEE-exact/no-FMA, NaN canon,
  stable insertion-ordered table storage, fixed seeded PCG32 `math.random`, Schubfach `tostring`, and an
  instruction-fuel limit. ReproducibleStandard/DeterministicSim expose insertion-order iteration; LuauCompat
  deliberately selects Luau-style hash iteration over the same authoritative storage.
  The Luau spec leaves transcendentals/NaN-bits/iteration-order unspecified, and PCG32+Schubfach are *what
  Luau already does* — so these are a legit (better, reproducible) Luau implementation, removing nothing.
- **Group B — restricted in `DeterministicSim`:** `collectgarbage`, `gcinfo`, `os.*`, optional host-supplied
  `loadstring`, and host registrations that read external mutable state or emit nondeterministic effects. Weak
  tables remain supported and are swept on deterministic instruction-count boundaries.

⇒ **ONE engine with THREE concrete `Profile` variants** (review M1):
- **`ReproducibleStandard` (the public default):** targets the full pinned standalone surface with Group A on
  and the Standard service surface available; the current 493-ref executable denominator is pending-free.
  It is cross-platform-reproducible for scripts inside the proven surface that avoid environment services.
  *Honest framing: this is
  reproducible-conformant, NOT byte-identical to reference Luau on unspecified behavior (iteration order,
  transcendental ULPs) — by deliberate, spec-legal choice.*
- **`DeterministicSim` (grabby/sim):** ReproducibleStandard minus the restricted Group B surface, with
  collection/weak sweeping pinned to the instruction counter.
  Missing environment globals currently fail through ordinary nil-call behavior; extension capabilities fail
  explicitly at registration. More precise service diagnostics remain a host-boundary hardening item.
- **`LuauCompat`:** a concrete runtime profile that currently selects reference-style hash iteration and shares
  the Standard service/capability surface. Additional compatibility knobs require explicit evidence rather than
  being implied by the name.
Mechanism: a per-VM runtime setting. There is no separate engine build or current Cargo feature that hard-strips
the restricted globals.

The three profiles are still the complete semantic profile set. A separate `GuestSurface` setting narrows what
compatible language environment a particular embedding exposes. `GuestSurface::HardenedLeaf`, selected by
`RustiaBuilder::deterministic_leaf()`, layers on `DeterministicSim` for authoritative stateless actions: globals,
standard libraries, and host namespaces are frozen; environment/debug/raw mutation, RNG, clocks/output,
buffers/vectors/coroutines, dynamic compilation, and captured upvalues are absent or rejected. A mandatory
logical-live memory limit and a versioned guest-surface ABI make this a fail-closed embedding contract without
inventing a fourth profile. See [`77-hardened-leaf-environment.md`](77-hardened-leaf-environment.md).

**GC scheduling per profile (review M2): two triggers over the current hybrid heap.**
- **ReproducibleStandard/LuauCompat:** allocation-debt-driven gc-arena pacing and weak sweeping are implemented.
  This is not yet a bounded-pause collector for ordinary language objects because their side stores are not
  reclaimed. Add reclamation/accounting first, then expose and tune a pacing multiplier.
- **DeterministicSim:** an instruction-count trigger finishes the gc-arena cycle and sweeps weak tables (never
  byte-debt, because object sizes differ 32/64-bit). **Version the collection interval as part of the determinism
  ABI.** Add ordinary-object reclamation, then tune on realistic heaps. Weak-table disappearance can occur at a
  different point than Standard by design, but remains deterministic within this profile.

## 3b. Determinism + snapshot spec (operational — applies to the Deterministic profile)

1. Bit-identical script result on x86_64-linux / aarch64-mac / wasm32 — pinned cross-arch golden that
   **exercises the f64 path** (`0/0`, `inf-inf`, `sqrt(-1)`, `0*inf`, `-0.0`, large/small), not just integers.
2. Tables iterate array slots in ascending order followed by hash entries in **insertion order**; logical
   deletion retains a tombstone so `next(t, k)` can resume.
3. **f64 / Value soundness:** canonicalize NaN at **every** f64→`Value` ingress (arith, `FORNLOOP`, host-fn
   return, serde-in, coercion, literal, constant fold). `-0.0` canon only at table keys. No FMA/fast-math/
   relaxed-SIMD. *(Tagged Value removes the NaN-aliasing memory-soundness angle; canon remains required for
   cross-engine determinism — wasm NaN payloads are nondeterministic.)*
4. **Deterministic fuel** (instruction-count) + recursion cap; fusion-invariant or versioned.
5. **GC determinism:** collection driven **only by the instruction counter** (never byte-debt); **full
   collection to completion** (no observable partial-incremental state); weak tables are swept at those
   deterministic boundaries, while `collectgarbage`/`gcinfo` are absent (Luau has no `__gc`).
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
   global/upvalue = an un-captured authoritative-state leak → replay desync. `DeterministicSim` hosts enforce
   this with the implemented `reset_decision()` ABI at every authoritative decision boundary.
10. **Effect-merge / tick-phase contract (the host-bridge):** scripted leaves run in the parallel-solve phase,
    read ONLY the frozen previous-tick snapshot (never another lane's this-tick writes), and emit into a per-lane
    typed effect-queue; the host commits effects single-threaded, sorted by **(emitter SimId, intra-agent
    emit-seq)** — deterministic regardless of lane/worker/timing. No `&World`/`&mut` sharing.

## 4. Build plan

- **P0 / P0.5 — DONE ✅.** Fused-op (~0.20 µs, private-fork validation) + the vertical slice (general-script
  ~0.9–1.0 µs, canon-ON golden, real `call_indirect`, gc-arena heap, retain+serde) measured. Axes B/D/H closed.
- **P1 — engine core (substantial, release gates remain).** Lexer + recursive-descent parser (full type-grammar
  parse, no typecheck) + compiler
  (constant-fold w/ pinned libm+canon, generic-superinstruction fusion, regalloc, O0 mode); `rustia-vm` with
  the **16-byte tagged Value**, thread-affine hybrid VM heap with gc-arena scheduling/owned long-string storage
  plus the explicitly tracked append-only object-store gap, big-match dispatch, fuel + recursion cap, and a
  stackless trampoline. Host-columnar rollback region. Prepare the
  cross-arch golden (incl. f64) for the FULL target matrix (§C0): native macOS-arm64/**x86_64-Intel**/Windows/
  Linux-x86_64/arm64/iOS-arm64/Android-arm64, and wasm in **V8 (Chrome) + SpiderMonkey (Firefox) + JSC
  (Safari/iOS)**. Hosted CI now
  runs release preflight; reproducible performance measurement remains operator-started on dedicated Linux.
  Fixture-driven differential hardening against the pinned reference is ongoing; a dedicated fuzz harness is
  still planned.
- **P2 — minimum-viable Luau (substantial; named core landed, semantic tail remains).** Broad opcode surface;
  full grammar + Luau niceties; libs
  math/table/string(+format+patterns)/bit32/coroutine + base globals; metatable OOP; floored `%`. Byte-compat:
  Schubfach `tostring` (high), `string.format` (high), ASCII/C-locale pattern classes (high), PCG32 (low).
- **P3 — drop into grabby (private fork).** Use the landed typed invocation/effect-transaction core through a
  koto-shaped shim; add zero-copy serde only where profiling justifies it. Wire behind the `Selector` trait +
  the `ai_bakeoff` Criterion harness as a continuous perf/golden gate. Rewrite the `.koto` scenario/console
  scripts → `.luau`. Register grabby's bespoke fused AI op via the typed host-module surface.
- **P4 — production hardening + public release.** `buffer`, `vector`, `utf8`, `string.pack`, and the major
  standalone library surface are already conformance-backed. The remaining release-critical work is complete
  object/program reclamation and reuse, memory accounting/quotas/OOM behavior, deterministic reset semantics,
  host-provided output/time services plus honest VM memory telemetry, host-owned `require` resolution/cache,
  transactional guest-module install/hot-reload failure semantics, live-object constructors/serde,
  diagnostic and semantic-tail hardening, realistic mixed-heap native+wasm evidence, the full
  cross-target correctness/golden matrix, API/semver stabilization, and crate publication. The gradual
  typechecker/LSP is a separate later track, not a
  runtime-release gate. Keep the public codebase clean (no grabby-specific code) under the settled dual
  `MIT OR Apache-2.0` license. *Optional:* native-only JIT (cranelift first). **VALIDATED tier — wasm-emit
  JIT/AOT-to-wasm (the in-browser speed answer, spike `65-wasmjit`):** emitting a hot kernel to wasm at runtime
  (via `wasm-encoder`) and instantiating it in V8 runs at **~4.6 ns/op = ~176× the interpreter** (within 1.35×
  of native LLVM), for a **~4.7 µs one-time standup → crossover ≈ 6 calls**; generated modules can import the
  VM's linear memory (no bounds-check penalty). Gate emission behind a **hotness counter** (`calls × interp_cost
  > ~5 µs`). Caveat: measured on a **monomorphic f64** kernel; dynamically-typed Luau needs **type guards +
  deopt/fallback** — which is exactly where **type-directed compilation** pays (Luau's types let us emit
  guard-free specialized wasm). So the in-browser high-speed path = **type-directed AOT-to-wasm for hot,
  proven-monomorphic kernels.** Use sync `new WebAssembly.Module` for the tiny (<1.1 KB) modules.

## 5. Public vs private (the release model)

- **Public rustia** = a clean, general-purpose engine targeting pinned standalone Luau 0.709 compatibility. Generic superinstructions for
  speed. An **op-extension hook** + a sealed embedding API. No grabby-specific code. Dual licensed
  `MIT OR Apache-2.0`.
- **gak's private fork** = registers bespoke fused ops (the AI-kernel `OP_UTIL`, ~25× koto) through the hook —
  a thin delta tracking upstream, never a hard fork.
- **Parity bar for public release = pinned standalone Luau runtime compatibility** (the §2-F deferred runtime
  items return at P4; analyzer, raw C API, Roblox, Lua 5.4, and bytecode compatibility remain separate tracks);
  grabby's *first* private
  use runs on the P2 minimum-viable subset.

## 6. Risk register (post-resolution)

| risk | sev | status / mitigation |
|---|---|---|
| Value-layout/correctness evidence is still incomplete across the full native/browser matrix; real **Safari/iOS JSC** remains open even though V8, Bun/JSC-family, and SpiderMonkey shell wasm are measured | **high** | close the residual §C0 public-release build/golden matrix while keeping timings dedicated-Linux-only; keep `Value` sealed/swappable until then |
| Earlier general-script perf evidence came from narrow spike workloads | med | use the committed multi-workload harness and generated per-workload claim gate |
| Live VM is thread-affine instead of `Send` | med | Avoids unsafe in production; hosts should construct VMs on worker threads and share only immutable compiled artifacts/configuration |
| Broad native/Wasm mixed-heap pressure evidence remains incomplete | **high** | object/program reuse, maintained accounting, quotas/OOM, epoch reset, a focused 600-decision native plateau, and a 300-decision Node/V8 wasm32 plateau are implemented; extend the external long-run pressure matrix before tuning intervals |
| GC full-collect cost scales O(live-objects) (~10 ns/obj; ~200 µs at 20k objects) | med | after reclamation lands, tune the instruction-count collection interval on realistic heap sizes and confirm bit-reproducible native vs wasm |
| Remaining semantic/diagnostic tail can move errors across compile/runtime boundaries | med | add focused pinned-Luau fixtures plus differential/fuzz tests and public stack-trace/source diagnostics |
| Representative mixed-heap/wasm per-VM RSS remains open; 16B Value doubles value memory, GcBox adds 16 B/obj, and wasm memory grows only | med | retain the exact paired 512-VM Linux corpus: the candidate's primary residual string contrasts are approximately 0 KiB/VM for shared short/long program bytes and loaded 13-byte strings, plus 7.078 KiB/VM for loaded long atoms; extend to representative heaps/native+wasm targets, then set the P1 budget |
| Public-release scope for pinned standalone Luau runtime compatibility is a long tail | low | P4; grabby ships privately on the P2 subset first; analyzer, raw C API, Roblox, Lua 5.4, and bytecode compatibility stay separate unless reclassified |

## 7. Open sub-decisions (deferred, not blocking)
- Shared compiled strings and 13-byte inline strings are quantitatively resolved for the exact Linux fixture by
  16 paired rounds and 160 accepted samples. Extend that sampler to representative mixed heaps and wasm memory
  before setting the full per-VM RSS/startup budget.
- Exact generic-superinstruction set beyond numeric-for — driven by P4 profiling of real scripts.
- Whether production AI runs inside the rollback sim (host-columnar) or as a separate non-rolled-back layer.
- Whether any transcendental runs in the 50 Hz per-tick path (vs one-shot scenario gen).
- Native JIT: cranelift vs copy-and-patch, if pursued at all.
