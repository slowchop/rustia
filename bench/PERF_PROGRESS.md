# Rustia Performance Progress

This is the working log for rustia VM performance experiments. Keep measured
improvements here so future table/atom/inline-cache/quickening work starts from
evidence instead of hunches.

## Current Kept Wins

Baseline for this branch is the old pre-perf CI artifact at `a728ee7`.

| commit | change | CI result |
| --- | --- | --- |
| `1551ff8` | throttle GC debt checks | about 6.5-7.9% faster total median vs old baseline |
| `3e18976` | reuse vacant hash probe on insert | about 1.1-1.6% faster total median vs prior kept commit |
| `441b29b` | spread numeric table hash keys | about 0.5-1.1% faster total median vs prior kept commit; about 9.4-9.9% faster vs old baseline |
| `8ba5167` | add Rustia bytecode summary | tooling only; exposes table, atom, inline-cache, and quickening candidates |

## Reverted Candidates

These had passing tests but regressed or failed to hold up on the self-hosted
CI bench runner. Do not retry them without a materially different approach.

| candidate | reverted by | result |
| --- | --- | --- |
| `cb5acde` small integer concat atom cache | `a48a5cb` | total median +2.03%, sparse +7.71% |
| `34c1ea1` concat string table index fast path | `56074fa` | repeated run regressed up to +1.79% |
| `d5c5c99` numeric array write fast path | `c8c4ee8` | repeat regressed +0.47% to +1.04% |
| `fc48901` shared string table inline-cache shapes | `2c6db02` | mixed repeat, average slower |
| `67eaf2c` combined iter metamethod cache validation | `67cb544` | average about +0.6% slower |
| `dedd94c` atom-keyed string table caches | `98f72e1` | total median +7.58%, sparse +19.16% |
| `3d13fea` avoid keyed caches for dynamic string writes | `016d61e` | total median +4.23%, sparse +7.46% |
| `7a9f45a` reuse string hash vacant probe on write | `cda4211` | total median +3.42%, sparse +7.92% |
| `6834b2d` combined integer table read helper | `b86b693` | run `28959453661`: total median +0.54% vs `441b29b` repeat, sparse +0.71% |
| `8f366af` in-place numeric `table.sort` | `0350940` | run `28959846761`: total median +3.42% vs `441b29b` repeat, dense +3.83% |

## Current Shape Feedback

`cargo run -p script-bench-harness -- --rustia-bytecode-summary` shows the
default workload hot shapes:

| workload | table-related opcodes | main signal |
| --- | ---: | --- |
| `dense_tables` | 6 | array writes/reads plus numeric `table.sort` |
| `sparse_tables` | 5 | concat string keys and scattered integer hash keys |
| `generic_iteration` | 17 | generalized iteration, pairs/ipairs, table field access |
| `stdlib_math_table` | 10 | global/string-const reads plus table writes/sort flush |

The compiler already emits specialized opcodes for the obvious sparse and
string-const patterns, and it already emits hash capacity hints for sparse-like
fresh tables. More tiny table probes have not paid for themselves so far.

## Next Serious Bets

1. Build real feedback-directed quickening for hot table sites instead of
   adding more cold-path helpers. Track hit/miss counts per IC site, then
   promote stable hot sites to a narrower quickened path.
2. Reduce interpreter dispatch/register overhead around hot table loops with
   superinstructions that fuse the current benchmark patterns, especially
   numeric-for plus table set/get sequences.
3. Profile native `table.sort` with a dedicated microbench before changing it
   again. The allocation-free version was slower because comparator overhead
   dominated the saved allocation.
4. Keep no-unsafe as a hard feature. All candidates above were safe Rust, and
   the crate-level `forbid(unsafe_code)` policy should remain intact.
