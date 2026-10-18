# Private API milestone and future 1.0 release

Current target: **a private 0.x milestone for API and real-application readiness**.
Keep the repository private and every workspace crate at `publish = false`.
The workspace version remains `0.0.1`; this plan does not authorize publication,
a public release candidate, or a version bump.

The future stable target remains **Rustia 1.0**, after a separate publication
and release-candidate decision. That compatibility promise will cover the
`rustia` embedding API and the CLI command contract. Parser, lexer, compiler,
and VM packages will retain separately versioned 0.x APIs; benchmark and
conformance tooling remain unpublished unless separately selected later.

The goal is straightforward: a Rust developer should be able to try Rustia in a
few minutes, understand its limits, and use it without application workarounds.
Make necessary API changes during this private phase and migrate active clients
together. Record the proposed API baseline after consumer validation; do not
freeze known design mistakes into 1.0.

[CURRENT.md](CURRENT.md) records what is built. The delivery order below defines
this private milestone. The detailed checklist after it, together with
[90-release-readiness.md](90-release-readiness.md), retains the eventual public
release requirements. Passing private acceptance does not close those broader
gates.

## Delivery order

Each batch may take several small PRs. API changes include tests, examples,
documentation, and active-client migration in the same rollout. Required checks
and independent reviews must pass before merge; use merge commits.

| Batch | Work | Done when |
|---|---|---|
| 1. Restore the baseline | Land the generated benchmark-progress repair in PR #61. | The reviewed revision passes full preflight and remote screening; this repair merged after its checks passed. |
| 2. Separate application APIs from engine controls | Inventory the entire reachable facade, including error payloads, trait bounds, conversion impls, and re-exports. Replace accidental VM/compiler/parser exposure with facade-owned types and deliberately implementable or sealed extension points. | Independent API review accepts the inventory; facade consumers do not need lower-level engine types. |
| 3. Settle construction and failure contracts | Make builder presets canonical; provide deliberate limits and settings readback. Preserve standard/live capability freedom. Make evolving classifications extensible and document conversion, ownership, reset, concurrency, and failure behavior. | External consumer tests cover defaults, error matching, data conversion, callbacks, module replacement, and rejected receivers without relying on implementation controls. |
| 4. Add typed Rust objects and methods | Implement the accepted object contract below through the existing callback conversions, capabilities, fuel, and panic rules. Migrate real consumers alongside the API. | The runnable game-turn example uses read-only deterministic objects with staged writes; civfun exercises real immediate mutation. Both examples and the application integration pass against the exact candidate. Collection, reset, rejection, and failure tests pass. |
| 5. Close runtime gaps | Finish yieldable `require`, audit compile-time versus runtime error parity, and repeat bounded fuzz and memory/reset coverage. Extend lifecycle tests for the new object and suspended-module state. | Pinned Luau fixtures and regression tests pass; no known private-milestone semantic blocker remains. Retain GC settings unless remote measurements justify a change. |
| 6. Verify the private platform baseline | Execute a shared correctness/golden corpus on Linux x86-64, macOS arm64, and Node/V8 Wasm, and retain the existing eight-target build matrix. | Exact revisions produce matching deterministic results on those executed targets; reports distinguish build coverage from execution. |
| 7. Accept a private candidate | Use one immutable Git revision in WOTS, civfun, and Grabby; verify packages, examples, CLI, feature combinations, recovery, and remote benchmark evidence. | Full CI, independent reviews, clean external integration, active-client runtime tests, and private-platform results pass. Record the API inventory, consumer revisions, test results, and remaining public-release gates. |

The main dependency is **1 → 2 → 3 → 4 → 7**. Runtime and platform work (5 and 6)
can proceed alongside API work after the baseline repair, with final tests
including the new object and module-loading behavior. Both tracks join before
private acceptance. Documentation and package checks run throughout.

