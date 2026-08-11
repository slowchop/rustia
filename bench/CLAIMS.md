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
- Measurement window: 2026-08-11 08:08:34 UTC to 2026-08-11 08:13:18 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 281 | 5 / 10 | mlua-lua54 | 6.075x | 4.444x | 0.852x | not public-ready: rustia trails mlua-luau by 4.444x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 52,136 | 5 / 10 | wasmi | 17.46x | 2.615x | 0.182x | not public-ready: rustia trails mlua-luau by 2.615x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 14,023 | 8 / 10 | wasmi | 41.85x | 3.058x | 2.075x | not public-ready: rustia trails mlua-luau by 3.058x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 99,708 | 7 / 10 | wasmi | 27.94x | 4.569x | 0.698x | not public-ready: rustia trails mlua-luau by 4.569x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,404 | 7 / 10 | wasmi | 55.11x | 5.88x | 0.858x | not public-ready: rustia trails mlua-luau by 5.88x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 18.13x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.336x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.944x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.603x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.094x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.862x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.719x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.349x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.184x | 0 / 5 | linux-x86_64 | 5 workloads |
