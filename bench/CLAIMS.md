# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 60
- Targets: `linux-x86_64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-07-22 00:41:32 UTC to 2026-07-22 00:46:05 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 5 of 5 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | closures_upvalues | one upvalue-mutating closure call | 648 | 10 / 12 | roto | 110x | 10.14x | 1.962x | not public-ready: rustia trails mlua-luau by 10.14x |
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 156,707 | 8 / 12 | wasmtime | 290x | 7.737x | 0.519x | not public-ready: rustia trails mlua-luau by 7.737x |
| linux-x86_64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 30,601 | 11 / 12 | wasmtime | 253x | 6.642x | 4.473x | not public-ready: rustia trails mlua-luau by 6.642x |
| linux-x86_64 | sparse_tables | fill+read 128 hash entries | 110,952 | 9 / 12 | wasmtime | 209x | 5.02x | 0.71x | not public-ready: rustia trails mlua-luau by 5.02x |
| linux-x86_64 | stdlib_math_table | math fastcalls + buffered push | 1,608 | 9 / 12 | wasmtime | 232x | 6.674x | 0.814x | not public-ready: rustia trails mlua-luau by 6.674x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (x86_64) | 149x | 4 / 5 | linux-x86_64 | 5 workloads |
| wasmi (x86_64) | 30.12x | 0 / 5 | linux-x86_64 | 5 workloads |
| luaur (x86_64) | 7.76x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-luau (x86_64) | 7.053x | 0 / 5 | linux-x86_64 | 5 workloads |
| mlua-lua54 (x86_64) | 6.605x | 0 / 5 | linux-x86_64 | 5 workloads |
| roto (x86_64) | 4.396x | 1 / 5 | linux-x86_64 | 5 workloads |
| rquickjs (x86_64) | 1.96x | 0 / 5 | linux-x86_64 | 5 workloads |
| ruau (x86_64) | 1.494x | 0 / 5 | linux-x86_64 | 5 workloads |
| koto (x86_64) | 1.214x | 0 / 5 | linux-x86_64 | 5 workloads |
| rustia (x86_64) | 1x | 0 / 5 | linux-x86_64 | 5 workloads |
| rhai (x86_64) | 0.613x | 0 / 5 | linux-x86_64 | 5 workloads |
| boa (x86_64) | 0.326x | 0 / 5 | linux-x86_64 | 5 workloads |
