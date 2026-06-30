# rustia — wasm-emit JIT / AOT-to-wasm spike (the in-browser speed answer)

> **Status: [SPIKE — verified] (2026-06-30).** Answers "can rustia approach native/LuaJIT speed *in the
> browser*?" Built + adversarially re-run; bit-identical checksum across all paths proves honesty. Crate:
> [`../spikes/spike-wasmjit/`](../spikes/spike-wasmjit/); raw: [`65-wasmjit-raw.json`](65-wasmjit-raw.json).

## The question
You cannot execute raw native machine code from *inside* the wasm sandbox — but you **can generate wasm at
runtime and instantiate it** (`new WebAssembly.Module`/`WebAssembly.instantiate`); the host engine (V8/JSC/
SpiderMonkey) then JITs *that* to native. Is the overhead (codegen + compile + instantiate + boundary) low
enough to be worth it? At what crossover?

## Method (honest)
The grabby IAUS argmax kernel (6 actions × 4 considerations, data-dependent inputs via a per-iteration LCG so
nothing constant-folds), run **the same logic 4 ways in V8** (node v25 + deno, both V8 14.9):
1. **Interpreter** — the *real* `spike-sk-rustia` register VM (big-`match`→`br_table`) on the kernel bytecode.
2. **Static-AOT** — the kernel as a Rust fn compiled to wasm32 (the steady-state ceiling).
3. **Runtime-generated** — wasm bytes built **at runtime** by `wasm-encoder` *inside* the host wasm module,
   read out, `new WebAssembly.Module` + instantiate, then run.
4. **Shared-heap** — generated module **imports the host's linear memory** and round-trips inputs through it.

All 6 paths + the native build produce a **bit-identical FNV checksum** (`0xd385c5e57bdc3538`), per-seed
distinct ⇒ same kernel + anti-DCE. Verifier independently reproduced every node number within noise.

## Results (V8, ns per kernel-op)
| path | ns/op | vs interpreter |
|---|---:|---:|
| interpreter (real VM, fuel off = its best case) | ~803 | 1× |
| **runtime-generated, unrolled/kind-specialized (shippable)** | **~4.6** | **~176×** |
| runtime-generated, naive looped | ~10.0 | ~80× |
| shared-heap (imported VM memory) | ~4.6 | ~176× (no bounds penalty) |
| static-AOT (LLVM ideal) | ~3.4 | ~237× |

- **Fixed standup ~4.7 µs** = codegen ~3 µs (wasm-encoder, in-VM) + sync compile ~0.7 µs + sync instantiate
  ~1.1 µs. (Async `WebAssembly.instantiate` ≈ 2× slower for these <1.1 KB modules → prefer sync.)
- **Crossover ≈ 6 kernel-ops** (`fixed_cost / per-op-saving` = 4720 / ~801 ns). Re-derived on node + deno.
- Runtime-generated reaches **within 1.35×** of LLVM static-AOT (residual = scalar hand-emit vs LLVM
  autovectorization).

## Verdict
- **GO for grabby:** the per-agent selector is called tens of millions of times → the ~4.7 µs standup amortizes
  after ~6 calls → an **unconditional ~176× win** on the hot path, reaching within 1.35× of native — *in the
  browser*. This is the route to LuaJIT-adjacent speed that survives wasm.
- **CONDITIONAL for general scripts:** gate emission behind a **hotness counter** (`calls × interp_cost > ~5 µs`).
- **Caveat (load-bearing):** this kernel is **monomorphic f64 compute.** Real dynamically-typed Luau needs
  **type guards + a deopt/fallback path** (per-op cost not modeled here). The fix is **type-directed
  compilation** — Luau's type info lets us emit **guard-free specialized wasm** where types prove monomorphism.
  So the in-browser high-speed path = **type-directed AOT-to-wasm for hot, proven-monomorphic kernels**;
  polymorphic/table/string-heavy code wins less and crosses over later.
- Engine note: absolute numbers will differ on JSC/SpiderMonkey, but the *structure* (emitted wasm → ~native
  after a ~µs standup) is engine-independent. Confirm at the P1 cross-engine gate.

## Design impact
Promotes wasm-emit JIT / AOT-to-wasm from "research option" to a **validated optimization tier** (ADR §4):
interpreter-primary everywhere; **wasm-emit JIT for hot kernels (browser + native)**; native machine-code JIT
(cranelift) optional native-only. Unifies with the **type-directed compilation** lever — same machinery, and
the type info is what removes the guard/deopt cost the caveat names.
