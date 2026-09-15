# rustia — cross-engine memory benchmark design

> **Status: [PROTOCOL, RECLAMATION, ACCOUNTING, AND QUOTAS IMPLEMENTED] (2026-07-29).** The isolated child/supervisor, cgroup-v2 sampler,
> repeated-epoch analyzer, portable controls, deterministic todo-app workloads, publication schema, and
> dashboard are implemented. The published cohort is `rustia-ci-070c387b173c`: 11 rustia rows from 99 raw
> samples, measured on the owned host ci-bench on 2026-09-05 through a delegated cgroup-v2 subtree. The
> cloud-measured cohorts — `pre-reclamation-18b58d8` (110 rows from 990 fresh processes) among them — were
> removed on 2026-09-06. The owned-host lane can now measure that same 110-row protocol as its `cross-engine`
> cohort, so the cross-engine memory comparison returns with the first such cohort dispatched on ci-bench;
> none has run yet. Tables, functions, upvalues, userdata, buffers,
> and vectors now reclaim backing allocations into generation-validated reusable slots; stale handles cannot
> alias replacement objects, and generation exhaustion retires the slot instead of wrapping. Loaded programs
> follow the same lifetime discipline. Maintained logical-live totals now back script counters, quota admission,
> a forced-collection retry, and atomic catchable OOM; reserved/peak telemetry stays explicit. A shared
> 96-epoch/3,072-wave native and Node/V8 wasm test now closes the mixed-heap correctness/reclamation gate before
> final GC tuning. External allocator/page-return and process-RSS history remain separate measurement work. This protocol extends the existing
> `vm-string-memory` evidence rather than replacing it. Canonical project state remains
> [`CURRENT.md`](CURRENT.md).

## Goal

Make memory a first-class benchmark dimension for rustia and every applicable competitor:

- capture short-lived allocation spikes that periodic RSS polling can miss;
- separate process/runtime baseline, source preparation, warm execution, peak working set, and retained tail;
- detect bounded plateaus versus sustained growth over repeated allocation/release epochs;
- retain reproducible raw phase/timeline evidence and publish compact current/history summaries;
- show memory in the benchmark dashboard without collapsing speed and memory into one opaque score.

The first rule is **measure before redesign**. Record the current append-only rustia behavior and the competitor
baselines with this protocol before implementing reclamation. That gives the memory work an honest before/after
gate and prevents a local heap counter from becoming the only definition of success.

## Measurement decisions

### 1. Fresh process and cgroup per engine/workload/sample

Every memory sample runs one engine and one workload in a new process inside its own cgroup-v2 leaf. The
supervisor is outside that leaf and, on the dedicated Linux host, runs on a different CPU from the measured
child.

This is required because the current throughput harness links several large runtimes into one binary and runs
engines sequentially. Measuring that long-lived process would mix allocator residue, JIT state, shared
libraries, and prior-engine state. The memory runner may reuse the adapter implementations and source inventory,
but it must invoke each measurement through an isolated child mode.

Memory and throughput remain separate runs. Memory observation, phase handshakes, and deliberate churn alter
timing; the existing uninstrumented throughput rows stay authoritative for speed.

### 2. Kernel-external primary metrics

The primary comparison is external to the engine:

| metric | source | purpose |
|---|---|---|
| baseline total/PSS/anonymous | `/proc/PID/smaps_rollup` | runtime and executable footprint before source preparation |
| phase total/PSS/anonymous | `/proc/PID/smaps_rollup` | prepared, warmed, quiescent, and final snapshots |
| kernel-recorded peak charge | cgroup v2 `memory.peak` | catches short spikes between snapshots, including descendants |
| current charge and breakdown | `memory.current` + keyed `memory.stat` | cgroup-level total, anonymous, file, kernel, and page-table context |
| pressure and failures | `memory.pressure` + `memory.events.local` | rejects pressure, reclaim, OOM, or swap-contaminated samples |
| page-fault deltas | cgroup `memory.stat` (`pgfault`, `pgmajfault`) | corroborates residency and cold-page effects |

