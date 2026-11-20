# Private platform and facade acceptance

The [private acceptance record](93-private-api-acceptance.md) identifies the
source candidate and links its final CI/review/merge evidence. Publication stays
disabled and the version stays
`0.0.1`.

## Executed targets

The private milestone requires real execution on Linux x86-64, macOS arm64,
and Node/V8 Wasm. These are correctness checks; no timings are collected or
used as performance evidence.

| CI lane | Executed coverage |
|---|---|
| Linux release preflight | Workspace tests/conformance, the shared golden in default and bare modes, and external archived-facade contracts |
| Native macOS arm64 | The shared golden, host functions, typed objects, live values, module resolution, yieldable loaders, and callable continuations in default and bare modes |
| Node/V8 Wasm | The same golden in default and bare modes, alongside the existing host-adapter, object, callback, live-value, yieldable-loader and callable-continuation suites |

The native jobs verify the actual OS and architecture and pass their target
triples explicitly to Cargo. The Mac job also checks the compiler host triple;
runner configuration cannot silently substitute Intel execution for arm64.
The private Mac job used the runner account’s Rustup installation. The current
[hosted macOS job](../.github/workflows/ci.yml) installs Rust 1.95.0 for
`aarch64-apple-darwin` using the pinned toolchain action. Compiler warnings are errors.
CI logs the tested checkout and the exact golden transcript.
The shared runner requires exactly one transcript sentinel, rejecting empty
test runs as well as failed execution. The private profile's policy self-tests
exercised wrong placement, missing checks and zero, duplicate or failed golden
execution. The hosted jobs still require the sentinel at runtime.

The separate eight-target engine build matrix remains unchanged. Build success
for Intel macOS, Linux arm64, Windows, iOS and Android is not runtime evidence.
The [browser and platform record](96-browser-platform-acceptance.md) records later
Firefox, Safari and mobile execution on specific candidates. Native Intel Mac and
remote desktop Safari remain open. Full supported-release acceptance requires testing
the final candidate.

## One shared transcript

`crates/rustia/tests/platform_golden.rs` executes through the public facade on
all three targets. Each run checks the same fixed bytes under both
`DeterministicSim` and `ReproducibleStandard`, then repeats after reset.

The transcript covers f64 NaN/infinity/negative-zero arithmetic, selected libm
results, ordered table iteration, a seeded random sequence, buffer byte order,
coroutine yield/resume, and a cyclic table. Numeric edge cases retain the
existing VM golden. The seeded sequence comes from the pinned Luau 0.709
conformance fixture. This is a bounded regression corpus, not proof of universal
determinism or complete language compatibility. Host/object lifecycle suites
remain separate so this transcript stays small and easy to review.

Local correctness commands are:

```sh
bash tools/run-private-platform-golden.sh native default aarch64-apple-darwin
bash tools/run-private-platform-golden.sh native bare aarch64-apple-darwin
```

On Linux x86-64, substitute `x86_64-unknown-linux-gnu` for the native target.
For Wasm, use the runner's `wasm` mode and the lock-matched
`wasm-bindgen-test-runner`. Local Darwin/arm64 execution can supplement CI;
it does not count as a completed Mac CI job.

## Clean external facade contracts

`node tools/check-external-facade.js` packages and extracts the five runtime
crates without publication. It verifies production source bytes against the
checkout and creates a fresh Cargo application outside the workspace. That
application depends directly on only `rustia` and `serde`; internal packages
are patched to their extracted archives.

The harness copies existing public contract tests from the facade archive,
checking that their bytes match the canonical sources. There is no second
copy of those tests to maintain. The selected suites cover values/Serde, live
cycles and aliases, callback bounds and results, typed objects and journals,
typed errors, limits/settings, reset, successful/failed module installation,
module cache/failure contracts, coroutine-owned module suspension, and callable
receiver recursion/yielding.
Core-only compiler tests are excluded from the application.

The candidate's lock seeds offline resolution. The harness permits no package
version/source/checksum absent from that lock, apart from its own application
root. The application uses the repository's pinned Rust toolchain. It then runs
all contracts with `--locked --offline` in default and bare
feature modes, forwarding the facade's `web-live` feature to the copied tests.
This tests the actual private archives without relying on a
registry placeholder or allowing a moving dependency update. It establishes
neither registry installation nor docs.rs publication.

The final immutable candidate, active-client pins, runtime-gap work, required CI,
independent reviews, and remote benchmark evidence are recorded in
[the private acceptance record](93-private-api-acceptance.md). Wider public-release
gates remain in [the release plan](92-first-release.md).
