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
- Measurement window: 2026-08-28 02:09:53 UTC to 2026-08-28 02:14:36 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 161 | 4 / 10 | mlua-lua54 | 3.554x | 2.552x | 0.488x | not public-ready: rustia trails mlua-luau by 2.552x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 52,748 | 5 / 10 | wasmi | 18.32x | 2.601x | 0.184x | not public-ready: rustia trails mlua-luau by 2.601x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 11,354 | 8 / 10 | wasmi | 32.13x | 2.462x | 1.661x | not public-ready: rustia trails mlua-luau by 2.462x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 79,263 | 5 / 10 | wasmi | 23.7x | 3.59x | 0.555x | not public-ready: rustia trails mlua-luau by 3.59x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,051 | 6 / 10 | wasmi | 41.18x | 4.399x | 0.636x | not public-ready: rustia trails mlua-luau by 4.399x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmi (x86_64) | 13.9x | 4 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 3.324x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 3.036x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 2.799x | 1 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 0.841x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 0.678x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 0.555x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.271x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.143x | 0 / 5 | linux-x86_64 | 5 workloads |
