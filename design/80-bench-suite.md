# rustia — continuous perf-benchmark suite + CI (design)

> **Status: [HARNESS IMPLEMENTED — measurement paused; standalone split pending] (updated 2026-10-29).** Initially scoped by the `bench-scoping` agent (clone+read of
> `khvzak/script-bench-rs`, the 11-workload seed in `spikes/bench-suite/`, and the 2025–26 tracker/wasm-shell
> tooling) under dedicated ephemeral-Linux constraints. The active public-source profile runs hosted
> correctness and artifact checks. New measurement and publication await a reviewed trusted private
> benchmark bridge; committed results are historical. The owned-host procedures below describe the
> retained private protocol. Cloud provisioning is retired. See [`CURRENT.md`](CURRENT.md) and
> [benchmark operations](81-benchmark-operations.md#public-source-profile).

## Decision 1 — REWRITE, not fork (LOCKED, gak + license evidence)
`khvzak/script-bench-rs` has **no LICENSE / no `license` field** → all-rights-reserved → vendoring into a
public-bound repo is unsafe. It's also a single Rust↔script *interop* workload; our seed already has 11
in-VM-throughput workloads. **Clean rewrite**, keep comparable workload coverage, and record provenance in
[`../bench/PRIOR-ART.md`](../bench/PRIOR-ART.md). (Optional, deferred: run script-bench-rs unmodified at a
pinned SHA in a separate job for bonus comparison columns — no vendoring, no license issue. Nice-to-have.)

## Tracking pipeline — explicit dedicated Linux lifecycle
Split by what's measurable; never use one for the other's job:
- **Release gate = generated-history integrity and conservative claim checks.** Correctness CI validates JSONL,
  generated markdown/JSON/HTML, wasm gate artifacts, and the claim policy without taking fresh noisy timings.
- **Headline numbers = harness wall clock**, with fresh prepared state, untimed warmup, checksum validation, and
  min/median summaries on the owned, attested `ci-bench` host. Claims remain per target/workload.
- **History/tracking = checked benchmark artifacts and historical series.** The retained private protocol
  records accepted main-branch measurements and publishes complete matrix cohorts through a pull request.
  Those workflows are absent from the active profile. [The coordinator](../tools/run-dedicated-benchmark.js)
  retains host attestation before building and measuring; resumption requires the trusted bridge.
  No lane provisions or deletes a cloud instance. See [the operations contract](81-benchmark-operations.md).
- **Dashboard = generated `bench/RESULTS.md`, current-results JSON/HTML, and claim markdown**, all derived from
  the same history.
- *Graduate-to later:* add instruction-count regression alerts or a queryable dashboard if the
  wall-clock history proves insufficient; do not make either a prerequisite for honest current measurements.

## The matrix
- **engine:** `rustia` · `boa` · `koto` · `luaur` · `mlua-luau` · `mlua-luau-jit` · `mlua-lua54` · `rhai` · `roto` · `ruau` ·
  `rquickjs` · `wasmi` · `wasmtime`; feature gates keep adapters independently buildable and Lua 5.4 separate
  from Luau.
- **retained performance protocol:** native `linux-x86_64` on the owned benchmark host `ci-bench`, plus
  wasm32 under Node/V8 on that same host. The active public-source profile has no timing authority; all
  recorded results, including Deno/V8, Bun/JavaScriptCore and SpiderMonkey portability evidence, are historical.
  The broader platform support matrix in `00-constraints.md` remains a correctness/build concern, not a
  license to mix noisy macOS or Windows timings into the active benchmark product.
- **determinism:** a **golden-hash column** (bit-identical cross-arch/engine?) + rustia's `mode`
  (LuauCompat/ReproducibleStandard/DeterministicSim). A pass/fail **correctness gate**, NOT a perf trend — a
  hash mismatch FAILS CI; never tracked as a number.
- **workload:** 20 source-covered entries, with five default measured families (`dense_tables`, `sparse_tables`,
  `closures_upvalues`, `generic_iteration`, `stdlib_math_table`) and opt-in additions including
  `string_building`.

**Canonical record** (one JSON line/measurement):
`{timestamp_unix, commit, target, host, engine, dialect, workload, unit, n, reps, ns_per_unit_min,
ns_per_unit_median, units_per_sec, checksum}`.

`host` names the machine the row was measured on — a provenance label published
in row data, never an address. The harness accepts its absence when reading
history, because rows written before the field existed do not carry it and that
absence means "measured before hosts were recorded" rather than "measured
somewhere unnamed"; a row that carries the key must carry a non-empty value.
Published tables must name exactly one host, which is enforced rather than
documented — see [`81-benchmark-operations.md`](81-benchmark-operations.md),
"Row provenance".

The stable JSONL record deliberately remains a min/median summary. An optional
`rustia.bench.raw-samples.v1` JSON sidecar stores execution-ordered per-repetition `ns/unit` values plus the
commit, target, workload, selected `n`, `n_selection` (`calibrated` or `fixed`), checksum, and summaries for
local analysis. Calibration probes and
recorded repetitions each create fresh prepared engine/workload state; a single untimed warmup immediately
precedes the one timed invocation. Each repetition's separate cold-start probe is destroyed before the
steady-state engine is constructed, warmed or timed; only one prepared workload remains live at a time.
Destruction is outside both timing regions. This releases the probe's objects, not necessarily allocator
pages or process RSS, and does not reset the process allocator between repetitions.
Baseline/candidate comparisons should retain separate sidecars and alternate
run order (ABBA/BAAB). `--fixed-n` bypasses calibration for exact equal-work comparisons. Sidecars are evidence
for analysis, not a new public-history schema. The dependency-free
`analyze-fixed-samples.js` validates fixed-work identity and summaries, then
emits deterministic pooled and leg-clustered hierarchical bootstrap JSON with
a conservative-envelope verdict that is retained with accepted evidence.

## Automation structure

1. Active correctness pull-request/push CI validates code, history, generated artifacts, and claims without measuring.
2. Measurement and publication are paused pending a reviewed trusted private bridge. No active workflow
   invokes the retained per-commit, screening/calibration, matrix, A/B acceptance or memory lanes.
3. The retained private protocol confines timing to the attested owned host `ci-bench`, including the
   separate Lua 5.4 binary, wasm adapters and dispatch check. Nothing creates or pays for a cloud instance.
4. Resumption must preserve the complete single-revision Linux matrix promotion gate. PR screening and
   A/B acceptance remain summary-only evidence and cannot publish history rows.

**Current implementation slice:** [correctness CI](../.github/workflows/ci.yml) runs release preflight
without fresh benchmark measurements. The [explicit profile](public-source-profile.json) and
[workflow validator](../tools/check-public-workflow-profile.js) allow hosted correctness/fuzz workflows only.
Private measurement workflows are absent; their [retained coordinator](../tools/run-dedicated-benchmark.js)
does not make them active or authorize a dispatch. The `bench` crate enumerates thirteen engine modes,
including separate Luau interpreter/JIT modes, five default measured workloads, and 20 workload entries including cross-adapter source
coverage for #15 `string_building`, portable memory probes #16–#18, deterministic todo-app probes #19–#21,
and the supplemental #24 `string_opcodes` probe. The string probe runs a top-level program with `BENCH_N`
so compiler specialization reaches all nine dedicated string opcodes; it is outside the five default workloads.
It also owns `bench/wasm-gate.json`, a deterministic pre-lock gate tracker
exposed by `script-bench --wasm-gate-json` and rendered by the generated HTML report. The wasm target bench builds
`bench/wasm/adapter-runner` and records `rustia`, `koto`, and `rhai` under Node/V8 in
`bench/wasm/current-results.json`; the accepted four-runtime matrix is retained under `bench/wasm/history/`, and
adapter exclusions are documented in `bench/wasm/ADAPTERS.md`. The recurring dispatch gate records `call_ref`
versus `br_table` under Node/V8 in `bench/wasm/dispatch-current-results.json`; the completed cross-engine result
is retained beside the historical matrix.
The native harness now offers `--samples-json` for raw sidecars and isolates every calibration probe and timed
repetition in freshly prepared state. It also offers `--fixed-n` for exact equal-work A/B runs.

### Historical cloud evidence (retired measurement paths)

The following results record earlier architecture experiments. Their rental commands and measurement rows
are retired from the active benchmark product; historical evidence remains in Git history. They do not
describe current operating instructions or current public performance claims. Follow
[benchmark operations](81-benchmark-operations.md) for the implemented owned-host protocol.

The earlier table/cache slice retained a cooled 1.29% dense-table signal; six later diagnostic legs overlapped
around +0.90%.
The accepted exact-commit Linode v2 comparison then measured 2.42% lower corrective-candidate ns/unit on pooled
`dense_tables` samples, with a conservative nominal 95% interval of -3.35% to -1.57%, excluding the predeclared
+5% material regression. Qualification spread was 0.50%, all contaminant gates passed, peak timed steal was
0.074% under the 0.1% ceiling, and three actual final-binary rows plus raw evidence were admitted to the
then-current history. This closed the dense material-regression gate but was workload/host-specific, not a blanket public
speed claim. A second exact Linode comparison measured 65.959% lower complete-candidate ns/unit on
`oop_metamethods`, with a conservative nominal 95% interval of -66.39% to -65.55% against a zero-regression
threshold, and retained three more final-candidate history rows. That candidate includes both the landed
constant-field/method paths and a weak-table GC-scan correction; the delta is not attributed to one opcode family.
The separate evidence-only 512-VM protocol retained 16 accepted paired rounds and 160 fresh-process samples. Its
primary candidate-minus-baseline contrasts saved 32.01, 258.00, 105.39, and 260.97 KiB/VM across unloaded-short,
unloaded-long, loaded-short, and loaded-long string effects, with a 7.08 KiB/VM loaded-long residual. It never
modified throughput history or public claims. `tools/check-workflows.js` keeps correctness workflows
validation-only and confines fresh measurements to the owned-host lanes.

The historical dense follow-up used provider-specific but protocol-preserving ephemeral runners, while
`bench/cloud/linode-oop-f208c12.json` and `bench/cloud/linode-vm-string-memory-1370d6c.json` recorded the accepted
OOP-throughput and evidence-only memory protocols. The now-removed qualification recipe `just bench-do-trials` compared regular
and Premium Intel 2-vCPU/4-GiB shared shapes without running comparison or
publication. Repeated nonzero-steal timed legs rejected that shared tier;
dedicated Akamai/Linode placements also remained KVM guests and eventually
violated the frozen v1 zero-steal gate. Those blocks remain discarded. The
prospectively versioned Linode v2 fixed-throughput protocol kept every pre-leg idle window at zero steal and
permitted no more than 0.1% steal during a timed leg, while preserving the fixed work, 3% qualification spread,
ABBA/BAAB ordering, contaminant exclusion, conservative bootstrap verdict, and publication rule. The v2 memory
config instead froze its own warmup, paired-round, evidence-only analysis, and unchanged-history rules.
The accepted Sydney dense and OOP runs satisfied their prospectively declared gates and published their three
final candidate rows and evidence; the memory run satisfied its separate evidence-only gate and left history
unchanged.
The now-removed `just bench-vultr` recipe explored a rented bare-metal host with qualification,
fixed-work comparison, and deletion before evidence promotion. It is not a current command.
Cloud lifecycle cost ceilings, ownership journals, and deletion gates are retired; release preflight
retains only offline checks of the shared acceptance rules.

At historical 12-adapter snapshot `a562919`, the prototype-polymorphic closure-call cache and direct immutable
captures moved rustia `closures_upvalues` median latency from 270.427 to
229.766 ns/call across separate, identically specified dedicated-Linux instances (15.036% lower). The generated
five-workload minimum-time gaps at that snapshot were 3.494x to `mlua-luau` and 3.833x to Luaur. Non-target rustia movements in
that cross-host comparison are stability signals, not causal regression claims; current generated artifacts and
the retained raw evidence remain authoritative.

The historical small-regression milestone used three independent fixed-N publication runs. Current A/B
acceptance uses the [owned-host acceptance lane](81-benchmark-operations.md#ab-acceptance-on-the-owned-host)
and publishes no history rows; the recurring language matrix has its own completeness and publication gate.

**Rustia's column:** the `bench` crate enumerates engines by **cargo feature**; `rustia` now runs through the
default `rustia-engine` feature while the external adapters stay independently gated for standalone builds.

## Closed wasm pre-lock gate; recurring evidence remains
Production rustia wasm now runs through the bench wasm adapter runner, with koto and rhai as wasm-compatible
comparison adapters. The recurring public benchmark uses Node/V8. Accepted V8, Bun/JavaScriptCore-family, and
SpiderMonkey measurements show `call_ref` behind `br_table` and remain archived as portability evidence. The
pre-lock wasm gate is closed. A future trusted bridge must refresh Node/V8 evidence as the harness changes;
that requirement remains pending while measurement is paused.

## Where it lives
```
bench/  Cargo.toml(feature-gated adapters)  src/main.rs  workloads/<dialect>/
        history/runs.jsonl  history/evidence/  RESULTS.md  current-results.{json,html}
        CLAIMS.md  wasm/  PRIOR-ART.md
```

## Open decisions (gak)
- **D1 — regression alerts:** decide whether longer history justifies an instruction-count gate; keep SaaS
  egress opt-in.
- **D2 — optional external comparison:** running script-bench-rs at a pinned SHA remains deferred and
  nonessential.