`smaps_rollup.anonymous` is the primary phase-attribution field because it excludes most executable/file-backed
mapping noise. PSS and total RSS remain corroborating fields. `VmRSS` and `VmHWM` from `/proc/PID/status` are
diagnostic only: the Linux manual explicitly describes them as inaccurate.

The fresh per-sample cgroup captures the whole measured process tree and provides the authoritative whole-run
short-spike high-water mark. Where the host kernel supports resettable per-FD peaks, the supervisor also opens
and resets a `memory.peak` file descriptor at each phase boundary and reads that same descriptor at phase end.
The protocol records this capability. A kernel without resettable per-FD peaks may publish the whole-run peak
plus phase snapshots, but must mark phase peaks unavailable instead of silently substituting sampled RSS.

Engine-internal heap/GC counters are optional corroboration. They are valuable for explaining a result and for
validating rustia's forthcoming accounting, but they are not the cross-engine ranking metric because engines
define heap, reserved capacity, JIT code, and external allocations differently.

### 3. Phase model

Each sample uses explicit child/supervisor phase handshakes:

1. **spawned** — process and runtime loaded, source untouched;
2. **prepared** — source parsed/compiled and callable retained;
3. **warmed** — one untimed invocation complete;
4. **steady work** — fixed work units complete;
5. **quiescent** — bounded idle/collection opportunity complete, without allocator-specific trimming;
6. **dropped** — prepared engine state dropped while the process remains alive long enough for a final snapshot.

Stress workloads add repeated **epoch** markers between warmed and quiescent. Timeline samples are taken at each
marker and, for area-under-curve diagnostics, at a fixed supervisor cadence. The kernel peak remains authoritative
for sub-cadence spikes.

Do not call `malloc_trim`, purge a specific allocator, or force a collector only for selected engines. Where a
portable script-visible full-collection operation exists, record a separate `forced-collection` lane; natural
policy remains the headline.

### 4. Published metrics

The dashboard publishes several independent metrics:

| name | definition | interpretation |
|---|---|---|
| **idle footprint** | spawned anonymous PSS and total PSS | cost of starting this engine mode |
| **prepared delta** | prepared anonymous minus spawned anonymous | source/compiler/program cost |
| **peak working-set delta** | maximum cgroup charge after phase reset minus phase-start charge | short-term headroom required |
| **quiescent retained delta** | quiescent anonymous minus warmed anonymous, adjusted by the null-control lane | memory still resident after work |
| **recovery ratio** | `(peak delta - quiescent delta) / peak delta` | how much of a burst was recovered |
| **tail growth** | robust KiB/epoch slope over the predeclared tail window | plateau versus sustained retention signal |
| **memory area** | integral of excess anonymous memory over time and per completed unit | memory-time cost; kept separate from peak |
| **minimum completion limit** | smallest tested `memory.max` bucket that completes correctly | periodic memory-pressure capability, not every-commit data |

Absolute and incremental values are both retained. Incremental values answer “what did the script cost?” while
absolute values answer “how much memory must a host provision?” JIT/native code pages remain part of the absolute
cost even when they are file-backed or executable.

There is deliberately no single speed-plus-memory score. Ranked bars may show a workload-complete geometric
mean for one selected memory metric, but the dashboard must state the metric and preserve the per-workload
heatmap.

### 5. Growth classification, not casual leak claims

RSS can remain high because an allocator or collector caches reusable pages. That is not by itself a leak.
For each epoch workload:

- use at least one warmup process and nine accepted fresh-process samples;
- record enough epochs to expose a plateau and predeclare the tail window;
- compute a Theil–Sen tail slope per sample;
- bootstrap at the fresh-process level, never at individual timeline-point level;
- require a bounded-live null control and an intentionally retained positive control in the same cohort;
- predeclare an absolute KiB/epoch and proportional percent/epoch material threshold.

Dashboard states:

- **stable in tested window** — the upper confidence bound is below both material thresholds;
- **growth signal** — the lower confidence bound exceeds either material threshold and the positive control
  validates the detector;
