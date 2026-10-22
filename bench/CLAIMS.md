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
- Measurement window: 2026-10-22 04:09:41 UTC to 2026-10-22 04:14:05 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 197 | 5 / 10 | mlua-lua54 | 4.822x | 3.861x | 0.569x | not public-ready: rustia trails mlua-luau by 3.861x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,657 | 5 / 10 | wasmi | 18.93x | 2.73x | 0.197x | not public-ready: rustia trails mlua-luau by 2.73x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 14,035 | 8 / 10 | wasmi | 44x | 3.366x | 1.899x | not public-ready: rustia trails mlua-luau by 3.366x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 87,555 | 5 / 10 | wasmi | 37.11x | 3.894x | 0.599x | not public-ready: rustia trails mlua-luau by 3.894x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,421 | 7 / 10 | wasmi | 53.03x | 5.986x | 0.925x | not public-ready: rustia trails mlua-luau by 5.986x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 18.76x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.285x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.833x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.296x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.017x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.831x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.652x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.325x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.166x | 0 / 5 | linux-x86_64 | 5 workloads |