Start with the settings/export boundary, then construction/errors/extensibility,
then typed objects. Keep the selected semantics fixed while breaking the
implementation into independently reviewable changes. Use README + rustdoc.

The full native/mobile/Firefox/Safari runtime matrix remains a **future public
release gate**, not a blocker for this private milestone. Do not remove that
matrix from the platform contract or imply that cross-compilation proves it.
Public package publication, docs.rs verification, public issue-tracker opening,
and 1.0 promotion are deferred. A documentation site, async/scoped callbacks,
custom object fields/metamethods, a JIT, VM-owned mutable-object snapshots, and
benchmark-project extraction are outside the private milestone.

## Agreed Rust-object contract

Typed Rust-object construction and method dispatch are **implemented in the
current private API change** through `ObjectType<T, H>`, `Object<T>`, and
`NewObject<T>`. Final CI, independent reviews, and real civfun object use remain
pending. The [object guide](83-rust-objects.md) documents the implemented API and
test coverage; this status does not close private acceptance.

- Provide typed object registration and VM/epoch-bound live handles, ordinary
  `object:method(...)` calls, safe Rust type checks, and reuse of existing typed
  callback arguments/results. Method definitions persist across reset; instances
  and their handles do not. Replacement changes future instances, while existing
  instances and detached methods keep their original definitions.
- Deterministic game logic reads replay-stable object input. Authoritative writes
  use the existing typed effect/slot-write journal, including method callbacks
  that receive the matching typed capability view. An invocation returns its
  ordered requests only on success; the application decides when to apply them.
  A failing method discards its staged suffix. An error caught by script
  `pcall` follows the existing journal semantics: earlier completed requests
  remain and the complete invocation is not automatically cancelled.
- Standard/live methods may mutate application state immediately when declared
  with the corresponding capability. External application changes and I/O are
  outside installation/invocation rollback and are not undone by reset.
- Deterministic construction rejects a method definition with forbidden
  capabilities atomically, before publishing bindings or allocating an instance.
  Do not silently drop methods. Read-only signatures and captured state still
  require the host's replay-stability promise.
- Host-side calls validate receiver VM/epoch before argument conversion, guest
  allocation, or invocation-fuel effects. Guest method dispatch checks the Rust
  receiver type before converting other arguments or executing host effects;
  normal script instruction and call fuel still applies. Keep callbacks
  synchronous, without a VM re-entry borrow or escaping payload references.
  Hardened leaves reject object construction before guest allocation.
- Admit VM-owned metadata and roots under the existing logical-live quota;
  arbitrary Rust payload allocations remain host memory, like callback captures.
  Validate all fresh-object registrations and live callback results before
  allocating guest return values. Reset releases payloads even when stale Rust
  handles remain.
  Cover aliases, collection, replacement, reset, panic containment, and single
  payload destruction. Destructors must not panic. Hidden ownership cycles in
  Rust captures/payloads remain the host's responsibility.

Use the existing typed invocation API for staged changes; do not invent a second
transaction or effect queue. Generic Rust-object snapshot/rollback is deferred.
The same Luau method syntax may stage a request or mutate immediately; the
registered Rust implementation and its capability declaration determine the
behavior, not the method's spelling.

The user accepted this validation split on 2026-10-18: a runnable game-turn
example for deterministic staged object changes, plus civfun for real immediate
mutation. Existing WOTS and console writes must keep their error semantics;
Grabby's hardened production leaf must continue rejecting objects. A new
production staged-object caller is outside this milestone.

## Private acceptance evidence

- Run a clean external application against the exact candidate using only the
  facade: values/Serde, live shared and cyclic tables, Rust/script calls, typed
  objects, errors, limits, reset, and successful/failed reload.
- Test staged-action success, escaping-error discard, and `pcall` recovery;
  verify standard/live immediate changes survive an ensuing script or install
  error. Reject forbidden deterministic registrations and wrong/stale/foreign
  receivers without conversion side effects or partial installation.
