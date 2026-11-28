# Rustia build and maintenance notes

Rustia is available for inspection and experimental use under MIT OR Apache-2.0.
Start with the [README](README.md) to try the API.

## Contributing

1. **Report bugs in public.** Search [GitHub Issues](https://github.com/slowchop/rustia/issues)
   for duplicates. Include the Rustia version or commit, a small reproduction,
   and the expected and actual results. Report vulnerabilities privately through
   the route in [SECURITY.md](SECURITY.md).
2. **Open focused pull requests.** Small fixes to docs, examples, and tests are
   welcome. Discuss new features, public API changes, and interpreter behavior
   changes in an issue before starting the implementation.
3. **Maintainers approve CI and merges.** External fork CI runs after a
   maintainer approves it. Public CI uses standard GitHub-hosted runners.
   Merging requires independent review and passing applicable CI checks.
4. **Use LLMs with verifiable results.** LLM assistance is welcome. Review the
   whole diff, explain the change, and be ready to answer questions and revise
   it. If an LLM drafted a substantial part of the change, tests, or technical
   analysis, briefly say what it helped with in the issue or PR. Routine
   autocomplete and copyediting need no disclosure.

   - **Bug report.** Run the reproducer on the revision you report and share
     what happened.
   - **Code fix.** Give the exact test command and result. For a behavior fix,
     show that the regression test fails before the fix and passes with it.
   - **Docs fix.** Say what you checked, such as the rendered page you previewed
     or the example you ran. A read-through is enough for a typo fix.

## Issue-driven development

For maintainers, [GitHub Issues](https://github.com/slowchop/rustia/issues) is the primary development
backlog. It tracks bugs, features, documentation, maintenance, priorities and work
in progress. Search existing open and closed issues before creating one. Each
issue should name a concrete outcome, relevant design/oracle evidence and checks
that establish completion. Use the bug or work-item template when helpful.

1. Select or create an issue before starting a substantive change. Read its
   discussion, linked design and existing PRs to avoid duplicate work. Record
   the working branch or PR and current progress on the issue.
2. Implement the scoped change with its tests and required design/conformance
   updates. File newly discovered independent work as linked follow-up issues.
3. Link the PR with `Closes #N` when it completes the whole issue, or `Refs #N`
   for partial work. Record validation and remaining acceptance items.
4. Obtain independent review, pass hosted release preflight and other applicable CI, and merge
   with a merge commit. Verify the result on main and the issue's final state.
   A parent tracker stays open until all its acceptance gates are resolved.

Issues own work status and priority. [CURRENT](design/CURRENT.md) records built
behavior and constraints; design docs and the pinned Luau oracle remain the
specification. Keep backlog checklists in Issues rather than duplicating them in
CURRENT or chat. Private infrastructure tasks belong in `rustia-ops`.
The experimental source release has no support or review deadline.

## Build and test

Install [Rust through rustup](https://rustup.rs/) and Node.js 22. Rustup selects the
compiler and components pinned in `rust-toolchain.toml` when you enter this
checkout. [Just](https://github.com/casey/just) is optional; its recipes are
shortcuts for commands you can also run directly.

```sh
git clone https://github.com/slowchop/rustia.git
cd rustia
node tools/install-git-hooks.js
cargo test --locked -p rustia
cargo run --locked -p rustia --example embedding
cargo run --locked -p rustia-cli -- --eval "print(40 + 2)"
```

`just install-hooks` is equivalent to the Node command above. Install this
checkout-local guard before pushing. For a frozen commit, prefer
`just validated-push`. It checks formatting, shared copy, and crate metadata,
then rechecks the clean checkout and pushes that exact commit. Ordinary
`git push` runs the same local checks through the hook. The complete hosted
correctness checks gate merges. See [the validation workflow](design/97-validation-workflow.md).

While making a change, run the tests closest to the code you edited. These checks
are a useful baseline:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
node tools/check-readme-examples.js
```

The README check compiles and runs its Rust examples. The complete release check
is `just release-preflight`, or `node tools/run-release-preflight.js`. It also
checks package contents, generated reports, the separate fuzzing workspace, and
additional build configurations. Hosted correctness CI runs it for pull requests.
It builds dependencies for several scripting engines, so the first run may need
more disk space than the focused tests.

**Do not run benchmarks on your development machine.** This includes
`cargo bench`, benchmark examples, profiling, and quick timing comparisons. Local
builds, correctness tests, and static checks are allowed; their elapsed times are
not performance evidence. This source repository has hosted correctness CI only,
with no `ci-bench` workflow or private runner access. Trusted maintainers can
manually review and dispatch exact-cohort runtime comparisons through a separate
private bridge. Review raw data independently before acceptance and retain it
privately. The public-source profile selects a reviewed aggregate for the
website charts. Automatic per-commit measurement, history and landscape refresh
remain paused. Only accepted `ci-bench` results can support performance decisions.

## Find your way around

Read [AGENTS.md](AGENTS.md) and [the current project state](design/CURRENT.md)
before changing code. `CURRENT.md` explains what is implemented and links to the
development backlog; other design documents can contain older plans.

| Directory | What you will find |
| --- | --- |
| `crates/rustia` | The embedding API and runnable Rust examples |
| `crates/rustia-cli` | The command-line runner |
| `crates/rustia-lexer`, `rustia-parser`, `rustia-compiler`, `rustia-vm` | Language implementation |
| `conformance` | Small scripts and expected results checked against the pinned language version |
| `design` | Behavior contracts, decisions, and release plans |
| `tools` | Repository and release checks |
| `bench` | Benchmark workloads, retained reports, and measurement tools |
| `fuzz` | Bounded randomized tests for parser and runtime failures |
| `spikes` | Historical experiments; these are not the supported API |

## Maintain behavior and documentation together

Rustia targets **standalone Luau 0.709**. It does not provide Roblox services.
When changing language behavior, check the relevant [design document](design/)
and [compatibility coverage](conformance/COVERAGE.md), and compare against the
pinned reference interpreter described in [the conformance guide](conformance/README.md).
Update the implementation, focused test, behavior documentation, and coverage row
together. Update `design/CURRENT.md` when the project's implemented state changes.
Keep user-facing documentation clear and show runnable examples early.

Reproducible execution is a core promise for the environments that select it.
Do not introduce hidden state, random hashing, wall-clock execution limits, or
platform-dependent math into those paths. `RustiaBuilder::live()` deliberately
allows real-world services such as clocks and entropy; preserve that option.
Use structured errors for bad scripts and host input. Public workspace crates
forbid unsafe code.

The experimental API is available from Git. All workspace crates are unpublished.
Necessary API improvements should update examples,
documentation, and active clients in the same rollout. The
[release checklist](design/90-release-readiness.md) and
[versioning policy](design/91-semver-policy.md) explain the release contract.

Generated reports name their source and regeneration command. Change the source
and rerun the generator; avoid editing generated output by hand. CI checks for
stale output.

## Maintainer delivery

Describe the problem, resulting behavior, and checks run for each change. Record
pending measurements. Use a **merge commit** to preserve reviewed commit identities.
Checks and independent reviews must pass without blocking findings; never bypass
protection, reviews, or failing checks.

Check the live ruleset and merge settings before delivery. Preserve the required
checks and use merge commits so the recorded source revisions stay reachable.

## Keep committed material public-safe

Everything committed here must be suitable for a public repository. Keep
credentials, infrastructure addresses, runner configuration, private tasks, and
client-specific integration details out of source and hosted logs. Private
operations material belongs in the separately versioned, git-ignored `private/`
checkout (`rustia-ops`). It is optional and is not needed to build or use Rustia.
Keep local agent notes and caches out of source control too.

The source uses the dual **MIT OR Apache-2.0** license; see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
