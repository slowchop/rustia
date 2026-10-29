# Rustia design index

[CURRENT.md](CURRENT.md) is the canonical project state and wins when dated
records disagree. This index provides navigation; it does not duplicate status,
fixture counts, or release acceptance. To use Rustia, start with the
[README](../README.md) and its API examples.

## Current contracts

- [00 — Constraints](00-constraints.md), [02 — Principles and scope](02-pillars.md), and
  [05 — Pinned Luau target](05-luau-target.md).
- [30 — Architecture](30-architecture.md) and [50 — Conformance](50-conformance.md).
  The executable [coverage record](../conformance/COVERAGE.md) names the tested surface.
- [70 — Host bridge](70-host-bridge-contract.md) and [72 — Host services](72-host-services.md).
- [75 — Embedding API](75-embedded-api.md) and [76 — Koto migration](76-koto-migration.md).
- [77 — Hardened leaf environment](77-hardened-leaf-environment.md) and
  [78 — Host functions](78-host-functions.md).
- [79 — Script diagnostics](79-script-diagnostics.md),
  [82 — Error behavior and fuzzing](82-error-behaviour-fuzzing.md), and
  [83 — Rust objects](83-rust-objects.md).

These documents describe contracts and intent. Use the code, tests, and
`CURRENT.md` to distinguish implemented behavior from remaining design work.

## Validation and release

- [57 — Memory measurement protocol](57-memory-benchmarking.md),
  [80 — Benchmark suite](80-bench-suite.md), and
  [81 — Benchmark operations](81-benchmark-operations.md). The
  [benchmark README](../bench/README.md) explains the paused source profile and timing policy.
- [90 — Release readiness](90-release-readiness.md),
  [91 — Versioning policy](91-semver-policy.md), and
  [92 — First release](92-first-release.md).
- [97 — Validation workflow](97-validation-workflow.md): staged checks and the full pre-push inventory.
- [98 — Source readiness](98-public-source-readiness.md): privacy review and visibility decisions,
  separate from package publication or supported releases.
- Generated package audits: [metadata](crate-metadata.json),
  [file lists](package-files.json), and [publishability](publishability.json).
  [The facade inventory](facade-api.json) records the reviewed API surface.
- [Public source profile](public-source-profile.json) and
  [history projection](history-projection.json): selected CI scope and the cleaned-history mapping.

## Dated acceptance and investigation records

These retain the revisions, methods, and limitations of particular reviews. A
passing historical experiment does not establish a current release or a new performance claim.

- [84 — Private platform acceptance](84-private-platform-acceptance.md).
- [93 — Private API acceptance](93-private-api-acceptance.md).
- [94 — Compatibility audit](94-release-compatibility-audit.md) and its
  [structured error audit](release-error-audit.json).
- [95 — Numeric-index contract and retained experiments](95-numeric-index-contract.md).
- [96 — Browser and native platform acceptance](96-browser-platform-acceptance.md).
- [99 — Benchmark history integrity](99-benchmark-history-integrity.md) and
  [the host-calibration review](99-benchmark-host-calibration.json).
- [100 — Human documentation review](100-documentation-review.md): completed fixes, reader tasks, and proposed additions/cuts.
- Retained benchmark summaries: [results](../bench/RESULTS.md) and
  [claim scope](../bench/CLAIMS.md).

## Historical research and raw artifacts

Research records explain alternatives and experiments. In particular, the Wasm
JIT spike is an experiment, not a shipped execution tier. Consult `CURRENT.md`
for the implementation selected from this research.

- [10 — Scripting-engine landscape](10-landscape.md) and [raw research](10-landscape-raw.json).
- [15 — Early gap analysis](15-gapclose.md) and [raw results](15-gapclose-raw.json).
- [20 — Skeleton results](20-skeleton-raw.json), [25 — Architecture review](25-review-raw.json),
  and [35 — Axis exploration](35-axes-raw.json).
- [40 — Spike receipts](40-spikes/) and the [experimental source tree](../spikes/).
- [50 — Conformance research](50-conformance-raw.json) and
  [55 — Adopted design improvements](55-improvements.md).
- [56 — Performance research](56-performance-research.md).
- [58 — VM Core 2 research](58-vm-core2.md), [census](58-vm-core2-c23-census.json),
  and [optimization preview](58-vm-core2-c23-o2-preview.json).
- [60 — Review responses](60-review-response-raw.json).
- [65 — Wasm JIT spike](65-wasmjit.md) and [raw results](65-wasmjit-raw.json).

The dated [progress view](progress.html), recorded on 2026-10-18, is generated from [progress.json](progress.json)
by [render-progress.js](render-progress.js); it is not an additional source of project status.
