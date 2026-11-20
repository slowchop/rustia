# Rustia Wasm code generation experiment in Node and Deno

> **Status: [SPIKE — verified] (2026-06-30).** Measures one numeric kernel in Node and Deno.
> The native build supplied a checksum comparison. Crate:
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
| **runtime-generated, unrolled/kind-specialized (spike)** | **~4.6** | **~176×** |
| runtime-generated, naive looped | ~10.0 | ~80× |
| shared-heap (imported VM memory) | ~4.6 | ~176× (no bounds penalty) |
| static-AOT (LLVM ideal) | ~3.4 | ~237× |

- **Fixed standup ~4.7 µs** = codegen ~3 µs (wasm-encoder, in-VM) + sync compile ~0.7 µs + sync instantiate
  ~1.1 µs. (Async `WebAssembly.instantiate` ≈ 2× slower for these <1.1 KB modules → prefer sync.)
- **Crossover ≈ 6 kernel-ops** (`fixed_cost / per-op-saving` = 4720 / ~801 ns). Re-derived on node + deno.
- Runtime-generated reaches **within 1.35×** of LLVM static-AOT (residual = scalar hand-emit vs LLVM
  autovectorization).

## What the experiment established

For this numeric kernel, the generated Wasm ran about 176 times faster than the
skeleton interpreter in V8. The static Rust-to-Wasm version ran about 1.35 times
faster than the generated version. Both timings came from Wasm running in V8.
The native executable was checked for the same result.

The kernel used known numeric types. General Luau would need guards and an
interpreter fallback, or proof that its types stay fixed. Production Rustia,
browser engines outside these V8 hosts, and the complete application workload
still need their own measurements before this experiment can support a release
claim. Rustia ships the interpreter. Wasm code generation remains a research option.
