# rustia — project pillars and non-goals

> **Status: [CANONICAL] (2026-07-09).** This is the project decision filter. A task is current rustia work only
> when it advances at least one pillar without weakening a higher-priority pillar. If a task is useful but does
> not fit these pillars, it belongs in a deferred or out-of-scope audit, not on the release-critical path.

## Pillars

| # | pillar | contract |
|---:|---|---|
| 1 | **Pinned Standalone Luau Compatibility** | Target Luau 0.709 standalone runtime behavior: parser, compiler, VM, source-visible stdlib, vector-3, flags-off. Target upgrades require an explicit re-pin and refreshed oracle/provenance. |
| 2 | **Conformance and Provenance** | Every compatibility claim needs an executable fixture, upstream link, generated audit, benchmark record, or explicit deferred/out-of-scope reason. Counts are useful only when tied to a named denominator. |
| 3 | **Deterministic Profiles and Replay ABI** | `ReproducibleStandard` is the public reproducible-conformant profile; `DeterministicSim` is the stricter lockstep/replay subset with versioned fuel, GC, capability, and profile behavior. |
| 4 | **Rollback-Safe Host Boundary** | rustia executes deterministic script decisions; the host owns authoritative cross-tick state, typed slots, rollback capture, ordered effect queues, and commit semantics. Lua globals/upvalues/GC heap do not become hidden rollback state. |
| 5 | **Layered Rust Embedding API** | The supported public surface is a small typed Rust facade first, then explicit layers for extensions, capabilities, fuel, retained handles, serde, and future advanced interop. Do not clone Luau's raw C API by default. |
| 6 | **One Safe Native + Wasm Engine** | Native and `wasm32-unknown-unknown` share one semantic engine. Public workspace code stays safe Rust; no target-specific semantic fork, wasm-GC dependency, relaxed SIMD, fast-math, or FMA assumption enters the product contract. |
| 7 | **Benchmark-Gated Performance Claims** | Performance claims are per workload, per target, and backed by committed benchmark history and generated claim gates. No blanket speed headline ships from one-off measurements. |
| 8 | **Public-Clean OSS Consumer Contract** | Public rustia crates stay generic, documented, packageable, semver-scoped, license-clean, CI-gated, and free of grabby-specific APIs. Lower crates may be advanced/tooling surfaces; the `rustia` facade is the consumer contract. |

## Non-goals

These are not release-critical rustia work unless a future decision explicitly reclassifies them.

- **Not Roblox compatibility:** no `game`, `workspace`, `Instance`, `DataModel`, Roblox scheduler, or Roblox-only APIs.
- **Not latest-Luau chasing:** Luau 0.709 remains the target until deliberately re-pinned and re-oracled.
- **Not Lua 5.4 compatibility:** do not add Lua 5.4-only globals, module semantics, GC options, or coroutine behavior as compatibility goals.
- **Not Luau bytecode compatibility:** no script-visible bytecode loading, `string.dump`, or untrusted LBC ingestion.
- **Not analyzer/typechecker parity for the runtime release:** type syntax is parsed for runtime erasure; analyzer/LSP work is a separate future track.
- **Not full native C API parity:** raw Lua stack APIs, atom callbacks, allocator hooks, lightuserdata, and raw debug inspection are future embedding decisions, not runtime-conformance requirements.
- **Not public grabby:** no `SimId`, effect catalog, slot schema, private fused AI op, or grabby integration code in public rustia APIs.
- **Not hidden cross-tick Lua state in `DeterministicSim`:** authoritative replay state must live in host-owned captured data, not Lua globals, upvalues, suspended coroutines, or the GC heap.
- **Not target-specific speed hacks:** wasm-GC, relaxed SIMD, fast-math, FMA, native-only JIT requirements, and target-dependent semantics stay out of the portable product contract.
- **Not public performance hype:** no "5x koto", "faster than Luau", or "near-native Luau" headline without the benchmark claim gate approving the exact workload and target.

## Decision Rule

Before starting or accepting a task, answer three questions:

1. Which pillar does this advance?
2. Which denominator proves progress: fixture count, upstream case map, benchmark history, release gate, API test, or docs contract?
3. Does it accidentally weaken a higher-priority pillar, especially pinned compatibility, determinism, rollback safety, or public API simplicity?

If the answer to the first question is unclear, the task is probably a rabbit hole. Put it in an audit or backlog
with an explicit defer reason instead of treating it as release-critical work.
