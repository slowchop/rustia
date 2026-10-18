# First release: a Rust API people can depend on

Target: **Rustia 1.0**, after a release-candidate cycle. Status: **in progress;
not ready to publish**. The workspace remains unpublished at `0.0.1`.

The goal is straightforward: a Rust developer should be able to try Rustia in a
few minutes, understand its limits, and keep using the same API throughout 1.x.
We will make the necessary API changes before release and migrate existing
consumers together. We cannot promise that a breaking change will never be
needed; we can avoid freezing known design mistakes into the first release.

This is the delivery checklist for the first release. [CURRENT.md](CURRENT.md)
records what is built; [90-release-readiness.md](90-release-readiness.md) holds
the detailed validation and packaging requirements. A passing preflight alone
does not complete this milestone.

## Delivery order

Use these batches to turn the acceptance checklist below into small, reviewable
changes. Each batch may take several PRs; close it with recorded evidence rather
than a percentage or a deadline. API changes include examples, documentation,
tests, and active-client migration in the same rollout.

| Batch | Work | Done when |
|---|---|---|
| 1. Establish a green baseline | Resolve the stale generated benchmark-progress report found by PR #60's full preflight; confirm current main's checks. Required checks and independent reviews must pass before merge. | The reviewed revision passes complete preflight and required checks; no blocking independent-review findings remain. |
| 2. Decide the 1.0 API boundary | Inventory every public type, function, trait, and reachable dependency type. Settle constructors/defaults, settings, profiles, errors, metadata, extension traits, and which crates need stable versions. Remove accidental engine controls from the facade. | Independent API review accepts the proposed surface, documented defaults have consumer tests, and every intended extension point can grow compatibly. |
| 3. Finish embedding workflows | Define the minimum useful opaque Rust-object API. Exercise conversions, live values, callbacks, reset, failure precedence, and module replacement from an external application. Migrate active clients with any necessary API changes. | A real consumer demonstrates each supported workflow without engine internals or workaround code; focused tests cover its failure and lifetime contracts. |
| 4. Close runtime gaps | Finish yieldable `require` and compile-time versus runtime error parity; repeat bounded fuzz and long-running memory/reset tests. Review collection scheduling against the existing contract and remote evidence. | Pinned Luau fixtures and regression tests pass, no known release-blocking semantic gaps remain, and any tuning is supported by `ci-bench`. |
| 5. Verify supported platforms | Execute the promised native and browser correctness/golden matrix, including real Firefox and Safari. | Exact candidate revisions and results cover the platform promises in `00-constraints.md`; build-only checks are not recorded as runtime proof. |
| 6. Exercise a release candidate | Propose the API freeze, record its public API baseline, finish docs/package audits, and use the exact candidate in the active applications. Verify minimum Rust, feature combinations, recovery, and remote benchmark evidence. | Clean external installations and active consumers pass; independent review, complete preflight, platform tests, and candidate evidence are green. No known API redesign remains. |
| 7. Publish and verify | Publish dependencies in order, then the embedding crate and CLI; tag the source and publish release notes. | Registry installation and documentation work, licenses and support statements agree, and the released API baseline is ready to check future 1.x changes. |

The main dependency is **1 → 2 → 3 → 6 → 7**. Runtime hardening and platform
validation (4 and 5) can proceed alongside API work, but both must finish before
the candidate is accepted. Package audits and documentation checks can start
early; final verification always uses the candidate revision.

Start batch 2 with two focused reviews: **public settings and exports**, then
**construction, extensible types, and errors**. Follow with a separate opaque
Rust-object design and implementation. Keep these decisions small enough to
review and exercise in a consumer before moving to the next one.

Use README + rustdoc for the first release. A documentation site, async Rust
callbacks, scoped callbacks, a JIT, a full mlua-style object system, and a
standalone benchmark project are outside this release unless an existing
acceptance requirement depends on them. The opaque-object requirement above
remains part of the release; deferring a larger object system does not waive it.

## 1. Settle the public API

| Requirement | Current status | Evidence needed to close it |
|---|---|---|
| Evaluate scripts, exchange typed values, register Rust callbacks, call script functions, and reload modules through one coherent API | Implemented; being reviewed | Runnable README examples, ordinary and typed-host consumer tests, and independent API review |
| Explain construction and defaults consistently | Review open | Document the difference between service-free constructors and builder presets; choose the final 1.0 contract and test it |
| Leave room to add error categories | Implemented in this change | Non-exhaustive facade errors, private classification storage, and typed handle-error regressions |
| Validate invalid handles before converting inputs or allocating guest values | Implemented in this change | Stale-handle and cross-VM tests, including conversion side effects and memory accounting |
| Separate stable settings from engine implementation details | Open | Review every facade export; replace exposed conformance/compiler/collector controls with a deliberate advanced boundary before freezing it |
| Allow public classifications and metadata to evolve | Review open | Audit re-exported VM errors, profiles, capabilities, diagnostic types, and owned value/key enums; document exhaustive contracts or make them extensible before 1.0 |
| Define host-created opaque userdata | Open | Registration, ownership, type checks, collection/reset, quota admission, and deterministic-profile rules with an actual consumer example |
| Complete host value construction and conversion ergonomics | Review open | Exercise binary strings, buffers, vectors, shared/cyclic tables, tuples, serde, live callbacks, and meaningful conversion errors from an external consumer |
| Specify conversion, concurrency, and failure contracts | Review open | Document missing results, sparse sequences, integer conversion, VM thread affinity, callback sharing, and validation precedence with consumer tests |
| Make callback and module lifetimes easy to understand | Implemented; review open | Document which values survive replacement/reset, callback capture responsibilities, and exact installation rollback limits |
| Publish an explicit compatibility contract | Open until candidate | Inventory the complete public facade, document extension points, and run source-compatibility checks against the candidate and active consumers |

