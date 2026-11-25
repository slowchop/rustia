# Benchmark and Repository Tools

## Overview

This directory contains internal Node and shell tools for benchmark validation, owned-host coordination, release checks, package metadata, and publication. They are repository tooling, not a public Rustia API.

## Interface

- [Project status validation](check-project-status.js) checks the pinned lists of 235 parser, 28 lexer and 200 compiler tests and resolves each applicable entry to a Rust test.

- [`check-public-source-candidate.js`](check-public-source-candidate.js) checks
  hosted workflow parity and negative fixtures for the selected source profile.
  CI selection is independent of visibility and benchmark provenance. See the
  [hosted workflow inventory](../.github/public-ci/README.md).

- [`benchmark-disk-space.js`](benchmark-disk-space.js) shares the 20 GiB and 20% available-filesystem reserve between coordinator admission and all three post-build runner checks. Numeric post-build snapshots, including refusals, are saved before quiet windows; no device names or paths are exported. The coordinator explicitly ships the helper with each runner and tests that inbox dependency list. This is admission and a post-build check, not continuous disk monitoring.
- [`run-dedicated-benchmark.js`](run-dedicated-benchmark.js) coordinates owned-host history, per-commit, PR screening, A/B acceptance, live-Serde, calibration, matrix, and memory work. It obtains its SSH destination from an environment variable; the published host label is deliberately not the connection target.
- `just check-benchmark-results` runs offline self-tests for benchmark analyzers, runners, and renderers plus committed-artifact validators ([Justfile](../Justfile)). `just release-preflight` invokes the full staged release preflight inventory; `just check-quick` runs its early checks.
- [`pre-push.js`](pre-push.js) runs the full inventory against a clean, unchanged checked-out commit. [`validated-push.js`](validated-push.js) provides the preferred one-shot path: it runs that same full inventory, rechecks exact HEAD and clean state, then pushes only that commit while skipping the duplicate hook invocation. It creates no reusable receipt. [`install-git-hooks.js`](install-git-hooks.js) installs the checkout-local guard while preserving existing hooks; [`check-pre-push.js`](check-pre-push.js), [`check-validated-push.js`](check-validated-push.js), and [`check-preflight.js`](check-preflight.js) validate these contracts.
- [`check-readme-comparisons.js`](check-readme-comparisons.js) checks the dated measurement table and [SVG heatmap](../crates/rustia/performance.svg) against the same retained cohort and verifies the copied Git pin plus immutable/current source-link targets against reachable commits, without taking timings. Original-history aliases do not count as fetchable revisions. [Chart rendering](render-readme-performance.js) checks each engine’s execution mode and native evidence. Use `node tools/check-readme-comparisons.js --write-chart` to regenerate the SVG offline. Commit the asset, then pin that revision in the README image link. The check compares the linked bytes with the generated chart. [sync-luau-comparison.js](sync-luau-comparison.js) checks the shared feature comparison in the browser demo. Its `--write` option refreshes the page from [compare.md](../crates/rustia/compare.md) and embeds the platform icons for offline Rustdoc and browser use. [`check-readme-examples.js`](check-readme-examples.js) builds the facade and compiles/runs the README Rust examples. [`check-external-facade.js`](check-external-facade.js) also checks the Cargo feature matrix: lower-level `rustia-vm/send` without forwarded `rustia/transferable` must fail, while the forwarded graph must compile.
- Memory config freezing detects committed v2 inputs and pins the shared protocol module in addition to sampler/analyzer/cohort/publisher source. The owned-host runner refuses fresh legacy execution; evidence validation retains historical v1 support and independently derives v2 warmup/control counts and hashes. Memory summaries and data merging separate measurement protocols as well as host identities.
- The package audit is [`check-package-files.js`](check-package-files.js); release validation is in [`run-release-preflight.js`](run-release-preflight.js). Workflow and facade boundaries are checked by [`check-workflows.js`](check-workflows.js), [`check-facade-api.js`](check-facade-api.js), and [`check-external-facade.js`](check-external-facade.js).
- Release preflight and `just check-examples` execute the introductory embedding, chunk-builder, mutable-object, and live-value examples, followed by game objects and the correctness-only [`hardened_decision` example](../crates/rustia/examples/hardened_decision.rs). [`check-preflight.js`](check-preflight.js) guards all six executions. Timing examples remain outside local execution.
- The [browser embedding recipe](../demos/web-live/README.md) has standalone formatting and Wasm Clippy checks in preflight. The hosted WebKit job builds its documented artifact and executes [`browser/web-live-example.cjs`](browser/web-live-example.cjs); public workflow fixtures reject missing build or execution commands.
- [`history-projection.js`](history-projection.js) validates the complete cleaned-history map and protected runtime-input fingerprints, resolves legacy source IDs without rewriting measurements, and checks projected metadata hashes. Its tests cover missing/duplicate mappings, unreachable commits and altered fingerprints. [`check-benchmark-commit-reachability.js`](check-benchmark-commit-reachability.js) refuses absent IDs and runs in the full preflight; its synthetic orphan-commit self-test supplies and verifies its own Git identity so CI runners need no global identity configuration.
- [`check-public-workflow-profile.js`](check-public-workflow-profile.js) enforces
  the source profile's correctness and differential-fuzz workflow inventory,
  standard hosted runners, read-only permissions, pinned dependencies and required
  execution blocks. It also locks the dispatch-only bulk trust fixture's four
  inputs, run title, hosted job inventory and checkout-verification order, with
  exact step inventories, protected checker provenance, token/permission guards
  and negative mutation fixtures. [`check-workflows.js`](check-workflows.js) selects it when
  the explicit source profile marker exists. The original private profile has
  separate runner and workflow checks.