- **inconclusive** — anything between those states;
- **expected retention** — the intentional positive control.

“Leak” is reserved for a diagnosed unreachable allocation that the implementation contract says must be
reclaimed. The cross-engine chart reports observable retention/growth.

## Workload plan

### Lane A — memory envelopes for every existing benchmark

All existing benchmark sources gain memory rows without changing their source:

- `dense_tables`
- `sparse_tables`
- `closures_upvalues`
- `generic_iteration`
- `stdlib_math_table`
- then the seven non-default inventory workloads as their adapter coverage permits.

These rows expose realistic preparation cost, short spikes, quiescent retention, and memory-time cost alongside
the existing speed comparison. They use exactly the same fixed work and checksums as throughput, but execute in
the separate memory protocol.

### Lane B — new portable allocation/retention sources

Add these source families for Luau, Lua 5.4, Koto, JavaScript, Rhai, and other engines whose semantics can express
them faithfully:

| id | shape | what it catches |
|---|---|---|
| `memory_burst` | construct a large batch of short-lived arrays/tables and strings, checksum it, then drop it | sub-sample peak and recovery ratio |
| `memory_churn` | repeat allocate/mutate/drop epochs while retaining only a fixed-size ring | plateau, free-list reuse, allocator/GC caching |
| `memory_retained_control` | same allocation shape, but intentionally append one batch to a global root each epoch | positive control and bytes-per-retained-item scaling |
| `memory_cycles` | create table↔table and closure↔table cycles, drop outer roots each epoch | tracing/cycle reclamation versus permanent retention |
| `memory_table_resize` | repeatedly grow, delete, refill, and discard dense+sparse tables | tombstones, capacity shrink/reuse, hash/array transitions |
| `memory_string_churn` | create unique short/long strings, concatenate/slice, retain a bounded subset | interning, backing-store retention, fragmentation |
| `memory_closure_coroutine_churn` | create closures/upvalues and coroutine stacks, run them, then discard | frame/upvalue/coroutine-store reclamation |

Each source returns a deterministic checksum and has three problem sizes. The bounded-live and retained-control
sources share the same allocation generator so their difference is meaningful.

Not every language has weak tables, coroutines, identity tables, or a tracing heap. Unsupported semantic shapes
are explicitly absent rather than replaced by a misleading approximation. Dashboard completeness is shown per
lane.

The first realistic stateful tranche complements those mechanism probes:

| id | shape | what it catches |
|---|---|---|
| `todo_steady` | seeded create/read/update/delete, tags, priorities, searches, and completion toggles over a bounded task set | ordinary bounded random-access application state |
| `todo_history` | the same command stream with a fixed-size undo/history ring | bounded retained history and replacement behavior |
| `todo_retained_control` | the same command stream with deleted tasks and history intentionally retained without a bound | positive growth control for realistic application objects |

Luau and Koto use the same Park–Miller PRNG, operation mapping, capacities, and command trace, and their
checksums are cross-validated. Synthetic sources diagnose allocation mechanisms; the todo tranche checks that
the conclusions survive a realistic stateful workload.

### Lane C — engine/API lifecycle probes

These cannot be represented honestly as one script function and therefore remain a separate adapter-level lane:

- compile/program creation and drop;
- VM creation, load, run, and drop at 1/16/128/512 VM scales;
- hot reload and failed reload;
- module load/cache/cycle and cache reset;
- deterministic decision reset/reuse — focused native ABI coverage includes 600 exact post-warmup plateau
  decisions, and the Node/V8 wasm32 lifecycle probe executes another 300 with the same exact plateau; extend
  this into the external dedicated-Linux long-running-host lane;
- retained public handles across reload and invalidation across reset — focused epoch coverage is implemented;
- aggregate quota and atomic OOM behavior.

Rustia must pass this lane before reclamation is considered production-safe. Comparable engines may participate
where their public embedding APIs expose the same lifecycle.

### Lane D — Wasm-specific memory

