# Documentation review

Date: 2026-10-29 (Australia/Hobart). Reviewed source baseline: `b84b5f6715e31756f00d1f39e72c8246f179b174`, followed by the documentation fixes in this change.

## Verdict and boundaries

Rustia needs a clearer route through its existing documentation, not a book or a documentation website. The most useful additions explain choices that currently require reading several API pages. The most useful cuts remove duplicate status, chronology and implementation detail from the user path.

At the October review, both repositories were private and the contribution policy was undecided. The [November launch record](public-source-activation.md) documents public source access and collaborator-only issue and pull request creation.

Four agent reviews covered first use, advanced embedding contracts, documentation structure, and an independent check of the resulting diff. Findings were compared with source, existing tests and the current source profile. These are technical and editorial reviews, not observations from usability tests with outside readers.

## What exists

At the baseline, the repository contains 130 tracked Markdown files, including 69 EGM code maps and 34 numbered design documents. Markdown under `design/` occupies about 12,500 lines. `CURRENT.md` and `bench/README.md` each exceed 1,100 lines. File counts describe the reviewed baseline, not a new metric to maintain.

The useful existing path is:

1. [README](../README.md): purpose, first executable example, environment choice, and links to tasks.
2. [Facade Rustdoc source](../crates/rustia/src/lib.rs): API choices and behavior next to the methods.
3. [Runnable examples](../crates/rustia/examples/): complete workflows.
4. [Build and maintenance notes](../CONTRIBUTING.md): repository work, separate from using the library.
5. [Current state](CURRENT.md) and [categorized design index](README.md): implementation contracts, dated acceptance records and research for maintainers.

EGM maps explain code boundaries. They do not need to become another end-user guide. Detailed specifications and evidence remain valuable without being prerequisites for embedding.

## Fixed in this change

| Problem | Change | Evidence or check |
| --- | --- | --- |
| The Cargo example pinned an original commit absent from the rewritten repo. | Pin the tested, reachable source baseline. Verify the actual pin and source-link targets independently of the Rust fence tests. | [README source checker](../tools/check-readme-comparisons.js); negative cases cover missing commits and missing source paths. |
| Original PR and Actions links pointed at the reused repository name. | Retarget those Markdown links to `rustia-historic`; map immutable README source links to cleaned commits. | Historical evidence remains private. No measurements or source IDs inside evidence records are rewritten. |
| Source access was presented as an invitation to contribute. | Remove invitations and review/support promises; state the policy is undecided. Keep useful maintainer instructions. | README, CONTRIBUTING, AGENTS, current state and release/source-access plans agree. |
| The CLI expression example printed nothing. | Use `print(40 + 2)`, explain discarded returns and the REPL expression form. | The CLI executes source with `exec`; the REPL separately formats expression results. |
| Product and API entry points followed repository-import mechanics and comparisons. | Put purpose, examples and API tasks first; keep provenance under project status. | No benchmark table values or evidence were changed. |
| Execution choices and unlimited defaults were easy to miss. | Add an `exec`/`eval`/`install` choice table and state that ordinary defaults impose no fuel or accounted-memory limit. | Existing chunk contracts and builder defaults. |
| Examples were difficult to discover. | Add task links for callbacks, objects, live values, modules, hot reload, host transactions and diagnostics. | Existing examples are reused rather than copied into another tutorial. |
| Builder docs promised once-per-epoch module execution. | Explain the module dependency cache and fresh installation cohorts. | [Resolver docs](../crates/rustia/src/extension_api.rs) and `each_install_has_a_fresh_lazy_require_cohort` in [installation tests](../crates/rustia/tests/transactional_install.rs). |
| Public error variants and diagnostic fields lacked explanations. | Document triggers, units, missing/truncated data, reporting limits and reset guidance; add a bounded fuel-error example. | Source and existing recovery tests; independent review corrected rejected-change wording and fallback report-budget scope. |
| Readers had to infer data ownership and threading rules. | Add owned/live/Serde choices, define reset generation, and state the worker-local VM rule. | Existing value and VM contracts; no API or implementation changes. |
| The design index repeated obsolete counts, rollout labels and research claims. | Categorize links and remove duplicate live-status claims. | Current status remains in CURRENT; specs, raw research and acceptance evidence remain linked. |
| Benchmark instructions implied local timings and active source workflows. | Put paused public benchmark automation and CI-only measurements first; distinguish available private manual runtime comparisons from public history and publication. | [Source profile](public-source-profile.json) and project benchmark policy. |

