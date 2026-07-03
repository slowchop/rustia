# Changelog

All notable public-facing changes to rustia will be recorded here.

Current unpublished workspace version: `0.0.1`.

## [Unreleased]

### Added

- Luau 0.709 conformance tracking is pending-free, with the current manifest at
  684 fixtures and 481 behavior refs.
- The public `rustia` facade has compile-checked examples for globals, typed
  host functions, deterministic extensions, retained script functions, serde,
  and hot reload.
- `rustia-cli` supports file, stdin, and `--eval` execution with profile, fuel,
  and source-name options.
- The benchmark harness has current-results JSON/HTML, generated per-workload
  claim gating, package-file history, and the requested adapter matrix.
- Release preflight now composes release notes, package files, leaf package dry
  runs, progress HTML, benchmark result freshness, and workspace tests.
- Crate publishability blockers are generated from cargo metadata and package
  file audits into `design/publishability.json`.
- GitHub Actions now has a hosted release-preflight workflow and a manual
  self-hosted benchmark measurement workflow.
- Workflow policy is checked in release preflight so hosted CI and benchmark
  runner labels stay public and generic.
- Crate metadata policy is generated from cargo metadata into
  `design/crate-metadata.json` and checked in release preflight.
- Typed host function conversion errors now include the host function name and
  argument or return position.
- Typed host functions and retained script calls now support up to eight
  typed arguments/results through the public tuple conversion traits.
- Owned-value conversions now cover `Option<T>`, `Vec<T>`, and direct
  `OwnedTable` values for globals, host functions, and retained calls.
- Retained script functions and export tables now expose the successful install
  generation they came from, making hot-reload handle semantics explicit.
- The public `Rustia` facade now has capability-specific host function helpers,
  and `RustiaExtension` can mark external mutable-state reads explicitly.
- `RustiaExtension` exposes namespace plus per-function name, fuel,
  capability, and parameter-count metadata before installation.
- Release-readiness docs now cover package metadata, package file-list audits,
  semver policy, changelog validation, and publish ordering.

## Release Policy

- `0.0.x` is an unpublished/private-preview line.
- The first public preview will start at `0.1.0` only after the release gates in
  `design/90-release-readiness.md` are satisfied.
- Breaking public-facade changes after `0.1.0` require migration notes here.
- Semver and API stability rules are defined in `design/91-semver-policy.md`.
