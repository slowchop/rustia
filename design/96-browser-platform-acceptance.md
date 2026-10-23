# Browser correctness acceptance

The first browser acceptance runs the same fixed transcript as native and Node
checks, plus the live adapters, host callbacks, typed objects, live-value
contract, yieldable modules, callable continuations and numeric-index contract.
Both default and bare feature modes must execute tests; a zero-test success is
an error. These are correctness runs, never benchmark evidence.

## Reproducible runner

`tools/run-browser-correctness.sh` selects an explicit Firefox or Safari driver,
checks exact browser/tool versions, and runs the suites with the pinned
wasm-bindgen 0.2.126 tools and Rust 1.95.0. Firefox's checked binary is passed
explicitly to geckodriver. Alternative engine overrides are rejected. No Node,
Chrome or simulated-browser fallback is accepted.

The manual `browser-correctness.yml` workflow uses generic Linux and Mac pools.
Exact installed browser versions are dispatch inputs; browser installation and
Safari automation permission are host prerequisites, not implicit CI mutations.
Every eligible pool member must satisfy the prerequisites before enabling these
jobs for ordinary pull requests. Local browser success does not prove remote
runner readiness.

## Observed evidence

On 2026-10-23 (Australia/Hobart), the runtime at
`04d502590448c14355f645b27ff14b2194cd98fe` passed real Firefox 158.0.1 on
macOS arm64 using geckodriver 0.37.1, Rust 1.95.0 and wasm-bindgen 0.2.126.
Both feature modes passed: 16 test groups, 88 executed tests, including the
unchanged shared transcript and the new numeric-index boundary test. This was
run using the browser script introduced in this change. No timing claim is made.

Safari 26.6.2 could not create a WebDriver session: it reports that Allow Remote
Automation is disabled. The user approved enabling automation, but the enable command requires local
administrator authentication. No Safari test has passed; coverage remains open
until the setting is enabled and a real run succeeds.

## Windows execution

On 2026-10-24 (Australia/Hobart), commit
`2bcfd4bef3f4a077008b21900f39a149118a9cc3` passed native Windows x86-64 MSVC
in [CI run 40021456000](https://github.com/slowchop/rustia/actions/runs/40021456000/job/120647451654).
The log contains 120 passing tests across default and bare feature modes,
including two identical shared golden transcripts, callbacks, typed objects,
live values, module resolution, yieldable modules, callable continuations and
the numeric-index contract. This is correctness evidence, not a timing claim.

The first Windows jobs failed before tests while starting the shared rustup
installation. The runner also lacks PowerShell Core (`pwsh`). The working job
uses Windows PowerShell (`powershell`) and bootstraps its own Rust installation
under the runner temporary directory with checksum-verified rustup 1.28.2 and
Rust 1.95.0. It exports those isolated Cargo/Rustup homes and executable path to
later steps; the shared host installation is not modified.

## Remaining public platform gates

Firefox on macOS arm64 and native Windows execution do not close the entire
platform matrix. Remote Firefox and Safari execution remain unverified. Linux
arm64, Intel Mac and the promised mobile targets still need native correctness
and deterministic-result evidence; their current cross-compilation jobs prove
only builds. Keep the full matrix in documents 00, 90 and 92 until each target
has actual execution evidence. Windows remains a manually dispatched CI job;
its successful run must be checked separately from ordinary pull-request CI.
