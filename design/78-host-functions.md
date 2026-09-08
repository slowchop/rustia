# Synchronous host functions

Status: synchronous API implemented; controlled-client rollout pending (2026-09-07).

The authorized rollout completes synchronous function construction and migrates the controlled
civfun, WOTS, and Grabby clients. A callback has one definition and one set of conversion,
capability, fuel, and error rules, whether registered persistently or constructed as a live value.

## Ownership

- A created `Function` is an ordinary VM/epoch-bound rooted value. Guest tables and closures can
  keep it alive after the host drops its handle. Replacing a binding does not invalidate other
  references to the old function. Collection releases an unreachable function's callback ownership.
- Persistent registrations own callback definitions as host configuration. Decision reset rebuilds
  their bindings while invalidating every old live handle. Replacing a registration releases the
  previous configuration's ownership, independently of guest references to its old function.
- Names provide diagnostics or select table bindings; they do not select callback lifetime.
- Heap function objects own callback definitions, including type-erased contextual handlers. There
  is no append-only disposable callback or contextual-dispatch registry.
- Heap transaction snapshots keep pre-existing handlers alive until commit or rollback. Failure
  releases provisional function ownership, and construction admits the function plus its public
  root under the logical-live quota atomically.

## Public shape

`HostFunction<C>` describes a synchronous callback and its capability/fuel/signature metadata.
`create_function` constructs a live function; `register_function` installs persistent configuration;
`HostModule::function` adds that same definition to a persistent namespace. Plain typed callbacks
remain concise, and the existing registration conveniences share the implementation.

Owned arguments/results use the existing typed conversions. The same typed signatures can accept
`Value`/`Table`/`Function` and return mixed owned/live tuples through arity eight, preserving graph
identity. Owned containers remain snapshots. Callback closures retain the
existing `Send + Sync + 'static` bounds; arbitrary live handles cannot be hidden in those captures.
Per-invocation state is supplied by the existing borrowed capability views. Captured host heap
allocations remain host-owned and are not claimed as VM quota-accounted memory.
Captured destructors must not panic; reclamation is not a guest finalizer or fallible callback.
The synchronous callback receives no mutable VM borrow and cannot re-enter guest execution.
Persistent configuration requires an idle VM without a guest-state savepoint, so configuration
changes cannot escape an enclosing guest rollback. `create_named_function` sets diagnostics only;
`HostFunction::with_capability` declares captured-state effects, while the typed view factories
(`input`, `effect`, etc.) supply per-invocation authority. The default fuel charge is one.

All ordinary profiles share the API. Capability checks preserve each selected environment contract.
Hardened leaves continue using pre-registered configuration and reject disposable construction
before allocation. Async callbacks, borrowed callback scopes, and userdata methods are separate work.

## Verification and rollout

Test rooted/unrooted lifetime, replacement aliases, registration replacement, typed dispatch,
reset invalidation and callback release, shared/cyclic guest graphs, live arguments/results,
cross-VM validation, quota failure/recovery, transaction rollback, and host panic behavior.
Run the canonical release preflight and required PR checks. Obtain any performance evidence only
from the PR's `ci-bench` workflow. Merge Rustia before updating client dependency pins/vendor copies.
Migrate direct client forwarding bindings where useful, preserve wrappers with meaningful argument
or result transformations, and verify all active clients in the same rollout.
