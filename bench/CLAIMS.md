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
- Measurement window: 2026-10-11 19:31:13 UTC to 2026-10-11 19:35:36 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 176 | 5 / 10 | mlua-lua54 | 4.254x | 3.424x | 0.515x | not public-ready: rustia trails mlua-luau by 3.424x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,980 | 5 / 10 | wasmi | 19.03x | 2.713x | 0.198x | not public-ready: rustia trails mlua-luau by 2.713x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,166 | 8 / 10 | wasmi | 42.04x | 3.064x | 1.781x | not public-ready: rustia trails mlua-luau by 3.064x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 87,426 | 5 / 10 | wasmi | 36.96x | 4.051x | 0.606x | not public-ready: rustia trails mlua-luau by 4.051x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,200 | 6 / 10 | wasmi | 44.74x | 5.068x | 0.791x | not public-ready: rustia trails mlua-luau by 5.068x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 17.56x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.994x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.576x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.125x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.926x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.761x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.613x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.3x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.152x | 0 / 5 | linux-x86_64 | 5 workloads |
