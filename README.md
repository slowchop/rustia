# rustia

A fast, low-memory, **deterministic** Luau interpreter written in Rust — built to replace **koto** as the
scripting layer in [grabby-aliens](https://github.com/slowchop/grabby-next), and intended for public release
at full Luau parity.

> **Status: P1 engine scaffold in progress.** The locked design is now being implemented in production crates;
> `spikes/` are throwaway prototypes that validated the design.
> Read **[`design/CURRENT.md`](design/CURRENT.md)** for the canonical state, then
> [`design/README.md`](design/README.md) for the full doc map.

## Highlights (measured in spikes; production engine still early)
- Designed to run at/near reference-Luau parity on measured workloads, with koto comparisons tracked
  per-workload; **near-native on hot numeric paths** via a runtime wasm-emit JIT (~176× the interpreter in V8,
  crossover ≈ 6 calls).
- **Bit-for-bit deterministic** across x86-64-linux / arm64-mac / wasm32 (f64 + pure-Rust libm + per-op
  NaN canonicalization, table-key `-0` normalization, and no-FMA), with two profiles: `ReproducibleStandard` (public) and
  `DeterministicSim` (lockstep multiplayer).
- One portable engine: big-`match`→`br_table` interpreter + a wasm-emit JIT/AOT tier; **gc-arena** GC,
  **16-byte tagged** values, own hand-written Luau parser, **Send-not-Sync** N-VM threading.
- Pinned to **Luau 0.709** (standalone luau.org surface, not Roblox).

## Layout
- `design/` — the design record: constraints → research → spikes → architecture (ADR) → conformance → improvements.
- `conformance/` — executable Luau fixtures and the upstream Luau 0.709 test-port plan.
- `crates/` — production P1 workspace: VM, lexer, parser, compiler, public API, CLI, and conformance runner.
- `spikes/` — throwaway prototypes (skeleton VM, libm cross-arch, wasm-JIT, gc-arena, …) with their measurements.

Built design-first with Claude Code.

## License
Licensed under either of **[Apache License, Version 2.0](LICENSE-APACHE)** or **[MIT license](LICENSE-MIT)**
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in this work, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional
terms or conditions.
