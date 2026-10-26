# Lua 5.4 Workloads

## Overview

These ten Lua 5.4 programs cover dense and sparse tables, closure calls, generic iteration, math/table operations, array mutation, string building, and memory controls. Lua 5.4 is a separate adapter from Luau in the harness ([bench design](../../../design/80-bench-suite.md#the-matrix)).

## Interface

- Each file returns a function taking `n` and returning a numeric result: [`01_dense_tables.lua`](01_dense_tables.lua), [`02_sparse_tables.lua`](02_sparse_tables.lua), [`04_closures_upvalues.lua`](04_closures_upvalues.lua), [`09_generic_iteration.lua`](09_generic_iteration.lua), [`10_stdlib_math_table.lua`](10_stdlib_math_table.lua), [`12_array_mutation.lua`](12_array_mutation.lua), [`15_string_building.lua`](15_string_building.lua), [`16_memory_burst.lua`](16_memory_burst.lua), [`17_memory_churn.lua`](17_memory_churn.lua), [`18_memory_retained_control.lua`](18_memory_retained_control.lua).

## Invariants

- `ci-bench` alone supplies timing evidence; these programs are inputs to the harness ([repository policy](../../../AGENTS.md#benchmark-policy)).
- The returned value is a checksum/aggregate from the described loop; preserve workload shape and return value when editing.
- `17_memory_churn.lua` retains only the latest eight batches in a ring; `18_memory_retained_control.lua` intentionally retains all batches ([churn](17_memory_churn.lua#L1), [control](18_memory_retained_control.lua#L1)).
- Lua 5.4 files are kept separate from Luau adapter inputs.

## Side Effects

- The programs allocate and mutate tables, arrays, and strings. The retained-memory control intentionally grows a reachable table across calls.

## Verification

Run from the repository root: `just check-benchmark-results` validates committed benchmark artifacts and offline tools. This does not parse or execute these guest sources and does not generate timing evidence.