The concurrent [design/source drift review](../2026-10-29_DRIFT_REPORT.md) is retained alongside this review. Integration keeps its protocol gaps and contract corrections; the README source-reference policy has one implementation, with current-path coverage added to its existing reachable-commit checks.

## Explanations that cross API boundaries

### 1. One complete browser setup recipe

Implemented as [the browser setup recipe](../demos/web-live/README.md), linked from README and Rustdoc. The standalone application pins its dependencies, uses the matching wasm-bindgen CLI, initializes the generated JavaScript module, returns `42` to the page, and prints `Hello from Luau` to the browser console. It sets explicit fuel and memory limits and explains default `web-live` features separately from selecting `RustiaBuilder::live()`.

The existing [raw-Wasm demo](../demos/live-wasm/README.md) is labeled as bare Wasm using the standard preset without stdout. No Rustia API or timing tool was added. The new application's exported function is its own JavaScript boundary.

Local Playwright 1.64.1/WebKit 27.2 executed the generated application on 2026-10-30: returned value and console output pass on two calls and after reload, with no page errors. The existing hosted WebKit job builds the documented artifact and executes [the same correctness check](../tools/browser/web-live-example.cjs); workflow fixtures reject missing build or execution commands. This evidence does not close real Safari or mobile release gates.

### 2. A small failure and reuse comparison

Implemented in the [crate introduction](../crates/rustia/src/lib.rs). The comparison starts with ordinary execution and installation, then covers typed invocations and hardened entries. It identifies guest-write rollback, staged-record publication, handle validity and reset behavior. The accompanying guidance distinguishes ordinary errors from contained host panics and explains why neither reset nor installation undoes host I/O or mutable Rust payload changes.

The existing [recovery tests](../crates/rustia/tests/error_recovery.rs), [installation tests](../crates/rustia/tests/transactional_install.rs), [typed invocation tests](../crates/rustia/tests/typed_invocation.rs) and [hardened lifecycle tests](../crates/rustia/tests/support/hardened_transactions.rs) remain the source of truth. This adds guidance, not a new rollback contract.

### 3. A callback-state choice recipe

Implemented beside the failure comparison in the [crate introduction](../crates/rustia/src/lib.rs). A choice table links simple callbacks, mutable objects, shared host definitions, borrowed invocation context and staged effects to the existing examples. It explains that `'static` callbacks may own captures, while `HostContext` borrows application data for one call. Declared capabilities remain an embedding trust boundary; Rustia cannot inspect captures for hidden mutable state.

### 4. A correctness-only hardened-decision example

Implemented as [hardened_decision.rs](../crates/rustia/examples/hardened_decision.rs), linked from README and Rustdoc. Run `cargo run --locked -p rustia --example hardened_decision`. It prints `success: 7; rejected: record limit; recovered: 9; epochs: 3` after checking all three calls. The example sets explicit memory, fuel, record-count and logical-record-byte budgets; reuses one compiled entry; checks that a second staged command exceeds the record budget without publishing either command; and checks automatic reset after success and failure before successful reuse.

Release preflight and `just check-examples` execute this correctness example. It uses the existing entry lifecycle covered by [hardened transaction tests](../crates/rustia/tests/support/hardened_transactions.rs), introduces no API, and takes no timings. The separate `leaf_lifecycle_bench` remains a CI-only measurement program.

## Change and simplify next

### CURRENT compaction and guide placement (2026-10-30)

Implemented: CURRENT now keeps current permissions, compact contract links, validation/measurement scope and one open-work list. The [pre-compaction snapshot](CURRENT-2026-10-30-snapshot.md) preserves its former text verbatim behind a historical banner. Detailed rollout chronology and dated measurement results are no longer prerequisites for reading current status. Active failure, determinism, memory, experimental-optimization and release caveats remain in CURRENT; no evidence or acceptance threshold was deleted.

