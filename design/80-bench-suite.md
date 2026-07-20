# rustia — continuous perf-benchmark suite + CI (design)

> **Status: [IMPLEMENTED — measurements ongoing] (updated 2026-07-18).** Scoped by the `bench-scoping` agent (clone+read of
> `khvzak/script-bench-rs`, the 11-workload seed in `spikes/bench-suite/`, and the 2025–26 tracker/wasm-shell
> tooling) + self-hosted benchmark-label constraints + privacy lean. Operationalises principle #3 ("measure,
> don't assert — the bench suite is a CI gate"). See [`CURRENT.md`](CURRENT.md).

## Decision 1 — REWRITE, not fork (LOCKED, gak + license evidence)
`khvzak/script-bench-rs` has **no LICENSE / no `license` field** → all-rights-reserved → vendoring into a
public-bound repo is unsafe. It's also a single Rust↔script *interop* workload; our seed already has 11
in-VM-throughput workloads. **Clean rewrite**, keep comparable workload coverage, and record provenance in
[`../bench/PRIOR-ART.md`](../bench/PRIOR-ART.md). (Optional, deferred: run script-bench-rs unmodified at a
pinned SHA in a separate job for bonus comparison columns — no vendoring, no license issue. Nice-to-have.)

## Tracking pipeline — FULLY SELF-HOSTED, no SaaS (generic runner label + privacy)
Split by what's measurable; never use one for the other's job:
- **Release gate = generated-history integrity and conservative claim checks.** Hosted CI validates JSONL,
  generated markdown/JSON/HTML, wasm gate artifacts, and the claim policy without taking fresh noisy timings.
- **Headline numbers = harness wall clock**, with fresh prepared state, untimed warmup, checksum validation, and
  min/median summaries on `[self-hosted, bench]`. Claims remain per target/workload.
- **History/tracking = append-only harness JSONL in the private `rustia-bench-data` repository.** Push and
  nightly jobs restore it, append real measurements, regenerate artifacts, and commit them back with no SaaS
  benchmark service.
- **Dashboard = generated `bench/RESULTS.md`, current-results JSON/HTML, and claim markdown**, all derived from
  the same history.
- *Graduate-to later:* add instruction-count regression alerts or a self-hosted queryable dashboard if the
  wall-clock history proves insufficient; do not make either a prerequisite for honest current measurements.

## The matrix
- **engine:** `rustia` · `boa` · `koto` · `mlua-luau` · `mlua-lua54` · `rhai` · `roto` · `rquickjs` ·
  `wasmi` · `wasmtime`; feature gates keep adapters independently buildable and Lua 5.4 separate from Luau.
- **target × arch:** native `{aarch64-mac, x86_64-linux, x86_64-mac(Intel), aarch64-linux, x86_64-windows}` ·
  wasm32 `{V8 (node/deno/d8), SpiderMonkey (jsvu sm), JSC (jsc/bun)}` — mirrors `00-constraints.md` §C0.
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

## CI structure (generic self-hosted label `[self-hosted, bench]`; no infrastructure detail in the repo)

1. Hosted pull-request/push CI validates code, history, generated artifacts, and claims without measuring.
2. Main pushes and manual `rustia` dispatch run the Rustia native matrix plus wasm target/dispatch gates.
3. Nightly schedule and manual `full` dispatch run the feature-gated comparison matrix, including the separate
   Lua 5.4 binary, plus the wasm gates.
4. Both measurement jobs restore the private history, regenerate/check all outputs, and push the updated files
   to `rustia-bench-data`.