For `wasmi`, `wasmtime`, and production browser-wasm adapters, retain:

- process/cgroup metrics for host cost;
- linear-memory initial/peak/final page counts;
- table size and compiled-code/cache cost where the runtime exposes stable counters;
- bounded `memory.grow` and repeated instance create/drop probes.

Linear-memory pages are displayed separately from process memory; they are not interchangeable.

## Statistical and host protocol

- Run only on the qualified owned benchmark host — runner group `bench-solo`, provenance label ci-bench —
  that produces the accepted timings, pinning two isolated cores so the supervisor never shares a core with
  the measured child.
- Disable swap for the measurement cgroup with `memory.swap.max=0`; reject any nonzero swap or OOM event.
- Record kernel, cgroup mode, base page size, THP configuration, libc/allocator, runtime versions, CPU, and VM
  identity.
- Pin the child to the benchmark CPU and the supervisor/sampler to a different CPU.
- Use balanced rotating engine order across rounds so no engine always runs first or last.
- Use fresh-process samples as the independent statistical unit.
- Retain raw snapshots, peaks, epoch timelines, checksums, exit status, and contamination gates.
- Summarize medians and quartiles; use seeded process-level bootstrap intervals for comparisons and growth
  classification.
- Do not use memory results from a sample whose checksum, cgroup event, swap, pressure, host-idle, or process-tree
  gate failed.

The regular cohort runs at a generous `memory.max` so the limit does not perturb policy. A slower periodic
minimum-completion lane bisects predeclared power-of-two/fine buckets in independent processes. Current GC
research recommends studying space/time behavior across heap sizes rather than at one arbitrary limit; those
results remain a separate pressure curve.

## Result artifacts

Memory data uses a separate versioned history rather than adding optional fields to timing JSONL:

```text
bench/memory/history/runs.jsonl
bench/memory/current-results.json
bench/memory/raw/<commit>/<cohort>.json
```

Proposed schemas:

- `rustia.bench.memory-raw.v1` — every fresh-process phase snapshot, cgroup peak, timeline point, event, and
  checksum;
- `rustia.bench.memory-run.v1` — one summarized engine/workload/mode/cohort row;
- `rustia.bench.memory-current-results.v1` — latest coherent complete cohort selected for the dashboard.

Every summary names its source hash, exact commit, host, mode, target, protocol version, sample count, problem
size, work/epoch count, memory source, and confidence method. Promotion remains fail-closed and runs entirely on the
owned host: the run attests the host before the build, the cohort must reach `memory_complete` status, its
evidence is validated against the frozen config, existing evidence is never overwritten, and the two
publication files are staged atomically with rollback.

A cohort is one run on one machine, and that is checked rather than assumed. The analyzer requires `--host` with
no default and stamps it on every row; `publish-memory-analysis.js` refuses an analysis whose rows disagree about
it; `tools/merge-rustia-ci-memory-data.js` refuses to merge a table spanning two machines; and the dashboard
refuses to render one. `bench/memory/current-results.json` is a per-slot merge across cohorts, so without that
rule a rustia-only refresh leaves every competitor row on whatever machine last measured it — which is what
happened on 2026-09-05, when 11 new rows joined 99 six-week-old cloud rows under metadata naming a single host.
See [`81-benchmark-operations.md`](81-benchmark-operations.md), "Row provenance".

## Dashboard

Add one top-level **Memory** section beside Linux Comparison, Cold Start, Claim Gate, and History:

1. **Ranked bars** — selected metric across the complete common workload set, lower is better.
2. **Workload heatmap** — engine × workload with completeness and confidence markers.
3. **Phase profile** — spawned → prepared → warmed → peak → quiescent → dropped for a selected row.
4. **Retention chart** — epoch on x, anonymous/PSS delta on y, with tail slope and classification.
5. **Memory history** — rustia peak/prepared/retained trends by commit or date, with selectable competitor
   reference lines.

Controls:

- absolute / incremental;
- peak / prepared / retained / memory-area / tail-growth;
- natural policy / forced-collection diagnostic;
- interpreter / JIT/native / all;
- commit / date for history.