- Extend existing collection/quota/mixed-heap tests for object aliases and
  suspended modules; exercise loader yield/resume, cycles, interleaving,
  abandonment, reset, cache commit, failure/retry, and shared fuel. Do not leave
  a whole-VM savepoint open across unrelated coroutine execution.
- Run full release preflight, README/rustdoc checks, default and bare feature
  tests, minimum Rust 1.95, CLI black-box checks including `--version`, bounded
  fuzz campaigns, and remote `ci-bench` screening on the reviewed candidate.
  Local correctness tests are allowed; local performance measurements are not.
- Pin WOTS/civfun to the candidate rather than a moving branch or machine-local
  path. Update Grabby through its documented faithful vendoring process and
  verify its recorded revision, dependency/features identity, and normal guards.
  Record consumer runtime/replay/reset results, not compilation alone.
- Audit archive contents and compile packaged dependencies in a clean temporary
  environment, including repository-only compiler test fixtures. No registry
  install or docs.rs success is claimed during private acceptance. Record the
  accepted private API baseline for later compatibility checks; the existing
  registry placeholder is not the baseline.

## Future public-release checklist

The requirements below remain open until verified. Public 1.0 applies to the
embedding facade and CLI; separately versioned internal 0.x crates must not leak
through their public contracts. Registry/public-candidate operations require a
separate publication decision.

## 1. Settle the public API

| Requirement | Current status | Evidence needed to close it |
|---|---|---|
| Evaluate scripts, exchange typed values, register Rust callbacks, call script functions, and reload modules through one coherent API | Implemented; being reviewed | Runnable README examples, ordinary and typed-host consumer tests, and independent API review |
| Explain construction and defaults consistently | Canonical builder contract implemented | Preserve tested presets, individual limits and immutable settings; final 1.0 acceptance remains open |
| Leave room to add error categories | Implemented in this change | Non-exhaustive facade errors, private classification storage, and typed handle-error regressions |
| Validate invalid handles before converting inputs or allocating guest values | Implemented in this change | Stale-handle and cross-VM tests, including conversion side effects and memory accounting |
| Separate stable settings from engine implementation details | Merged in PR #63; Grabby CI/merge pending | Immutable facade `Settings`, builder configuration, owned profiles/errors/diagnostics, sealed callback adapters, and preserved private engine-control regressions; WOTS #7 and civfun #51 merged |
| Allow public classifications and metadata to evolve | Review open | Audit re-exported VM errors, profiles, capabilities, diagnostic types, and owned value/key enums; document exhaustive contracts or make them extensible before 1.0 |
| Provide typed Rust objects with methods | Implemented; final checks/reviews and civfun object use pending | Read-only deterministic input plus staged actions; capability-declared mutable standard/live methods; ownership, collection/reset, quota, and failure tests in a real consumer |
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
| Match Luau's compile-time versus runtime error behavior | Focused builtin fixes implemented; wider audit open | Eight new Luau 0.709 fixtures plus call-shape, expansion, argument-effect and repeated-suspension tests cover specialization fallback and builtin lookup order; final CI and review remain pending |
| Allow a required module to yield within the host loading contract | Implemented in the current follow-up; final review and CI pending | ABI v3 continuations, cache commit after completion, coroutine-owned cycle/error cleanup, shared fuel, close/collection/reset, and installation rollback tests |
| Confirm collection settings suit long-running applications | Correctness coverage exists; tuning open | Preserve the deterministic scheduling contract; use remote measurements for any tuning or performance claim |
| Retain clear source diagnostics and recovery behavior | Implemented | Keep the diagnostics and error/fuzz regressions green; add fixtures for newly found gaps |

The current corpus has 720 fixtures covering 517 named behaviors with zero pending
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

This document records the private milestone and the later public-release gates.
It does not implement the planned API, close runtime or consumer-validation
requirements, or authorize public publication.
