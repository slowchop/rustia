# rustia — host-bridge / effect-queue contract (grabby integration)

> **Status: [CONTRACT — reset implemented, host context incomplete] (updated 2026-07-27).** The deterministic seam between rustia scripts and
> grabby's (parallel, rollback) sim, agreed with the grabby parallelism agent. grabby's threading is
> design-stage/parked, so the **envelope is stable** (grounded in grabby's existing `ControlInput`/`host_discrete`/
> `SimId` model); only the `Effect` variant set grows. See [`CURRENT.md`](CURRENT.md) + the integration memory.

## The envelope (stable)
```rust
struct EffectRecord { emitter: SimId /*u64*/, seq: u32 /*intra-agent, reset each tick 0,1,2…*/, effect: Effect }
```
- **Per-lane collection:** each lane owns its own `Vec<EffectRecord>` (disjoint append, no shared mutation).
- **Commit:** concat all lanes → **stable-sort by `(emitter, seq)`** → apply single-threaded in that order.
  `(emitter, seq)` is globally unique within a tick ⇒ a **total, schedule-independent** order. Conflicts on the
  same target resolve in that order; each `Effect` variant defines its combine (Damage accumulates,
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
1. **Host-fn registration** where fns take integer/Q16.16 args (the Q16.16 boundary; f64→`Fixed` is a
   deterministic integer conversion).
2. **Deterministic, ordered host-fn dispatch** — so grabby's host-bridge can stamp `emitter` + `seq`
   (host-bridge owns `emitter` — it knows which agent's VM is running — and counts `seq` per decision).
3. **Host-provided typed-slot accessors** = the `SetSlot` mechanism.

Current implementation boundary: capability-labelled registration helpers exist, but typed callbacks do not
yet receive a per-VM/per-call host context. Mutable worker-local slot/effect state currently needs captured
synchronization or interior mutability. A cheap context-rebinding seam is therefore part of completing this
contract. The decision-reset gate below is implemented.

grabby's host-bridge maps rustia host-fn calls → `EffectRecord`/`Effect` and performs the `(emitter, seq)`
commit + `SetSlot`↔columnar writeback.

## Mechanical enforcement of the cross-tick invariant
**Implemented:** `Vm::reset_decision()` / `Rustia::reset_decision()` provide the **scratch heap reset between
decisions** required by `DeterministicSim`, so a global/upvalue/coroutine physically cannot survive to influence
a future tick; the only sanctioned cross-tick channel is `SetSlot`↔host columnar slots. Decision-reset ABI v1
preserves the profile, settings, service providers, extension operations, installed global host functions and
host modules, host-owned immutable compiled modules, and lifetime memory peaks. It discards guest globals,
tables/functions/upvalues, coroutines, loaded programs/caches, tracebacks, coverage rows, and retained roots;
RNG, logical time, compatibility GC state, and deterministic collection scheduling restart from construction
values. Per-call fuel already starts from `VmSettings::initial_fuel` on every execution. Ordinary inserted
globals are decision inputs and must be rebound after reset.

Every successful reset advances a checked `u64` decision epoch. All `Function`/`Exports` clones retain their
creation epoch and reject every operation after reset; the raw `RetainedValue` registry is replaced as the
lower-level epoch boundary. Reset is idle-only and is rejected from guest/host-callback execution. A
600-decision native table/coroutine/program/retained-export test remains exactly flat after warm-up, including
across the compact per-slot generation range. The wasm32 Node/V8 probe runs a further 300 decisions and checks
module reuse, host-binding restoration, input clearing, RNG restart, stale handles, and the same exact plateau.
Replay/golden protocols must record
`RUSTIA_DECISION_RESET_ABI_VERSION` (currently 1). Outside an explicit reset epoch, ordinary retained function
calls retain their documented survival across successful hot reload.
(Scenario run-once + console use are separate paths, not this hot loop.)

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
