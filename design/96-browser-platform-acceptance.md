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

## Remaining public platform gates

Firefox on macOS arm64 is executed evidence, not a claim of the entire platform
matrix. Remote Firefox and Safari execution remain unverified. Windows, Linux
arm64, Intel Mac and the promised mobile targets still need native correctness
and deterministic-result evidence; the current cross-compilation jobs prove
only builds. The manually dispatched Windows job in `ci.yml` prepares native MSVC
execution of the shared transcript and embedding contracts in both feature modes.
Repository access to the Windows runner group was approved and verified.
The first Windows jobs failed before tests while starting rustup through Bash.
The runner also lacks PowerShell Core (`pwsh`). The follow-up uses Windows
PowerShell (`powershell`) to invoke the native Rust tools and explicitly
adds Cargo home to later steps. Windows correctness remains unverified until
that job succeeds. Keep the full matrix in documents 00, 90 and 92 until each target
has actual execution evidence.
