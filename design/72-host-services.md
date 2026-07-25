# rustia — host services bus (provider seam for print / os.time / os.clock)

> **Status: [IMPLEMENTED v1] (2026-07-25).** The provider
> seam that replaces four faked environment-dependent stdlib functions with host-pluggable sockets
> (`collectgarbage("count")`/`gcinfo` stay canned until the reclamation arc gives them honest values).
> Implements the provider slice of [`CURRENT.md`](CURRENT.md) known-gap #3. v2 incorporates a six-lens
> adversarial review (VM internals, conformance/oracle, public API, determinism/lockstep, KISS/pillars,
> docs coherence); v2.1 adds the `live()` preset; v2.2 replaces the `0.0` `os.clock` fallback with a
> deterministic logical clock and records the input/output service rule. Scope discipline: no recorder,
> verifier, permission ceremony, taxonomy renames, GC-schedule wrapping, tier API, or memory telemetry
> in this change.

## Problem

Four stdlib functions return canned values because determinism requires pinned answers but no seam
exists for Standard hosts to install real ones:

| function | today (`crates/rustia-vm/src/vm.rs`) |
|---|---|
| `print` | no-op, returns nil (`native_print`, 10071–10073) |
| `os.clock` | always `0.0` (`native_os_clock`, 11503–11506) |
| `os.time()` no-arg | fixed `OS_TIME_NOW_SECONDS` (`native_os_time`, 11536–11543) |
| `os.date()` no-timestamp | same fixed constant (`native_os_date`, 11520–11523) |

Deferred (this slice does **not** touch): `collectgarbage("count")` → `0.0` (11445) and `gcinfo`
synthetic counter (11490–11501); honest values need heap accounting from the bounded-lifetime arc.
Everything else in the Luau 0.709 stdlib is real. In `DeterministicSim`, `os.*`/`collectgarbage`/
`gcinfo` are not registered at all (16176–16196 + RTTI mirror 16496–16500).

## The input/output service rule

Services split by data direction, and the direction decides the default:

- **Inputs** (`os.time`, `os.clock`, `os.date` defaults) change what scripts *compute*. In the sealed
  box (`DeterministicSim`) they are **absent** unless the host plugs a deterministic provider — a
  script cannot accidentally depend on something that is not there.
- **Outputs** (`print`) only let data *leave*; they cannot contaminate any computation. They are
  **present-but-swallowed** by default and sink-attachable on every profile.

Why `print` is a no-op and not nil in the sealed box: (1) the dev-to-sim pipeline — the same script
file is developed on the console (prints visible) and deployed into the lockstep sim (prints
swallowed); a nil `print` makes every leftover debug line fatal in production; (2) `print` is a base
global alongside `tostring`/`pairs`, which scripts reasonably assume exists (pillar 1); (3) upstream
Luau's own conformance harness globally silences print (`lua_silence`, Conformance.test.cpp) — a
no-op print in production is Luau's own behavior. This is recorded in `50-conformance.md:620`
(print is outside the determinism hash).

## Decision (one rule + one default principle)

**A service reads from the provider plugged into its socket; with no host provider plugged, the VM's
built-in deterministic defaults apply.** Nothing silently appears, disappears, or changes value for
any existing construction path — with one documented exception (`os.clock`, below).

- **rustia-vm level:** `VmServices::default()` is empty; empty sockets fall back to the built-in
  deterministic defaults on non-Sim profiles (pinned epoch for `os.time`/`os.date`, logical clock for
  `os.clock`, print no-op). Direct `Vm::new`/`Vm::with_settings` consumers (conformance runner, bench,
  tests) see no registration change — `datetime.luau` and `sort.luau` stay green (`sort.luau` only
  prints `os.clock` timings, asserts nothing about them).
- **rustia facade level:** `RustiaBuilder::standard()` (= `builder()`, `Profile::ReproducibleStandard`)
  additionally installs a real stdout print sink on native targets. Clocks keep the deterministic
  built-in defaults unless the host explicitly overrides — real time is always an explicit,
  visible-in-code choice (`.wall_clock(ServiceClock::system())`), never a default. This preserves
  pillar "reproducible by default": the default build's `os.time` stays replay-stable.
