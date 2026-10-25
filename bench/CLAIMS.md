# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 65
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-10-25 18:23:16 UTC to 2026-10-25 18:27:40 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 190 | 5 / 10 | mlua-lua54 | 4.659x | 3.717x | 0.55x | not public-ready: rustia trails mlua-luau by 3.717x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,336 | 5 / 10 | wasmi | 18.88x | 2.714x | 0.196x | not public-ready: rustia trails mlua-luau by 2.714x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 14,422 | 8 / 10 | wasmi | 45.93x | 3.461x | 1.97x | not public-ready: rustia trails mlua-luau by 3.461x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 92,181 | 6 / 10 | wasmi | 39.18x | 4.117x | 0.639x | not public-ready: rustia trails mlua-luau by 4.117x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,431 | 7 / 10 | wasmi | 53.43x | 6.102x | 0.936x | not public-ready: rustia trails mlua-luau by 6.102x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 19.01x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.337x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.878x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.328x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.014x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.848x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.662x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.343x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.165x | 0 / 5 | linux-x86_64 | 5 workloads |
