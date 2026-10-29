# design

## Overview

Design specifications, canonical project state, structured evidence, and a generated HTML progress view. Code behavior remains authoritative for what is built; CURRENT.md is the canonical status document.

## Interface

- Canonical state: [CURRENT.md](./CURRENT.md).
- Architecture: [30-architecture.md](./30-architecture.md).
- Error and fuzzing contract: [82-error-behaviour-fuzzing.md](./82-error-behaviour-fuzzing.md).
- Numeric-index behavior and retained performance experiments: [95-numeric-index-contract.md](./95-numeric-index-contract.md).
- Executed browser/native platform evidence and remaining gates: [96-browser-platform-acceptance.md](./96-browser-platform-acceptance.md).
- Staged validation and the complete pre-push inventory: [97-validation-workflow.md](./97-validation-workflow.md).
- Source-access preparation, history-audit scope and visibility gates: [98-public-source-readiness.md](./98-public-source-readiness.md). Separate from supported releases and package publication.
- [history-projection.json](./history-projection.json) maps the frozen private history to its cleaned projection and fingerprints protected runtime inputs; [public-source-profile.json](./public-source-profile.json) selects hosted correctness CI, keeps public benchmark automation paused, and records the private bridge's exact-cohort-only availability. Original signatures and private metadata bytes are not authenticated by the projection.
- Benchmark graph publication ordering, host-control investigation and history presentation: [99-benchmark-history-integrity.md](./99-benchmark-history-integrity.md).
- Latest reviewed host-calibration result used by the graph: [99-benchmark-host-calibration.json](./99-benchmark-host-calibration.json). This is a dated review record, not a live runner-health feed or a replacement for the control ledger.
- Progress renderer: [render-progress.js](./render-progress.js), input [progress.json](./progress.json), generated output [progress.html](./progress.html).

## Invariants

- Do not treat generated progress HTML as the source of truth.
- Renderer validates required metadata, non-empty lists, percentages and fixed area order; it escapes text inserted into HTML.
- Renderer default mode writes progress.html; check mode compares generated content without writing.

## Side Effects

- `node design/render-progress.js` writes `design/progress.html`; `--check` reads and validates current generated output.

## Verification

Run from repository root: `node --check design/render-progress.js`; `node design/render-progress.js --check`.
