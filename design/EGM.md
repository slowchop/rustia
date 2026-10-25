# design

## Overview

Design specifications, canonical project state, structured evidence, and a generated HTML progress view. Code behavior remains authoritative for what is built; CURRENT.md is the canonical status document.

## Interface

- Canonical state: [CURRENT.md](./CURRENT.md).
- Architecture: [30-architecture.md](./30-architecture.md).
- Error and fuzzing contract: [82-error-behaviour-fuzzing.md](./82-error-behaviour-fuzzing.md).
- Progress renderer: [render-progress.js](./render-progress.js), input [progress.json](./progress.json), generated output [progress.html](./progress.html).

## Invariants

- Do not treat generated progress HTML as the source of truth.
- Renderer validates required metadata, non-empty lists, percentages and fixed area order; it escapes text inserted into HTML.
- Renderer default mode writes progress.html; check mode compares generated content without writing.

## Side Effects

- `node design/render-progress.js` writes `design/progress.html`; `--check` reads and validates current generated output.

## Verification

Run from repository root: `node --check design/render-progress.js`; `node design/render-progress.js --check`.
