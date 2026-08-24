# rustia — continuous perf-benchmark suite + CI (design)

> **Status: [IMPLEMENTED — measurements ongoing; standalone split pending] (updated 2026-07-23).** Scoped by the `bench-scoping` agent (clone+read of
> `khvzak/script-bench-rs`, the 11-workload seed in `spikes/bench-suite/`, and the 2025–26 tracker/wasm-shell
> tooling) + dedicated ephemeral-Linux constraints + privacy lean. Operationalises principle #3 ("measure,
> don't assert — the bench suite is a CI gate"). See [`CURRENT.md`](CURRENT.md).

## Decision 1 — REWRITE, not fork (LOCKED, gak + license evidence)
`khvzak/script-bench-rs` has **no LICENSE / no `license` field** → all-rights-reserved → vendoring into a
public-bound repo is unsafe. It's also a single Rust↔script *interop* workload; our seed already has 11
in-VM-throughput workloads. **Clean rewrite**, keep comparable workload coverage, and record provenance in
[`../bench/PRIOR-ART.md`](../bench/PRIOR-ART.md). (Optional, deferred: run script-bench-rs unmodified at a
pinned SHA in a separate job for bonus comparison columns — no vendoring, no license issue. Nice-to-have.)

## Tracking pipeline — explicit dedicated Linux lifecycle
Split by what's measurable; never use one for the other's job:
- **Release gate = generated-history integrity and conservative claim checks.** Hosted CI validates JSONL,
  generated markdown/JSON/HTML, wasm gate artifacts, and the claim policy without taking fresh noisy timings.
- **Headline numbers = harness wall clock**, with fresh prepared state, untimed warmup, checksum validation, and
  min/median summaries on a dedicated ephemeral Linode. Claims remain per target/workload.
- **History/tracking = checked benchmark artifacts in this repository.** An explicit operator-started lifecycle
  provisions the Linux host, validates the complete matrix, confirms deletion, and only then promotes results.
- **Dashboard = generated `bench/RESULTS.md`, current-results JSON/HTML, and claim markdown**, all derived from
  the same history.
- *Graduate-to later:* add instruction-count regression alerts or a queryable dashboard if the
  wall-clock history proves insufficient; do not make either a prerequisite for honest current measurements.

## The matrix
- **engine:** `rustia` · `boa` · `koto` · `luaur` · `mlua-luau` · `mlua-lua54` · `rhai` · `roto` · `ruau` ·
  `rquickjs` · `wasmi` · `wasmtime`; feature gates keep adapters independently buildable and Lua 5.4 separate
  from Luau.
- **active performance targets:** native `linux-x86_64` on a dedicated Linode; wasm32 under Node/V8 on that
  same Linux host. The accepted Deno/V8, Bun/JavaScriptCore, and SpiderMonkey matrix is historical portability
  evidence. The broader platform support matrix in
  `00-constraints.md` remains a correctness/build concern, not a license to mix noisy macOS or Windows timings
  into the active benchmark product.
- **determinism:** a **golden-hash column** (bit-identical cross-arch/engine?) + rustia's `mode`
  (LuauCompat/ReproducibleStandard/DeterministicSim). A pass/fail **correctness gate**, NOT a perf trend — a
  hash mismatch FAILS CI; never tracked as a number.
- **workload:** 12 source-covered entries, with five default measured families (`dense_tables`, `sparse_tables`,
  `closures_upvalues`, `generic_iteration`, `stdlib_math_table`) and opt-in additions including
  `string_building`.

**Canonical record** (one JSON line/measurement):
`{timestamp_unix, commit, target, engine, dialect, workload, unit, n, reps, ns_per_unit_min,
ns_per_unit_median, units_per_sec, checksum}`.

The stable JSONL record deliberately remains a min/median summary. An optional
`rustia.bench.raw-samples.v1` JSON sidecar stores execution-ordered per-repetition `ns/unit` values plus the
commit, target, workload, selected `n`, `n_selection` (`calibrated` or `fixed`), checksum, and summaries for
local analysis. Calibration probes and
recorded repetitions each create fresh prepared engine/workload state; a single untimed warmup immediately
precedes the one timed invocation. Baseline/candidate comparisons should retain separate sidecars and alternate
run order (ABBA/BAAB). `--fixed-n` bypasses calibration for exact equal-work comparisons. Sidecars are evidence
for analysis, not a new public-history schema. The dependency-free
`analyze-fixed-samples.js` validates fixed-work identity and summaries, then
emits deterministic pooled and leg-clustered hierarchical bootstrap JSON with
a conservative-envelope verdict that is retained with accepted evidence.

## Automation structure

1. Hosted pull-request/push CI validates code, history, generated artifacts, and claims without measuring.
2. An explicit `just` command plans and starts the billed Linode lifecycle; CI never starts it automatically.
3. The remote runner builds the feature-gated comparison matrix, including the separate Lua 5.4 binary, then
   measures the recurring wasm adapters and dispatch check under Node/V8 on the same Linux placement.
4. Promotion requires a complete single-revision Linux matrix and confirmed Linode deletion.

