# Browser correctness acceptance

## PR checks and release candidates

Every PR runs the same browser suites in headless Playwright WebKit on Docker-capable
Linux runners. Playwright is pinned to 1.64.1 in `tools/browser/package-lock.json`;
the matching official container is pinned by digest and includes its browser and
system libraries. A macOS service-account trial timed out launching WebKit, so
PR execution does not depend on a logged-in Mac desktop. This
uses no Safari window or Safari automation permission. The required Release
preflight status also waits for this job and rejects failure or skipped execution.

`webkit-runner.cjs` starts the pinned wasm-bindgen test server on an ephemeral
loopback port and opens it with Playwright WebKit in headless mode. It rejects
missing, zero-test, failed and duplicate result summaries, bounds startup and
test waits, and closes the browser and server after each suite. The shared
golden script still requires exactly one executed transcript. Local validation
passed all 88 tests in 16 groups with matching bare/default golden transcripts
on WebKit 27.2 (Playwright build 2370). Remote execution is also verified
for the checked-cast candidate in the evidence below.

WebKit is not the Safari product shipped by Apple. The manually dispatched
`browser-correctness.yml` remains release-candidate acceptance in real Firefox
and Safari. Safari requires a prepared, logged-in desktop session and explicit
automation permission; it is never launched by routine PR CI. Keep its version
and execution evidence separate from Playwright WebKit.


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

Safari uses visible automation windows and requires a logged-in desktop session;
it has no true headless mode. The pinned wasm-bindgen runner prints “headless”
for its automatic WebDriver path, including Safari. Setting `NO_HEADLESS` would
instead start a manual test server and is not a fix. The script compiles both
feature modes before browser execution, announces Safari's visible behavior,
and reports completion only after both modes pass. It never edits permissions.

On 2026-10-24 (Australia/Hobart), Safari 26.6.2 on macOS arm64 passed
all 88 tests in 16 groups at
`ea751050b96ca135bb592e2d7beb3150f327bc38`, including two identical shared
golden transcripts across bare and default features. Rust 1.95.0 and wasm-bindgen 0.2.126 were used. This is local real-browser
correctness evidence; remote runner readiness and performance are not implied.

An earlier run at `bfe5a84a` passed all 45 bare-mode tests before automation was
disabled. The diagnostic log shows an AppKit UI action immediately before Safari
wrote its WebDriver preferences and terminated the session. It does not identify
the control or source of the action; the user reports no stop control was clicked.
A later attempt to enable the setting through UI automation triggered Safari's
click-interference protection and required the user to authenticate. After that
authentication, closing Settings and running without UI interaction completed
both modes. This establishes a working setup, not a proven cause for the earlier
disabling. The script never changes permissions. The pinned runner closes the
test window and kills its driver during cleanup; a printed SIGKILL alone is not
a Safari crash diagnosis. Local diagnostic logs stay outside the repository.

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

## Checked-cast candidate execution

On 2026-10-26 (Australia/Hobart), candidate
`f54b077427e9a332ce4f43241aa2045c31c5368f` passed the shared contracts in
default and bare feature modes. Actual job logs, rather than workflow status
alone, establish the counts below.

