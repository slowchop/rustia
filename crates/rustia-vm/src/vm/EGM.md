# VM Execution Submodules

## Overview

This private module directory separates specialized VM call paths and host integration out of the main [vm.rs](../vm.rs) engine file.

## Interface

- [builtin_arguments.rs](./builtin_arguments.rs): argument shape checks for specialized builtins.
- [bytecode_calls.rs](./bytecode_calls.rs) and [expanded_calls.rs](./expanded_calls.rs): ordinary bytecode calls, expanded argument/result chains, and continuation handling.
- [import_continuations.rs](./import_continuations.rs): validates/copies the bounded import descriptor and resumes the next lane after an implicit lookup. Expanded-call resume variants retain already evaluated arguments and method keys, so late lookup never replays argument evaluation, receiver selection or fuel charges.
- Generalized `__iter` preparation saves its pending instruction and resumes the returned iterator/state/control triple once. The setup metamethod retains its non-yieldable boundary; the actual iterator step keeps the existing coroutine continuation. Borrowed stateless entry calls do not install a loaded program.
- Transferred errors regain the saved triggering instruction/source before protected unwinding, matching synchronous opcode errors without replacing an existing location or explicit suppression.
- [implicit_operations.rs](./implicit_operations.rs) and [implicit_operators.rs](./implicit_operators.rs): shared direct/callback resolution for indexed/global reads and writes, method lookup, equality, arithmetic, ordering, concatenation and length. Bytecode saves first/discard/truthy/method-validation completion without repeating lookup or effects. Original cache paths remain shared with synchronous native users.
- [native_continuations.rs](./native_continuations.rs): isolated saved-completion prototype for function-based gsub replacements, tostring metamethod results and foreach/foreachi iteration cursors; the existing driver owns their roots, program references, fallible snapshots and control-memory accounting. Foreach retains its selected key and snapshotted array length, reads later values after each callback, and stops on the first non-nil result. [native_completion_tests.rs](./native_completion_tests.rs) covers borrowed stateless entries, callback depth precedence and native-service quota admission.
- [host_functions.rs](./host_functions.rs): dispatch to facade-owned typed host callbacks.
- [scoped_host.rs](./scoped_host.rs): borrowed low-level callback execution scope;
  nested calls retain outer fuel, caller depth and dispatcher. Legacy dispatchers
  forward unchanged. This does not implement scheduler ownership or facade reentry.
- [captures.rs](./captures.rs): rooted script capture/source/environment inspection, idle capture edits, raw one-pair table traversal and trusted readonly/metatable control; mutations preserve quota admission and savepoints. [captures_tests.rs](./captures_tests.rs) checks legacy metadata, active-call rejection, quotas, rollback and reclaimed/slot-reused roots.
- [program_entry.rs](./program_entry.rs): checked retained main-function construction in a rooted lexical environment, sharing dynamic-main allocation with loadstring and builtin require. Construction does not execute or reset fuel/depth. [program_entry_tests.rs](./program_entry_tests.rs) checks environment lifetime, ordinary bytecode validation, source identity, quota admission and rollback.
- [host_objects.rs](./host_objects.rs): Rust payload type registration, construction, receiver validation, method-first lookup and five optional field/equality/display/call hooks. Hook operations use ordinary capability admission, fuel and invocation dispatch; original protected definitions stay attached to existing instances.
- [require.rs](./require.rs): host-resolved module loading, coroutine-owned continuation, and cache publication.

## Invariants

- Public execution boundaries in the parent VM release error tracebacks after the continuation driver completes; protected handlers retain their snapshots until then.
- The driver transfers owned-program Host/Native calls, admits saved state before effects, and leaves the large bytecode frame before synchronous Host dispatch. Borrowed stateless/hardened entries preserve their previous direct route and no-loaded-program contract; their nesting behavior is unchanged. The bounded implicit field/operator set transfers its selected callback and resumes the pending result operation; NameCall retains and traces its original receiver/key. Import/expanded late lookup saves its next lane/stage, generalized iterator setup resumes once, and selected native callback loops use explicit completion frames. Other native helpers and table-based gsub replacements retain their synchronous paths.
- Native completion preserves the existing Rustia yield boundary: gsub callbacks observe non-yieldable status; tostring keeps its prior status. An escaping yield discards inner callback/protected frames and becomes a boundary error before outer protected handlers resume. This preserves a known preexisting difference from the pinned Luau oracle.
- Call continuation state is explicit in VM frames so suspension/resumption does not rely on growing the native stack.
- A required module's zero/one result enters cache only after successful completion. Load ownership belongs to its coroutine/cohort and is released on failure, cancellation, reset, or abandoned coroutine collection.
- Retained program entries inherit the current cohort without resolving/caching modules. The environment and returned entry are VM/epoch-checked roots; hardened leaves reject construction. Host module policy remains outside the primitive.
- Host object receiver type is checked before callback argument conversion. Host payload state is outside guest savepoint rollback.
- Specialized builtin paths must preserve source evaluation order and fall back to ordinary calls when specialization shape does not apply.

## Side Effects

Call paths execute guest and host callbacks, charge fuel, and mutate VM state. `require` invokes a configured resolver and retains module programs while loading. Host method calls may perform immediate Rust-side effects according to their declared capabilities.

## Verification

From the repository root, run `cargo test --locked -p rustia-vm` and `cargo test --locked -p rustia --test yieldable_require`.
