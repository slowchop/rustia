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
- Measurement window: 2026-10-04 18:40:00 UTC to 2026-10-04 18:44:23 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 181 | 5 / 10 | mlua-lua54 | 4.376x | 3.55x | 0.517x | not public-ready: rustia trails mlua-luau by 3.55x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,544 | 5 / 10 | wasmi | 18.72x | 2.691x | 0.197x | not public-ready: rustia trails mlua-luau by 2.691x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,110 | 8 / 10 | wasmi | 45.17x | 3.054x | 1.763x | not public-ready: rustia trails mlua-luau by 3.054x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 86,875 | 5 / 10 | wasmi | 36.67x | 4.053x | 0.603x | not public-ready: rustia trails mlua-luau by 4.053x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,207 | 6 / 10 | wasmi | 45.04x | 5.108x | 0.793x | not public-ready: rustia trails mlua-luau by 5.108x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 17.83x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.09x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.599x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.135x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.928x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.766x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.612x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.315x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.153x | 0 / 5 | linux-x86_64 | 5 workloads |
