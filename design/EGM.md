# design

## Overview

Design specifications, canonical project state, structured evidence, and a generated HTML progress view. Code behavior remains authoritative for what is built; CURRENT.md is the canonical status document.

## Interface

- Canonical state: [CURRENT.md](./CURRENT.md), kept to current contracts, permissions and one open-work list.
- Historical pre-compaction text: [CURRENT-2026-10-30-snapshot.md](./CURRENT-2026-10-30-snapshot.md). Its original authority statements are historical; it must never override CURRENT.
- Architecture: [30-architecture.md](./30-architecture.md).
- Memory benchmark protocol v2, historical v1 compatibility and pending measured acceptance: [57-memory-benchmarking.md](./57-memory-benchmarking.md).
- Error and fuzzing contract: [82-error-behaviour-fuzzing.md](./82-error-behaviour-fuzzing.md).
- Numeric-index behavior and retained performance experiments: [95-numeric-index-contract.md](./95-numeric-index-contract.md).
- Executed browser/native platform evidence and remaining gates: [96-browser-platform-acceptance.md](./96-browser-platform-acceptance.md).
- Staged validation and the complete pre-push inventory: [97-validation-workflow.md](./97-validation-workflow.md).
- Hosted executor-accounting path counts are a default-off CI diagnostic, described
  in the validation contract and [VM Core 2](./58-vm-core2.md); they provide no timing acceptance.
- Source-access preparation, history-audit scope and visibility gates: [98-public-source-readiness.md](./98-public-source-readiness.md). Separate from supported releases and package publication.
- [history-projection.json](./history-projection.json) maps the frozen private history to its cleaned projection and fingerprints protected runtime inputs; [public-source-profile.json](./public-source-profile.json) selects hosted correctness CI, keeps public benchmark automation paused, and records the private bridge's exact-cohort-only availability. Original signatures and private metadata bytes are not authenticated by the projection.
- Benchmark graph publication ordering, host-control investigation and history presentation: [99-benchmark-history-integrity.md](./99-benchmark-history-integrity.md).
- Latest reviewed host-calibration result used by the graph: [99-benchmark-host-calibration.json](./99-benchmark-host-calibration.json). This is a dated review record, not a live runner-health feed or a replacement for the control ledger.
- Human documentation audit, completed browser and practical embedding recipes, introductory example guidance, and remaining add/change/cut recommendations: [100-documentation-review.md](./100-documentation-review.md). This is a dated review, not a separate status authority or contribution policy.
- Progress renderer: [render-progress.js](./render-progress.js), input [progress.json](./progress.json), generated output [progress.html](./progress.html).

## Invariants

- Do not treat generated progress HTML as the source of truth.
- Renderer validates required metadata, non-empty lists, percentages and fixed area order; it escapes text inserted into HTML.
- Renderer default mode writes progress.html; check mode compares generated content without writing.

## Side Effects

- `node design/render-progress.js` writes `design/progress.html`; `--check` reads and validates current generated output.

## Verification

Run from repository root: `node --check design/render-progress.js`; `node design/render-progress.js --check`.