At 1.0, every public item reachable through `rustia` is covered by its compatibility
promise, including re-exported types. Calling a public item “advanced” does not
make a breaking change compatible. Keep implementation details private or in
separately versioned lower-level crates before release. Evolving classifications
should be non-exhaustive; configurable types should have constructors/builders
that can accept new options without requiring callers to update struct literals.
Do not add a required method to a public extension trait during 1.x.

## 2. Close the remaining runtime gaps

| Requirement | Current status | Evidence needed to close it |
|---|---|---|
| Enforce memory limits consistently on optimized execution paths | Implemented; final CI pending | Memory-limited VMs use scalar instruction boundaries for batched paths; cross-profile quota tests and execution counters cover the [former unchecked window](58-vm-core2.md) |
| Match Luau's compile-time versus runtime error behavior | In progress | Focused Luau 0.709 oracle fixtures and differential campaigns; valid runtime failures must remain catchable at runtime |
| Allow a required module to yield where the pinned Luau contract permits it | Open | A resumable loading continuation with cache commit on completion, cycle/error cleanup, and shared fuel tests |
| Confirm collection settings suit long-running applications | Correctness coverage exists; tuning open | Preserve the deterministic scheduling contract; use remote measurements for any tuning or performance claim |
| Retain clear source diagnostics and recovery behavior | Implemented | Keep the diagnostics and error/fuzz regressions green; add fixtures for newly found gaps |

The current corpus has 712 fixtures covering 511 named behaviors with zero pending
cases. This describes the tested inventory, not universal compatibility. Roblox
APIs, a Luau typechecker, raw Luau C API compatibility, async Rust callbacks, and a
JIT are outside this first release. These exclusions do not waive any existing
runtime or embedding requirement above.

## 3. Verify the platforms we promise

- [ ] Complete the native correctness and deterministic-result matrix in
  [00-constraints.md](00-constraints.md), using real execution where promised.
  Cross-compiling alone does not prove runtime behavior.
- [ ] Run the declared browser correctness tests in real Firefox and Safari,
  including the supported JavaScript adapters. Existing Node/V8 coverage remains
  part of the gate.
- [ ] Repeat the bounded fuzz and long-running memory/reload/reset checks on the
  final candidate. Record exact commits and CI links.
- [ ] Keep the public workspace free of unsafe code and all required correctness,
  formatting, lint, and independent-review checks green.

## 4. Make the project welcoming

- [x] Lead the README with a short explanation and executable API examples.
- [x] Explain common environment choices and error handling in everyday language.
- [x] Show feature tradeoffs and per-workload speed comparisons with source links.
- [x] Add a contributor guide and keep local worktrees out of Git's untracked list.
- [ ] Complete the API reference: defaults, failure behavior, lifetime/reset rules,
  and at least one useful example for each supported public workflow.
- [ ] Open the public bug tracker and provide a clear way to report problems
  before inviting public users. Issues are disabled during the private preview.
- [ ] Check all documentation links in the packaged crates and build the docs
  with warnings treated as errors. Both checks now run in release preflight;
  final candidate verification remains required.
- [ ] Review what is actually shipped in every crate archive. Keep private notes,
  local state, research-only material, and generated build files out of packages.

Detailed engineering records belong in `design/`; readers should not need to
learn the VM implementation to embed the library. Historical research remains
available without dominating the landing page.

The initial documentation surface is the README and rustdoc. The README gives
complete examples and comparisons; rustdoc provides a short task index and
method-level contracts for ordinary embedding, callbacks, values, errors, and
limits. Its Rust examples and the README examples run in the existing test gates.
Finish the remaining API decisions before treating this reference as a 1.0
contract; this documentation pass does not close the full API-reference gate.

## 5. Make comparisons useful and reproducible

- [ ] Record the release candidate on `ci-bench` and publish the exact revision,
  workload, target, engine versions, execution mode, and measurement date.
- [ ] Show interpreter and JIT results separately. State each workload's unit and
  keep feature comparisons distinct from throughput rankings.
- [ ] Keep README numbers tied to the committed generated results. Refresh or
  clearly date them when the snapshot changes.
- [ ] Present disadvantages as clearly as strengths. A speed lead over every
  other engine is not a release requirement; accurate evidence is.

No performance benchmarks run on a developer's machine. PR screening and any
fresh measurements use the repository's remote benchmark workflows. A missing
remote result remains pending.

## 6. Ship a candidate, then 1.0

1. Close the API and runtime requirements above. Migrate all active controlled
   clients in the same rollout for any incompatible API change.
2. Prepare a release candidate and a complete public API inventory. Test it from
   a clean external application using only documented imports and packaged files.
3. Run `just release-preflight`, documentation/package checks, the promised runtime
   target matrix, and remote benchmark screening on the final reviewed revision.
   Exercise the candidate in the active consumers and record their results.
4. Resolve all blocking independent-review findings. Land via a merge commit;
   required CI and reviews continue to gate auto-merge.
5. Review release notes, supported platforms, minimum Rust version, feature flags,
   license files, examples, and migration notes together. Flip publication policy
   only after the complete gate passes, then publish dependencies in the order
   specified by [91-semver-policy.md](91-semver-policy.md).
6. Tag the released source, verify installation and documentation from the public
   packages, and record the release/API baseline for future 1.x checks.

The current change starts this milestone. It improves the landing page, contributor
workflow, error/handle API, and quota enforcement; it does not claim that the open runtime, platform,
consumer-validation, or publication gates have passed.
