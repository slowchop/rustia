# Documentation review

Date: 2026-10-29 (Australia/Hobart). Reviewed source baseline: `b84b5f6715e31756f00d1f39e72c8246f179b174`, followed by the documentation fixes in this change.

## Verdict and boundaries

Rustia needs a clearer route through its existing documentation, not a book or a documentation website. The most useful additions explain choices that currently require reading several API pages. The most useful cuts remove duplicate status, chronology and implementation detail from the user path.

Source access, supported releases and accepting outside contributions are separate decisions. Both source repositories remain private and packages remain unpublished. An outside contribution, issue-reporting or support policy has not been chosen. This review does not choose one, invite submissions, enable Issues, change repository protections, publish documentation or authorize packages.

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
| Benchmark instructions implied local timings and active source workflows. | Put paused source automation and CI-only measurements first; label historical/private protocols. | [Source profile](public-source-profile.json) and project benchmark policy. |

The concurrent [design/source drift review](../2026-10-29_DRIFT_REPORT.md) is retained alongside this review. Integration keeps its protocol gaps and contract corrections; the README source-reference policy has one implementation, with current-path coverage added to its existing reachable-commit checks.

## Add next: explanations that cross API boundaries

### 1. One complete browser setup recipe

The README advertises WebAssembly, but the [browser demo](../demos/live-wasm/README.md) uses bare Wasm with default features disabled. Its instruction to use the wasm-bindgen toolchain for JavaScript services is not a complete alternative setup path. The crate introduction explains `web-live` but does not show how an application loads it.

Add a short recipe showing one tested integration: exact prerequisites, Cargo features, build command, loading code, a returned value and output behavior. Explicitly distinguish enabling JavaScript adapters from selecting `RustiaBuilder::live()`. Label the existing raw-Wasm demo by what it does. Do not claim browser support from a compile-only check.

Place it beside the demo or in an existing reader-facing document, then link it from README and Rustdoc. Do not add a separate website, tool wrapper or browser API for documentation alone.

Acceptance: a reader can follow the commands, see a result, and explain whether the example uses bare Wasm or JavaScript services. Verify the generated artifact in a browser before calling the recipe complete.

### 2. A small failure and reuse comparison

Ordinary execution, transactional installation, typed invocations and hardened entries have different failure boundaries. The details exist in [Chunk](../crates/rustia/src/chunk.rs), [Function](../crates/rustia/src/function.rs), [typed invocations](../crates/rustia/src/invocation.rs) and the [hardened tests](../crates/rustia/tests/hardened_leaf.rs). Readers should not need to reconstruct the differences from all four.

Add a compact comparison beside the relevant Rustdoc task links: which guest writes survive, when staged records publish, whether handles remain usable, and whether reset is required or automatic. Start with ordinary execution and installation; put the advanced cases after those. Explain that reset does not undo host I/O or Rust payload mutation. Do not imply that every script error poisons the VM.

Acceptance: a reader can choose the right operation and predict what remains after a failure. Reuse existing recovery tests as the source of truth.

### 3. A callback-state choice recipe

Readers can currently discover `add_fn`, `HostFunction`, borrowed `HostContext` and mutable Rust objects only by comparing several references. Add a short choice table with one-sentence reasons and links to the existing examples. Make clear that the engine trusts declared capabilities; it cannot inspect callback captures. Explain `'static` captures versus data borrowed for one invocation.

Start with the simple callback and mutable-object cases. Put typed staged game changes after them. Avoid presenting a multi-associated-type host schema as a prerequisite for calling one Rust method.

### 4. A correctness-only hardened-decision example

The existing leaf lifecycle example is a measurement program and cannot be run on a development machine under project policy. A small non-timing example would make the supported workflow accessible: mandatory memory limit, one compiled entry, a successful decision, a bounded failure, no published records after that failure, and the documented reset behavior.

Reuse the existing hardened contract tests. Do not introduce a new execution abstraction or a second hardened API.

## Change and simplify next

### Shorten CURRENT without losing decisions

The nominal one-paragraph state spans hundreds of lines, and the file contains competing action queues. It mixes current guarantees, completed rollout chronology, dated benchmark experiments and historical plans. Some later paragraphs still describe active benchmark workflows or unfinished diagnostics after the top section records their newer state.

Keep one current summary, implemented contract links, genuine open gates and one next-action list. Move completed chronology into its existing dated acceptance/audit record, preserving citations and decisions. Review this as its own change: CURRENT is authoritative, so a wholesale rewrite without checking every retained requirement would be risky. Do not delete evidence to achieve a line target.

### Turn existing examples into teaching tools

Add short purpose/run/expected-result headers and a few comments marking the important steps. Introduce simple mutable objects before deterministic staged game changes. Remove redundant selection of default profiles and incidental `unwrap` calls from introductory examples when changing those examples; use their existing correctness checks to verify the edits.

The first Serde recipe should list the application's direct `serde` dependency and derive feature. Show dense arrays versus sparse/mixed tables, checked integer conversion, and copied versus identity-preserving results where readers choose a conversion. Method-level details belong beside the methods, not in a new serialization chapter.

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
3. Get visible CLI output and exit its REPL. Separately, follow the future browser recipe and identify its feature and service choices.
4. Find a lifecycle or compatibility rule in Rustdoc, then follow a link to its detailed contract or test without navigating the entire design archive.

Fresh application, README, Rustdoc, CLI and repository preflight checks can establish technical correctness. They do not replace later observation of real readers.

## Validation and remaining decisions

The documentation patch requires the unchanged-commit full release-preflight inventory through the normal push guard and all applicable hosted CI. README source references have local reachability/path checks; a fresh external application should exercise the actual Git dependency as well. No local timing or profiling is part of this review.

Remaining decisions are outside contribution/issue/support policy, eventual source visibility, repository protection settings and package/release acceptance. They are not silently resolved by fixing documentation.
