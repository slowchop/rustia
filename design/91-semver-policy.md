# Semver And API Policy

This policy defines how rustia will move from the unpublished
workspace to publishable public crates. It applies to the public rustia project
only; grabby-specific integration code remains outside this repository.

Repository visibility is separate from package versions and publication.
Rustia’s source is public for inspection and experimental use. The workspace
remains unpublished and its API is still under development. The [launch record](public-source-activation.md)
covers source access.

## Current Status

- The unpublished workspace is staged at `0.2.0`.
  [PR #108](https://github.com/slowchop/rustia/pull/108) configures only the
  six preview packages for crates.io. [Issue #83](https://github.com/slowchop/rustia/issues/83)
  tracks preview validation and publication.
- The 0.x milestone validates the API with controlled client applications.
  The preview requires its applicable checks and independent review before
  merge. Registry publication and a GitHub release each require separate explicit
  authorization. See [the preview gates](90-release-readiness.md#rustia-020-interpreter-preview)
  and [the supported-release plan](92-first-release.md).
- `0.0.x` means implementation-preview only. APIs may still change while P1
  engine, conformance, benchmark, and release gates are open.
- Until Rustia’s first published crate release, prefer the cleanest coherent long-term
  API over compatibility with controlled clients. Treat necessary completeness
  as part of the feature and migrate every active client in the same rollout.
- The public compatibility target is Luau 0.709 until `design/05-luau-target.md`
  is deliberately re-pinned and the conformance oracle is refreshed.
- The minimum supported Rust version is the workspace `rust-version` in
  `Cargo.toml`.
- The first stable release target is `1.0.0`, following a release-candidate cycle
  and the acceptance checklist in [`92-first-release.md`](92-first-release.md).
  The public API remains under development while those gates are open.
- The future 1.0 promise covers the `rustia` embedding API and the CLI command
  contract. Supporting lexer/parser/compiler/VM packages retain separately
  versioned 0.x APIs; conformance and benchmark tooling remain unpublished.
  The first interpreter preview stages the six selected crates at `0.2.0`.
  Separate the supporting crate versions before the facade and CLI reach 1.0.

## Public Crate Roles

- `rustia` is the primary public embedding crate. Its planned stable facade covers
  `Rustia`, `RustiaBuilder`, `RustiaExtension`, `Profile`, `Settings`,
  `OwnedValue`, conversion traits, retained `Function` handles, `Exports`,
  serde helpers, and public error types.
- `rustia-cli` is the command-line runner surface.
- `rustia-lexer`, `rustia-parser`, `rustia-compiler`, and `rustia-vm` are
  lower-level crates. They may be published for transparency and tooling, but
  their APIs are more implementation-shaped than the `rustia` facade.
- `rustia-conformance` is a tooling crate for validating the Luau 0.709 corpus.
- `script-bench-harness` is intended to split into a standalone OSS benchmark
  project before publication.

## Stability Tiers

- **Public facade:** every public item reachable through the `rustia` crate,
  including re-exports and public field/trait contracts. At `1.0`, all of this
  surface follows SemVer; the promise is not limited to README examples. An
  “advanced” label does not exempt an exported type from compatibility.
- **Lower-level crates:** parser/compiler/VM bytecode, raw VM handles,
  conformance helpers, and implementation knobs have their own package-version
  contracts. They can change in minor releases while their package is pre-1.0;
  a lower-level crate at 1.0 follows the same major-version rule as any other
  stable crate. Before facade 1.0, audit re-exports so engine details do not
  accidentally become permanent facade commitments.
- **Generated/data artifacts:** conformance coverage, benchmark history, current
  result JSON, and generated HTML. Schema changes require schema-version bumps
  and migration notes.

## Release Line Rules

- `0.0.x`: unpublished development snapshots, including the experimental public source. Breaking changes are
  allowed, but docs and examples should stay current.
- `0.1.x`: optional public preview. The embedding facade should compile from the
  examples and README without private setup. Breaking API changes are allowed
  only in minor releases, with migration notes.
- `0.y.z` for `y > 1`: follow normal Rust pre-1.0 convention: patch releases
  avoid breaking public facade APIs; minor releases may break with documented
  migration notes.
- `1.0.0`: reserved for a stable public facade, documented profile semantics,
  completed release gates, and credible per-workload benchmark evidence. It
  does not require being faster than every other engine.
- `1.x`: add compatible functionality in minor releases and fixes in patch
  releases. Do not add required trait methods, remove public items, change
  public signatures, or add variants to exhaustive public enums without a
  major release. Prefer private fields and builders for evolving configuration
  and non-exhaustive enums for classifications expected to grow. Match
  non-exhaustive classifications with a fallback arm.

Before the first candidate, record the complete facade API and run consumer
compilation tests. After 1.0, compare proposed changes with that released API
baseline in CI. Migration notes explain a breaking change; they do not make it
compatible with 1.x. Replay/fuel/module ABI identifiers remain separately
versioned and must be included in an application's replay identity.

## Pre-release facade inventory

[`facade-api.json`](facade-api.json) records the current reachable embedding API
for native and browser Wasm, with default and bare features. This reviewed snapshot
is available in the public source tree. The 1.0 compatibility baseline remains open. Update it with
`node tools/check-facade-api.js --write` when deliberately changing the facade;
release preflight verifies the committed result.

The checker follows semantic rustdoc IDs through exports, signatures, associated
types, supertraits, and implementations. It rejects callable lexer/parser/
compiler/VM types, including aliases and implicit conversion impls. Private
callback methods are excluded only while they require a private, unconstructible
token. Negative self-tests cover those traversal boundaries. Macro declarations
are inventoried; expansion behavior is tested through consumer examples.

Rustdoc JSON remains unstable, so this check uses the pinned Rust toolchain and
schema version. `RUSTC_BOOTSTRAP=1` is limited to the documentation command to
obtain JSON; it does not change Rustia's supported compiler or enable unstable
features in production code. A compiler/schema upgrade must review the traversal
before accepting a new inventory. The final candidate still needs independent
API review, real-consumer checks, and a later released baseline for 1.x diffs.

## Breaking Change Checklist

Before a breaking public-facade change lands after `0.1.0`:

- Update `crates/rustia/examples/` and README commands.
- Add or update tests proving the new behavior.
- Document migration impact in [`../CHANGELOG.md`](../CHANGELOG.md).
- Update [CURRENT.md](CURRENT.md) with the implemented change. The October 2026
  progress JSON and HTML preserve the project state at that date.
- If a schema changes, bump its schema version and keep the old behavior
  documented until downstream consumers can migrate.

## Publish Order

After a separate publication decision, publish the initial public packages in
dependency order:

1. `rustia-lexer`
2. `rustia-parser`
3. `rustia-vm`
4. `rustia-compiler`
5. `rustia`
6. `rustia-cli`

Conformance and benchmark tooling are excluded from that initial publication.
Publishing either tool later requires its own packaging and publication decision.
Confirm each package is visible in the crates.io index before uploading its
dependants.

The benchmark crate remains unpublished until the rustia adapter strategy is
chosen: publish `rustia-*` first, move the adapter into a git/path-only
integration crate, or keep it as an example adapter outside the core benchmark
package.
