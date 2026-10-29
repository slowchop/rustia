# Private API acceptance baseline

The private 0.x milestone uses immutable source revision
`92a85d65eb33724c17103d43477d34a633392768`.
The integrated CI commit `0e8422acdfd71d3d92d568696d9a200ee58e0d01`
changes only the preflight time limit and its explanatory comments. Runtime
sources, tests, manifests, dependency locks, tools and the API inventory remain
byte-identical to this baseline. Both packaged and workspace conformance still
run in full; the job now allows 60 minutes after the earlier 30-minute limit
cancelled corpus execution without a failed-test verdict.
The workspace remains at `0.0.1`, with publication disabled. This baseline gives
future API work a concrete comparison point; the stable 1.0 decision and public
release gates remain in [the release plan](92-first-release.md).

The milestone is complete only when the upstream and application PRs below have
passed their required checks and independent reviews and have merged. Their
check pages carry the final CI verdicts. This record does not waive a gate.

## What this baseline includes

- Builder presets and facade-owned settings, profiles, errors and diagnostics;
  applications do not need interpreter implementation types.
- Typed callbacks, owned data, live handles, Serde, module installation, reset,
  source reports and explicit fuel/memory limits, with working README examples.
- Typed Rust objects: deterministic methods read stable inputs and stage actions;
  declared standard/live methods may mutate host state immediately.
- Coroutine-owned module loading, with cache publication after completion,
  cancellation/retry and installation rollback. Module-resolution ABI v3 records
  the suspension and failure rules.
- Shared call continuations for expanded and ordinary calls, including callable
  receiver recursion/yielding. Repeated-string output work charges fuel before
  allocation across specialized and ordinary calls.

The reviewable API inventory is [facade-api.json](facade-api.json), checked with
the pinned compiler in native/Wasm and default/bare configurations. Its SHA-256
at this baseline is
`9dab65d93ef3d4da1058cfec5dc8ee80ce39c77653729704187afc4971b27842`.
It covers reachable signatures, trait implementations and extension points;
subsequent API changes must update and review that inventory. This private
baseline is not a 1.x compatibility promise.

## Verification and rollout

| Area | Evidence |
|---|---|
| Application boundary | [PR #63](https://github.com/slowchop/rustia-historic/pull/63), merged `8f8558ab`; facade-owned types, sealed callback adapters and versioned settings identity |
| Typed objects | [PR #65](https://github.com/slowchop/rustia-historic/pull/65), merged `2187c8d1`; deterministic game-turn and mutable-object examples, lifecycle/quota/receiver tests |
| Runtime calls | [PR #64](https://github.com/slowchop/rustia-historic/pull/64), merged `71524dba`; full preflight, Node, eight builds, differential smoke and remote screening passed |
| Platform checks | [PR #66](https://github.com/slowchop/rustia-historic/pull/66); actual Linux x86-64, macOS arm64 and Node/V8, with explicit target/host checks and one shared transcript |
| Integrated candidate | [PR #68](https://github.com/slowchop/rustia-historic/pull/68); final preflight, packaged conformance, platform execution, differential smoke and remote screening |
| External application | Twelve public contract suites, 252 tests across default/bare modes, compiled from extracted runtime archives with only facade/Serde dependencies |
| Documentation | Three executable README examples and 34 rustdoc examples; strict docs and semantic API checks |
| Resource recovery | Seven builtin phase/budget tests across four profiles; the same test bodies pass in supplementary Node 25.8.1 runs with Wasm runner 0.2.126, in default/bare modes |
| Language corpus | 723 fixtures, 517 named behavior references, zero pending rows; 705 Luau 0.709 fixtures and 18 offline PUC 5.1.5 fixtures |

The remote PR screening is the performance evidence for this rollout. The
README's compact comparisons retain their explicitly dated, checked CI-bench
cohort; this acceptance record adds no speed or ranking claim.

All three application rollouts use the exact source revision above:

| Application | Rollout | Runtime acceptance |
|---|---|---|
| WOTS | [PR #8](https://github.com/slowchop/wots/pull/8) | 451 tests pass, with 15 existing ignored tests; strict workspace/all-target Clippy passes |
| civfun | [PR #53](https://github.com/civfun/civfun/pull/53), following [Player integration #52](https://github.com/civfun/civfun/pull/52) | 62 host tests, strict lint and CLI build pass; all three complete saved-map grids match the previous rollout byte for byte |
| Grabby | [PR #1083](https://github.com/slowchop/grabby-next/pull/1083) | Faithful vendoring, eight-root dependency/feature checks, native/Wasm identity, real console effects, scenarios and hardened-worker recovery |

Washington and Attila retain exact 4,160-plot matches. Aztec retains its known
3,842/4,160 result (92.36%); its documented residual is not newly attributed to
this interpreter change. Grabby's production leaf continues to reject Rust
objects, while its ordinary console retains immediate mutation. Fuel and memory
failure tests isolate their budgets without changing production limits.

## Work left for public release

The [wider platform matrix](00-constraints.md), including real Firefox/Safari
and native/mobile execution, remains open. Cross-compilation covers buildability.
General language parity, the wider error-order/message audit, public package and
docs.rs validation, and the stable compatibility decision remain public-release
gates. The broader audit must close or recheck the previously recorded method-lookup
order and yielded-lookup error-prefix differences.

Mutable Rust payload snapshots, async/scoped callbacks, custom object fields or
metamethods, a JIT and a documentation site remain outside this milestone.
Host payload allocations, captured-state promises and external effects retain
the limits documented in [the object guide](83-rust-objects.md) and
[embedding contract](75-embedded-api.md).