The timing and memory panels may cross-link the same engine/workload, but they do not merge scores. While the
accepted cohort carries no competitor rows, the generated dashboard shows it as absolute Rustia figures under
a panel headed “rustia Memory on ci-bench” and says that no competitor engines are present yet; the “memory
baseline pending” state appears only when there is no accepted cohort at all. Fabricated zeroes are never
shown, and a comparison is never drawn against rows measured on another host.

## Diagnostic tools

The publication metric stays low-overhead and external. Use heavier tools only after a regression is found:

- Valgrind Massif for allocation/page profiles and peak allocation trees;
- heaptrack or an equivalent native allocation tracer for temporal attribution;
- LeakSanitizer/Miri where applicable for Rust/C/C++ ownership defects;
- rustia internal heap-object census and reachability dumps for safe-Rust logical retention;
- engine-native heap snapshots as explanatory evidence.

Massif can profile heap allocations or pages, but its own manual documents peak-snapshot approximation and
instrumentation cost. Such traces explain *why* a cohort moved; they do not replace the cgroup/smaps comparison.

## Implementation order

1. Add the child phase protocol and Linux cgroup-v2 supervisor with self-tests using a synthetic spike child.
2. Run every existing workload through one-engine/one-workload fresh-process memory samples.
3. Add `memory_burst`, `memory_churn`, and `memory_retained_control` across the broad dialect matrix; validate
   detection before adding a dashboard growth label.
4. Add memory history/current-result generators and the ranked-bar/heatmap/phase dashboard.
5. Record and promote the pre-reclamation dedicated-Linux baseline.
6. Add cycles, table resize, string, closure/coroutine, VM-scale, reset, module, and OOM lanes.
7. Implement reclamation/quotas/reset, compare against the frozen baseline, and retain raw evidence. The shared
   ordinary-object mark/sweep phase and generation-safe reusable slot stores are implemented for tables,
   functions, upvalues, userdata, buffers, and vectors. Public exported handles use VM-specific reference-counted
   root leases. Tests cover collection after dropping parent exports, repeated reuse around retained closures,
   stale-handle rejection, generation-exhaustion retirement, bounded churn, cross-VM rejection, reachable versus
   unreachable cycles, function environments/captures, userdata payloads, metatables, vector keys, and weak
   edges. Runtime string atoms now use the same generation-safe lifetime model, and explicit host quiescence
   completes reclamation, prunes unreachable coroutine records, compacts bounded application tables, and shrinks
   heap/interner/stack capacity. Loaded programs now use generation-safe reusable handles; quiescence preserves
   programs referenced by live closures/coroutines/host roots and reclaims obsolete bytecode, constants, caches,
   and coverage rows without stale-handle aliasing. Script-triggered and quota-forced collection also reclaims
   unreachable runtime-compiled programs while preserving active top-level and suspended-coroutine program
   roots. VM-owned live/reserved accounting and opt-in
   instruction-boundary peaks are implemented as internal corroboration. Compact per-store generation arrays
   replace tree-set mark scratch. A logical-live per-VM quota now forces one collection before catchable OOM,
   preflights large string/table/buffer requests, and commits growing table writes only after an exact candidate
   fits. Maintained heap/program/control live totals now back the script-visible counters and quota hot path
   without a whole-VM census at ordinary instruction boundaries; exact reserved telemetry remains on demand.
   The published rustia-only cohort is
   `rustia-ci-070c387b173c`: 11 rows from 99 native samples measured on ci-bench on 2026-09-05. Exact-commit
   cohort `safe-memory-588b541` retained 99 native samples earlier on a rented host and passed the wasm32
   512 KiB quota/recovery probe; relative to the frozen competitor cohort of that era, Rustia tied the
   256 KiB best shared-workload peak, ranked first for stress peak, second for stress retention, second for
   todo peak, and third for todo retention, with a shared retained mean of 14.4 KiB against mlua/Luau/JIT's
   24.8 KiB. That evidence was removed with the other cloud-measured cohorts on 2026-09-06. The owned-host
   lane can now measure the same 110-row protocol as its `cross-engine` cohort, so the cross-engine memory
   comparison and the mlua/Luau/JIT reference return with the first such cohort dispatched on ci-bench; none
   has run yet. Decision-reset ABI v1 adds focused 600-decision native
   table/coroutine/program/retained-handle plateau evidence. The shared mixed-heap test adds 96 native and
   Node/V8 wasm epochs with exact post-reset VM accounting and stable warmed wasm linear-memory pages.
   Broader allocator page-return and external dedicated-Linux history remain next.