Practical library guides belong in the crate Rustdoc introduction, with reference details beside methods. Browser prerequisites, build commands and JavaScript loading belong beside the browser demo, linked from README and Rustdoc. This uses the existing reading path; it does not add a documentation website or another live status page.

### Finish teaching guidance for examples

The introductory pass is implemented in [embedding](../crates/rustia/examples/embedding.rs), [chunk builder](../crates/rustia/examples/chunk_builder.rs), [mutable objects](../crates/rustia/examples/mutable_objects.rs) and [live values](../crates/rustia/examples/live_values.rs). Each starts with its purpose, repository-root run command and expected outcome, with a few comments marking the integration boundaries. README already presents simple mutable objects before staged game objects. The live-value example removes redundant default-profile selection, propagates builder errors, and requires the typed `StaleRetainedValue` error after reset. `just check-examples` and release preflight execute all four assertion sets alongside game objects and hardened decisions; all six pass.

The embedding header now lists the application's direct `serde` dependency and derive feature. Remaining conversion guidance should show dense arrays versus sparse/mixed tables, checked integer conversion, and copied versus identity-preserving results where readers choose a conversion. Extend useful headers to the more advanced examples when touching them. Method-level details belong beside the methods, not in a new serialization chapter.

### Treat the progress dashboard as a dated snapshot

[progress.json](progress.json) is dated 2026-10-18. It still describes required modules crossing a non-yieldable boundary and an old accepted cohort as latest. The generated HTML repeats that snapshot. Relabel it as dated planning evidence or remove it from current-state navigation. Initially retain artifacts and renderer checks; do not add another status generator simply to synchronize this duplicate layer.

### Improve facade completeness gradually

The strict Rustdoc build catches broken Rustdoc links and warnings but does not currently enable `missing_docs`. Passing it is not proof that all public variants, fields, methods or workflows are explained. Finish meaningful facade gaps, then consider a facade-only missing-doc check. Avoid a workspace-wide campaign of comments that merely repeat names.

## Remove from the default reading path

- Import, rewrite and benchmark-operation mechanics before the first usable example. Retain them in source-readiness and evidence records.
- Duplicate live corpus counts, rollout statuses and next-action lists. Link the canonical source instead.
- Historical research presented as shipped product behavior, especially JIT spike claims. Keep the research with an explicit historical role.
- The implied expectation to contribute, report Issues, receive support or wait for review. Those channels require a separate policy decision.
- Copied code tutorials that duplicate existing runnable examples without explaining a new choice.

Do not remove specs, benchmark evidence, the original history, EGM maps or tests simply because they are not for ordinary users. Removing something from onboarding is not deleting it from the repository.

## Work to cut or defer

A documentation website, full book, tutorial series, separate status service, new execution tier, new wrapper API and public contribution process do not address the concrete reader problems above. Defer them until actual usage identifies a need.

Keeping the conventional CONTRIBUTING filename is not a contribution invitation when its content says otherwise. Renaming it to DEVELOPMENT is optional editorial cleanup, not a prerequisite for source access.

## Reader acceptance tasks

1. Create a fresh Rust application from the dependency and hello-world instructions, obtain `42`, then read a small Serde struct without consulting a workspace manifest or design specification.
2. Set explicit budgets, trigger and classify an error, show a named-source report, and explain whether guest writes, external effects and handles survive.
3. Get visible CLI output and exit its REPL. Separately, follow the browser recipe and identify its feature and service choices.
4. Find a lifecycle or compatibility rule in Rustdoc, then follow a link to its detailed contract or test without navigating the entire design archive.

Fresh application, README, Rustdoc, CLI and repository preflight checks can establish technical correctness. They do not replace later observation of real readers.

## Validation and remaining decisions

The documentation patch requires the unchanged-commit full release-preflight inventory through the normal push guard and all applicable hosted CI. README source references have local reachability/path checks; a fresh external application should exercise the actual Git dependency as well. No local timing or profiling is part of this review.

At the review date, visibility, outside contributions, support, repository protections and package acceptance were open decisions. See [current state](CURRENT.md) for their status.
