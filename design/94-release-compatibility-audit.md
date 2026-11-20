# Release compatibility and API audit

Status: compatibility, measurement, closeout, and diagnostic follow-ups delivered; the wider public-release audit remains open. Baseline: `07f3ee7ae30b5803251a3b1d736760361f52a8b7`.
This follows the private acceptance recorded in [93](93-private-api-acceptance.md).
Public benchmark dashboard updates were approved on 2026-10-22. Package
publication and version changes remain outside this milestone.

## Delivery order

1. Audit language error behaviour, method lookup and coroutine boundaries against
   the pinned Luau 0.709 oracle. Fix confirmed gaps with conformance fixtures,
   four-profile regressions and bounded differential coverage. Reconcile older
   parity claims with executable evidence.
2. Obtain fresh CI benchmark evidence for the resulting candidate. Check the
   workload, revision and measurement provenance before updating comparisons.
   Local correctness runs provide no performance evidence.
3. Review the final API using Grabby, civfun and WOTS. Record concrete call sites,
   conversion/failure/lifetime contracts and any necessary migrations. Complete
   required checks and independent review before merging changes.

## Broader diagnostic audit

The reproducible [27-case inventory](release-error-audit.json) checks protected
runtime errors, missing/wrong builtin arguments, table mutation, coroutine
validation, arbitrary guest error values and frontend rejection. On `189ae963`,
14 cases match and 13 expose the differences below. This sample complements the
existing error-phase, builtin argument-order, coroutine and conformance tests;
it is not exhaustive language parity.

Build the locked fuzz `error-oracle-worker` with `--features oracle`, then run
`node tools/audit-release-errors.mjs`. The tool emits the complete observations
and exits nonzero when differences remain. It never rewrites expectations.
Only the oracle vehicle's source-name wrapper is normalized. No line number,
argument detail or message text is removed. Successful object addresses are
excluded by guest-side type labels.

| Case | Difference observed at the audit baseline |
|---|---|
| Length of nil | Generic type error rather than Luau's length error |
| Missing `rawget`, `rawset`, `type` arguments | Missing numbered argument diagnostics |
| Invalid `string.len`, missing `string.sub` argument | Missing function/argument/type details |
| Four-argument `table.insert`, invalid `table.sort` comparator | Wrong argument diagnostic |
| Invalid key to `next` | Extra source prefix |
| Invalid `coroutine.create` / `coroutine.resume` argument | Missing function/argument/type details |
| Freezing an already frozen or protected-metatable table | Wrong diagnostic text |

These were confirmed gaps, not deliberate differences. The diagnostic follow-up
below fixes them; regression fixtures test
direct and aliased calls, argument side effects and validation precedence.
Structured host errors and resource-exhaustion rules must remain intact. The
pending-free manifest describes its executable inventory; it does not erase
these independently observed findings.

The intentional differences remain the selected environment contracts:
reproducible iteration/RNG/numeric choices and deterministic capability limits,
as described in `05-luau-target.md`, `50-conformance.md` and `72-host-services.md`.
They must not be used to excuse ordinary language or builtin errors.

## Initial oracle findings

The existing `error-oracle-worker` was rebuilt from the baseline with its locked
`oracle` feature. Each source below was run in separate `luau` and `rustia`
processes. These are confirmed gaps, not accepted divergences.

### Method selection follows argument execution

```lua
local t = {m = function() return 'old' end}
local function arg()
    t.m = function() return 'new' end
    return 1
end
return t:m(arg())
```

Luau returns `new`; the baseline Rustia returns `old`. Computed receivers and
upvalues are captured before arguments, but a local receiver (including a
parenthesized local) is read after them, matching Luau `getExprLocalReg`. Method
selection observes argument effects. Cover fixed results,
return expansion, table append, nested argument expansion, varargs, argument
failure and argument suspension. Parenthesized/dot-call forms need separate
oracle expectations; they must not inherit colon-call ordering by assumption.

### Missing-method errors depend incorrectly on call shape

```lua
local t = {}
local function arg() observed = 1; return 1 end
local ok, e = pcall(function() return t:missing(arg()) end)
return tostring(ok)..':'..tostring(observed)..':'..tostring(e)
```

Both execute `arg`. Luau names the missing method (`attempt to call missing
method 'missing' of table`); Rustia reports `attempt to call a nil value`.
Compare message content separately from the oracle vehicle's chunk-name wrapper.
Do not normalize away meaningful error text in regression expectations.

