# rustia — hardened stateless action-leaf environment

> **Status: IMPLEMENTED (2026-07-28).** This document defines the generic
> public Rustia surface intended for authoritative, per-decision action leaves.
> Grabby-specific schemas, effects, slots, and commit logic remain in Grabby.

## Goal

An authoritative leaf must behave as a bounded pure function of captured host
input and captured host slots:

```text
(input, slots) -> (return value, ordered effects, ordered slot writes)
```

Rustia may provide deterministic computation and a transactional output lane,
but it must not provide a second cross-tick state store, ambient authority, or
guest-visible mutation path around the host capture/commit protocol.

This is a **guest surface layered on `DeterministicSim`**, not another runtime
profile. It does not add a `Profile` variant.

## Public construction

```rust
let mut vm = RustiaBuilder::deterministic_leaf()
    .memory_limit_bytes(4 * 1024 * 1024)
    .build_for_host::<GameHost>()?;
vm.install_host_module(game_module)?;
```

`deterministic_leaf()` selects:

- `Profile::DeterministicSim`;
- `GuestSurface::HardenedLeaf`;
- no stdout, clocks, dynamic compiler, or conformance-only facilities.

`RustiaExecutionProfile::DETERMINISTIC_HARDENED_LEAF` is the typed,
byte-stable description of the first two choices. It constructs the preset,
provides canonical identity-component bytes and a compact compatibility ID,
and can be read back from both `RustiaBuilder::execution_profile()` and the
built `Rustia::execution_profile()`. The descriptor is not a complete runtime
identity: embedders must bind budgets, host ABI, dependency/features, settings,
and any permitted services separately. Hardened-leaf validation forbids
services and dynamic-code facilities, so this descriptor plus Grabby's
separately bound budgets and host ABI closes that narrower use without a
parallel caller-maintained profile label.

A logical-live memory limit is mandatory and must be selected explicitly by the
embedder. Builder validation fails loudly when the hardened surface is combined
with another profile, a missing memory limit, host output/time services, dynamic
code, or conformance-only facilities. The low-level VM also forces the
deterministic profile and strips service/dynamic-code settings so bypassing the
facade cannot silently widen the environment.

The guest-surface contract is versioned separately as
`RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION` (currently 1).

## Guest-visible surface

The hardened surface is allowlisted. It keeps deterministic scalar operations
and frozen `math`, `string`, `utf8`, `bit32`, and `table` libraries. The
following authority or mutation surfaces are absent:

- `_G`, `getfenv`, `setfenv`, `setmetatable`, `getmetatable`, and `newproxy`;
- `debug`, `print`, and `os`;
- `rawget`, `rawset`, `rawlen`, and `rawequal`;
- `math.random` and `math.randomseed`;
- `buffer`, `vector`, `coroutine`, and `RTTI`;
- dynamic compilation and legacy extension opcodes.

The globals table, standard-library tables, and installed host-module namespace
tables are frozen before the first guest instruction. Ordinary
`Rustia::insert_global` input is rejected: decision inputs must come through the
fresh typed `HostContext`.

Installed host functions remain an embedding trust boundary. Safe Rust prevents
a borrowed invocation context from escaping, but it cannot prove that a
`'static` callback avoids captured globals or interior mutation. Authoritative
hosts must register capture-free callbacks or callbacks over audited immutable
configuration.

## Program restrictions

`compile_entry` validates the complete bytecode program once and wraps it in an
opaque artifact; the compatibility module path performs the same validation
before execution. The policy rejects:

- captured upvalues;
- global or upvalue writes;
- raw table-access opcodes;
- legacy extension calls.

The hardened module path remains available for compatibility, but every module,
export handle, and closure is decision-local and becomes stale at reset.
Authoritative hot leaves should use the stateless entry path below.

## Stateless compiled entries

`Rustia::compile_entry` compiles a top-level decision body that returns its
result directly. `Invocation::call_entry` executes that body without constructing
an exports table, retaining a closure, or installing a loaded program:

```rust
let entry = Rustia::compile_entry(
    "move_leaf.luau",
    r#"
local target = game.target()
local delta = target - game.slot(0)
game.move(delta)
game.remember(0, target)
return delta
"#,
)?;

let receipt = vm.invoke_entry_into::<i32>(
    &mut context,
    InvocationLimits::new(fuel)
        .with_max_records(max_records)
        .with_max_record_bytes(max_record_bytes),
    &mut lane,
    &entry,
)?;
```

A `CompiledEntry` cannot contain nested function prototypes or captured
upvalues. Inputs and authoritative state therefore cross through the typed host
context, while effects and slot writes remain inside the existing bounded,
record-transactional invocation. `invoke_entry_into` is the complete
authoritative lifecycle: it publishes only successful records and resets the
decision epoch after success, guest failure, conversion failure, resource
exhaustion, or contained host panic. A hardened entry rejects an ordinary guest
surface, and low-level hardened execution rejects unmetered or
non-transactional execution. Panic containment applies to unwind-capable
builds; the wasm benchmark adapter uses `panic = "abort"`, so its host
callbacks are contractually non-panicking and an unexpected panic traps.

## Reset isolation and performance

Once the hardened environment is sealed, every persistent heap object belongs
to an immutable allocation prefix: globals, standard libraries, native
functions, and host namespaces. `reset_decision()` preserves that frozen prefix
in place and discards every allocation appended by guest execution. It also
clears loaded programs, retained roots, traces, coverage, journals, poisoning,
and deterministic counters, then advances the checked decision epoch.

This specialized reset has the same isolation contract as the general reset;
it avoids reconstructing identical immutable standard-library and host-module
tables on every decision. Sealing fails closed unless the pre-guest heap is
append-only, which prevents a later guest allocation from occupying a slot
inside the preserved prefix. It also rejects externally retained values before
the initial seal: retained handles are decision-local, and treating one as a
configuration root would let arbitrary mutable state enter the immutable
prefix. Native tests hold an exact plateau across 600 heterogeneous typed
entries. Cross-target mixed-heap/reset and wasm linear-memory plateaus are
covered by the separate shared pressure test; the hardened typed-entry
transaction itself remains native-test coverage.

The checked-in `leaf_lifecycle_bench` measures one complete native release
decision with:

- four typed host calls;
- two ordered records;
- caller-lane publication;
- hardened reset.

On the development Apple Silicon host, three ABBA-ordered optimized runs of the
module install/lookup/call/reset control measured 22.292–23.750 microseconds
p95. The semantically checked hardened-entry path measured 6.625–6.958
microseconds p95 over 20,000 decisions after 2,000 warmups. These are local,
workload-specific engineering measurements, not a blanket Rustia speed claim.
Native Linux and wasm32 application measurements remain required at the Grabby
integration gate.

## Grabby ownership boundary

Rustia owns the generic hardened environment, typed capability enforcement,
shared fuel, bounded journal, dense per-invocation sequence, stateless entry,
and reset isolation.

Grabby owns:

- its input, slot, effect, slot-write, and record types;
- content identity and versioning;
- the one-successful-invoke-per-`(tick, phase, emitter)` rule;
- worker lanes and stable `(emitter, sequence)` commit;
- slot capture/restore and authoritative state-machine transitions;
- native/wasm performance budgets and multiplayer/replay validation.