| Runtime | Passing tests | Groups | Evidence |
|---|---:|---:|---|
| Native macOS arm64 | 120 | 16 | [PR CI job](https://github.com/slowchop/rustia/actions/runs/40234605092/job/121293369338) |
| Headless Playwright WebKit | 88 | 16 | [PR CI job](https://github.com/slowchop/rustia/actions/runs/40234605092/job/121293369066) |
| Node/V8 WebAssembly | 90 | 18 | [PR CI job](https://github.com/slowchop/rustia/actions/runs/40234605092/job/121293369159) |
| Native Windows x86-64 MSVC | 120 | 16 | [Manual CI job](https://github.com/slowchop/rustia/actions/runs/40234679489/job/121293588462) |
| Native Linux arm64 | 120 | 16 | Exact-source local correctness run described below |

Each runtime emitted two byte-identical shared transcripts. All ten transcripts
also match across the five runtimes. These are correctness results, not timing
measurements or an assertion of complete target conformance.

The Linux arm64 run extracted `git archive` of the exact candidate into a native
aarch64 Linux Docker VM, without instruction-set emulation. The image was
`rust:1.95.0-bookworm@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1`.
The run checked `uname -m` as `aarch64` and Rust 1.95.0's compiler host as
`aarch64-unknown-linux-gnu`, then ran `platform_golden`, `host_functions`,
`typed_objects`, `live_value_contract`, `module_resolution`, `yieldable_require`,
`callable_continuations` and `numeric_index_contract` with explicit target
`aarch64-unknown-linux-gnu`, in both feature modes. The retained local log is
`/private/tmp/rustia-linux-arm64-f54b0774.log`; its evidence note is
`/private/tmp/rustia-linux-arm64-f54b0774-acceptance.md`.
An earlier exact-source Linux arm64 run at `78b783c9` also passed 120 tests and
two goldens; its separate log and note remain retained. The new run supplies
execution evidence for the changed runtime rather than inheriting that result.

## Remaining public platform gates

Firefox on macOS arm64 and native Windows execution do not close the entire
platform matrix. Remote Firefox and Safari execution remain unverified; local real-browser runs
passed for both engines. Native Linux arm64 execution is verified for the
shared contracts above. Intel Mac and the promised mobile targets still need
native correctness and deterministic-result evidence; their current
cross-compilation jobs prove only builds. Keep the full matrix in documents 00, 90 and 92 until each target
has actual execution evidence. Windows remains a manually dispatched CI job;
its successful run must be checked separately from ordinary pull-request CI.

The string-dispatch extraction candidate `c1913422` independently passed the
same native Linux arm64 contract suite: 120 tests across 16 groups, default and
bare modes, with two byte-identical golden transcripts. This used an exact
source archive and the same pinned image above. The log and evidence note are
`/private/tmp/rustia-linux-arm64-c1913422.log` and
`/private/tmp/rustia-linux-arm64-c1913422-acceptance.md`. The same runtime, at delivery head
`02c0b1749b68baee91463207e63ee1632794b20e`, passed the remaining shared
platform contracts on 2026-10-26 (Australia/Hobart):

| Runtime | Passing tests | Groups | Evidence |
|---|---:|---:|---|
| Native macOS arm64 | 120 | 16 | [Manual CI job](https://github.com/slowchop/rustia/actions/runs/40246586880/job/121329330294) |
| Native Windows x86-64 MSVC | 120 | 16 | [Manual CI job](https://github.com/slowchop/rustia/actions/runs/40246586880/job/121329330318) |
| Node/V8 WebAssembly | 90 | 18 | [PR CI job](https://github.com/slowchop/rustia/actions/runs/40246584261/job/121329319338) |
| Headless Playwright WebKit | 88 | 16 | [Manual CI job](https://github.com/slowchop/rustia/actions/runs/40246586880/job/121329330289) |

Actual logs establish zero failures and two identical transcripts on each runtime.
All ten transcripts, including native Linux arm64, match byte for byte. The
first PR WebKit job was terminated when its runner's user service stopped;
Docker's socket disappeared during the test. That infrastructure failure is
retained, and the failed PR jobs were retried after Docker recovered. The
independent manual CI run completed successfully. The
[retried PR WebKit job](https://github.com/slowchop/rustia/actions/runs/40246584261/job/121335358325)
also passed all 88 tests and both goldens, and the aggregate PR CI passed.
These results do not establish
performance acceptance or close Intel Mac, mobile or real-browser release gates.

## Caller-cell proof candidate

Runtime `aa432f17` independently passed native Linux arm64 correctness from an
exact source archive using the same pinned Rust 1.95.0 image above. Default and
bare modes passed 120 tests across sixteen groups, with zero failures. Its two
byte-identical golden transcripts match the earlier `c1913422` result. The log
and evidence note are `/private/tmp/rustia-linux-arm64-aa432f17.log` and
`/private/tmp/rustia-linux-arm64-aa432f17-acceptance.md`. This is correctness
evidence only.

Delivery head `4be30aca` also passed native macOS arm64 and Windows x86-64
in the [new candidate's platform run](https://github.com/slowchop/rustia/actions/runs/40253022712).
The actual logs for jobs `121348804463` and `121348804611` each record 120 tests
across sixteen groups, zero failures, and two golden transcripts byte-identical
to the Linux arm64 result. Aggregate CI and the remaining browser checks are
still pending; this evidence does not establish performance acceptance.
