# Semver And API Policy

This policy defines how rustia will move from the current unpublished `0.0.x`
workspace to publishable public crates. It applies to the public rustia project
only; grabby-specific integration code remains outside this repository.

## Current Status

- All public workspace crates are currently `0.0.1` and `publish = false`.
- `0.0.x` means implementation-preview only. APIs may still change while P1
  engine, conformance, benchmark, and release gates are open.
- Until Rustia's first public release, prefer the cleanest coherent long-term
  API over compatibility with controlled clients. Treat necessary completeness
  as part of the feature and migrate every active client in the same rollout.
- The public compatibility target is Luau 0.709 until `design/05-luau-target.md`
  is deliberately re-pinned and the conformance oracle is refreshed.
- The minimum supported Rust version is the workspace `rust-version` in
  `Cargo.toml`.

## Public Crate Roles

- `rustia` is the primary public embedding crate. It owns the stable facade:
  `Rustia`, `RustiaBuilder`, `RustiaExtension`, `Profile`, `VmSettings`,
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

- **Stable facade:** the `rustia` crate items used by the public examples and
  README. These require migration notes for breaking changes after `0.1.0`.
- **Advanced/internal:** parser/compiler/VM bytecode, raw VM value handles,
  conformance helpers, and implementation knobs. These can change in minor
  releases before `1.0` when needed for correctness or performance.
- **Generated/data artifacts:** conformance coverage, benchmark history, current
  result JSON, and generated HTML. Schema changes require schema-version bumps
  and migration notes.

## Release Line Rules

- `0.0.x`: unpublished or private-preview snapshots. Breaking changes are
  allowed, but docs and examples should stay current.
- `0.1.x`: first public preview. The embedding facade should compile from the
  examples and README without private setup. Breaking API changes are allowed
  only in minor releases, with migration notes.
- `0.y.z` for `y > 1`: follow normal Rust pre-1.0 convention: patch releases
  avoid breaking public facade APIs; minor releases may break with documented
  migration notes.
- `1.0.0`: reserved for a stable public facade, documented profile semantics,
  completed release gates, and credible per-workload benchmark claims.

## Breaking Change Checklist

Before a breaking public-facade change lands after `0.1.0`:

- Update `crates/rustia/examples/` and README commands.
- Add or update tests proving the new behavior.
- Document migration impact in [`../CHANGELOG.md`](../CHANGELOG.md).
- Keep `design/CURRENT.md`, `design/progress.json`, and generated progress HTML
  aligned with the new status.
- If a schema changes, bump its schema version and keep the old behavior
  documented until downstream consumers can migrate.

## Publish Order

When publishing starts, publish crates in dependency order:

1. `rustia-lexer`
2. `rustia-parser`
3. `rustia-vm`
4. `rustia-compiler`
5. `rustia`
6. `rustia-cli`
7. `rustia-conformance`

The benchmark crate remains unpublished until the rustia adapter strategy is
chosen: publish `rustia-*` first, move the adapter into a git/path-only
integration crate, or keep it as an example adapter outside the core benchmark
package.
