# JavaScript Workloads

## Overview

These ten programs provide JavaScript adapter equivalents for table, closure, iteration, math, string, array-mutation, and memory workload families.

## Interface

- Each source defines a callable workload taking `n` and returning a numeric checksum or aggregate: [`01_dense_tables.js`](01_dense_tables.js), [`02_sparse_tables.js`](02_sparse_tables.js), [`04_closures_upvalues.js`](04_closures_upvalues.js), [`09_generic_iteration.js`](09_generic_iteration.js), [`10_stdlib_math_table.js`](10_stdlib_math_table.js), [`12_array_mutation.js`](12_array_mutation.js), [`15_string_building.js`](15_string_building.js), [`16_memory_burst.js`](16_memory_burst.js), [`17_memory_churn.js`](17_memory_churn.js), [`18_memory_retained_control.js`](18_memory_retained_control.js).

## Invariants

- `ci-bench` alone supplies timing evidence; this directory contains workload inputs, not a local timing path ([repository policy](../../../AGENTS.md#benchmark-policy)).
- Workload loops and arithmetic produce a result consumed as the adapter checksum; preserve the loop and return semantics across adapters.
- `17_memory_churn.js` keeps a ring of eight batches, while `18_memory_retained_control.js` intentionally appends batches without releasing them ([churn](17_memory_churn.js#L1), [positive control](18_memory_retained_control.js#L1)).
- These files are workload inputs, not timing authority. Do not execute locally for performance results.

## Side Effects

- Workloads allocate and mutate arrays and objects; the closure workload mutates a captured variable; the retained control deliberately keeps every created batch reachable.

## Verification

Run from the repository root: `for f in bench/workloads/javascript/*.js; do node --check "$f" || exit; done` checks syntax only and does not execute the workload functions.
