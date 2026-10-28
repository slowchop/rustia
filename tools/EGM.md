# Benchmark and Repository Tools

## Overview

This directory contains internal Node and shell tools for benchmark validation, owned-host coordination, release checks, package metadata, and publication. They are repository tooling, not a public Rustia API.

## Interface

- [`benchmark-disk-space.js`](benchmark-disk-space.js) shares the 20 GiB and 20% available-filesystem reserve between coordinator admission and all three post-build runner checks. Numeric post-build snapshots, including refusals, are saved before quiet windows; no device names or paths are exported. The coordinator explicitly ships the helper with each runner and tests that inbox dependency list. This is admission and a post-build check, not continuous disk monitoring.
- [`run-dedicated-benchmark.js`](run-dedicated-benchmark.js) coordinates owned-host history, per-commit, PR screening, A/B acceptance, live-Serde, calibration, matrix, and memory work. It obtains its SSH destination from an environment variable; the published host label is deliberately not the connection target.
- `just check-benchmark-results` runs offline self-tests for benchmark analyzers, runners, and renderers plus committed-artifact validators ([Justfile](../Justfile)). `just release-preflight` invokes the full staged release preflight inventory; `just check-quick` runs its early checks.
- [`pre-push.js`](pre-push.js) runs the full inventory against a clean, unchanged checked-out commit. [`install-git-hooks.js`](install-git-hooks.js) installs the checkout-local guard while preserving existing hooks; [`check-pre-push.js`](check-pre-push.js) and [`check-preflight.js`](check-preflight.js) validate these contracts.
- The package audit is [`check-package-files.js`](check-package-files.js); release validation is in [`run-release-preflight.js`](run-release-preflight.js). Workflow and facade boundaries are checked by [`check-workflows.js`](check-workflows.js), [`check-facade-api.js`](check-facade-api.js), and [`check-external-facade.js`](check-external-facade.js).
- [`history-projection.js`](history-projection.js) validates the complete cleaned-history map and protected runtime-input fingerprints, resolves legacy source IDs without rewriting measurements, and checks projected metadata hashes. Its tests cover missing/duplicate mappings, unreachable commits and altered fingerprints. [`check-benchmark-commit-reachability.js`](check-benchmark-commit-reachability.js) refuses absent IDs and runs in the full preflight; its synthetic orphan-commit self-test supplies and verifies its own Git identity so hosted runners need no global identity configuration.
- [`check-public-workflow-profile.js`](check-public-workflow-profile.js) enforces the public-source hosted correctness/fuzz workflow profile, including exact `$GITHUB_WORKSPACE`-only Git safe-directory setup before the containerized WebKit tests; wildcard trust is rejected. [`check-workflows.js`](check-workflows.js) routes to it only when the explicit profile marker exists and retains the private workflow checks otherwise.
- [`render-benchmark-operations-status.js`](render-benchmark-operations-status.js) renders either the unchanged private v2 operations policy or the separate paused public-source policy into `bench/operations-status.json`. The public profile has no timing authority or publication lane.
- Other generators and artifact renderers include [`render-benchmark-data-readme.js`](render-benchmark-data-readme.js) and the config creators listed in this directory.
- Publication-related tools include [`land-matrix-publication.js`](land-matrix-publication.js), [`merge-rustia-ci-memory-data.js`](merge-rustia-ci-memory-data.js), and [`push-benchmark-data-publication.js`](push-benchmark-data-publication.js). They form an operational boundary and are not verification commands.
- [`bench-host-control.js`](bench-host-control.js) records and validates same-source host controls, retains prior warning counts in status, and exports ledger validation for the series checker and report renderer. [`check-benchmark-series.js`](check-benchmark-series.js) refuses fingerprinted rows whose control ledger is missing. The main series workflow publishes its graph through the existing reusable publisher only after a confirmed data push, including retained findings from a failed drift check.
- The Lua 5.1.5 oracle build scripts are in [`puc51-oracle/EGM.md`](puc51-oracle/EGM.md).
- [`browser/EGM.md`](browser/EGM.md) describes the pinned headless WebKit runner. [`run-browser-correctness.sh`](run-browser-correctness.sh) coordinates browser correctness and [`run-private-platform-golden.sh`](run-private-platform-golden.sh) runs explicit-target shared runtime contracts.
- The dedicated coordinator's `strings` mode measures the top-level string-opcode workload with frozen source identity and checksums. Its `profile` mode collects diagnostic counters and ABBA generic-iteration cycle locations. Raw perf recordings remain outside exported evidence; only allowlisted Rustia symbol-relative counts, adaptive period sums and raw hashes are exported. Frequency-mode sampling keeps kernel accounting ticks active during the diagnostic. Sampling includes startup/warmup and may skid; neither changes normal acceptance thresholds or publishes history rows.
- The acceptance workflow retains completed per-pass exports and summaries for all modes, including failures, as private CI artifacts for 30 days. These survive runner scratch cleanup; an identical-binary summary skip does not replace raw-sample review.
- [`observe-benchmark-process.py`](observe-benchmark-process.py) is the CI-only unprivileged observer used by the separate `observe` mode. It records child CPU time, faults, context switches, interrupt totals, requested clock-state residency and sanitized code/heap/stack mapping bases. It exports no paths, process IDs or device labels. The observe runner crosses fresh copies and a shared inode in sixteen balanced source legs, then independently replays selected retained files in eight legs. Launch paths are stable and ASLR policy remains unchanged; anonymous file tokens and all diagnostic artifacts survive in private CI exports, including failures. Normal acceptance and history are unchanged. Offline parser/resource/lifecycle tests run through the coordinator self-test.
- Other direct files include [`check-workflows.js`](check-workflows.js), [`check-benchmark-results.js`](check-benchmark-results.js), [`run-linux-matrix-benchmark.js`](run-linux-matrix-benchmark.js), [`run-linux-memory-benchmark.js`](run-linux-memory-benchmark.js), and [`run-linode-benchmark.js`](run-linode-benchmark.js). The operations design describes the last as retired apart from offline self-test use; the legacy name alone does not establish a provisioner.

