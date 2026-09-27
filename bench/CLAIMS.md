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
- Measurement window: 2026-09-27 19:02:14 UTC to 2026-09-27 19:06:38 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 180 | 5 / 10 | mlua-lua54 | 4.427x | 3.53x | 0.522x | not public-ready: rustia trails mlua-luau by 3.53x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,783 | 5 / 10 | wasmi | 19.03x | 2.702x | 0.196x | not public-ready: rustia trails mlua-luau by 2.702x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,077 | 8 / 10 | wasmi | 42.08x | 3.036x | 1.787x | not public-ready: rustia trails mlua-luau by 3.036x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 87,388 | 5 / 10 | wasmi | 37.61x | 3.973x | 0.594x | not public-ready: rustia trails mlua-luau by 3.973x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,221 | 6 / 10 | wasmi | 45.56x | 5.176x | 0.799x | not public-ready: rustia trails mlua-luau by 5.176x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 17.59x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.123x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.589x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.118x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.923x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.764x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.613x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.316x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.153x | 0 / 5 | linux-x86_64 | 5 workloads |
