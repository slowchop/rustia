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
| `488b5da` | fuse numeric table read into local self-add | two CI runs averaged 5.4-5.9% faster total median vs `441b29b`, with dense tables 14.7% faster; about 14.4-14.8% faster vs old baseline |
| `31955f8` | fuse sparse table read-or-zero adds | two CI runs averaged 4.6% faster total median vs `488b5da`, with sparse tables 11.5% faster; about 18.5-18.7% faster vs old baseline |
| `1c802df` | skip fuel cost lookup when fuel is disabled | runs `28964187207`/`28964565694`: average total median 0.8% faster vs `31955f8`, with dense/generic/stdlib/closures faster and sparse median 0.3% slower; about 19.1-19.4% faster vs old baseline |
| `c669e48` | add quickening sequence feedback to the bytecode summary | tooling only; reports adjacent table/iteration pairs and loop-body opcode sequences so superinstruction candidates are evidence-backed |
| `9105988` | feature-gate dynamic opcode profiling and hot-site summary | tooling only; runs `28968775343`/`28968901905` averaged total median +0.20% and total min -0.05% vs `1c802df` average, with the second run back at normal `generic_iteration` levels |

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
| `4c9c452` cached inline-cache site slots | `d7b818e` | run `28960416444`: total median +3.21% vs `441b29b` repeat, sparse +7.35% |
| `0505391` direct builtin `table.sort` opcode | `f321452` | runs `28961247501`/`28961367517`: average total median -0.33% vs `441b29b` repeat, but dense median +0.42% and closures +13.86%; too small and noisy to keep |
| `f2669e8` combined `ipairs(c.items)` field prep | `1a67c03` | runs `28963553868`/`28963671573`: average total median +0.04% vs `31955f8` average, total min +0.11%, and targeted `generic_iteration` median +0.61%; extra bytecode surface without a proven win |
| `85c3a86` string-const direct table-get fast path | `ffc4d9d` | runs `28965406975`/`28965521735`: average total median +0.45% vs `1c802df` average, total min +0.63%; dense and stdlib improved, but generic_iteration +1.60% and sparse +0.63% made it a net loss |
| `ebb5329` table-op `ForLoopStepOne` tail quickening | `7e4e6e8` | run `28966423248`: total median +11.56% and total min +11.39% vs `1c802df` average; every workload regressed, so opportunistic dispatch fusion was worse than the regular loop dispatch |
| `02c5867` combined sparse read-pair opcode | `9cacc6b` | runs `28967201259`/`28967318773`: average total median -0.64% and sparse median -3.52% vs `1c802df` average, but generic_iteration regressed +5.85%; too much cross-workload damage for a small total win |

## Current Shape Feedback

`cargo run -p script-bench-harness -- --rustia-bytecode-summary` shows the
default workload hot shapes, adjacent table/iteration opcode pairs, and
loop-body opcode sequences:

| workload | table-related opcodes | main signal |
| --- | ---: | --- |
| `dense_tables` | 6 | array writes plus fused `AddIndexNumber` sum reads and numeric `table.sort` |
| `sparse_tables` | 5 | fused read-or-zero adds for concat string keys and scattered integer hash keys |
| `generic_iteration` | 17 | generalized iteration, pairs/ipairs, table field access |
| `stdlib_math_table` | 10 | global/string-const reads plus table writes/sort flush |

The compiler already emits specialized opcodes for the obvious sparse and
string-const patterns, and it already emits hash capacity hints for sparse-like
fresh tables. More tiny table probes have not paid for themselves so far.
The failed `ebb5329` experiment also shows that blindly folding the loop tail
after table ops is not enough; any next dispatch-reduction attempt needs to
remove more work than a single `ForLoopStepOne` dispatch or use a narrower
compiled superinstruction with better register/branch locality.
The failed `02c5867` sparse read-pair opcode showed that even a targeted
superinstruction can disturb unrelated workloads enough to lose branch/codegen
locality; future sparse work should measure whole-suite impact before keeping
small targeted wins.

`cargo run -p script-bench-harness -- --rustia-inline-cache-summary` runs the
default workloads with `n=64` and reports final atom/IC side-table occupancy.
Current feedback:

| workload | IC signal |
| --- | --- |
| `dense_tables` | one direct table-get slot; array writes and dynamic numeric writes do not leave exact-key IC entries |
| `sparse_tables` | no table IC occupancy; dynamic concat string keys and scattered numeric keys intentionally avoid exact-key IC churn |
| `generic_iteration` | one direct table-get slot, one direct table-set slot, plus one iter metamethod slot and one len slot |
| `stdlib_math_table` | one global-get slot and one direct table-get slot from setup; the hot loop is mostly math/table write/sort work |

`cargo run -p script-bench-harness --features rustia-hot-site-summary -- --rustia-hot-site-summary`
runs the default workloads with `n=64` and dynamic VM opcode profiling enabled.
The profiling code is behind the non-default `rustia-hot-site-summary` bench
feature, which enables the VM's non-default `opcode-profiling` feature, so
normal benchmark and release builds do not compile the opcode profiling field,
API, or dispatch-path branch.

## Next Serious Bets

1. Build real feedback-directed quickening for hot table sites instead of
   adding more cold-path helpers. Track hit/miss counts per IC site, then
   promote stable hot sites to a narrower quickened path.
2. Reduce interpreter dispatch/register overhead around hot table loops with
   superinstructions that fuse the current benchmark patterns, especially
   numeric-for plus table set/get sequences. `488b5da` shows this can pay when
   it removes an operation from a very hot loop; follow-ups should recover the
   small dense regression from `31955f8` before adding more dispatch surface.
3. Profile native `table.sort` with a dedicated microbench before changing it
   again. The allocation-free version was slower because comparator overhead
   dominated the saved allocation, and a direct opcode did not hold up enough
   on CI to justify the extra dispatch surface.
4. Keep no-unsafe as a hard feature. All candidates above were safe Rust, and
   the crate-level `forbid(unsafe_code)` policy should remain intact.