- `RustiaBuilder::deterministic()` = `Profile::DeterministicSim`, no services (unchanged from today).
- `RustiaBuilder::live()` = "normal Luau": `Profile::ReproducibleStandard` + real stdout +
  `ServiceClock::system()` on both clock sockets + `loadstring` enabled with the stock compiler —
  the what-any-Luau-user-expects preset, one word instead of manual plumbing. The language core
  stays deterministic (insertion order, pinned PCG32, Schubfach, libm; `math.randomseed` available);
  only the services go live. Named `live()` rather than `nondeterministic()` or `dynamic()`: accurate
  (only IO goes live; `dynamic` collides with the VM's existing `dynamic_program` = loadstring
  vocabulary and with `dyn`/dynamic-typing associations), positively framed, and forms the
  environment axis with the other presets. The default `builder()` remains reproducible-by-default —
  the pillar holds because the live world is an explicit, greppable one-word choice, never the
  silent default. On wasm, `live()` fails loudly (no system clock exists) and tells the host to plug
  providers. `collectgarbage("count")`/`gcinfo` stay canned even under `live()` until the reclamation
  arc; `math.random` stays pinned in every profile.
- **LuauCompat** follows the same service rules as ReproducibleStandard (it exposes the Standard
  service surface per `CURRENT.md` Determinism-model section). Its rustdoc gains a "this is not the
  `live()` preset" pointer for users hunting normal Luau.

### The logical `os.clock` (v2.2 — replaces the `0.0` fallback)

A stopwatch that always reads `0.0` is a measurement lie in a way a pinned epoch is not. The built-in
default for `os.clock()` therefore becomes a **logical clock**: `executed_instructions /
LOGICAL_CLOCK_INSTRUCTIONS_PER_SECOND` (constant pinned in the fuel cost-table version; initial value
1_000_000). It is fully deterministic (same run ⇒ same reading at the same point, on every peer and
target), monotonically non-decreasing per VM, and actually measures something — Luau documents
`os.clock` as CPU time consumed by the program, and instruction count is CPU time in deterministic
units. Scripts can benchmark themselves inside a sandbox. This is the one intentional behavior change
for existing hosts (`os.clock()` was `0.0`); it is called out in the changelog. A plugged
`.monotonic_clock(...)` provider overrides it on any profile; on `DeterministicSim` the function
remains absent unless a provider is plugged (the logical clock is a *default*, not a service the
sealed box gets for free — Sim keeps its nothing-ambient-unless-plugged rule).

## Registration rule (per-function, not whole-table)

The `os` table is **always registered on non-Sim profiles with all four members** (`time`, `clock`,
`date`, `difftime`). Pure paths (`os.time(table)`, `os.date(fmt, ts)`, `os.difftime`) are real today
and stay available regardless of providers — consistent with adopted improvement #10
("present-but-raises beats nil", `55-improvements.md:150`). Providers only feed the env-dependent
reads (no-arg `os.time()`, no-timestamp `os.date()`, `os.clock()`).

On `DeterministicSim`: the `os` table (all four members) is registered **iff at least one clock
provider is plugged**; the RTTI mirror block follows the same rule in lockstep. Plugging no provider
keeps today's exact Sim surface. `print` stays always-registered on every profile; with no sink it is
today's no-op (see the input/output rule). This narrows `CURRENT.md`'s "DeterministicSim minus …
`os.*`" line — the Determinism-model paragraph is updated in this change (gap #3 already sanctions
"deterministic providers" for Sim).

## Public API (crate `rustia`, extends `extension_api.rs` builder)

```rust
// presets (sugar over Profile + Services; single source of truth)
RustiaBuilder::standard()        // ReproducibleStandard + stdout sink (native); deterministic clocks
RustiaBuilder::deterministic()   // DeterministicSim, no services
RustiaBuilder::live()            // "normal Luau": stdout + system clocks + stock loadstring

// service sockets — the provider is the argument, so the chosen world is visible in code
.stdout(impl FnMut(&str) + 'static)          // closure only; errors are the host closure's business
.wall_clock(ServiceClock)                    // backs no-arg os.time + default os.date timestamp
.monotonic_clock(ServiceClock)               // backs os.clock (overrides the logical default)
```