- [`render-benchmark-operations-status.js`](render-benchmark-operations-status.js) renders either the unchanged private v2 operations policy or the separate public-source policy into `bench/operations-status.json`. Public timing authority, history automation, landscape refresh, and publication stay paused; the profile may record a private manual-cohort bridge without changing those permissions.
- Other generators and artifact renderers include [`render-benchmark-data-readme.js`](render-benchmark-data-readme.js) and the config creators listed in this directory.
- Publication-related tools include [`land-matrix-publication.js`](land-matrix-publication.js), [`merge-rustia-ci-memory-data.js`](merge-rustia-ci-memory-data.js), and [`push-benchmark-data-publication.js`](push-benchmark-data-publication.js). They form an operational boundary and are not verification commands.
- [`bench-host-control.js`](bench-host-control.js) records and validates same-source host controls, retains prior warning counts in status, and exports ledger validation for the series checker and report renderer. [`check-benchmark-series.js`](check-benchmark-series.js) refuses fingerprinted rows whose control ledger is missing. In the predecessor private pipeline, the main series workflow publishes its graph through the reusable publisher only after a confirmed data push, including retained findings from a failed drift check. This source-profile checkout contains no measurement or publication workflows.
- The Lua 5.1.5 oracle build scripts are in [`puc51-oracle/EGM.md`](puc51-oracle/EGM.md).
- [`browser/EGM.md`](browser/EGM.md) describes the pinned headless WebKit runner. [`run-browser-correctness.sh`](run-browser-correctness.sh) coordinates browser correctness, including type grammar/context/depth admission and no-execution regressions in both feature modes, and [`run-private-platform-golden.sh`](run-private-platform-golden.sh) runs explicit-target shared runtime contracts.
- The dedicated coordinator's `strings` mode measures the top-level string-opcode workload with frozen source identity and checksums. Its `profile` mode collects diagnostic counters and ABBA generic-iteration cycle locations. Raw perf recordings remain outside exported evidence; only allowlisted Rustia symbol-relative counts, adaptive period sums and raw hashes are exported. Frequency-mode sampling keeps kernel accounting ticks active during the diagnostic. Sampling includes startup/warmup and may skid; neither changes normal acceptance thresholds or publishes history rows.
- The predecessor private acceptance workflow retains completed per-pass exports and summaries for all modes, including failures, as private CI artifacts for 30 days. Those artifacts remain in the predecessor repository; this checkout's source profile has no acceptance workflow. An identical-binary summary skip does not replace raw-sample review.
- [`observe-benchmark-process.py`](observe-benchmark-process.py) is the CI-only unprivileged observer used by the separate `observe` mode. It records child CPU time, faults, context switches, interrupt totals, requested clock-state residency and sanitized code/heap/stack mapping bases. It exports no paths, process IDs or device labels. The observe runner crosses fresh copies and a shared inode in sixteen balanced source legs, then independently replays selected retained files in eight legs. Launch paths are stable and ASLR policy remains unchanged; anonymous file tokens and all diagnostic artifacts survive in private CI exports, including failures. Normal acceptance and history are unchanged. Offline parser/resource/lifecycle tests run through the coordinator self-test.
- Other direct files include [`check-workflows.js`](check-workflows.js), [`check-benchmark-results.js`](check-benchmark-results.js), [`run-linux-matrix-benchmark.js`](run-linux-matrix-benchmark.js), [`run-linux-memory-benchmark.js`](run-linux-memory-benchmark.js), and [`run-linode-benchmark.js`](run-linode-benchmark.js). The operations design describes the last as retired apart from offline self-test use; the legacy name alone does not establish a provisioner.