**Current implementation slice:** `.github/workflows/ci.yml` runs release-preflight checks on hosted Linux CI
without fresh benchmark measurements; no benchmark-measurement workflow is present. The `bench` crate now enumerates all 12
requested adapters, five default measured workloads, and 19 workload entries including cross-adapter source
coverage for #15 `string_building`, portable memory probes #16–#18, and deterministic todo-app probes #19–#21.
It also owns `bench/wasm-gate.json`, a deterministic pre-lock gate tracker
exposed by `script-bench --wasm-gate-json` and rendered by the generated HTML report. The wasm target bench builds
`bench/wasm/adapter-runner` and records `rustia`, `koto`, and `rhai` under Node/V8 in
`bench/wasm/current-results.json`; the accepted four-runtime matrix is retained under `bench/wasm/history/`, and
adapter exclusions are documented in `bench/wasm/ADAPTERS.md`. The recurring dispatch gate records `call_ref`
versus `br_table` under Node/V8 in `bench/wasm/dispatch-current-results.json`; the completed cross-engine result
is retained beside the historical matrix.
The native harness now offers `--samples-json` for raw sidecars and isolates every calibration probe and timed
repetition in freshly prepared state. It also offers `--fixed-n` for exact equal-work A/B runs. The earlier
table/cache slice retained a cooled 1.29% dense-table signal; six later diagnostic legs overlapped around +0.90%.
The accepted exact-commit Linode v2 comparison then measured 2.42% lower corrective-candidate ns/unit on pooled
`dense_tables` samples, with a conservative nominal 95% interval of -3.35% to -1.57%, excluding the predeclared
+5% material regression. Qualification spread was 0.50%, all contaminant gates passed, peak timed steal was
0.074% under the 0.1% ceiling, and three actual final-binary rows plus raw evidence are now in append-only
history. This closes the dense material-regression gate but is workload/host-specific, not a blanket public
speed claim. A second exact Linode comparison measured 65.959% lower complete-candidate ns/unit on
`oop_metamethods`, with a conservative nominal 95% interval of -66.39% to -65.55% against a zero-regression
threshold, and retained three more final-candidate history rows. That candidate includes both the landed
constant-field/method paths and a weak-table GC-scan correction; the delta is not attributed to one opcode family.
The separate evidence-only 512-VM protocol retained 16 accepted paired rounds and 160 fresh-process samples. Its
primary candidate-minus-baseline contrasts saved 32.01, 258.00, 105.39, and 260.97 KiB/VM across unloaded-short,
unloaded-long, loaded-short, and loaded-long string effects, with a 7.08 KiB/VM loaded-long residual. It never
modifies throughput history or public claims. `tools/check-workflows.js` keeps hosted workflows validation-only
and forbids fresh benchmark commands there.

The exact dense follow-up has provider-specific but protocol-preserving ephemeral runners, while
`bench/cloud/linode-oop-f208c12.json` and `bench/cloud/linode-vm-string-memory-1370d6c.json` freeze the accepted
OOP-throughput and evidence-only memory protocols. Qualification-only `just bench-do-trials` compared regular
and Premium Intel 2-vCPU/4-GiB shared shapes without running comparison or
publication. Repeated nonzero-steal timed legs rejected that shared tier;
dedicated Akamai/Linode placements also remained KVM guests and eventually
violated the frozen v1 zero-steal gate. Those blocks remain discarded. The
prospectively versioned Linode v2 fixed-throughput protocol keeps every pre-leg idle window at zero steal and
permits no more than 0.1% steal during a timed leg, while preserving the fixed work, 3% qualification spread,
ABBA/BAAB ordering, contaminant exclusion, conservative bootstrap verdict, and publication rule. The v2 memory
config instead freezes its own warmup, paired-round, evidence-only analysis, and unchanged-history rules.
The accepted Sydney dense and OOP runs satisfied their prospectively declared gates and published their three
final candidate rows and evidence; the memory run satisfied its separate evidence-only gate and left history
unchanged.
`just bench-vultr` also validates an
Ubuntu 24.04 `vbm-4c-32gb` actual-bare-metal server in Singapore, its one-hour
`$0.164` minimum, and its two-hour `$0.328` lifecycle ceiling; transfers a local
Git bundle; builds the frozen
baseline/candidate with the documented candidate-harness overlay, rejects
virtualization, qualifies and CPU-pins the Linux host, runs the exact eight
legs, and conditionally publishes the three final rows. It records resource
ownership for idempotent cleanup and confirms server deletion before evidence
promotion. The physical-host source, qualification, cooldown, zero-steal,
analysis, verdict, and publication protocol is unchanged. Live cloud work remains outside release
preflight; only dependency-free coordinator/runner self-tests are part of the
offline benchmark artifact gate.

The current coherent 12-adapter snapshot is exact commit `a562919`. Its prototype-polymorphic closure-call
cache and direct immutable captures moved rustia `closures_upvalues` median latency from 270.427 to
229.766 ns/call across separate, identically specified dedicated-Linux instances (15.036% lower). The generated
five-workload minimum-time gaps are 3.494x to `mlua-luau` and 3.833x to Luaur. Non-target rustia movements in
that cross-host comparison are stability signals, not causal regression claims; current generated artifacts and
the retained raw evidence remain authoritative.

The three-independent-run fixed-N rule in `bench/README.md` is the acceptance protocol for this small-regression
milestone. The broader calibrated language matrix has a separate completeness gate and is not a substitute for
the controlled milestone A/B and three-run publication evidence.

**Rustia's column:** the `bench` crate enumerates engines by **cargo feature**; `rustia` now runs through the
default `rustia-engine` feature while the external adapters stay independently gated for standalone builds.

## Closed wasm pre-lock gate; recurring evidence remains
Production rustia wasm now runs through the bench wasm adapter runner, with koto and rhai as wasm-compatible
comparison adapters. The recurring public benchmark uses Node/V8. Accepted V8, Bun/JavaScriptCore-family, and
SpiderMonkey measurements show `call_ref` behind `br_table` and remain archived as portability evidence. The
pre-lock wasm gate is closed; keep the Node/V8 measurements fresh as the harness changes.

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
