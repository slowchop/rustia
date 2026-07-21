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
- Measurement window: 2026-07-21 22:24:20 UTC to 2026-07-21 22:27:53 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 612 | 8 / 10 | roto | 104x | 9.528x | 1.847x | not public-ready: rustia trails mlua-luau by 9.528x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 177,348 | 6 / 10 | wasmtime | 312x | 9.196x | 0.613x | not public-ready: rustia trails mlua-luau by 9.196x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 29,877 | 9 / 10 | wasmtime | 247x | 6.772x | 4.372x | not public-ready: rustia trails mlua-luau by 6.772x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 116,897 | 7 / 10 | wasmtime | 216x | 5.251x | 0.808x | not public-ready: rustia trails mlua-luau by 5.251x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,584 | 7 / 10 | wasmtime | 228x | 6.664x | 0.954x | not public-ready: rustia trails mlua-luau by 6.664x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 149x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 30.95x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 7.303x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 6.621x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 4.458x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.993x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 1.307x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.644x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.329x | 0 / 5 | linux-x86_64 | 5 workloads |