### A forbidden yield during lookup has an extra error prefix

```lua
local t = setmetatable({}, {__index = function()
    coroutine.yield('lookup')
    error('failed')
end})
local c = coroutine.create(function() return t:missing() end)
local a, b = coroutine.resume(c)
local d, e = coroutine.resume(c)
return tostring(a)..':'..tostring(b)..':'..tostring(d)..':'..tostring(e)
```

Both reject the first resume and leave a dead coroutine. Luau returns the bare
`attempt to yield across metamethod/C-call boundary`; Rustia prepends its source
name and line. Replacing `t:missing()` with `t.missing` reproduces the same gap.
This is a rejected yield, not a successful suspended metamethod continuation.

The rejected-yield prefix fix now passes the four-profile facade regression for
ten contexts. Its new conformance fixture was independently run through the
pinned oracle (`ok / 0 / pass`). These were initial local results; final review and runtime acceptance are recorded below.

Method ordering now passes 14 oracle-derived call shapes, eight missing-method
error shapes and nine failure/GC/receiver/vararg cases across four profiles,
including double suspension and ordinary dot-call controls. Expanded method
lookups use validated continuation metadata; fixed calls select after arguments.
Missing-method messages now use checked lookup in these paths. Malformed metadata is rejected before argument execution. Explicit GC cases,
argument failure, protected-call recovery and repetition after reset pass.
The differential smoke passed before the local-receiver/append-vararg follow-up
and was rerun successfully for runtime candidate `8b8bc17b`. Compiler unit tests (124), VM unit tests (244), existing
builtin expansion/order tests, ten deferred-call tests and strict workspace
Clippy pass. The initial full corpus passed (725 fixtures / 519 refs); it predates the
local-receiver/append-vararg follow-up and is not final-candidate evidence.

## Completion evidence

Closeout PR #72 merged after required checks; diagnostic follow-up PR #73 merged
as `90bc2ae4` after required CI and remote screening. See [CURRENT](CURRENT.md)
and the delivery records below. Wider parity and public-release gates remain open.

## Delivery status

