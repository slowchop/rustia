# rustia — continuous perf-benchmark suite + CI (design)

> **Status: [DESIGN — implementation started] (2026-07-03).** Scoped by the `bench-scoping` agent (clone+read of
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
- **Regression GATE (PR blocker) = `iai-callgrind`** — Valgrind/callgrind **instruction counts**, run **locally
  on `[self-hosted, bench]`**. <1% variance (deterministic), no third-party egress, and on-brand: rustia's own
  determinism meter is instruction-fuel — "count, not clock." Catches algorithmic regressions independent of
  HW noise. (This is the local equivalent of CodSpeed; chosen over CodSpeed to avoid SaaS egress.)
- **Headline "bar" numbers = criterion wall-clock, min-of-N**, native + each wasm engine, on `[self-hosted,
  bench]`. Environment-specific tuning and toolchain details stay private.
  Carries the "≈ reference-Luau parity / 5–9× koto" claims (which must be wall-clock).
- **History/tracking = `github-action-benchmark` with `external-data-json-path`** — MIT, commits history JSON
  into the repo (a `bench-data` branch), regression alerts (`fail-on-alert`/`comment-on-alert`), no account, you
  own all data. (NOT the gh-Pages mode — that needs paid GitHub for private repos.)
- **Dashboard = generated, committed `bench/RESULTS.md`** matrix table — diffable in PRs, zero infra.
- *Graduate-to later:* self-hosted **Bencher** if a queryable testbed-matrix dashboard is wanted (Bencher Cloud
  has no free private tier; self-host is free). CodSpeed only if a hosted dashboard is ever desired (SaaS egress).

## The matrix
- **engine:** `rustia` *(reserved — pending P1)* · `koto 0.16.1` · `mlua+Luau 0.709` *(the oracle = the bar)* ·
  `mlua+Lua 5.4` · `rhai` · `rune` · *(optional yardsticks: piccolo, rquickjs/boa)*.
- **target × arch:** native `{aarch64-mac, x86_64-linux, x86_64-mac(Intel), aarch64-linux, x86_64-windows}` ·
  wasm32 `{V8 (node/deno/d8), SpiderMonkey (jsvu sm), JSC (jsc/bun)}` — mirrors `00-constraints.md` §C0.
- **determinism:** a **golden-hash column** (bit-identical cross-arch/engine?) + rustia's `mode`
  (LuauCompat/ReproducibleStandard/DeterministicSim). A pass/fail **correctness gate**, NOT a perf trend — a
  hash mismatch FAILS CI; never tracked as a number.
- **workload:** the 11 seed workloads + additions **#12** userdata-sort interop (the script-bench-rs port,
  credited) · **#13** serde round-trip (exercises C4) · **#14** fuel/limit overhead (rustia-specific) · **#15**
  string building/concat. `#05 string_patterns` + `#09 generic_iteration` + `#08 pcall` stay **must-match** per CURRENT.md.

**Canonical record** (one JSON line/measurement; Bencher-BMF-compatible for later):
`{engine, engine_version, target, arch, os, mode, workload, unit, ns_per_unit_min, ns_per_unit_median,
units_per_sec, n, golden_hash, commit, date, runner}`.

## CI structure (GitHub Actions, self-hosted benchmark runner via a GENERIC label `[self-hosted, bench]` — no infra
detail in the repo)
1. **`bench-native`** — build the `bench` crate per engine feature; `iai-callgrind` instruction-count = the
   **gate**; also emit criterion wall-clock JSON.
2. **`bench-wasm`** *(P1-gated — see below)* — `rustia→wasm32` under V8 (node/deno) + SpiderMonkey (jsvu `sm`)
   + JSC (`bun`/`jsc`), via an `eshost`/`jsvu` harness reusing `spikes/spike-wasmjit/harness.mjs` (warmup +
   min-of-N + checksum).
3. **`golden-determinism`** — run each workload across native arches × wasm engines, hash output, **FAIL on
   mismatch** (this is the still-open H3 cross-engine gate, in bench form).
4. **`track`** — collect JSON → `github-action-benchmark` `external-data-json-path` → regenerate `RESULTS.md`.
   Triggers: PR = instruction-count gate + posts the table; push-main = record history; nightly cron = full
   matrix incl. the expensive macOS/JSC + Intel-Mac + Windows + arm-Linux arches.

**Current implementation slice:** `.github/workflows/ci.yml` runs release-preflight checks on hosted Linux CI
without fresh benchmark measurements. `.github/workflows/benchmarks.yml` is a manual `workflow_dispatch`
measurement job on `[self-hosted, bench]`; it appends current benchmark history in the checkout, regenerates the
benchmark artifacts, verifies them, and uploads the resulting files. It does not yet publish the JSONL history
back to a tracking branch or static host.

**Reserving rustia's column:** the `bench` crate enumerates engines by **cargo feature**; `rustia` is
`cfg`-gated to a no-op stub emitting `— (pending P1)`. Baseline engines tracked **now**; flip the feature at P1
→ column populates with zero CI/schema change.

## ⚠️ P1-gating (important): the WASM matrix has nothing rustia to run yet
mlua+Luau is native C; only koto compiles to wasm. So the V8/SpiderMonkey/**JSC** matrix + the JSC-in-CI
question are **deferred to P1** (when rustia has a wasm build). **Build NOW:** the `bench/` crate + the native
baseline benches (koto, mlua+Luau, rhai, rune) + the golden hasher + the `RESULTS.md` generator + the tracking
pipeline. Reserve the wasm/JSC columns like rustia's.

## Where it lives
```
bench/  Cargo.toml(features: rustia stub→real, koto, mlua-luau, mlua-lua54, rhai, rune)  src/(runner+golden hasher)
        benches/(criterion + iai-callgrind per engine)  workloads/{luau,koto,rhai,rune}/  wasm/harness.mjs
        RESULTS.md(generated)  data/history.json(committed)  PRIOR-ART.md(credits khvzak)
```

## Open decisions (gak)
- **D1 — fully-self-hosted (recommended) vs add CodSpeed.** Recommend the iai-callgrind + criterion +
  github-action-benchmark pipeline, entirely self-hosted, no SaaS. CodSpeed only if a hosted dashboard is ever
  wanted (SaaS egress). **Lean: fully self-hosted.**
- **D2 — JSC-in-CI approach: DEFERRED to P1** (nothing rustia to bench in wasm yet). At P1: canonical macOS
  `jsc` nightly + `bun`-JSC-on-Linux per-PR (cheap signal; flag it's "a JSC," not Safari's shipping one).
- **D3 — the optional external-comparison job (run script-bench-rs at a pinned SHA): defer** (nice-to-have).