`ServiceClock` constructors carry a determinism label:

```rust
ServiceClock::system()          // real wall/monotonic time; labeled nondeterministic
ServiceClock::fixed(t)          // constant; deterministic
ServiceClock::from_fn(f)        // host asserts determinism (host is trusted, label is greppable)
```

`live()`'s rustdoc states the `loadstring` enablement in its first line (runtime codegen is
attack-surface for hosts running untrusted scripts).

**Validation split (follows the existing `loadstring_compiler`/host-fn precedent):** the facade
`build()` returns a typed error (`ErrorKind::CapabilityRemoved`-style) for a nondeterministic provider
on `DeterministicSim`; the infallible VM constructors silently drop a nondeterministic provider on Sim
(same rule as `loadstring_compiler`, vm.rs:1263–1264) and document it. Preset/override ordering
(`.standard().profile(DeterministicSim)`) is caught by the same build error.

**Representation:** shared providers (`Rc<RefCell<dyn FnMut …>>`) with a manual `Debug`, so
`VmSettings` keeps its load-bearing `Clone + Debug` derives (rustia-cli clones settings; VM tests
clone). Cloning settings shares providers; documented. `ServiceClock` and any new public enum get
`#[non_exhaustive]` per facade convention.

**Explicitly cut from this slice** (KISS review; may return when a real caller exists):
`GcSchedule` wrapper (the existing public `VmSettings.deterministic_gc_interval: NonZeroU64` remains
the single protocol knob; GC tuning stays deferred until after reclamation per `CURRENT.md`);
`determinism_tier()` (no in-repo consumer; derivation rule kept as a doc sentence only); writer/
channel `IntoOutputSink` conversions (unspecified error semantics); async sinks. Peer note (docs,
not API): lockstep peers must agree providers out-of-band; provider identity is not fingerprinted
by the engine.

## VM internals (crate `rustia-vm`)

1. `VmSettings` gains `services: VmServices` (default empty; see representation above). ~59 in-repo
   `VmSettings { .. }` literals use `..VmSettings::default()` — mechanical churn only.
2. Registration per the rule above; RTTI mirror (`build_standard_global_rtti`, 16496–16500) follows
   in lockstep so `typeof(os)` never diverges from reality.
3. Natives: `native_print` composes per the locked rule in `50-conformance.md:619` — `tostring` per
   argument (honoring `__tostring`), args joined with `"\t"`, trailing `"\n"` — then writes to the
   sink (no-op without one). `__tostring` needs call context (`value_to_tostring_with_metamethod`,
   5445–5467; `tostring` is special-dispatched "needs call context", 8957): `native_print` moves to
   the call-context dispatch path like `tostring`. `native_os_time`/`native_os_date` read the
   wall-clock provider for the no-arg/default case, falling back to the pinned epoch.
   `native_os_clock` reads the monotonic provider, falling back to the logical clock (above).
   Provider-returned `f64`s are NaN-canonicalized via `Value::number` (`-0` passes through, matching
   script arithmetic). Explicit-argument paths unchanged.
4. Fuel: existing costs unchanged; the provider call happens inside the native — no new dispatch
   point, no hot-path cost. Sim with no providers pays nothing.
5. Sink re-entrancy: `FnMut(&str)` receives no `&mut Vm` (strictly weaker than host functions), so it
   cannot touch VM state. Caveat documented: a sink that mutates host state later read by a
   slot-read host fn is the same exposure class as existing host fns.
6. wasm32-unknown-unknown: the workspace is std-time-free today. `ServiceClock::system()` is
   `cfg`-gated to non-wasm targets; on wasm the facade `standard()` installs no clock (built-in
   deterministic defaults apply, same script-visible values as native — the logical clock is
   target-independent) and print defaults to no-op (no stdout exists). Script-visible surface is
   therefore identical across targets — no semantic fork (pillar 6); only the print destination
   differs, which scripts cannot observe.
7. `collectgarbage("count")`/`gcinfo` stay canned; docs and errors say so.