The method/vararg/yield batch merged in [PR #70](https://github.com/slowchop/rustia-historic/pull/70).
Its initial head passed eight target builds, native macOS arm64, Node/V8 and
differential CI. Preflight found stale generated publishability file counts;
those counts have been regenerated. A subsequent malformed-metadata regression
also made hardened-program validation reject out-of-range late receiver registers
at construction, matching runtime validation. Final-head checks passed in
[CI run 39715901379](https://github.com/slowchop/rustia-historic/actions/runs/39715901379).

The method fixture now contains 31 cases, including a local receiver changed
after argument suspension, an upvalue control, and a parenthesized local. All
four profiles pass it before and after reset. The broader diagnostic findings
were recorded here while GitHub Issues was disabled. The later [semantic coverage audit, #31](https://github.com/slowchop/rustia/issues/31),
is closed. The diagnostic follow-up below records the outcome for this inventory.

Independent reviews of the compiler/VM changes and conformance evidence found
no blocking issue. Eight additional receiver/argument probes matched the pinned
oracle; both complete fixtures returned `ok / 0 / pass`, and the diagnostic audit
reproduced 14 matches and 13 differences. The review corrected an outdated case
count in the prose provenance record. These reviews do not replace final CI.

Preflight also exposed stale benchmark queue metadata and its rendered HTML.
Both were regenerated without changing measurements or speed claims. The
benchmark runner reconnected and runtime screening passed; the full fresh
cross-engine comparison is recorded below.


## Dense-table regression investigation

All release checks passed on `897bfddb`. Its stronger
[CI acceptance comparison](https://github.com/slowchop/rustia-historic/actions/runs/39694175435)
against `07f3ee7a` confirmed dense tables at +4.62% (99% interval +4.17% to
+5.04%), beyond the calibrated 2.08% screening threshold. Closures/upvalues
improved by 7.64%; the other three workload intervals include zero. A successful
workflow was not performance acceptance, so the compatibility batch was held
unmerged while this result was investigated.

Static compilation found identical dense-workload bytecode and identical
normalized Linux LLVM IR for its sort and table-access helpers. The shared
interpreter gained register-slice selection and call arguments for deferred
method receivers. This is a code-generation hypothesis, not a measured cause.
The receiver-state candidate stores the receiver binding in continuation state: a traced cell
for captured locals, or the stable value for uncaptured locals. Late read fuel,
collection and method lookup order remain unchanged. It removes the caller
register parameters from the continuation driver. The final remote comparison
below establishes its performance acceptance.

The application review found one concrete client correction: WOTS ignored its
per-feed fuel argument. [WOTS #9](https://github.com/slowchop/wots/pull/9), merged
as `7960b5af6621f8af49ca399da270a6463e014a57`, uses existing invocation limits;
zero keeps its established default, and explicit limits stay below the engine
ceiling. All 33 BBS tests, strict Clippy and independent review passed, including
exhaustion, preserved immediate writes and same-session recovery. No public
Rustia API addition or dependency change was needed. Grabby and civfun retain
the accepted application contracts recorded in document 93.


The [first isolation comparison](https://github.com/slowchop/rustia-historic/actions/runs/39706980966)
measured `189ae963` against the baseline with four legs per side: dense tables
-0.59% (99% interval -1.21% to +1.45%), with no slowdown detected. The later
VM delta moves receiver bounds validation into `descriptor`; it does not add
work to dense-table guest execution. The receiver-binding refactor was temporarily withdrawn
to test a smaller isolation candidate that prevented expanded-call construction
(including descriptor setup and validation) from being inlined into shared
dispatch code. The descriptor helper was already out of line; its annotation
alone left dispatch unchanged in static inspection. All validation remains enabled.
The following remote measurement rejected that candidate.


The constructor-only candidate `53ec889a` did not fix the regression:
[eight-leg acceptance](https://github.com/slowchop/rustia-historic/actions/runs/39709187008)
measured dense tables +4.43% (99% interval +4.04% to +6.14%). Static dispatch
size is not performance evidence. That candidate was withdrawn.

The receiver-binding candidate `b13e9914` had
[screened at +0.04%](https://github.com/slowchop/rustia-historic/actions/runs/39707621551)
for dense tables (interval -0.40% to +0.69%). Its reviewed implementation is
restored for full acceptance: retain a traced capture cell or stable uncaptured
value in the continuation and remove caller-register arguments from resumed
calls. The clone/root regression, existing method-order and malformed-descriptor
tests, and strict Clippy passed. Screening alone was insufficient; the stronger
comparison below supplies the final performance evidence.


## Accepted receiver-state measurements

The [final eight-leg comparison](https://github.com/slowchop/rustia-historic/actions/runs/39712053958)
measured runtime candidate `8b8bc17bd9008a8e9d0a540146e4c0f7ad5b3828`
against baseline `07f3ee7ae30b5803251a3b1d736760361f52a8b7`.
All measurements are remote CI results; local compilation supplied only static
code-inspection and correctness evidence.

| Workload | Change | 99% interval |
|---|---:|---:|
| Dense tables | +0.04% | -0.22% to +0.76% |
| Sparse tables | -0.46% | -2.04% to +2.31% |
| Closures/upvalues | -0.63% | -1.68% to +0.10% |
| Generic iteration | -0.39% | -1.95% to +0.92% |
| Standard library/math/table | +1.11% | -0.86% to +2.92% |

Every interval includes zero. The dense-table interval is below its calibrated
2.08% screening threshold, resolving the earlier +4.62% regression. This is
acceptance of the compatibility change against the old Rustia baseline, not a
new comparison with other engines. The README's cross-engine table must wait
for a fresh full matrix.

The implementation retains receiver bindings in continuation state, preserving
late reads, fuel and GC roots while avoiding caller-register arguments through
the continuation driver. It is the same reviewed VM source that screened at
`b13e9914`; the constructor-only alternative is absent. Method-order regressions,
malformed-metadata checks, continuation cloning/root coverage, full VM tests and
strict Clippy pass. Required CI gates final PR delivery; its status is available
on [PR #70](https://github.com/slowchop/rustia-historic/pull/70).


## Fresh cross-engine comparison and closeout

[Matrix run 39719875275](https://github.com/slowchop/rustia-historic/actions/runs/39719875275)
measured the merged runtime `9b12a5a8540f69fe7c0f6daebe467c795bb576c3`.
The accepted cohort contains 65 native rows (13 engine variants, five workloads)
and 15 WebAssembly adapter rows. Publication [PR #71](https://github.com/slowchop/rustia-historic/pull/71)
merged as `7a1b67640a71b5e987260aab0dba27c936f47ec5`.
The native measurement window is 2026-10-22 04:09:41–04:14:05 UTC.

The README uses the four interpreter columns from that immutable native cohort,
with checked units and three-decimal microsecond values. Rustia trails upstream
Luau on all five workloads; it beats Koto and Rhai on four, with generic iteration
slower than both. These are workload-specific observations, not a universal speed
ranking. Compilation, first-call costs and JIT variants remain in the full report.
The matrix is a cross-engine snapshot, not a paired regression experiment;
the eight-leg comparison above remains the regression acceptance evidence.

The three requested follow-up tracks had the following evidence at closeout
PR #72, before the diagnostic follow-up below:

| Track | Evidence | Remaining gate |
|---|---|---|
| Compatibility audit | Oracle fixtures, four-profile regressions, bounded differential tests and required CI in merged PR #70; broader 27-case diagnostic inventory | Thirteen diagnostic differences remain explicitly tracked for further parity work; no full-parity claim |
| Fresh benchmarks and README | Accepted matrix on the merged runtime; immutable history and publication; README comparison verifier | Closeout documentation CI and review |
| Application API review | Grabby/civfun contracts in document 93; WOTS fuel correction in merged PR #9, 33 BBS tests, Clippy and independent review | No new API or client migration required |

The private API baseline in document 93 remains the common application pin;
this runtime-only follow-up does not claim that all applications were repinned.
Public package publication, a version bump, exhaustive Luau parity and the wider
public platform execution matrix remain separate release gates in documents 90
and 92.

Closeout verification rechecked that Grabby #1083 (`79240f45`), civfun #53
(`074b2e16`), WOTS #8 (`aabfde94`) and the WOTS fuel fix #9 (`7960b5af`)
are merged. The facade inventory still has SHA-256
`9dab65d93ef3d4da1058cfec5dc8ee80ce39c77653729704187afc4971b27842`,
matching the accepted baseline. The README's 20 numeric cells match history,
its three executable examples pass, and project-status and generated benchmark
report checks pass. Required CI on publication merge `7a1b6764` also passed
([run 39721705546](https://github.com/slowchop/rustia-historic/actions/runs/39721705546)).


## Diagnostic parity follow-up

The 13 differences above reproduced on the merged closeout baseline. The fix
uses shared typed-argument diagnostics for the affected builtins, including
missing versus explicit-nil arguments, native names through aliases, and numeric
coercions. Specialized string and table instructions use the same checks.
Validation retains Luau's order: table type before arity, insert position before
mutation, sort readonly state before comparator type, and frozen state before
metatable protection. Invalid `next` keys and native table-mutation errors retain
the oracle's bare messages; script-origin errors keep their source locations.

The original 27-case audit now reports 27 matches and zero differences. The
historical JSON observations remain unchanged so the original failures remain
reviewable. CI reruns the audit against the pinned oracle and fails on any new
difference. The expanded 85-case diagnostic fixture also checks aliases, source prefixes,
argument side effects and competing validation failures across all four profiles
and after reset. This follow-up merged in PR #73 as `90bc2ae4` after final
review, full CI, and remote screening. This closes the enumerated findings,
not the wider public-release parity gate.

Independent runtime review found no blocker and checked additional competing
errors and thread/buffer type names. It also identified a separate, pre-existing
numeric-conversion difference: `return string.sub(12,0/0)` and
`return string.sub(12,1,math.huge)` return `12` in the local pinned oracle, while
Rustia rejects the nonfinite index through the unchanged `string_library_index`.
The subsequent [numeric-index contract](95-numeric-index-contract.md) resolves
this policy question with checked signed-64 conversion and an exclusive upper
bound. Its final runtime passed cross-target correctness and the direct
original-baseline acceptance comparison before delivery in PR #74 as `b5982752`;
see [95](95-numeric-index-contract.md) and [96](96-browser-platform-acceptance.md).
It is not covered by the claim that the 13 diagnostic findings are fixed.

## Diagnostic inventory triage — 2026-11-13

The active #27/#28 parser candidate re-ran the locked 27-case inventory with
`node tools/audit-release-errors.mjs`: **27 matches, zero differences**. Only
the documented oracle source-name wrapper was normalized. The historical thirteen
differences above are not current defects in this fixed inventory. This is local
correctness evidence, not complete semantic or platform acceptance; review,
preflight and PR CI were the delivery gates for that candidate. The
[platform record](96-browser-platform-acceptance.md) links the separate Intel Mac
and remote Safari follow-ups.
