# Typed Rust objects

Implementation merged in [PR #65](https://github.com/slowchop/rustia-historic/pull/65);
civfun's real Player integration merged in [#52](https://github.com/civfun/civfun/pull/52).
The [private acceptance record](93-private-api-acceptance.md) identifies the
common source, verification and required rollout gates. Stable 1.0 acceptance
remains in [the release plan](92-first-release.md).

## Register methods, then create objects

`ObjectType<T, H>` describes the methods available on a Rust payload. Register it
on `Rustia<H>`, then move a payload into `create_object`. The default schema is
`()`, for ordinary embedding:

```rust
use rustia::{ObjectType, Rustia};

struct Player {
    score: i64,
}

fn main() -> rustia::Result<()> {
    let mut lua = Rustia::builder().build()?;
    lua.register_object_type(
        ObjectType::<Player>::new("Player")
            .method("score", 1, |player: &Player| player.score),
    )?;
    let player = lua.create_object(Player { score: 42 })?;
    lua.globals()?.set_live(&mut lua, "player", &player.as_value())?;
    assert_eq!(lua.eval::<i64>("return player:score()")?, 42);
    Ok(())
}
```

The `1` is the method's nonzero host-call fuel cost. Methods accept up to eight typed
arguments after the receiver and use the existing owned/live conversions,
fallible results, and panic containment. A callback receives a temporary `&T`
or `&mut T`; it cannot keep that reference or re-enter the interpreter.

Before registration, `name`, `fuel_policy`, and `methods` expose the definition's
diagnostic name and versioned method metadata, matching host modules. Parameter
metadata includes the receiver first; capability views are supplied by Rustia.

`Object<T>` is a live root, not a copy of `T`. Cloning its handle preserves
identity. `Value::as_object::<T>` checks the Rust payload type and returns `None`
for a different type. Stale or foreign handles return an error first. Guest
method calls check the receiver type before converting other arguments or
borrowing application context.

Rust can call a method with `Object::call_method`, or use
`call_method_values` when results must stay live. The corresponding `_in`
methods run inside a typed `Invocation`, sharing its fuel and record journal.
Host-side calls validate receiver ownership and epoch before converting
arguments, allocating guest values, or spending invocation fuel.

Callbacks can accept and return `Object<T>`. Returning `NewObject::new(payload)`
creates a fresh instance of an already registered type. Ordinary values, live
handles, and fresh objects may share one result tuple. Rustia validates all
fresh-object registrations and live result handles before allocating guest
return values. These checks do not undo application work already done by the
callback. Owned snapshots cannot represent Rust objects; use live results.

### Length and generalized iteration

Issue [#63](https://github.com/slowchop/rustia/issues/63) adds reset-safe,
typed collection hooks without exposing raw VM handles:

```rust
ObjectType::new("Words")
    .length(1, |words: &Words| words.items.len() as i64)
    .iterator(
        1,
        |words: &Words| Ok::<_, rustia::Error>((words.items.clone(), 0_usize)),
        |(items, index), _: Option<i64>| {
            *index += 1;
            items.get(*index - 1).cloned().map(|word| (*index as i64, word))
        },
    )
```

`length` implements Luau `__len`. `iterator` takes a fallible factory and a
step. The factory briefly borrows the object once per loop and returns owned
state; the step mutates only that loop's private state and receives Luau's
previous control value for typed validation. `None` completes the loop.
For a scalar or tuple result, the first value is both the next control and the
first loop variable, matching an ordinary Luau iterator function. Its Rust type
must exactly match the incoming `Option<C>` control; empty and mismatched step
results do not compile. Fallible steps use the
normal callback diagnostic and protected-call behavior.

Both hooks are synchronous. The iterator factory's read borrow is released
before its state is admitted to the guest heap; steps never borrow the original
object. The factory and each step charge the declared host fuel. Hook
definitions survive reset, while object instances and controls retain the usual
VM/epoch rules. The same implementation is used by the default, transferable,
native and Wasm builds. Nested/interleaved loops get independent snapshots, and
an early break lets the unreachable state be collected. This is deliberately
not a broad Rust collection API.
`length_with_capability` and `iterator_with_capability` declare captured-state
behavior when the payload callback is not replay-stable; profile admission then
uses the same rules as capability-specific methods.

### Low-level owned hooks

The issue-#50 `rustia-vm` definition additionally has optional `__index`,
`__newindex`, `__eq`, `__tostring`, `__call`, `__len`, and generalized iterator
host operations. They use the
same checked receiver, capability admission, fuel, scoped dispatcher and panic
boundary as methods. Existing instances retain their original protected
definition after replacement. This is the primitive Luvy uses for ECS-backed
views. `ObjectType` exposes the bounded length/iterator subset; typed field,
equality, display and call builders remain tracked by issue #54.

Run the complete [game-turn example](../crates/rustia/examples/game_objects.rs)
with `cargo run -p rustia --example game_objects`. It applies a successful
move, cancels a later decision, and checks that the cancelled requests never
change the game. The [mutable example](../crates/rustia/examples/mutable_objects.rs)
shows an immediate change surviving a script error.

## Choose how a method changes the application

All methods use the same Luau syntax, such as `player:request(5)`. The Rust
definition determines whether a call reads input, stages a request, or changes
state immediately.

| `ObjectType` method | Callback receives | Contract |
|---|---|---|
| `method` | `&T` | Read-only payload; replay-stable data and captures |
| `method_with_capability` | `&T` | Explicit capability for captured state |
| `input` / `slot_read` | `&T`, then `Input` / `Slots` | Read this invocation's input or saved-state snapshot |
| `effect` / `slot_write` | `&T`, then `Effects` / `SlotWrites` | Stage changes in the existing ordered journal |
| `method_mut` | `&mut T` | Change the payload immediately |
| `external` / `external_mut` | `&T` / `&mut T`, then `HostMut` | Access external application state immediately |
| `method_mut_nondeterministic`, `nondeterministic`, `nondeterministic_mut` | Mutable payload or external context as selected | Explicit nondeterministic work |

The context helpers use `ObjectType<T, H>` with `build_for_host::<H>()` and the
existing borrowed `HostContext`. Their adapter traits are sealed marker bounds;
applications supply closures or function pointers, not custom dispatch code.

Deterministic game logic uses replay-stable read-only payloads plus the
`input`, `slot_read`, `effect`, and `slot_write` methods. Requests share the
invocation's ordering and record-count/byte limits. A failing method discards
its own staged suffix. A failure escaping the outer invocation discards every
record. If guest `pcall` catches an error, earlier completed requests remain;
the successful outer invocation returns them for the application to apply.
There is no second object-specific queue or automatic application commit.

`standard()` and `live()` allow explicitly declared immediate mutation.
Payload changes, application changes, and I/O survive a later script error or
failed installation. Reset drops payloads but does not undo effects on the
external application. Existing Rust payloads are shared across guest-state
savepoints rather than copied for rollback; provisional objects are released
when their guest state is discarded.

A deterministic VM rejects the entire definition if any method declares a
forbidden capability. Rejection preserves a previous registration; it never
silently removes methods. A read-only receiver is still a promise by the host:
captures and interior mutability must honor the declared capability.
Hardened leaves reject registration and construction before guest allocation.

## Lifetime and limits

Type definitions are persistent configuration and require an idle VM to change.
Replacing a definition changes future instances. Existing objects and detached
methods keep their original method definitions. The method tables are frozen
and their metatables are protected from guest modification.

Guest aliases and host roots keep an instance alive. Collection releases an
unreachable payload. Decision reset invalidates all old handles and releases
instance payloads even when stale Rust handles remain. It rebuilds only the
current registered definitions. Dropping the VM releases its remaining payloads.

Owned conversions root partial tables, table keys, and earlier arguments while
later values allocate. Memory-pressure collection must preserve values until
the enclosing table write or function/object call has consumed them.

Rustia admits method metadata, instance storage, and public roots under its
logical-live memory quota. Arbitrary allocations inside `T` remain application
memory, like callback captures. `T` must be `'static`, but need not be `Clone`,
`Send`, or `Sync` in the default local mode. The facade's opt-in `send`
feature selects the lower-level `send` mode and
requires payloads to be `Send + Sync` and uses checked shared read/write guards;
it still permits only one exclusive VM executor. Method closures retain the
normal `Send + Sync + 'static` bounds. Payload and capture
destructors must not panic. The application must avoid hidden Rust ownership
cycles that would keep its own data alive.

Object definitions and handles keep the exact registered Rust type. Payload and
schema brands are invariant, including for function-pointer subtypes, so a safe
Rust coercion cannot change a type after its callback adapters have erased it.
Compile-only regressions cover these brands without expanding the public API
reference's examples.

Typed-facade custom fields/metamethods, borrowed scopes, async methods, and
automatic Rust payload snapshots remain outside this milestone; only the
lower-level owned-hook primitive described above is implemented.

## Evidence and remaining work

`crates/rustia/tests/typed_objects.rs` exercises public registration, replacement,
identity, callback arguments/results, receiver validation, reset, panic handling,
immediate mutation, and the shared typed journal. Private VM tests in
`crates/rustia-vm/src/vm/host_objects/tests.rs` cover guest aliases, single payload
destruction, frozen methods, quota rejection, rollback, and reset reconstruction.
`object_context.rs` tests borrowed views and context validation.

These tests define the implemented contract. Final CI, reviews, the deterministic
game-turn example, and the merged civfun Player integration are recorded against
the exact candidate in [93](93-private-api-acceptance.md), alongside yieldable
`require` and private-platform acceptance. Wider runtime parity and the remaining
public-release gates stay open under [the release plan](92-first-release.md).
