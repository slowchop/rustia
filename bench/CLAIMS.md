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
- Measurement window: 2026-09-21 05:41:56 UTC to 2026-09-21 05:46:19 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 173 | 5 / 10 | mlua-lua54 | 4.266x | 3.387x | 0.472x | not public-ready: rustia trails mlua-luau by 3.387x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 51,483 | 5 / 10 | wasmi | 18.69x | 2.683x | 0.197x | not public-ready: rustia trails mlua-luau by 2.683x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 13,154 | 8 / 10 | wasmi | 45.8x | 3.028x | 1.773x | not public-ready: rustia trails mlua-luau by 3.028x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 87,426 | 5 / 10 | wasmi | 36.97x | 3.999x | 0.61x | not public-ready: rustia trails mlua-luau by 3.999x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,195 | 6 / 10 | wasmi | 44.57x | 5.064x | 0.783x | not public-ready: rustia trails mlua-luau by 5.064x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 17.7x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 4.097x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.542x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 3.097x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.913x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.756x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.601x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.299x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.151x | 0 / 5 | linux-x86_64 | 5 workloads |
