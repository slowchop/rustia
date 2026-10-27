# Contributing to Rustia

Thanks for helping make Rustia a useful, dependable scripting engine. Bug reports,
clearer examples, and small fixes are all welcome. Start with the
[README](README.md) if you want to try the API first.

## Build and test

Install [Rust through rustup](https://rustup.rs/) and Node.js 22. Rustup selects the
compiler and components pinned in `rust-toolchain.toml` when you enter this
checkout. [Just](https://github.com/casey/just) is optional; its recipes are
shortcuts for commands you can also run directly.

```sh
git clone https://github.com/slowchop/rustia.git
cd rustia
cargo test --locked -p rustia
cargo run --locked -p rustia --example embedding
cargo run --locked -p rustia-cli -- --eval "return 40 + 2"
```

For a change, run the tests closest to the code you edited. Before opening a pull
request, these checks are a useful baseline:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
node tools/check-readme-examples.js
```

The README check compiles and runs its Rust examples, so readers can trust the
code they copy. The complete release check is `just release-preflight`, or
`node tools/run-release-preflight.js`. It also checks package contents, generated
reports, the separate fuzzing workspace, and additional build configurations.
CI runs it for pull requests. It builds dependencies for several scripting
engines, so the first run may need more disk space than the focused tests.

**Do not run benchmarks on your development machine.** This includes
`cargo bench`, benchmark examples, profiling, and quick timing comparisons. Local
builds, correctness tests, and static checks are welcome; their elapsed times are
not performance evidence. Push your branch and open a pull request to use its
remote `ci-bench` check. Only results from that check can support speed claims or
benchmark updates. If it is unavailable, leave the measurement pending.

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
| `bench` | Benchmark workloads, reports, and remote measurement tools |
| `fuzz` | Bounded randomized tests for parser and runtime failures |
| `spikes` | Historical experiments; these are not the supported API |

## Make a change easy to review

Keep changes focused and explain what a user will notice. For a bug report,
include a small script or Rust example, the result you expected, and the result
you observed. Include your Rustia revision and platform when they matter. The
issue tracker is not open during the private preview; include a report with a
pull request or bring it to a maintainer. Public bugs and features will use this
repository's GitHub Issues when the project opens for release.

Source access does not mean the crates have been released or the API is stable.
Support and review are best-effort; a report or contribution does not guarantee
an implementation or response date. Keep suspected credentials, private data and
security-sensitive reproduction details out of public reports. Before opening
the repository, maintainers must establish a private security-reporting channel.

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

## Pull requests and merging

Describe the problem, resulting behavior, and checks you ran. Mention any pending
remote measurement. Required CI checks and independent reviews must pass without
blocking findings. Enable auto-merge when the pull request is ready so these
requirements can gate delivery.

Use a **merge commit**. Squash and rebase merging are disabled because benchmark
history refers to the original reviewed commit identities. Do not bypass branch
protection, required reviews, or failing checks.

## Keep this repository public-safe

Everything committed here must be suitable for a public repository. Keep
credentials, infrastructure addresses, runner configuration, private tasks, and
client-specific integration details out of commits, issues, and public logs.
Maintainers keep private operations material in the separately versioned,
git-ignored `private/` checkout (`rustia-ops`). It is optional and is not needed
to build or use Rustia. Public workflows identify self-hosted runners only by
generic labels. Keep local agent notes and caches out of source control too.

Contributions use the project's dual **MIT OR Apache-2.0** license; see
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
