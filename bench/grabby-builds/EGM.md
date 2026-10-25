# Grabby Build Results Renderer

## Overview

This leaf renders a selected, checked-in public dataset of Grabby build and runtime comparisons. It is separate from Rustia interpreter workload history.

## Interface

- [`render.js`](render.js) reads `results.json`, validates its schema, source identities, and paired-statistic arithmetic, then renders the Grabby build section.
- Its renderer and validators are internal reporting code, not a Rustia embedding interface.

## Invariants

- `ci-bench` is the only timing authority; this renderer reads recorded data and does not take measurements ([repository policy](../../AGENTS.md#benchmark-policy)).
- The JSON must be canonical and contain only the allowed public fields; validation checks the recorded protocol, provenance, and derived paired summaries ([`render.js`](render.js#L1)).
- The page describes two distinct sequential experiments, so their paired speedups must not be multiplied into one comparison ([renderer source](render.js#L175)).

## Side Effects

- Rendering reads `results.json` and returns generated HTML to its caller. It does not collect measurements or access the source reports.

## Verification

Run from the repository root: `node --check bench/grabby-builds/render.js` checks JavaScript syntax. `just check-benchmark-results` includes the committed Grabby dataset validator; it does not take timings.
