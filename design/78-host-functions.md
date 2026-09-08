# Synchronous host functions

Status: synchronous API and controlled-client integrations implemented and verified (2026-09-08).

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

### Delivery record

The runtime merged in [Rustia #30](https://github.com/slowchop/rustia/pull/30) as
`fc0d19a56ba7cc58a4d4c2f03cd32bf3a6d352c9`, after the canonical release preflight,
platform checks, native/wasm execution and PR benchmark screening passed. Client pins and vendor
copies use that exact merged runtime revision, not an unmerged implementation branch.

- WOTS: [#4](https://github.com/slowchop/wots/pull/4) merged. Its board callbacks use shared definitions
  in a persistent host module; format, strict Clippy and 447 tests pass, including stock/cold-open playthroughs.
- civfun: [#42](https://github.com/civfun/civfun/pull/42) merged. Direct Map forwarding functions are
  host-created live values; captured-state callbacks use shared definitions with explicit capabilities.
  All 49 tests, strict Clippy, two independent reviews, audit and full-map smoke checks pass.
- Grabby: [#1021](https://github.com/slowchop/grabby-next/pull/1021) adopts shared definitions across
  scenario generation, both consoles and the typed Leisure module. Its five runtime crates match the exact
  merged revision; all six roots retain the verified canonical dependency closure. All 3,249 full-suite tests,
  78 scenario tests, strict Clippy, real native/wasm attestation and independent pre/post reviews pass.

Each client PR records its own required delivery checks and merge state. The rollout does not bypass
any client repository's merge gate or include production deployment.

No runtime performance improvement is claimed by this rollout. Async callbacks, borrowed scopes,
userdata registration and stronger stack diagnostics remain separate follow-on work.
