# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: `bench/history/runs.jsonl`
- Rows: 52
- Targets: `linux-x86_64`, `macos-aarch64`
- Workloads: `closures_upvalues`, `dense_tables`, `generic_iteration`, `oop_metamethods`, `sparse_tables`, `stdlib_math_table`
- Measurement window: 2026-07-03 01:53:44 UTC to 2026-07-20 04:42:11 UTC
- Global public speed headline: **not approved from this snapshot**
- Reason: 7 of 7 current workload claims are not public-ready against the mlua-luau reference row.

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
| linux-x86_64 | dense_tables | build+sum+sort of 256-elem array | 296,285 | 1 / 1 | rustia | 1x | n/a | n/a | blocked: missing mlua-luau baseline |
| linux-x86_64 | oop_metamethods | (p+v)*s + :dot + :len2 step | 6,783 | 1 / 1 | rustia | 1x | n/a | n/a | blocked: missing mlua-luau baseline |
| macos-aarch64 | closures_upvalues | one upvalue-mutating closure call | 421 | 8 / 10 | roto | 111x | 19.91x | 3.48x | not public-ready: rustia trails mlua-luau by 19.91x |
| macos-aarch64 | dense_tables | build+sum+sort of 256-elem array | 132,121 | 8 / 10 | wasmtime | 685x | 13.54x | 1.336x | not public-ready: rustia trails mlua-luau by 13.54x |
| macos-aarch64 | generic_iteration | __iter(32)+pairs(5)+ipairs(32) pass | 17,533 | 10 / 10 | wasmtime | 222x | 8.247x | 7.56x | not public-ready: rustia trails mlua-luau by 8.247x |
| macos-aarch64 | sparse_tables | fill+read 128 hash entries | 113,418 | 8 / 10 | wasmtime | 373x | 12.24x | 2.084x | not public-ready: rustia trails mlua-luau by 12.24x |
| macos-aarch64 | stdlib_math_table | math fastcalls + buffered push | 1,203 | 9 / 10 | wasmtime | 478x | 13.44x | 1.943x | not public-ready: rustia trails mlua-luau by 13.44x |

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
| wasmtime (aarch64) | 235x | 4 / 5 | macos-aarch64 | 5 workloads |
| wasmi (aarch64) | 61.61x | 0 / 5 | macos-aarch64 | 5 workloads |
| mlua-luau (aarch64) | 12.96x | 0 / 5 | macos-aarch64 | 5 workloads |
| mlua-lua54 (aarch64) | 10.56x | 0 / 5 | macos-aarch64 | 5 workloads |
| roto (aarch64) | 5.111x | 1 / 5 | macos-aarch64 | 5 workloads |
| rquickjs (aarch64) | 3.098x | 0 / 5 | macos-aarch64 | 5 workloads |
| koto (aarch64) | 2.696x | 0 / 5 | macos-aarch64 | 5 workloads |
| rhai (aarch64) | 1.418x | 0 / 5 | macos-aarch64 | 5 workloads |
| rustia (x86_64) | 1x | 2 / 2 | linux-x86_64 | 2 workloads |
| rustia (aarch64) | 1x | 0 / 5 | macos-aarch64 | 5 workloads |
| boa (aarch64) | 0.629x | 0 / 5 | macos-aarch64 | 5 workloads |
