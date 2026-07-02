# rustia conformance fixtures

This directory is the public conformance spine for rustia. Engine work should add or update fixtures here
before relying on unit tests in an individual crate.

## Target

- Luau release: `0.709`
- Commit: `b968ef742741bb2b703afc3b3c53f06608c87481`
- Surface: standalone luau.org runtime, vector width 3, flags-off
- Authoritative docs: `design/05-luau-target.md`, `design/50-conformance.md`

## Layout

- `MANIFEST.tsv` lists the fixtures the rustia harness executes.
- `REFS.tsv` declares every stable conformance topic a fixture may reference.
- `fixtures/` contains source files checked into this repo.
- `upstream/` holds provenance notes and the import plan for upstream Luau tests.

## Local Runner

Run the conformance manifest directly:

```sh
cargo run -p rustia-conformance -- .
```

List unresolved manifest rows without executing them:

```sh
cargo run -p rustia-conformance -- --list-pending
```

The same runner is used by `cargo test --workspace`; CI should call this crate rather than reimplementing
manifest parsing.

Manifest columns:

```text
id<TAB>category<TAB>refs<TAB>profile<TAB>oracle<TAB>expect<TAB>path
```

`profile` selects which rustia mode executes the fixture:

- `LuauCompat` - reference-Luau compatibility posture. Use this for fixtures that assert implementation-defined
  or unspecified Luau 0.709 behavior, such as hash-slot `pairs` order.
- `ReproducibleStandard` - public default profile. It is Luau-conformant, but uses rustia's reproducible choices
  for unspecified behavior, including insertion-ordered hash traversal.
- `DeterministicSim` - sandboxed deterministic subset for grabby/sim behavior. Use this for rustia-only
  deterministic invariants and narrow smoke fixtures that do not require Standard-only features.

`refs` is a comma-separated list of stable conformance topics from `design/50-conformance.md`.
It is deliberately required: fixtures are not generic regression tests; they prove named Luau behaviors.
Every ref must exist in `REFS.tsv`, so typos and one-off labels fail the harness.
Every `REFS.tsv` row must also be used by at least one manifest fixture.

Reference catalog columns:

```text
ref<TAB>category<TAB>source<TAB>status<TAB>summary
```

`status` has the same meaning as manifest `oracle` except that `pending-fixture` is also allowed for topics
that have been declared but do not yet have an executable fixture.
The catalog status must match its evidence source: `luau-0.709` refs cite pinned upstream evidence, while
`pending-oracle` refs remain design-sourced until their oracle is re-derived.

`oracle` tracks expectation provenance:

- `luau-0.709` - expectation was derived from the pinned oracle.
- `pending-oracle` - temporary P1 scaffold expectation; must be re-derived before the conformance row can be
  counted complete.
- `pending-impl` - provenance is known, but rustia does not implement enough of the fixture yet; the harness
  counts it but does not execute it.
- `rustia-only` - rustia-specific invariant outside Luau compatibility, such as a deterministic profile
  restriction. Prefer adding a Luau fixture too when possible.

The current pending manifest rows and pending catalog refs are exact-allowlisted in `rustia-conformance`
tests. The remaining `pending-impl` rows are also force-run in tests and matched against their current
missing-harness failures. Adding, removing, or promoting pending debt is allowed, but it must be a deliberate
test-reviewed change.
The upstream provenance TSVs are also mechanically checked for the pinned repo, commit, local path,
classification, license, and oracle command shape.

Supported expectation atoms in the current harness:

- `number:<decimal>`
- `number_bits:<16-hex-u64>`
- `pass`
- `bool:true`
- `bool:false`
- `nil`
- `string`
- `string:<exact-utf8>`
- `string_hex:<hex-bytes>`
- `error_contains:<substring>`

## Upstream Luau Test Port Workflow

1. Use the pinned upstream commit only: `b968ef742741bb2b703afc3b3c53f06608c87481`.
2. Record provenance for every copied or translated file in `upstream/PROVENANCE.md` before committing it.
3. Keep Roblox-only tests excluded unless rewritten to standalone Luau semantics.
4. Classify each test as one of:
   - `api-native`
   - `core-language`
   - `parser`
   - `parser-runtime`
   - `runtime`
   - `stdlib`
   - `error-message`
   - `determinism`
   - `excluded`
5. Generate oracle expectations from the pinned Luau 0.709 oracle, not from rustia.
6. Store exact script-observable errors separately from rich host diagnostics.
7. Add rustia implementation only after a failing fixture exists, unless the fixture depends on syntax rustia
   cannot parse yet.

## Exclusion Policy

Excluded tests need a reason in the manifest or provenance notes:

- Roblox engine APIs, scheduler, `Instance`, `DataModel`, or `typeof` extensions outside standalone Luau.
- Analyzer/typechecker-only diagnostics not emitted by the runtime parser.
- Bytecode or VM-internal tests that depend on Luau's internal bytecode format.
- Tests for features deliberately removed in `DeterministicSim`; these must still run under
  `ReproducibleStandard` where Luau requires them.

## Completion Bar

The conformance goal is not complete until every row in `design/50-conformance.md` is backed by either:

- an executable fixture in this directory,
- an upstream Luau test mapped by provenance,
- or an explicit exclusion with a documented reason.
