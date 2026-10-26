# Luau Workloads

## Overview

These twenty Luau programs cover interpreter and application shapes, including table operations, metamethods, closures, patterns, formatting, coroutines, protected calls, iteration, serialization, memory controls, and deterministic todo state.

## Interface

- The nineteen general workloads return a function taking `n` and returning an aggregate/checksum. Sources: [`01_dense_tables.luau`](01_dense_tables.luau), [`02_sparse_tables.luau`](02_sparse_tables.luau), [`03_oop_metamethods.luau`](03_oop_metamethods.luau), [`04_closures_upvalues.luau`](04_closures_upvalues.luau), [`05_string_patterns.luau`](05_string_patterns.luau), [`06_string_format.luau`](06_string_format.luau), [`07_coroutines.luau`](07_coroutines.luau), [`08_pcall_xpcall.luau`](08_pcall_xpcall.luau), [`09_generic_iteration.luau`](09_generic_iteration.luau), [`10_stdlib_math_table.luau`](10_stdlib_math_table.luau), [`11_serialization.luau`](11_serialization.luau), [`12_array_mutation.luau`](12_array_mutation.luau), [`15_string_building.luau`](15_string_building.luau), [`16_memory_burst.luau`](16_memory_burst.luau), [`17_memory_churn.luau`](17_memory_churn.luau), [`18_memory_retained_control.luau`](18_memory_retained_control.luau), [`19_todo_steady.luau`](19_todo_steady.luau), [`20_todo_history.luau`](20_todo_history.luau), [`21_todo_retained_control.luau`](21_todo_retained_control.luau).

- [`24_string_opcodes.luau`](24_string_opcodes.luau) instead executes at top level, reads the harness-supplied `BENCH_N` global, and returns one checksum. This exposes all nine specialized string opcodes to the compiler for the supplemental acceptance lane.

## Invariants

- `ci-bench` alone supplies timing evidence; these programs are inputs to the harness ([repository policy](../../../AGENTS.md#benchmark-policy)).
- The todo workloads use a fixed initial pseudo-random state so repeated command traces are deterministic ([steady](19_todo_steady.luau#L2), [history](20_todo_history.luau#L2), [retention control](21_todo_retained_control.luau#L2)).
- The memory controls intentionally differ: `17` caps reachable batches at eight, `18` is a positive retention control, and `21` keeps command history unbounded ([17](17_memory_churn.luau#L1), [18](18_memory_retained_control.luau#L1), [21](21_todo_retained_control.luau#L1)).
- The Luau generic-iteration source exercises Luau's `__iter` generalized-for path; it is not the Lua 5.4 `__pairs` workload ([source](09_generic_iteration.luau#L1)).

## Side Effects

- These guest programs allocate and mutate tables, arrays, strings, and captured state; memory controls intentionally retain state across repeated calls.

## Verification

Run from the repository root: `just check-benchmark-results` validates committed benchmark artifacts and offline tool self-tests. It does not execute these guest programs or establish performance results.
