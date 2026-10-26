# Benchmark and Repository Tools

## Overview

This directory contains internal Node and shell tools for benchmark validation, owned-host coordination, release checks, package metadata, and publication. They are repository tooling, not a public Rustia API.

## Interface

- [`run-dedicated-benchmark.js`](run-dedicated-benchmark.js) coordinates owned-host history, per-commit, PR screening, A/B acceptance, live-Serde, calibration, matrix, and memory work. It obtains its SSH destination from an environment variable; the published host label is deliberately not the connection target.
- `just check-benchmark-results` runs offline self-tests for benchmark analyzers, runners, and renderers plus committed-artifact validators ([Justfile](../Justfile)). `just release-preflight` invokes the full staged release preflight inventory; `just check-quick` runs its early checks.
- [`pre-push.js`](pre-push.js) runs the full inventory against a clean, unchanged checked-out commit. [`install-git-hooks.js`](install-git-hooks.js) installs the checkout-local guard while preserving existing hooks; [`check-pre-push.js`](check-pre-push.js) and [`check-preflight.js`](check-preflight.js) validate these contracts.
- The package audit is [`check-package-files.js`](check-package-files.js); release validation is in [`run-release-preflight.js`](run-release-preflight.js). Workflow and facade boundaries are checked by [`check-workflows.js`](check-workflows.js), [`check-facade-api.js`](check-facade-api.js), and [`check-external-facade.js`](check-external-facade.js).
- Generators and artifact renderers include [`render-benchmark-operations-status.js`](render-benchmark-operations-status.js), [`render-benchmark-data-readme.js`](render-benchmark-data-readme.js), and the config creators listed in this directory.
- Publication-related tools include [`land-matrix-publication.js`](land-matrix-publication.js), [`merge-rustia-ci-memory-data.js`](merge-rustia-ci-memory-data.js), and [`push-benchmark-data-publication.js`](push-benchmark-data-publication.js). They form an operational boundary and are not verification commands.
- The Lua 5.1.5 oracle build scripts are in [`puc51-oracle/EGM.md`](puc51-oracle/EGM.md).
- [`browser/EGM.md`](browser/EGM.md) describes the pinned headless WebKit runner. [`run-browser-correctness.sh`](run-browser-correctness.sh) coordinates browser correctness and [`run-private-platform-golden.sh`](run-private-platform-golden.sh) runs explicit-target shared runtime contracts.
- The dedicated coordinator's `strings` mode measures the top-level string-opcode workload with frozen source identity and checksums. Its `profile` mode collects diagnostic counters and ABBA generic-iteration cycle locations. Raw perf recordings remain outside exported evidence; only allowlisted Rustia symbol-relative location counts and raw hashes are exported. Sampling includes startup/warmup and may skid; neither changes normal acceptance thresholds or publishes history rows.
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
