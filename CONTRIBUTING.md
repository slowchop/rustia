# Rustia build and maintenance notes

Source access, package releases, and outside contributions are separate decisions.
The policy for accepting outside contributions has not been decided. These notes
explain how to build the source and maintain the repository; they do not establish
an issue-reporting or review service. Start with the [README](README.md) to try the API.

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
checkout-local guard before pushing. Every push must pass the full
`just release-preflight` inventory through the hook on the unchanged commit;
focused checks do not replace it. See [the validation workflow](design/97-validation-workflow.md).

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
not performance evidence. This source repository currently has hosted correctness
CI and no `ci-bench` workflow. New measurements are pending a trusted private
benchmark bridge. Only `ci-bench` results can support speed claims or benchmark
updates; leave measurements pending while that service is unavailable.

## Find your way around

Read [AGENTS.md](AGENTS.md) and [the current project state](design/CURRENT.md)
before changing code. `CURRENT.md` explains what is implemented and what remains;
other design documents can contain older plans.

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

The API is still unpublished. Necessary API improvements should update examples,
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

As of private staging on 2026-10-29, the new repository's protection and merge
settings await approval: no required-check ruleset is configured, auto-merge is
off, and squash/rebase remain available. The intended configuration requires
checks, permits only merge commits, and enables auto-merge when changes are ready.
That configuration has not yet been applied.

## Keep committed material public-safe

Everything committed here must be suitable for a public repository. Keep
credentials, infrastructure addresses, runner configuration, private tasks, and
client-specific integration details out of source and hosted logs. Private
operations material belongs in the separately versioned, git-ignored `private/`
checkout (`rustia-ops`). It is optional and is not needed to build or use Rustia.
Keep local agent notes and caches out of source control too.

The source uses the dual **MIT OR Apache-2.0** license; see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
