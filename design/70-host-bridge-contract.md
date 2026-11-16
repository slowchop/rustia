# rustia — host-bridge / effect-queue contract (grabby integration)

> **Status: [CONTRACT + TYPED INVOCATION + HARDENED LEAF IMPLEMENTED] (updated 2026-07-28).** The deterministic seam
> between rustia scripts and grabby's (parallel, rollback) sim, agreed with the grabby parallelism agent.
> grabby's threading is design-stage/parked, so the **envelope is stable** (grounded in grabby's existing
> `ControlInput`/`host_discrete`/`SimId` model); only the `Effect` variant set grows. See
> [`CURRENT.md`](CURRENT.md) + the integration memory.

## The envelope (stable)
```rust
struct EffectRecord { emitter: SimId /*u64*/, seq: u32 /*one successful invoke: 0,1,2…*/, effect: Effect }
```
- **Per-lane collection:** each lane owns its own `Vec<EffectRecord>` (disjoint append, no shared mutation).
- **Invocation identity:** grabby permits exactly one successful rustia invocation for each
  `(tick, phase, emitter)`. Rustia resets its dense sequence at every invocation, so this host rule makes
  `(emitter, seq)` unique within the phase; a failed attempt publishes nothing and may be retried.
- **Commit each phase separately:** concat all lanes → **stable-sort by `(emitter, seq)`** → apply
  single-threaded in that order. `(emitter, seq)` is globally unique within the phase ⇒ a **total,
  schedule-independent** order. Conflicts on the same target resolve in that order; each `Effect` variant
  defines its combine (Damage accumulates,
  SetNavGoal last-wins).
- **Payload = integer/fixed-point ONLY** — no `f64`/`usize`/pointers in any field. Continuous values cross at
  **Q16.16** (`Fixed = i32`, `FixedVec2 = [i32;2]`, angle = fixed-turns `i32`). Scripts compute f64-internally
  (deterministically) and convert to `Fixed` at the host-fn boundary — a deterministic integer op.
- **`Spawn` carries NO `SimId`** — the host assigns ids at commit in `(emitter, seq)` order from a reserved
  per-tick stride, so id allocation can't depend on lane-completion order.
- **`SetSlot { slot: u8, value: i64 }` is THE sanctioned cross-tick AI-memory write** — it writes a host typed
  slot in the captured columnar array, never a Lua global/upvalue. This satisfies the "no GC-heap authoritative
  state" rollback invariant *through the effect channel*.
- The `Effect` enum is **versioned** (schema version is part of the golden/protocol contract).

## Share the type, don't mirror it
`Effect`/`EffectRecord` live in a **shared crate** (`grabby-effects` / sim's public API); byte-identity is the
same Rust type, not two encodings. A serialized form is needed ONLY for the **no-SAB message-passing transport**:
**fixint little-endian + explicit enum discriminants** (bincode-fixint or hand-rolled LE) — deterministic and
arch-stable because every field is already integer/fixed. On the SAB path, records stay native structs in the
shared heap (no serialize).

## rustia's side (the public/private split — load-bearing)
`Effect`/`EffectRecord`/`grabby-effects` are **grabby-specific → they live on grabby's side** (the host-bridge /
private-fork glue), **NOT in public rustia** (public stays clean, no grabby-specific code). rustia-public
provides the **generic seam**:
1. **A lifetime-independent `HostSchema`** stored by `Rustia<H>` and its registered functions. The schema names
   input, slot, external, effect, slot-write, and final record types without owning one tick's values.
2. **A fresh `HostContext<H>` per invocation.** The concrete context may contain block-local borrows into
   worker-local input/slot snapshots. It constructs records with Rustia's dense `sequence` and supplies their
   deterministic logical byte charge; grabby's context adds `emitter`.
3. **Capability-derived callback views.** `Input` and `Slots` expose immutable projections; `Effects` and
   `SlotWrites` append typed records; `HostMut` exists only for external/nondeterministic registrations, which
   `DeterministicSim` rejects. Registration selects the view and capability together rather than accepting an
   arbitrary capability label.
4. **Host-fn integer/Q16.16 signatures** (the Q16.16 boundary; f64→`Fixed` is a deterministic integer
   conversion) and namespaced `HostModule<H>` registration.

Typed VMs execute module top-level code and retained functions only through a lexical `Invocation`; there is no
stored context pointer to become stale and no callback VM handle with which to re-enter. Callback types remain
`Fn + Send + Sync + 'static`, while the views they receive are short borrows tied to the invocation. The
callback itself is nevertheless a semantic trust boundary: safe Rust cannot stop a `'static` closure from
reading a captured global or mutating captured/interior state. Grabby must keep deterministic registrations
capture-free (or capture only audited immutable data).

