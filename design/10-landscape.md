# rustia — research brief: landscape, internals & technique menu

> **Status: [RESEARCH v1] (2026-06-30).** Synthesis of a 6-stream research workflow (koto internals,
> Rust scripting landscape, Luau spec, official Luau C++ VM, SOTA VM/JIT/WASM techniques, grabby
> binding+determinism contract) + an adversarial completeness critic. Full raw output (all findings,
> quantitative data, sources) in [`10-landscape-raw.json`](10-landscape-raw.json). **The critic
> returned a hard "NOT ready to start design exploration"** — see "Gating gaps" at the bottom; those
> are being closed by the gap-close workflow before we explore the axes.

## The three decisive findings

1. **The historical Koto comparison changed two things.** The integer variant with host calls took
   5.4 µs per decision. The floating-point variant without host calls took 4.0 µs. The second variant
   took about 26% less time. Measuring the host-call cost alone would require holding the arithmetic fixed.
2. **A good register Luau interpreter is the credible MVP target.** Cross-engine bench (script-bench-rs,
   M5 Max): mlua+**Luau 11.94 ms** vs **koto 49.21 ms** vs rhai 114 ms — ~4× koto. SOTA interpreter
   (Deegen / LuaJIT-Remake, CPS tail-call) beats LuaJIT's hand-written asm interpreter by **+31%**.
   ⚠️ *But* that 4× is a string-sort/userdata bench, **not** grabby's integer-AI kernel — unverified on the
   real workload (this is gating spike A).
3. **The fastest native techniques INVERT on wasm.** CPS tail-call dispatch is ~1.6× faster than a
   match-loop on native ARM64 but reportedly **1.2–4.6× SLOWER** once compiled to wasm; copy-and-patch JIT
   **cannot run inside a wasm sandbox** at all. ⇒ rustia must be **one opcode-handler core with
   target-split dispatch** (tail-call/threaded on native, big `match`→`br_table` on wasm), interpreter as
   the in-browser product, JIT as a *native-only* stretch tier. (This single source needs replication —
   gating spike B.)

## Reusable components (verdict)

| component | use for | verdict |
|---|---|---|
| **gc-arena** (kyren) | safe incremental cycle-collecting GC; its mutate-XOR-collect invariant *forces* the stackless/steppable VM we want | best safe GC **but hard NOT Send/Sync** — collides with N-VMs model. Adopt the *idea*; resolve Send first. |
| **piccolo** (kyren) | stackless trampoline register-VM arch (fuel + recursion + DoS resilience ~free) | study/port architecture; **don't fork for perf** (pre-1.0, unoptimized, Lua-5.x not Luau, gc-arena Send blocker). |
| **full_moon** | lossless multi-dialect parser incl. **Luau**; battle-tested (StyLua/selene) | use as **Luau-compat oracle + bootstrap frontend**; trivia-rich AST too heavy to feed a bytecode compiler directly — budget a lowering pass. |
| **mlua** | embedding-API **gold standard** (UserData/FromLua/IntoLua/Scope) | mirror the *surface*; **don't depend** (C bindings, not wasm-clean). Also the vehicle for gating spike A. |
| **koto 0.16.1** | copy its insertion-ordered seedless-hash map (`IndexMap<…,FxHasher>`); dual-number idea; the API shim | copy the good parts; avoid its sinks (169-variant decode-by-value, always-heap strings, `Arc<RwLock>` per container, no fuel, no recursion cap, cycle leaks). |
| **libm** (pure Rust) | deterministic cross-arch transcendentals + Q16.16 LUTs | **mandatory** for any f64 surface (rapier recipe). Bit-identity across the target triple is *asserted, not verified* — gating spike C. |
| **Luau C++ src** (rel 727) | technique reference: register VM + AUX-word encoding, self-patching inline caches, GETIMPORT/FASTCALL, incremental mark&sweep, the new **int64 `LUA_TINTEGER`** subtype | **port techniques not code.** int64 may resolve the f64 tension — but it's FFlag-gated and *not yet wired into hot arith opcodes*; reach unverified. |

## Decision axes (each gets ~5 directions explored — see [`README.md`](README.md))

Execution/dispatch · value representation · **number model / determinism (the central T1 tension)** ·
GC/memory · WASM-JIT feasibility & tiering · parser/frontend · Rust embedding API · steppability/fuel/snapshot.
**+ two directions the critic says we must add before exploring:**
- **Embed/fork Luau-C++-compiled-to-wasm vs build-from-scratch** — the unexamined alternative that could
  *obviate the project*. Must be verified (wasm32-unknown-unknown has no libc; does it even build?), not assumed.
- **Batched-SoA / typed-subset specialization** — decide-N-agents-per-VM-call + compile the small, fixed,
  monomorphic AI kernel to specialized straight-line bytecode/closures — attacks per-call marshalling +
  dispatch-count directly, and gets much of the JIT win on wasm *without* a JIT.

## Central tensions (full list in raw)

- **T1 — Luau f64-only vs grabby's i64/Q16.16 float-free determinism boundary.** The pivotal one.
  Candidate resolutions: f64-internal + pure-libm + no-FMA/no-fast-math (most Luau-faithful) · first-class
  int64 subtype (Luau-727-aligned) · strict capability-gated float-free mode (zero-risk, non-conformant).
- **gc-arena is ideal yet not-Send/Sync** vs N-Send+Sync-VMs. May dissolve if grabby only needs **Send**
  (move owned VMs to pool threads), not **Sync** — *being verified*.
- **JIT is the perf prize; *native-machine-code* JIT can't run inside the wasm sandbox** (but emitting **wasm**
  at runtime + host instantiation IS a real browser JIT — viable for hot/long-lived code, see `30-architecture`)
  ⇒ interpreter-primary + native JIT + a wasm-emit/AOT tier for hot kernels.
- **Insertion-ordered maps (determinism) vs inline-cache hidden-class shapes** (shape explosion on
  delete/re-add; shape-id determinism).
- **Deterministic f64 via pure libm is slower** than system libm — micro-tension per transcendental.

## Gating gaps — MUST close before design exploration (per the critic)

1. **Luau language-spec stream failed** (placeholder "test"). Re-run deep: metatables/metamethods,
   type system + type-info bytecode, vector/buffer/string libs, `string.format`/`next()`/`math.random`
   determinism surfaces, weak tables/`__gc`, sandbox/safeenv, in-scope syntax. *Blocks frontend + number model.*
2. **Spike A (highest value):** port the *actual* `ai_bakeoff` kernel to Luau, measure µs/decision under
   mlua+Luau on grabby's Criterion harness. Collapses the whole "4×" extrapolation onto the real workload.
3. **Spike B (go/no-go gate):** wasm32 register-bytecode dispatch microbench (`match`→`br_table`) in
   V8(node/deno)/wasmtime vs native — the wasm tax is currently 100% extrapolated from one blog.
4. **Spike C:** libm bit-identity across **x86_64-linux (docker) + aarch64-mac + wasm32 (node)**.
5. **Send vs Sync:** read grabby's real thread topology (`Selector` at `ai_bakeoff/src/lib.rs:165`;
   console VM is Bevy `NonSend`). If only `Send` is needed, the gc-arena tension largely evaporates.
6. **Profile koto** (samply) to replace the *inferred* "74% is dispatch+boxing" with measured per-symbol cost.