- [`package-ios-test.py`](package-ios-test.py) packages one native iOS test executable in a new scratch app using an existing eligible device/profile/signing identity; the reproducible mobile execution and evidence rules are in [platform acceptance](../design/96-browser-platform-acceptance.md#reproduce-native-mobile-contracts). It creates no signing credentials and prints no identifiers.

## Invariants

- `check-public-workflow-profile.js` also requires the fixed feature-gated executor
  accounting diagnostic in the hosted workspace stage; it is not a local profiling command.

- `ci-bench` alone is authorized to produce timing evidence. Local builds, correctness checks, and artifact validation are not performance evidence ([AGENTS.md](../AGENTS.md#benchmark-policy), [CURRENT.md](../design/CURRENT.md)).
- The checked-in design says the repository does not create or pay for cloud instances, and describes `run-linode-benchmark.js` as retired except for offline self-test use ([operations design](../design/81-benchmark-operations.md#cost)).
- Do not add hostnames, addresses, credentials, or other private infrastructure details to public tool documentation ([AGENTS.md](../AGENTS.md#where-things-live-public-vs-private-boundary)).
- Preflight refuses evidence-regeneration overrides. Quick, workspace, and release stages preserve the full command inventory; one stage cannot substitute for a validated push. Ordinary `git push` retains the full hook, while `just validated-push` executes the full gate and the exact push in one process ([validation contract](../design/97-validation-workflow.md)).

## Side Effects

- `check-package-files.js` reads Cargo's package file list and writes `design/package-files.json` unless invoked with `--check`.
- `validated-push.js` captures the commit SHA and remote push URL, runs the complete release preflight and, only if HEAD, destination and the clean checkout remain unchanged, pushes that SHA to the same-name branch at the captured URL. First-push tracking is configured only after success. Its optional force route uses only an exact observed `--force-with-lease`; it writes no validation receipt.
- Renderers and generators read checked-in inputs and write generated artifacts. Publication helpers can update benchmark-data outputs; the dedicated coordinator transfers, runs, and collects an owned-host lane.
- `run-linode-benchmark.js` remains a legacy-named offline rules/self-test tool per the operations design; the name does not imply an available provisioner.

- `package-ios-test.py` reads the supplied development profile, writes only a new scratch app and adjacent entitlements, and invokes existing `codesign` to sign/verify. Device execution is a separate explicit `devicectl` action.

## Verification

- [`sync-project-copy.js`](sync-project-copy.js) renders the shared intro and feature copy
  from [`project-copy.json`](../project-copy.json). Edit the JSON and run
  `node tools/sync-project-copy.js` to refresh the marked blocks. The crate metadata
  check used by release preflight runs `--check` and reports stale output.
  `--github-about` prints the short blurb for GitHub About.

Run from the repository root: `node --check tools/check-workflows.js` performs syntax validation; `just check-benchmark-results` runs the offline benchmark-tool checks and committed-artifact validation. Neither is a timing result.

## Spec Alignment

[The benchmark design](../design/80-bench-suite.md), [CURRENT](../design/CURRENT.md), and [operations contract](../design/81-benchmark-operations.md) describe the predecessor owned-host lifecycle implemented by `run-dedicated-benchmark.js`. The explicit source profile selects standard GitHub-hosted correctness CI and keeps public measurement automation and publication paused. Its private bridge permits reviewed manual exact-cohort comparisons only; results remain private and do not update public history or authorize claims. Earlier cloud protocols are historical evidence; provisioning, billing ceilings, and deletion gates are retired.