**Current implementation slice:** `.github/workflows/ci.yml` runs release-preflight checks on hosted Linux CI
without fresh benchmark measurements. `.github/workflows/benchmarks.yml` runs on main pushes, a nightly schedule,
or manual `rustia`/`full` dispatch on `[self-hosted, bench]`; it restores history from the private benchmark-data
repository, appends current measurements, regenerates and verifies the artifacts, then commits them back to that
repository. The `bench` crate now enumerates all 10
requested adapters, five default measured workloads, and 12 workload entries including cross-adapter source
coverage for #15 `string_building`. It also owns `bench/wasm-gate.json`, a deterministic pre-lock gate tracker
exposed by `script-bench --wasm-gate-json` and rendered by the static dashboard. The wasm target bench now builds
`bench/wasm/adapter-runner` and records `rustia` plus `koto` on Node/V8, Deno/V8, and Bun/JavaScriptCore-family
runtimes in `bench/wasm/current-results.json`; adapter exclusions are documented in `bench/wasm/ADAPTERS.md`.
The dispatch gate now records `call_ref` versus `br_table` on V8, JavaScriptCore-family, and SpiderMonkey
runtimes in `bench/wasm/dispatch-current-results.json`, closing the pre-lock wasm gate.
The native harness now offers `--samples-json` for raw sidecars and isolates every calibration probe and timed
repetition in freshly prepared state. It also offers `--fixed-n` for exact equal-work A/B runs. The earlier
table/cache slice retained a cooled 1.29% dense-table signal; six later diagnostic legs overlapped around +0.90%.
The accepted exact-commit Linode v2 comparison then measured the corrective candidate 2.42% faster on pooled
`dense_tables` samples, with a conservative nominal 95% interval of -3.35% to -1.57%, excluding the predeclared
+5% material regression. Qualification spread was 0.50%, all contaminant gates passed, peak timed steal was
0.074% under the 0.1% ceiling, and three actual final-binary rows plus raw evidence are now in append-only
history. This closes the dense material-regression gate but is workload/host-specific, not a blanket public
speed claim. `tools/check-workflows.js` guards hosted/self-hosted separation, triggers, exact runner labels, data-repo
publication, and generated-artifact policy.

The exact dense follow-up now has provider-specific but protocol-preserving
ephemeral runners. Qualification-only `just bench-do-trials` compared regular
and Premium Intel 2-vCPU/4-GiB shared shapes without running comparison or
publication. Repeated nonzero-steal timed legs rejected that shared tier;
dedicated Akamai/Linode placements also remained KVM guests and eventually
violated the frozen v1 zero-steal gate. Those blocks remain discarded. The
prospectively versioned Linode v2 protocol keeps every pre-leg idle window at
zero steal and permits no more than 0.1% steal during a timed leg, while
preserving the fixed work, 3% qualification spread, ABBA/BAAB ordering,
contaminant exclusion, conservative bootstrap verdict, and publication rule.
The accepted Sydney run satisfied those gates, independently excluded a +5%
material regression, and published its three final candidate rows and evidence.
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

The three-independent-run fixed-N rule in `bench/README.md` is the acceptance protocol for this small-regression
milestone. Routine main-push and nightly automation continues to append one calibrated matrix run per workflow;
it is trend history, not a substitute for the controlled milestone A/B and three-run publication evidence.

**Rustia's column:** the `bench` crate enumerates engines by **cargo feature**; `rustia` now runs through the
default `rustia-engine` feature while the external adapters stay independently gated for standalone builds.

## ⚠️ Remaining wasm gate
Production rustia wasm now runs through the bench wasm adapter runner, and koto is the current wasm-compatible
comparison adapter. V8, Bun/JavaScriptCore-family, and SpiderMonkey measurements exist, and the dispatch
microbench shows `call_ref` behind `br_table` on those same families. The pre-lock wasm gate is closed; keep
these measurements fresh as the harness changes.

## Where it lives
```
bench/  Cargo.toml(feature-gated adapters)  src/main.rs  workloads/<dialect>/
        history/runs.jsonl  history/evidence/  RESULTS.md  current-results.{json,html}
        CLAIMS.md  wasm/  site/  PRIOR-ART.md
```

## Open decisions (gak)
- **D1 — regression alerts:** decide whether longer history justifies an instruction-count gate or self-hosted
  dashboard; keep SaaS egress opt-in.
- **D2 — real Safari/iOS JSC:** Bun supplies the current JavaScriptCore-family signal; a shipping Safari/iOS
  runtime remains desirable when an operational runner is available.
- **D3 — optional external comparison:** running script-bench-rs at a pinned SHA remains deferred and
  nonessential.
