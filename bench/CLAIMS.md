# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 60
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-07-22 09:19:16 UTC to 2026-07-22 09:23:49 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 270 | 7 / 12 | roto | 45.86x | 4.244x | 0.802x | not public-ready: rustia trails mlua-luau by 4.244x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 53,272 | 6 / 12 | wasmtime | 98.37x | 2.714x | 0.186x | not public-ready: rustia trails mlua-luau by 2.714x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 11,764 | 10 / 12 | wasmtime | 97.16x | 2.688x | 1.726x | not public-ready: rustia trails mlua-luau by 2.688x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 80,965 | 6 / 12 | wasmtime | 152x | 3.62x | 0.56x | not public-ready: rustia trails mlua-luau by 3.62x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,118 | 8 / 12 | wasmtime | 161x | 4.65x | 0.672x | not public-ready: rustia trails mlua-luau by 4.65x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 72.59x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 15.96x | 0 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.757x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.495x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.147x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 2.098x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.954x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.761x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.627x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.307x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.156x | 0 / 5 | linux-x86_64 | 5 workloads |
