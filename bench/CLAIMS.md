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
- Measurement window: 2026-07-23 03:40:25 UTC to 2026-07-23 03:45:09 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 229 | 5 / 10 | mlua-lua54 | 4.945x | 3.544x | 0.682x | not public-ready: rustia trails mlua-luau by 3.544x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 55,552 | 5 / 10 | wasmi | 16.66x | 2.789x | 0.194x | not public-ready: rustia trails mlua-luau by 2.789x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 12,105 | 8 / 10 | wasmi | 32.87x | 2.698x | 1.755x | not public-ready: rustia trails mlua-luau by 2.698x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 90,355 | 5 / 10 | wasmi | 29.08x | 3.945x | 0.616x | not public-ready: rustia trails mlua-luau by 3.945x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,177 | 6 / 10 | wasmi | 45.62x | 4.874x | 0.708x | not public-ready: rustia trails mlua-luau by 4.874x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 15.23x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.833x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.483x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.179x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.928x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.742x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.632x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.307x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.154x | 0 / 5 | linux-x86_64 | 5 workloads |