## Tests (same change; conformance corpus untouched)

- **No new conformance fixtures and no `rustia-only` rows** (the `rustia_only == 0` gate and the
  stock-oracle rule stand; provider-fed exact values cannot come from the oracle). Emitted-text and
  provider-value assertions live in **crate tests** (`rustia-vm` and `rustia`): provider-fed
  `os.time`/`os.clock` exact values, NaN canonicalization of provider outputs, print composition
  (tab-join, `\n`, `__tostring`), sink capture, Sim rejection of `ServiceClock::system()` (facade
  error + VM silent-drop), Sim `os` presence iff clock provider plugged, settings-clone sharing,
  `live()` preset installs system clocks + stock `loadstring` on native and fails loudly on wasm,
  logical clock determinism (two identical runs read identical `os.clock` values; monotonic
  non-decreasing; value derives from executed instructions; provider override wins).
- Conformance runner: unchanged VM behavior by default; optionally installs a capture sink for
  output cleanliness. Existing `print_*`/`gcinfo_*`/`collectgarbage_basic` fixtures stay green;
  692/491/0-pending stands.
- New fixtures must not canonize pre-existing oracle divergences (out of scope, scheduled under
  semantic-tail hardening): Luau `os.date` without `!` is localtime (rustia is UTC-only and ignores
  `!`), Luau `os.time(table)` defaults `hour` to 12 (rustia 0), Luau `*t` sets `isdst` (rustia omits).

## Docs updated in this change

`CURRENT.md` (gap #3 narrowing; Determinism-model paragraph: Sim admits deterministic clock/output
providers; logical `os.clock` default; doc-map row), `design/README.md` (index row + TL;DR),
`75-embedded-api.md` (services section, limitation list, boundary paragraph 21–24, build-order item 8),
`70-host-bridge-contract.md` (one note: the hot per-decision AI leaf keeps time as a Q16.16 decision
input via host fns; ambient clock providers are for Standard/console paths, not the hot leaf),
`50-conformance.md` (rows 619/804 print sink + `__tostring` now landed; 715–719 `os.*` wording),
`progress.json` + regenerated `progress.html`, `README.md` (narrow "host runtime providers" bullets
~48–49/89–91 + highlight + the **three-preset comparison table** — print / os.time / os.clock /
pure os fns / collectgarbage / loadstring / capabilities / GC / reproducibility / wasm rows across
`deterministic()` / `standard()` / `live()` — so evaluators see the differences at a glance), `CHANGELOG.md` (`### Added` + `### Changed` with explicit 0.0.x behavior
notes: facade default print writes to stdout on native; `os.clock()` returns the logical clock
instead of `0.0`).

## Non-goals (explicit)

`require`/module-loader socket; memory telemetry and honest `count`/`gcinfo` (reclamation arc);
recording/replay providers; WASI-style permission UI; `GcSchedule`/`determinism_tier()` public API;
paced-GC knobs; `Send`/`Sync` providers (the VM is thread-affine); async sinks; fixing the
pre-existing `os.date`/`os.time(table)` oracle divergences; perf claims (service calls are cold-path).

## Implementation order (strangler, always green)

1. `rustia-vm`: `VmServices` (shared-provider representation) + settings plumbing, per-function `os`
   registration + RTTI lockstep, natives read providers with built-in fallbacks (logical clock for
   `os.clock`), print moves to call-context dispatch with locked composition, NaN canonicalization,
   Sim silent-drop.
2. `rustia`: presets, `ServiceClock`, closure `.stdout`, typed build errors, wasm cfg behavior.
3. Crate tests (both crates) + conformance runner sink.
4. Docs + changelog; `just release-preflight` green (fmt, strict clippy native + wasm target,
   audits, workspace tests).
5. **grabby-aliens impact assessment (read-only):** survey `../grabby-aliens/` for what the API
   changes imply — profile → preset mapping, where `TickClock`/lane-log sinks and future
   decision-input providers plug in, any `os.*`/`print` usage in scripts that changes behavior
   (facade print now writes to stdout; `os.clock()` is the logical clock). Report findings only;
   do not modify that repo.
