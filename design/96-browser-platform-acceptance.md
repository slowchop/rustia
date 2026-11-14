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

WebKit is not the Safari product shipped by Apple. Real Firefox and Safari
remain release-candidate acceptance requirements. The private
`browser-correctness.yml` workflow is historical and is absent from the active
correctness profile; the retained local runner supports explicit real-browser checks.
Safari requires a prepared, logged-in desktop session and explicit automation
permission; routine PR CI never launches it. Keep its version and execution
evidence separate from Playwright WebKit.


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

The historical private `browser-correctness.yml` workflow used generic Linux
and Mac pools with exact browser versions as dispatch inputs. It is absent from
this tree. The retained local script accepts those versions explicitly; browser
installation and Safari automation permission remain prerequisites, not implicit
mutations. The active PR gate uses headless WebKit. Local real-browser success
does not prove remote Firefox/Safari runner readiness; that requirement remains open.

## Manual remote Firefox execution

The active `ci.yml` includes a private, `workflow_dispatch`-only real Firefox job
on the existing generic Linux Docker correctness pool. It uses the same pinned
container as the WebKit job for dependencies, but launches the official Mozilla
Firefox 160.0 release through geckodriver 0.37.1. Playwright's patched Firefox is
not selected. Downloaded archives are verified before extraction against the
committed Mozilla SHA-512 and upstream geckodriver SHA-256, then installed only
inside the disposable job container's temporary directory. A Firefox-only wrapper
uses its own temporary browser home and drops inherited desktop authentication/
runtime paths. Cargo, Rustup, Git and runner-mounted home ownership stay unchanged.

The job invokes the existing browser runner, which verifies versions and
requires eighteen nonzero passing suite groups and both goldens across default
and bare modes. The workflow policy checks the private/manual guard, ordinary
Docker pool, pinned image/tools/checksums and actual execution command. Negative
fixtures reject PR execution, missing checksum checks and omitted execution.
Existing required PR checks remain in place. No host/browser permission setting,
runner-group access or benchmark authorization is changed.

After a reviewed branch is pushed, invoke `gh workflow run ci.yml --ref BRANCH`.
The dispatch runs the existing Windows/shared checks and real Firefox. Inspect
actual Firefox job logs, test summaries and goldens on that exact head before
claiming remote acceptance. Adding the job alone is not execution evidence.
Remote Safari still requires a prepared logged-in Mac and existing automation
permission; mobile Safari is a separate physical-device check.

### Observed remote Firefox result — 2026-11-13

