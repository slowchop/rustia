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
- Measurement window: 2026-08-13 00:48:59 UTC to 2026-08-13 00:53:43 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 271 | 5 / 10 | mlua-lua54 | 5.829x | 4.137x | 0.826x | not public-ready: rustia trails mlua-luau by 4.137x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 50,616 | 5 / 10 | wasmi | 18.52x | 2.619x | 0.177x | not public-ready: rustia trails mlua-luau by 2.619x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 14,431 | 8 / 10 | wasmi | 40.05x | 3.285x | 2.107x | not public-ready: rustia trails mlua-luau by 3.285x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 99,083 | 5 / 10 | wasmi | 30.09x | 4.458x | 0.673x | not public-ready: rustia trails mlua-luau by 4.458x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,382 | 7 / 10 | wasmi | 54.35x | 5.833x | 0.837x | not public-ready: rustia trails mlua-luau by 5.833x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 18.03x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.238x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.92x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.534x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.069x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.835x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.705x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.332x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.178x | 0 / 5 | linux-x86_64 | 5 workloads |
