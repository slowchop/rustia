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
to the Linux arm64 result. Node/V8 job `121348804515` executed ninety tests in
eighteen groups with zero failures and the same two goldens. All eight target
build jobs also passed. Aggregate CI and headless WebKit are still pending;
this evidence does not establish performance acceptance.


The same runtime's manual CI completed successfully. WebKit job `121348804132`
executed 88 tests across sixteen groups with zero failures and two identical
goldens, matching all ten transcripts across the five recorded runtimes.
Documentation-only integration `c4185197` preserved runtime and build inputs
and passed [PR CI](https://github.com/slowchop/rustia/actions/runs/40254807221)
and [differential smoke](https://github.com/slowchop/rustia/actions/runs/40254807181)
(27 matches, zero differences). Its Mac, Node and WebKit logs re-established
120/16, 90/18 and 88/16 tests/groups respectively; workspace Linux goldens also
match. All eight target builds and the required aggregate passed. These results
remain historical after the caller-proof lookup correction; final-runtime
correctness and performance gates must be checked again.


## Caller-proof lookup correction

Runtime `6fd0e482`, delivered as `6b0122bd`, passed native Linux arm64
correctness from an exact source archive with the same pinned Rust 1.95.0
image. Default and bare modes executed 120 tests across sixteen groups with
zero failures and two byte-identical golden transcripts. The retained log is
`/private/tmp/rustia-linux-arm64-6fd0e482.log`; the corresponding acceptance
note records the source archive and image digest.

Both [PR CI](https://github.com/slowchop/rustia/actions/runs/40259959422) and
[manual platform CI](https://github.com/slowchop/rustia/actions/runs/40260017553)
passed on delivery head `6b0122bd`. Actual platform logs establish:

| Runtime | Manual CI job | Tests | Groups | Goldens |
|---|---:|---:|---:|---:|
| macOS arm64 | 121370153631 | 120 | 16 | 2 |
| Windows x86-64 | 121370153909 | 120 | 16 | 2 |
| Node/V8 | 121370153944 | 90 | 18 | 2 |
| Headless WebKit | 121370153877 | 88 | 16 | 2 |

Every group passed with zero failures; every golden matches native Linux
arm64 byte-for-byte. Both runs passed all eight target builds. PR workspace
job `121369974430` records 1,022 passes across 63 groups; release job
`121369974428` records 311 passes across 36 groups, including packaged facade
acceptance in default and bare modes. Required aggregate `121375169183`
verified successful preflight and WebKit inputs. Manual CI also passed all
sixteen jobs, including aggregate `121376187277`.

[Current-head differential smoke](https://github.com/slowchop/rustia/actions/runs/40259959466)
matched all 27 oracle cases with zero differences and passed its execution
test. These correctness results do not establish performance acceptance.
Intel Mac and mobile runtime execution remain open; real Safari remains a
release-candidate gate.

The subsequent non-vector table-probe and unary-math argument-copy removals
change runtime code again. The evidence above remains tied to `6fd0e482`; the
new candidate must re-establish native ARM and final-head platform gates.


## Redundant table-probe and unary-math copy removal

Runtime `550d9b1b`, delivered as `8c96cf24`, passed native Linux arm64 from
an exact source archive with the pinned Rust 1.95.0 image above. The retained
log `/private/tmp/rustia-linux-arm64-550d9b1b.log` establishes 120 passing tests
in sixteen groups, zero failures, and two golden transcripts byte-identical
to the earlier platforms. The companion acceptance note records source and
image identity.

[PR CI](https://github.com/slowchop/rustia/actions/runs/40282602943) passed all
fifteen applicable jobs; Windows is intentionally covered by the
[manual CI run](https://github.com/slowchop/rustia/actions/runs/40282662208),
which passed all sixteen jobs. Independently audited manual platform logs are:

| Runtime | Job | Tests | Groups | Goldens |
|---|---:|---:|---:|---:|
| macOS arm64 | 121439184545 | 120 | 16 | 2 |
| Windows x86-64 | 121439184582 | 120 | 16 | 2 |
| Node/V8 | 121439184624 | 90 | 18 | 2 |
| Headless WebKit | 121439184253 | 88 | 16 | 2 |

All groups passed with zero failures and matching golden transcripts. Both
runs passed all eight target builds. Workspace jobs `121439006108` and
`121439184599` each report 1,022 passes across 63 groups; release jobs
`121439006049` and `121439184499` each report 311 passes across 36 groups,
including the twelve-suite archived facade checks in default and bare modes.
Aggregates `121444811481` and `121445198112` verified successful preflight
and WebKit inputs. [Differential smoke](https://github.com/slowchop/rustia/actions/runs/40282603016)
matched all 27 cases with zero differences and passed its execution test.
Performance acceptance remains separate. Intel Mac and mobile execution and
real Safari release-candidate checks remain open release requirements.

The fresh-frame synchronization correction `3b776b31` passes native Linux
ARM64 correctness from its exact Git source archive in the pinned Rust 1.95.0
bookworm image. Default and bare modes execute 122 tests across 16 groups,
with zero failures and two matching platform golden transcripts. This includes
the new suspended-caller capture mutation regression in each feature mode.
Remote platform evidence must still be checked for delivery head `4846cd1e`;
these correctness results do not establish performance acceptance.

Delivery `4846cd1e` passes all fifteen applicable jobs in
[PR CI](https://github.com/slowchop/rustia/actions/runs/40291330000).
[Manual CI](https://github.com/slowchop/rustia/actions/runs/40291366559)
has fifteen successes and one Windows infrastructure failure. Both runs
independently verify macOS 122 tests/16 groups, Node 92/18, and WebKit 90/16,
with zero test failures, the new regression twice, and two matching goldens
each. All eight target builds pass in both runs. Workspace jobs each execute
1,023 tests across 63 groups; release jobs each execute 313 across 36 groups,
including archived facade acceptance in both feature modes. Both aggregates
receive successful preflight and WebKit results. Differential smoke reports
27 matches and zero differences.

Windows job `121466967072` lost runner communication during checkout; toolchain
installation and tests never began, and its log endpoint returns BlobNotFound.
The failure annotation and job metadata are retained. This is not current-head
Windows correctness evidence: successful Windows execution remains required.


## Isolated unary math argument borrowing

Runtime `ecbd774d`, delivered as `6b06808b`, reverts the table-probe and
fresh-frame scan experiments while retaining borrowed unary math arguments.
Native Linux arm64 correctness from its exact Git source archive passes 122
tests across sixteen groups with zero failures and two matching golden
transcripts, retained in `/private/tmp/rustia-linux-arm64-ecbd774d.log`.

[PR CI](https://github.com/slowchop/rustia/actions/runs/40305528112) passes all
fifteen applicable jobs and [manual CI](https://github.com/slowchop/rustia/actions/runs/40305723455)
passes all sixteen. Windows job `121514224792` executes 122 tests across sixteen
groups with zero failures and two matching goldens; the earlier checkout
infrastructure failure remains historical evidence. Both runs independently
verify macOS 122 tests/16 groups, Node 92/18 and WebKit 90/16, with zero failures
and two matching goldens each. All eight target-build logs verify successful
`cargo check` commands.

Workspace jobs `121513566983` and `121514224844` each report 1,023 passes across
63 groups. Release jobs `121513567046` and `121514224779` each report 313 across
36 groups, including twelve reused external archived facade suites in default
and bare modes. Both stages have zero failures and two matching goldens. Both
aggregate jobs explicitly receive successful preflight and WebKit inputs.
[Differential smoke](https://github.com/slowchop/rustia/actions/runs/40305528187)
matches all 27 cases with zero differences and passes its execution test.
These results establish correctness evidence for this candidate, not performance
acceptance. Intel Mac and mobile runtime execution remain open release gates;
real Safari remains a release-candidate check.


The subsequent out-of-line unary-math wrapper is runtime `2e347dad`, delivered
as `457d7715`. Its exact source archive passes native Linux arm64 correctness:
122 tests across sixteen groups, zero failures and two matching goldens,
retained in `/private/tmp/rustia-linux-arm64-2e347dad.log`.

[PR CI](https://github.com/slowchop/rustia/actions/runs/40311547800) passes all
fifteen applicable jobs; [manual CI](https://github.com/slowchop/rustia/actions/runs/40311573884)
passes all sixteen. Independent log audits verify macOS and Windows 122 tests
across sixteen groups, Node 92/18 and WebKit 90/16, all with zero failures,
two matching goldens and the suspended-capture regression in both feature modes.
All sixteen target-build logs across the two runs verify the intended target
commands and successful compilation.

Workspace jobs `121533697826` and `121533794690` each execute 1,023 tests across
63 groups; release jobs `121533697744` and `121533795048` each execute 313 across
36 groups, including twelve external archived facade suites in default and bare
modes. Both stages have zero failures and two matching goldens; both aggregates
receive successful preflight and WebKit inputs. [Differential smoke](https://github.com/slowchop/rustia/actions/runs/40311547738)
verifies 27 matches, zero differences and a passing execution test. Performance
acceptance remains separate; the open public-release platform gates remain.


The shared unary-math value operation is runtime `12d16851`, delivered as
`6a8b61b1`. Exact-source Linux arm64 execution passes 122 tests across sixteen
groups, zero failures and two matching goldens; the retained log is
`/private/tmp/rustia-linux-arm64-12d16851.log`.

[PR CI](https://github.com/slowchop/rustia/actions/runs/40317696020) passes all
fifteen applicable jobs; [manual CI](https://github.com/slowchop/rustia/actions/runs/40317811651)
passes all sixteen. Independent completed-log audits verify macOS and Windows
122 tests/16 groups, Node 92/18 and WebKit 90/16, all with zero failures,
the suspended-capture regression twice and two matching golden transcripts.
All sixteen target-build logs across both workflows verify successful target
compilation.

Workspace jobs `121554321268` and `121554727199` each execute 1,025 tests across
63 groups, including the new native/opcode validation-order and public unary
call-shape regressions. Release jobs `121554321360` and `121554727337` each
execute 313 tests across 36 groups, including twelve archived facade suites in
default and bare modes. All have zero failures and two matching goldens.
Aggregates `121561950917` and `121563157648` receive successful preflight and
WebKit inputs. [Differential smoke](https://github.com/slowchop/rustia/actions/runs/40317695934)
verifies 27 matches, zero differences and a passing execution test. This is
correctness evidence, not performance acceptance; public-release platform
gates remain open as above.


## Iterator-slot candidate (historical after revert)

Runtime `cd1071cfaf626959bcec0120d0c8806b442a3e61`, delivery
`2eaf304e24bde9e0bccf304106c11b561a343df9`, passes
[PR CI](https://github.com/slowchop/rustia/actions/runs/40324655813) and
[manual CI](https://github.com/slowchop/rustia/actions/runs/40324774754).
Both workspace logs execute 1,026 tests in 63 groups, including the new iterator
regression. Both release logs execute 313 tests in 36 groups, including twelve
archived facade suites in each feature mode. Both aggregate gates receive
successful preflight and WebKit inputs. All eight target builds pass in each run.

Each macOS and Windows run executes 122 tests in 16 groups; each Node run
executes 92 in 18; each WebKit run executes 90 in 16. All have zero failures
and two matching goldens. Exact-source native Linux ARM64 independently executes
122 tests in 16 groups with two matching goldens. The differential run matches
all 27 cases without differences. Candidate-only serde screening is not
performance acceptance. The failed performance isolation and ensuing production
revert are retained in [95](95-numeric-index-contract.md).


## Scalar metadata candidate (historical after iterator correction)

Runtime `12521d6016d7810eb729520802c507ff4d0c7b44`, delivery
`7cbdd8665f9b12ceb966039c173fc28dda425e8c`, passes
[PR CI](https://github.com/slowchop/rustia/actions/runs/40332537230) and
[manual CI](https://github.com/slowchop/rustia/actions/runs/40332724032).
Each workspace log executes 1,026 tests in 63 groups, including the retained
array regression and expanded closure guard regression. Each release log
executes 313 tests in 36 groups, including twelve archived facade suites in
both feature modes. Both aggregate gates explicitly receive successful
preflight and WebKit inputs. All sixteen target-build logs across the two runs
show successful checks.

Each macOS and Windows run executes 122 tests in 16 groups; each Node run
executes 92 in 18; each WebKit run executes 90 in 16. Exact-source native Linux
ARM64 executes 122 tests in 16 groups. All audited logs have zero failures and
two matching goldens. Differential smoke matches 27 cases without differences.
Candidate-only serde screening is not acceptance. The failed performance
isolation and pending iterator correction remain recorded in [95](95-numeric-index-contract.md).