## Invariants

- `ci-bench` alone is authorized to produce timing evidence. Local builds, correctness checks, and artifact validation are not performance evidence ([AGENTS.md](../AGENTS.md#benchmark-policy), [CURRENT.md](../design/CURRENT.md)).
- The checked-in design says the repository does not create or pay for cloud instances, and describes `run-linode-benchmark.js` as retired except for offline self-test use ([operations design](../design/81-benchmark-operations.md#cost)).
- Do not add hostnames, addresses, credentials, or other private infrastructure details to public tool documentation ([AGENTS.md](../AGENTS.md#where-things-live-public-vs-private-boundary)).
- Preflight refuses evidence-regeneration overrides. Quick, workspace, and release stages preserve the full command inventory; one stage cannot substitute for the pre-push gate ([validation contract](../design/97-validation-workflow.md)).

## Side Effects

- `check-package-files.js` reads Cargo's package file list and writes `design/package-files.json` unless invoked with `--check`.
- Renderers and generators read checked-in inputs and write generated artifacts. Publication helpers can update benchmark-data outputs; the dedicated coordinator transfers, runs, and collects an owned-host lane.
- `run-linode-benchmark.js` remains a legacy-named offline rules/self-test tool per the operations design; the name does not imply an available provisioner.

## Verification

Run from the repository root: `node --check tools/check-workflows.js` performs syntax validation; `just check-benchmark-results` runs the offline benchmark-tool checks and committed-artifact validation. Neither is a timing result.

## Spec Alignment

[The benchmark design](../design/80-bench-suite.md), [CURRENT](../design/CURRENT.md), and [operations contract](../design/81-benchmark-operations.md) describe the owned-host lifecycle implemented by `run-dedicated-benchmark.js`. Earlier cloud protocols are historical evidence; provisioning, billing ceilings, and deletion gates are retired.