8. Add the periodic `memory.max` pressure curve and targeted attribution tooling.

## Maintained mixed-heap pressure gate

`crates/rustia/tests/mixed_heap_pressure.rs` is one source compiled for native
and `wasm32-unknown-unknown`; the hosted Node/V8 job runs the wasm artifact.
Each target executes:

- 96 decision epochs and 32 waves per epoch (3,072 waves total);
- a 2 MiB VM logical-live quota with four bounded public root leases;
- cyclic tables, runtime strings, closures/upvalues, suspended coroutines,
  `newproxy` userdata, buffers, vectors, and weak-key/weak-value tables in each
  wave;
- quiescent weak collection with exactly four live entries and zero after all
  strong roots are cleared;
- alternating successful hot reloads plus continued calls through a retained
  pre-reload function;
- an intentionally oversized 8 MiB buffer request that must fail catchably,
  preserve the existing graph, and admit a subsequent allocation;
- epoch invalidation and an exact VM-owned live/reserved plateau after reset.

Within an epoch, every four-wave allocation phase holds reserved capacity
exactly and live accounting inside a 64-byte free-list band after warm-up. On
wasm, the linear-memory page count must remain unchanged throughout the second
half of the run. These are reclamation, accounting, quota, and lifecycle
invariants; they do not assert equal native/wasm byte layouts, allocator
metadata, or process RSS.

## Acceptance gate before memory work is declared complete

- Every headline competitor has memory rows for every shared default workload it can already execute.
- The synthetic sub-poll spike is captured by cgroup `memory.peak`.
- The bounded churn null control plateaus, and the retained positive control is classified as expected growth.
- Rustia demonstrates bounded native and wasm plateaus across tables, strings, functions/upvalues, coroutines,
  userdata/buffers/vectors, and loaded programs.
- Public-handle roots, weak tables, cyclic graphs, hot reload, decision reset, quotas, and atomic OOM retain
  correctness under the same native/Node-V8 wasm source and lifecycle.
- The dashboard reads committed memory history and exposes absolute, incremental, phase, retention, and history
  views without changing the timing claims.

## Primary references

- [Linux cgroup v2 memory controller](https://www.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html) —
  `memory.current`, resettable per-FD `memory.peak`, `memory.stat`, events, swap limits, and cgroup PSI.
- [Linux `/proc` documentation](https://www.kernel.org/doc/html/latest/filesystems/proc.html) —
  `smaps_rollup`, PSS, anonymous/file/shmem breakdowns, and snapshot caveats.
- [`proc_pid_status(5)`](https://www.man7.org/linux/man-pages/man5/proc_pid_status.5.html) — documents the
  inaccuracy of `VmRSS`/`VmHWM`.
- [Linux Pressure Stall Information](https://www.kernel.org/doc/html/latest/accounting/psi.html) — measurable
  memory-pressure stall time and per-cgroup monitoring.
- [DaCapo Chopin: Rethinking Java Performance Analysis (ASPLOS 2025)](https://www.dacapobench.org/assets/pdf/dacapo-asplos-2025-with-appendix.pdf)
  — motivates minimum-heap multiples and memory-area measurements rather than one arbitrary heap size.
- [Valgrind Massif manual](https://valgrind.org/docs/manual/ms-manual.html) — allocation/page profiling,
  retained-but-reachable growth, and documented peak/instrumentation limitations.