The lower-level issue-#50 consumer seam is separate from that facade contract.
A trusted `VmHostDispatcher` may receive a short-lived `VmHostCall` and re-enter
guest code with the exact active `VmExecution`, fuel allowance, caller depth and
dispatcher. The VM never stores the borrowed dispatcher. Owned bytecode callers
are saved and fully rooted before the synchronous host boundary, so re-entry does
not keep a large Rust bytecode frame live or create a second budget. This seam is
used by Luvy's scheduler-owned integration; the ordinary typed facade callbacks
above still receive no VM handle, and concurrent access to one VM remains invalid.

Authoritative leaves use `RustiaBuilder::deterministic_leaf()` and a host-owned `CompiledEntry`. This guest
surface layers on `DeterministicSim`: its globals, standard libraries, and host namespaces are sealed and
frozen; environment/debug/raw mutation, VM RNG, clocks/output, buffers/vectors/coroutines, dynamic compilation,
and captured upvalues are absent or rejected. `Invocation::call_entry` executes the top-level decision body
directly, avoiding a retained module/export/closure path. The complete generic contract is
[`77-hardened-leaf-environment.md`](77-hardened-leaf-environment.md).

grabby's host-bridge maps rustia host-fn calls → `EffectRecord`/`Effect` and performs the `(emitter, seq)`
commit + `SetSlot`↔columnar writeback.

## Invocation record transaction (implemented)

`InvocationLimits::new(fuel).with_max_records(count).with_max_record_bytes(bytes)` applies one explicitly named,
monotonic budget to the whole host operation. Every `Function::call_in`, module install/run, metamethod, nested
guest call, and host-function fuel charge consumes the same fuel counter. Effects and slot writes share one
bounded journal and one dense sequence, starting at zero. `record_bytes` is a host-defined deterministic
protocol size, not allocator size.

The journal has two rollback levels:

- each host callback starts at a checkpoint; if that callback returns an error or panics, records appended by
  that callback are removed;
- if the Rust closure passed to `invoke`/`invoke_into` returns an error—including guest failure or result
  conversion failure—the entire invocation publishes no records.

This deliberately follows guest `pcall` structure. A completed host call stays complete when a later guest
error is caught by `pcall`; a host callback that appends and then fails loses only its own suffix, after which a
catching script may continue and append the next dense record. If the failure escapes the outer invocation, all
records disappear.

Fuel exhaustion, OOM, record-count/byte overflow, an invalid yield across a boundary, a missing typed-host
dispatcher, and host panic are hard transaction aborts and cannot be hidden by a callback or guest `pcall`.
This hard-transaction treatment is scoped to typed transactional execution; it does not globally redefine
ordinary legacy execution's catchable OOM/yield behavior. Rustia catches a host panic, rolls back records, and
marks the VM poisoned; every later guest execution entry point rejects until `reset_decision()` succeeds.
The VM-wide containment layer covers typed and context-free host functions, low-level extension ops, stdout and
clock providers, and the configured `loadstring` compiler. Containment requires
`panic = "unwind"`; an aborting build terminates or traps before
`catch_unwind` can translate the panic, so its host callbacks must not panic.

The outer Rust closure passed to `invoke`/`invoke_into` is caller control flow, not a VM callback. If it or a
caller-side value conversion panics, normal Rust unwinding drops the private journal without publication and
ends the VM/context borrows; Rustia does not translate or poison on that path. VM stacks are already idle
between lexical guest calls, so the facade remains reusable. Guest heap/context/service mutations completed
before the panic remain, matching the normal record-only transaction rule.
`Invocation::install` is the deliberate guest-state exception; its nested
savepoint does not make external host effects transactional.
`invoke_entry_into` is stronger than the general helpers: it catches such an
unwind, performs the hardened decision reset, and then resumes the original
panic.

The general invocation guarantee is **record-transactional, not a general state transaction**. Ordinary
`eval`/`eval_live`/`eval_serde`/`exec` and retained-function calls do not undo guest heap/global writes,
mutation through `HostMut`, captured callback state, or `HostContext` mutation hidden behind interior
mutability. `Invocation::install`, however, deliberately adds a nested guest-state savepoint around the module
candidate: if installation fails, guest heap/global/coroutine/RNG/logical-time/GC/module-cache state and
provisional retained roots are restored, and the surrounding invocation still publishes no records. External
host and resolver effects remain outside that savepoint. `DeterministicSim` excludes `HostMut`, and grabby keeps
cross-tick authority in captured slots plus the record lane, but embedders must still audit callback captures.

On success, `invoke` returns an `InvocationOutcome` containing the value, a slice of Rustia's reusable journal,
and usage (fuel, record count, logical bytes); the slice lasts until the next mutable VM use. `invoke_into`
reserves the caller's lane before guest execution, moves committed records into it without an allocation or
fallible step during the final append, and returns a non-borrowing `InvocationReceipt`. This is the intended
general worker-lane path. Hardened authoritative leaves use `invoke_entry_into`, which performs the same
publication and then resets the decision epoch after either success or failure. If reset itself fails, it
removes any records just published and returns the reset error.

