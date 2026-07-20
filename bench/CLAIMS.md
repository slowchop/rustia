# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 50
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-07-20 10:16:58 UTC to 2026-07-20 10:20:31 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 844 | 8 / 10 | roto | 144x | 13.24x | 2.583x | not public-ready: rustia trails mlua-luau by 13.24x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 274,757 | 6 / 10 | wasmtime | 509x | 14.08x | 0.963x | not public-ready: rustia trails mlua-luau by 14.08x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 38,617 | 10 / 10 | wasmtime | 320x | 9.036x | 5.665x | not public-ready: rustia trails mlua-luau by 9.036x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 160,487 | 8 / 10 | wasmtime | 302x | 7.143x | 1.058x | not public-ready: rustia trails mlua-luau by 7.143x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 2,119 | 8 / 10 | wasmtime | 306x | 8.901x | 1.285x | not public-ready: rustia trails mlua-luau by 8.901x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 210x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 44.61x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 10.14x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 9.152x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 6.122x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 2.759x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 1.805x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.902x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.473x | 0 / 5 | linux-x86_64 | 5 workloads |