Immutable head `65701387649ad51fe59067682030c9a4daa10060` executed real Firefox
160.0 with geckodriver 0.37.1 on the existing Linux x86-64 Docker correctness
pool in [manual run 43364203240, job 131684665212](https://github.com/slowchop/rustia/actions/runs/43364203240/job/131684665212).
Actual raw logs contain eighteen nonzero passing summaries: bare 49, default 47,
96 tests total, zero failures. Both type-grammar tests executed in each mode.
Both golden payloads match the retained native reference byte-for-byte, SHA-256
`cf890817d7b66cc2e8ab61030cbe6d0fe3235efe1c8a3dd7a1e4bf94f3b6a9a8`.
Rust 1.95.0 and wasm-bindgen/test-runner 0.2.126 were used. The job verified the
committed official browser/driver archive checksums before extraction; the
Firefox-only temporary home resolved the runner/container ownership mismatch.
This establishes remote Firefox shared-contract execution for that exact head,
not remote Safari, physical-iPad Safari, full language parity or a later release candidate.

## Explicit remote Safari execution

The private `ci.yml` has a manual Safari job on the existing generic macOS ARM64
correctness pool. It is disabled by default and runs only for a private
`workflow_dispatch` with the boolean `safari` input set. Ordinary PR/main checks
and normal Windows/Firefox dispatches do not open Safari windows.

The job first requires the runner account to own the logged-in console desktop
and have a live GUI session, then checks the exact installed `safari_version`.
The readiness step prints only console-ownership and GUI-domain-presence
booleans, not account names. Missing prerequisites fail before browser tests
rather than reporting an empty pass. It uses account-provisioned Rustup and checksum-pinned wasm-bindgen 0.2.126
binaries in the job's temporary directory. No system package install, runner
access change or Safari automation permission toggle is performed. Existing
Safari automation permission is required. The normal browser runner executes
both feature modes and rejects zero-test or missing-golden results.

After independent review and the normal push guard, request the visible browser
execution explicitly:

```bash
gh workflow run ci.yml --ref BRANCH -f safari=true -f safari_version=26.6.2
```

Use the exact version installed on the selected prepared runner. A failed GUI,
version or automation prerequisite is an unfulfilled acceptance gate, not Safari
execution evidence. Inspect actual eighteen-group test results and both goldens
on the immutable head before accepting the remote Safari item in #40.

### Intel Mac deferral — 2026-11-15

The user has no Intel Mac and explicitly deferred that portion of #31. #39 stays
open as the separate future native-Intel execution task. Existing Linux ARM64,
Windows and Mac ARM64 evidence remains delivered. Native Intel is unverified;
the Apple Silicon Safari path does not substitute for it. The full release
matrix in design/00 retains Intel Mac as a future requirement. #31 can close
once its remaining non-deferred remote Safari acceptance is actually met.

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
in [CI run 40021456000](https://github.com/slowchop/rustia-historic/actions/runs/40021456000/job/120647451654).
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
| Native macOS arm64 | 120 | 16 | [PR CI job](https://github.com/slowchop/rustia-historic/actions/runs/40234605092/job/121293369338) |
| Headless Playwright WebKit | 88 | 16 | [PR CI job](https://github.com/slowchop/rustia-historic/actions/runs/40234605092/job/121293369066) |
| Node/V8 WebAssembly | 90 | 18 | [PR CI job](https://github.com/slowchop/rustia-historic/actions/runs/40234605092/job/121293369159) |
| Native Windows x86-64 MSVC | 120 | 16 | [Manual CI job](https://github.com/slowchop/rustia-historic/actions/runs/40234679489/job/121293588462) |
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
| Native macOS arm64 | 120 | 16 | [Manual CI job](https://github.com/slowchop/rustia-historic/actions/runs/40246586880/job/121329330294) |
| Native Windows x86-64 MSVC | 120 | 16 | [Manual CI job](https://github.com/slowchop/rustia-historic/actions/runs/40246586880/job/121329330318) |
| Node/V8 WebAssembly | 90 | 18 | [PR CI job](https://github.com/slowchop/rustia-historic/actions/runs/40246584261/job/121329319338) |
| Headless Playwright WebKit | 88 | 16 | [Manual CI job](https://github.com/slowchop/rustia-historic/actions/runs/40246586880/job/121329330289) |

Actual logs establish zero failures and two identical transcripts on each runtime.
All ten transcripts, including native Linux arm64, match byte for byte. The
first PR WebKit job was terminated when its runner's user service stopped;
Docker's socket disappeared during the test. That infrastructure failure is
retained, and the failed PR jobs were retried after Docker recovered. The
independent manual CI run completed successfully. The
[retried PR WebKit job](https://github.com/slowchop/rustia-historic/actions/runs/40246584261/job/121335358325)
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
in the [new candidate's platform run](https://github.com/slowchop/rustia-historic/actions/runs/40253022712).
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
and passed [PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40254807221)
and [differential smoke](https://github.com/slowchop/rustia-historic/actions/runs/40254807181)
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

Both [PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40259959422) and
[manual platform CI](https://github.com/slowchop/rustia-historic/actions/runs/40260017553)
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

[Current-head differential smoke](https://github.com/slowchop/rustia-historic/actions/runs/40259959466)
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

[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40282602943) passed all
fifteen applicable jobs; Windows is intentionally covered by the
[manual CI run](https://github.com/slowchop/rustia-historic/actions/runs/40282662208),
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
and WebKit inputs. [Differential smoke](https://github.com/slowchop/rustia-historic/actions/runs/40282603016)
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
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40291330000).
[Manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40291366559)
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

[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40305528112) passes all
fifteen applicable jobs and [manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40305723455)
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
[Differential smoke](https://github.com/slowchop/rustia-historic/actions/runs/40305528187)
matches all 27 cases with zero differences and passes its execution test.
These results establish correctness evidence for this candidate, not performance
acceptance. Intel Mac and mobile runtime execution remain open release gates;
real Safari remains a release-candidate check.


The subsequent out-of-line unary-math wrapper is runtime `2e347dad`, delivered
as `457d7715`. Its exact source archive passes native Linux arm64 correctness:
122 tests across sixteen groups, zero failures and two matching goldens,
retained in `/private/tmp/rustia-linux-arm64-2e347dad.log`.

[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40311547800) passes all
fifteen applicable jobs; [manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40311573884)
passes all sixteen. Independent log audits verify macOS and Windows 122 tests
across sixteen groups, Node 92/18 and WebKit 90/16, all with zero failures,
two matching goldens and the suspended-capture regression in both feature modes.
All sixteen target-build logs across the two runs verify the intended target
commands and successful compilation.

Workspace jobs `121533697826` and `121533794690` each execute 1,023 tests across
63 groups; release jobs `121533697744` and `121533795048` each execute 313 across
36 groups, including twelve external archived facade suites in default and bare
modes. Both stages have zero failures and two matching goldens; both aggregates
receive successful preflight and WebKit inputs. [Differential smoke](https://github.com/slowchop/rustia-historic/actions/runs/40311547738)
verifies 27 matches, zero differences and a passing execution test. Performance
acceptance remains separate; the open public-release platform gates remain.


The shared unary-math value operation is runtime `12d16851`, delivered as
`6a8b61b1`. Exact-source Linux arm64 execution passes 122 tests across sixteen
groups, zero failures and two matching goldens; the retained log is
`/private/tmp/rustia-linux-arm64-12d16851.log`.

[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40317696020) passes all
fifteen applicable jobs; [manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40317811651)
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
WebKit inputs. [Differential smoke](https://github.com/slowchop/rustia-historic/actions/runs/40317695934)
verifies 27 matches, zero differences and a passing execution test. This is
correctness evidence, not performance acceptance; public-release platform
gates remain open as above.


## Iterator-slot candidate (historical after revert)

Runtime `cd1071cfaf626959bcec0120d0c8806b442a3e61`, delivery
`2eaf304e24bde9e0bccf304106c11b561a343df9`, passes
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40324655813) and
[manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40324774754).
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
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40332537230) and
[manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40332724032).
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


## Iterator-resolution candidate (historical after probe correction)

Runtime `e292ef2b67e8497bac9bb399d80b9d7bd2e7d59a`, delivery
`c754371f0b7622a4ab811ff0c4ed97bae7e6fbc2`, passes
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40341828709) and
[manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40341952742).
Each workspace log executes 1,028 tests in 63 groups; each release log executes
315 tests in 36 groups, including twelve archived facade suites in both feature
modes. Both aggregate gates explicitly receive successful preflight and WebKit
inputs. All sixteen target-build logs across the two runs verify successful checks.

Each macOS and Windows run executes 124 tests in 16 groups; each Node run
executes 94 in 18; each WebKit run executes 92 in 16. Exact-source native Linux
ARM64 independently executes 124 tests in 16 groups. Every audited runtime log
has zero failures and two matching goldens, including the new iterator coroutine
regression in both feature modes. Differential smoke matches 27 cases without
differences. Candidate-only screening is not acceptance. Failed original-baseline
performance and the next isolated probe correction remain recorded in [95](95-numeric-index-contract.md).


## Vector-probe candidate (historical after revert)

Runtime `8a1c6ac4aa32fe9bf1aa29218ff7d86a2b20ceaa`, delivery
`f1e6de3648a263ef1e03be2313c420f0d27b1920`, passes
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40361453671) and
[manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40361508072).
Each workspace log executes 1,028 tests in 63 groups; each release log executes
315 tests in 36 groups, including twelve archived facade suites in both feature
modes. Both aggregate gates have successful preflight and WebKit inputs. All
sixteen target-build logs across the two runs verify successful checks.

Each macOS and Windows run executes 124 tests in 16 groups; each Node run
executes 94 in 18; each WebKit run executes 92 in 16. Exact-source native Linux
ARM64 executes 124 tests in 16 groups. All have zero failures and two matching
goldens. Differential smoke matches 27 cases without differences. Candidate-only
screening is not acceptance. The failed isolated measurement, production revert
and next boundary-guard correction remain recorded in [95](95-numeric-index-contract.md).


## Combined-boundary candidate (historical after revert)

Runtime `dc2adfd2418972987a6a22c3036ed53cba746072`, delivery
`7b450eb33051aaa89c9fb99e64ee67a9b6b8670f`, passes
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40369390185) and
[manual CI](https://github.com/slowchop/rustia-historic/actions/runs/40369563630).
Each workspace log executes 1,029 tests in 63 groups; each release log executes
315 tests in 36 groups, including twelve archived facade suites in both feature
modes. Both aggregate gates explicitly receive successful preflight and WebKit
inputs. All sixteen target-build logs verify successful checks.

Each macOS and Windows run executes 124 tests in 16 groups; each Node run
executes 94 in 18; each WebKit run executes 92 in 16. Exact-source native Linux
ARM64 executes 124 tests in 16 groups. Every audited runtime log has zero
failures and two matching goldens. Differential smoke matches 27 cases without
differences. Candidate-only screening is not acceptance. The inconclusive
performance isolation and production revert remain recorded in
[95](95-numeric-index-contract.md); these checks do not validate a later runtime.


## Cycle-diagnostic tooling deliveries (historical after tape-first correction)

Tooling heads `e4985396`, `b840a94d` and `844df778` pass their respective
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40378938461),
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40382863978) and
[PR CI](https://github.com/slowchop/rustia-historic/actions/runs/40387282169).
Each independently audited workspace log executes 1,029 tests in 63 groups;
each release log executes 315 tests in 36 groups, including twelve archived
facade suites across both feature modes. All eight target builds per run pass;
the aggregate gates receive successful preflight and WebKit inputs.
Each macOS run passes 124 tests in 16 groups, Node passes 94 in 18 and WebKit
passes 92 in 16, with zero failures and two matching goldens. Differential
checks match 27 cases without differences. Screening is candidate-only.
These tooling deliveries retain the previous runtime and do not validate the
next tape-first runtime correction. Its exact-head platform checks remain pending.


PR #74 delivery `60077eafa66b60e217aa74a65da1c4483ce3073b` passes PR CI
[40394468138](https://github.com/slowchop/rustia-historic/actions/runs/40394468138)
and manual CI [40394576399](https://github.com/slowchop/rustia-historic/actions/runs/40394576399).
Independent artifact audits verify 1,029 workspace tests in 63 summaries and
315 release tests in 36 summaries per run, including twelve archived facade
suites; all eight target builds pass in each run. Mac and Windows pass 124 shared
contracts, Node passes 94 and WebKit passes 92, with matching goldens and zero
failures. Native Linux ARM64 passes 124 contracts in sixteen summaries and two
goldens from the exact `7478e426286bbdb3762315f33d533097c9dd9b19` runtime archive.
Differential run 40394468203 verifies 27 matches and zero differences plus smoke.
These are historical correctness results once the tape-handoff correction changes
runtime code; performance rejection remains recorded in design 95.


## Direct tape-completion runtime evidence

Runtime `f116e72731004af43bdee8fafefc40a05fd3a7ad`, delivered as
`b5982752df3711c593b5249908bea01651f4364e`, passes
[PR CI 40402650611](https://github.com/slowchop/rustia-historic/actions/runs/40402650611)
and [manual CI 40402811341](https://github.com/slowchop/rustia-historic/actions/runs/40402811341).
Independent audits verify all 33 completed job artifacts. Each workspace stage
passes 1,029 tests in 63 groups, each release stage passes 315 tests in 36 groups
with twelve archived facade suites, and both aggregate gates receive successful
preflight and WebKit inputs. All sixteen target-build jobs (eight per run) finish
successfully on the exact delivery head.

Mac and Windows each pass 124 shared-contract tests in sixteen groups; Node
passes 94 in eighteen groups and WebKit passes 92 in sixteen. All runtime jobs
have zero failures and two matching golden transcripts. Native Linux ARM64
passes 124 tests in sixteen groups and two matching goldens from a source archive
byte-for-byte equal to `git archive f116e727`; its SHA-256 is
`68f0d60476385afbbf4ff08847127d616366f1f62195baaf36b8fedf119b8a9b`.
Differential run 40402650266 verifies 27 matches, zero differences and generated
smoke. Screening 40402650245 remains candidate-only. The completed mandatory
pre-push inventory has 98 summaries and 1,466 passes with zero failures.
Intel Mac and mobile runtime execution remain public-release gates; these target
builds do not claim those runtime checks. Real Safari remains RC-only.

## Type-grammar candidate execution — 2026-11-13

Immutable candidate `6bcf4f4cfb98064acd2ce0a4353fd997d48a8418` in
[PR #43](https://github.com/slowchop/rustia/pull/43) passed the following actual
execution checks. An independent artifact audit checked raw test summaries,
controller exits, Android executable hashes and all 1,746 Linux archive blobs.
These are shared correctness contracts, not full language or supported-release acceptance.

| Execution | Default | Bare | Groups total | Tools/target |
|---|---:|---:|---:|---|
| Native macOS ARM64 | 64 | 64 | 18 | [PR CI job](https://github.com/slowchop/rustia/actions/runs/43357089689/job/131661591797); Rust 1.95.0 |
| Native Windows x86-64 MSVC | 64 | 64 | 18 | [Manual CI job](https://github.com/slowchop/rustia/actions/runs/43357096085/job/131661618559); Rust 1.95.0 |
| Local real Safari 26.6.2 | 47 | 49 | 18 | macOS ARM64; system safaridriver |
| Local real Firefox 160.0 | 47 | 49 | 18 | macOS ARM64; geckodriver 0.37.1 |
| Physical iPad Pro 11-inch (M4), iPadOS 27.0.1 | 64 | 64 | 18 | `aarch64-apple-ios`; Xcode 26.4.1 (17E202) |
| Android 15 ARM64 emulator | 64 | 64 | 18 | `aarch64-linux-android`; NDK 28.2.13676358, API 35 |
| Native Linux ARM64 VM | 64 | 64 | 18 | `aarch64-unknown-linux-gnu`; pinned Rust image below |

Every group executed nonzero tests with zero failures. All fourteen golden payloads
are byte-identical, SHA-256
`cf890817d7b66cc2e8ab61030cbe6d0fe3235efe1c8a3dd7a1e4bf94f3b6a9a8`.
The current native macOS/Windows job logs include both type-grammar regressions
and their two golden transcripts; these match the retained native reference. Both new
`type_grammar` tests actually ran, including deep-nesting rejection before guest effects.
Rust was pinned to 1.95.0. Browser execution used wasm-bindgen/test-runner 0.2.126;
the temporary release archive was checked against its upstream SHA-256
`7df536babe345deb68828148dbdc71179118afdab42d83547c7cebfbf1426bd5`.

Android used the existing ARM64 AVD with Hypervisor.Framework acceleration:
`emulator -accel-check` succeeded, and the running ARM64 process explicitly used
`-accel on`. This is emulator execution, not physical Android hardware or an
Intel instruction-set emulator. Linux used a native aarch64 Docker VM and
`rust@sha256:6258907abe69656e41cd992e0b705cdcfabcbbe3db374f92ed2d47121282d4a1`,
with the exact candidate extracted by `git archive`, explicit native target and
both feature modes. Neither virtualization run is a performance measurement.

Local raw logs and manifests are retained outside Git: `rustia-38-safari.log`,
`rustia-38-firefox.log`, `rustia-38-linux-arm64.log`,
`rustia-ipad-6bcf4f4c-tests/` and `rustia-android-6bcf4f4c/` under the local temporary
directory. Device IDs, signing-team IDs, profile contents and private runner
configuration must stay outside this repository. Remote Firefox/Safari execution,
Intel Mac execution, mobile-browser execution and later release candidates remain
separate unclosed requirements in #31/#39/#40/#41.

### Reproduce native mobile contracts

Use a clean checkout at the exact recorded candidate and the pinned toolchain.
Do not change HEAD or source files during builds/execution. Keep each candidate's
build JSON, raw logs and summaries in a separate private output directory. Record
`git rev-parse HEAD`, `rustc -Vv`, target OS/version and actual device versus emulator.
The nine suites, in both default and `--no-default-features` modes, are:

```bash
suites=(platform_golden host_functions typed_objects live_value_contract
        module_resolution yieldable_require callable_continuations
        numeric_index_contract type_grammar)
args=()
for suite in "${suites[@]}"; do args+=(--test "$suite"); done
cargo test --locked -p rustia --target "$RUSTIA_MOBILE_TARGET" \
  --target-dir "$RUSTIA_OUTPUT/build" --no-run --message-format=json \
  "${args[@]}" > "$RUSTIA_OUTPUT/default-build.jsonl" \
  2> "$RUSTIA_OUTPUT/default-build.log"
cargo test --locked -p rustia --target "$RUSTIA_MOBILE_TARGET" \
  --target-dir "$RUSTIA_OUTPUT/build" --no-default-features \
  --no-run --message-format=json "${args[@]}" \
  > "$RUSTIA_OUTPUT/bare-build.jsonl" 2> "$RUSTIA_OUTPUT/bare-build.log"
```

Run with Bash `set -euo pipefail`. For each mode/suite select the `executable`
from its `compiler-artifact` JSON entry whose `target.name` equals that suite;
require exactly one matching test executable. Preserve its SHA-256. Do not take
an executable from a different build/mode or infer runtime success from compilation.

For iOS, `RUSTIA_MOBILE_TARGET=aarch64-apple-ios`. Use a paired physical device
with Developer Mode and an existing eligible development profile/signing identity.
Obtain device information with `xcrun devicectl device info details`; decode the
existing profile with `security cms -D -i` and inspect existing identities with
`security find-identity -v -p codesigning`. Store their identifiers privately.
No new profile, certificate, account permission or keychain change is part of this procedure.
For every executable, package into a fresh scratch app. Set `RUSTIA_IOS_PACKAGER`
to the absolute path of `tools/package-ios-test.py` from this reviewed tooling
delivery; the older recorded runtime candidate predates that helper. Keep the
helper outside that candidate checkout rather than dirtying its source:

```bash
python3 "$RUSTIA_IOS_PACKAGER" --executable "$RUSTIA_EXECUTABLE" \
  --profile "$RUSTIA_IOS_PROFILE" --identity "$RUSTIA_IOS_IDENTITY" \
  --device-udid "$RUSTIA_IOS_UDID" --bundle-id "$RUSTIA_TEST_BUNDLE_ID" \
  --output "$RUSTIA_NEW_APP"
xcrun devicectl device install app --device "$RUSTIA_IOS_DEVICE" "$RUSTIA_NEW_APP"
xcrun devicectl device process launch --console --terminate-existing \
  --json-output "$RUSTIA_CONTROLLER_JSON" --device "$RUSTIA_IOS_DEVICE" \
  "$RUSTIA_TEST_BUNDLE_ID" --test-threads=1 --nocapture
```

The helper validates device/profile membership, profile expiration, certificate
membership and the allowed bundle ID, signs with the existing identity and verifies
the bundle. Use a unique test bundle ID allowed by that profile. It never creates
signing credentials. Raw controller output may contain private IDs: retain it privately.
Require launch exit zero, successful termination/exit code zero in its JSON, and
exactly one nonzero passing test summary. CoreDevice can report error 10004 after a
very short process exits; retain the failed attempt and retry only that explicit
terminal PID race, at most three launches. A failed launch is never accepted as a pass.

For Android, `RUSTIA_MOBILE_TARGET=aarch64-linux-android`. Set
`CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` to the existing NDK's
`aarch64-linux-android35-clang` for this API-35 run. Select one exact `adb -s` device,
verify `uname -m=aarch64`, ABI `arm64-v8a` and OS version before running. If using an
emulator, record its ARM64 image and acceleration status separately. For each binary:

```bash
adb -s "$RUSTIA_ANDROID_DEVICE" push "$RUSTIA_EXECUTABLE" "$RUSTIA_REMOTE_TEST"
adb -s "$RUSTIA_ANDROID_DEVICE" shell chmod 700 "$RUSTIA_REMOTE_TEST"
adb -s "$RUSTIA_ANDROID_DEVICE" shell "$RUSTIA_REMOTE_TEST" --test-threads=1 --nocapture
adb -s "$RUSTIA_ANDROID_DEVICE" shell rm "$RUSTIA_REMOTE_TEST"
```

Use a unique owned filename under `/data/local/tmp/`, retain the process exit and
raw output, and require exactly one nonzero passing summary with no failures.
For each platform/mode the golden suite must emit exactly one
`RUSTIA_PRIVATE_PLATFORM_GOLDEN ` payload. Extract its exact ASCII bytes through
the line boundary, excluding any preceding libtest label. CoreDevice console
transport can frame boundaries as literal `\r\n`; strip only that framing,
never rewrite guest payload bytes. Compare all payloads byte-for-byte with the
same-candidate native reference. Recheck clean HEAD/source after all runs and
remove only the generated test app/files when no longer needed.


## Physical iPad Safari execution — 2026-11-14

After explicit user authorization for temporary LAN test serving, immutable
candidate `21167c04bfb64e8974d639f2041697cfb2510d22` executed the complete shared
browser contracts in **Safari 27.0.1 on a physical iPad Pro 11-inch (M4),
iPadOS 27.0.1**. Fresh Wasm builds and native references used the same clean
candidate, pinned Rust 1.95.0 and wasm-bindgen/test-runner 0.2.126.
The [sanitized receipt](mobile-safari-2026-11-14.json) records each suite count and
input-Wasm SHA-256, plus the exact harness digest.

| Mode | Executed groups | Passing tests | Failed |
|---|---:|---:|---:|
| Bare | 9 | 49 | 0 |
| Default | 9 | 47 | 0 |
| Total | 18 | 96 | 0 |

Every group has exactly one nonzero successful summary; none were failed,
ignored or filtered out. Both type-grammar regressions actually executed in
each mode, including deep nesting and no guest effects during type erasure.
The two iPad golden payloads match fresh default/bare native macOS ARM64
references from this same candidate byte-for-byte, full payload SHA-256
`cf890817d7b66cc2e8ab61030cbe6d0fe3235efe1c8a3dd7a1e4bf94f3b6a9a8`.

The pinned official wasm-bindgen manual runner served one loopback backend per
suite. A temporary LAN proxy used an unguessable per-run URL and added only an
output observer to the official HTML; generated JS/Wasm assets were forwarded
unchanged. It launched Apple's built-in Safari on the paired physical iPad via
`devicectl --payload-url`. One owned test tab navigated between suites and sent
its DOM output, console transcript and browser user agent back to the observer.
An initial one-test golden probe was separate from the 96-test acceptance run.
No browser/OS automation permission setting was changed.

The user agent uses a desktop-compatible Macintosh/Intel label; the paired
CoreDevice details and launch receipt establish that this was the physical iPad.
It does not constitute native Intel Mac execution. The proxy, all loopback
backends and test-runner processes were stopped afterward; independent artifact
review checked the raw results, Cargo artifact modes/hashes, launch/controller
success and matching native transcripts. Raw peer/device identifiers and the
full private harness/controller records remain outside source history.

Retained local evidence is under `rustia-ipad-safari-21167c04/` in the private
temporary directory, with `rustia-ipad-safari-batch.py` as the exact harness.
To repeat, build both browser modes and the nine shared suites as in
`tools/run-browser-correctness.sh`, serve the official manual test pages only
with explicit device/network authorization, retain actual nonzero DOM results
and goldens, verify the candidate remains clean and stop every server afterward.

This closes the physical-iPad Safari shared-contract execution item in #40.
Remote desktop Safari, native Intel Mac, full language parity and later release
candidates remain separate unproven gates in #31/#39/#40.