## Mechanical enforcement of the cross-tick invariant
**Implemented:** `Vm::reset_decision()` / `Rustia::reset_decision()` provide the **scratch heap reset between
decisions** required by authoritative `DeterministicSim` leaves, so a global/upvalue/coroutine physically cannot
survive to influence a future tick; the only sanctioned cross-tick channel is `SetSlot`↔host columnar slots.
Decision-reset ABI v1
preserves the profile, settings, service providers, extension operations, installed global host functions and
host modules, host-owned immutable compiled modules, and lifetime memory peaks. It discards guest globals,
tables/functions/upvalues, coroutines, loaded programs/caches, tracebacks, coverage rows, and retained roots;
RNG, logical time, compatibility GC state, and deterministic collection scheduling restart from construction
values. Ordinary legacy calls still receive their configured per-execution fuel; typed invocations instead use
the explicit shared fuel in `InvocationLimits`. Ordinary inserted globals are decision inputs and must be
rebound after reset.

Every successful reset advances a checked `u64` decision epoch. All `Function`/`Exports` clones retain their
creation epoch and reject every operation after reset; the raw `RetainedValue` registry is replaced as the
lower-level epoch boundary. Reset is idle-only and is rejected from guest/host-callback execution. A
600-decision native table/coroutine/program/retained-export test remains exactly flat after warm-up, including
across the compact per-slot generation range. The shared native/Node-V8 mixed-heap test runs 96 further
decision epochs and checks bounded public roots, rich cyclic graphs, weak collection, module replacement, quota/OOM
recovery, stale handles, exact post-reset VM accounting, and stable warmed wasm linear-memory pages.
Replay/golden protocols must record
`RUSTIA_DECISION_RESET_ABI_VERSION` (currently 1). Outside an explicit reset epoch, ordinary retained function
calls retain their documented survival across successful module replacement.
(Scenario run-once + console use are separate paths, not this hot loop.)

For `GuestSurface::HardenedLeaf`, the sealed globals and their reachable standard-library/host configuration
are immutable rather than guest state. Reset preserves that checked allocation prefix in place and discards
every post-seal allocation, loaded program, and retained root. This keeps the same isolation boundary without
reconstructing identical library/module tables on every decision. The guest contract is separately versioned by
`RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION` (currently 1).

## Illustrative catalog (grows on grabby's side)
`SetNavGoal{target:FixedVec2}` · `Move{dir:FixedVec2}` · `Fire{weapon:u8, aim:i32}` ·
`Interact{target:SimId, kind:InteractKind}` · `Damage{target:SimId, amount:Fixed}` ·
`Spawn{template:TemplateId, at:FixedVec2}` (no id) · `SetSlot{slot:u8, value:i64}`.

## Slot properties + the per-decision-pure-function consequence (round 4, locked)
- **Slot indices are append-only and part of the golden/protocol contract** (like the `Effect` schema version
  and the bytecode version): never renumber an existing `slot:u8` — that breaks replay/golden. A new persistent
  AI field = next free index + a new column in the captured array + a map entry, **all grabby-side**.
- **The columnar slot array is captured in the rollback ring** (it *is* the persistent AI state) → the `i64`s
  are in the snapshot and restore on rollback/reconcile (a Q16.16 field is stored as its `i64` bit pattern).
- **The authoritative AI leaf is a PER-DECISION PURE FUNCTION** (a direct consequence of the scratch-heap reset):
  it **cannot be a coroutine that suspends across ticks** — its continuation state would be exactly the
  un-captured cross-tick state the reset forbids. Multi-tick / temporal behaviour ("wait N ticks", "phase 2 of a
  plan") is expressed via **`SetSlot` columnar state** (a captured countdown / phase index), never a suspended
  coroutine. **Nuance:** rustia *does* implement coroutines (axis G) — they work fine *within a single decision*
  and suspend normally on the **scenario-run-once + console paths** (`ReproducibleStandard`, persistent VM, no
  per-decision reset). The restriction is *only* the `DeterministicSim` per-agent AI leaf. (This supersedes
  grabby's old `console-scripting-plan.md` §3/§6 `yield wait_ticks(n)` model *for the hot AI leaf* — that
  coroutine model belongs to scenario/console.)
- **Time crosses the hot leaf as a decision input, never as an ambient service.** The per-decision AI leaf
  receives time as a Q16.16 argument (tick index, fixed dt) through its typed host fns, alongside the other
  decision inputs. The ambient wall/monotonic clock providers from
  [`72-host-services.md`](72-host-services.md) (the `os.time`/`os.clock`/`os.date` sockets) are for
  Standard/console paths, not the hot `DeterministicSim` leaf: even though Sim now admits deterministic
  clock providers, plugging one into the hot leaf would let a decision depend on provider state outside the
  captured slot ring and the `(emitter, seq)` envelope.
