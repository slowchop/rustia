# VM Source

## Overview

[lib.rs](./lib.rs) exports selected engine types. [vm.rs](./vm.rs) integrates the heap, services, settings, bytecode execution, resource accounting, transactions, reset, and standard libraries. Internal modules hold heap and dispatch implementation details.

## Interface

- Public modules: `bytecode`, `diagnostic`, `error`, `extension`, `instruction_facts`, `liveness`, `number`, `profile`, and `vm`.
- `Value` is the sealed runtime value; `Program`, `Function`, `Instruction`, and `Opcode` describe bytecode.
- `Function::upvalue_names` holds optional lexical capture metadata alongside descriptors. `upvalue_name` returns no name when metadata is absent or its length differs from the descriptor count; it does not infer names for hand-authored bytecode. Program memory accounting includes name vectors and string allocations.
- `Vm` exposes explicit `run`, call, retention, transaction, reset, and settings/service APIs. `RetainedValue` is checked against VM ownership and decision epoch.
- `Vm::retain_program_entry` constructs a callable main function with an exact rooted environment without executing code. Program/function/root growth is quota-admitted together; its ordinary call path retains source metadata, parameters and validation. This does not replace builtin require policy or create a new execution budget.
- Host object definitions may include length plus generalized-iterator factory/step operations. Factory and step definitions are rebuilt with persistent definitions on reset; per-loop states remain decision-local guest userdata.
- The saved-caller driver covers owned bytecode callers, implicit field/operator/import/iterator callbacks, and selected native helpers (`gsub`, `tostring`, `foreach`, and `foreachi`). Transferred effects first admit complete saved control memory. Synchronous native callbacks share the direct route's non-yieldable boundary: they report non-yieldable status and reject an attempted yield as an ordinary catchable Luau error at the call site. Boundary depth is coroutine-local, so resumed child coroutines remain independently yieldable and restore the parent boundary on return. Borrowed stateless entry ownership remains unchanged; this is not unrestricted deep re-entry for every native helper.
- [`vm/EGM.md`](./vm/EGM.md) documents the private call/host-object/require implementation.
- Source roles: [bytecode.rs](./bytecode.rs) (program/opcodes), [diagnostic.rs](./diagnostic.rs) and [error.rs](./error.rs) (runtime reporting), [extension.rs](./extension.rs) (host operations), [instruction_facts.rs](./instruction_facts.rs) and [liveness.rs](./liveness.rs) (analysis), [number.rs](./number.rs) (numeric rules), [profile.rs](./profile.rs) (engine policy), [value.rs](./value.rs) (runtime value), and [vm.rs](./vm.rs) (execution integration).
- Internal storage/execution modules: [heap.rs](./heap.rs), [host_object.rs](./host_object.rs), [ownership.rs](./ownership.rs) (feature-selected owners and service/root guards), [hot_ir.rs](./hot_ir.rs), [register_blocks.rs](./register_blocks.rs), [tape.rs](./tape.rs), and [executor_census.rs](./executor_census.rs) (diagnostic-only counters). They are not the facade API.

## Invariants

- `Vm::create_retained_string_bytes` admits arbitrary byte strings and their roots together, rechecking atom-index growth after quota collection. Luvy's runtime adapter exercises NUL-bearing panic strings through this boundary.

- Caller-selected error diagnostics resolve source and mapped spans from the exact caller program/frame, with location-only fallback. Wrapped coroutines carry original handle-free diagnostics across propagation; guest strings and explicit string-rethrow sites are not rewritten. Non-table writes share oracle-checked index errors with reads.

- Public execution returns release internal error tracebacks. Debug snapshots preserve function/environment metadata without saved register roots, so failed allocation graphs can be reclaimed before VM reuse. OOM host diagnostics record the stopped instruction while keeping bare guest error text.

- Call frames retain prototype/PC coordinates for the active instruction, including expanded-call dispatch and restored continuations; implicit calls resolve those coordinates lazily for error levels and debug caller locations. Generic iterator callbacks also receive the active loop call site.
- `setfenv` keeps logical call headers and transferred bytecode frames coherent. Level zero updates only the current coroutine's recorded call/active-frame bases, so nested resumers retain their own thread environment across child yield and return.

- Plain dispatch attempts the predecoded tape before fetching canonical bytecode. Completed tape runs fall directly into canonical admission at the returned PC, without retrying tape eligibility. Loader-established tape boundaries preserve scalar calls and HotIR-owned backedges. Guard exits refresh roots and enter scalar semantics without retrying fast admission or repeating accounting.

- Recognized array-iterator leaves validate the table handle and reject metatables before the existing boundary calculation. The guard precedes index mutation; general length and lookup APIs are unchanged.
- Core numeric helpers canonicalize arithmetic NaNs and table-key zero representations. The default Luau byte-string numeric coercion is shared with compiled callers through `number::parse_numeric_text`.
- The native `string.gmatch` iterator matches `%a+` over ASCII letters and `%w+` over ASCII letters and digits. It advances by byte offsets.
- String-index conversion accepts the half-open signed-64 range and truncates accepted fractions toward zero; `2^63` is rejected ([numeric-index contract](../../../design/95-numeric-index-contract.md)). The nine dedicated string-opcode bodies share a non-inlined helper within `vm.rs`, preserving register access, fuel charges, and error locations.
- Successful scalar leaf calls skip caller-register synchronization only when loader analysis of actual bytecode proves the caller cannot attach local capture cells. `GetLocalCell`, `SetLocalCell`, or `LoadFunction` keeps the scan; unknown frames fall back conservatively. The proof is read with the existing caller-site cache lookup on eligible scalar calls, rather than at every frame entry. Optional capture hints do not establish this proof ([VM design](../../../design/58-vm-core2.md)).
- Under `cfg(test)` or the default-off `executor-diagnostics` feature, `ExecutorCensus` partitions root refresh attempts into clean-accounted, dirty-skip, and no-frame counts. The partition records the accounting state at the refresh call; it does not infer refreshes from opcode classifications or count optimized calls that return before root refresh.
- Iterator steps reuse a resolved function kind after built-in rejection through existing shared executors with iterator-specific entry glue; no guest execution or heap mutation crosses that reuse, and invalid handles/nonfunctions keep ordinary fallback and error precedence.
- Scalar leaf-call admission borrows function kind and captures from one generation-checked slot access. It copies only the selected capture pair before cache mutation, without caching capture values or changing fallback diagnostics.
- Native and opcode unary math entries validate their own call shape, then share value coercion and the numeric operation. Native calls do not copy arguments into a temporary register vector or fabricate an opcode instruction.
- Program logical-unit schedules must cover instructions with units of at least one; VM accounting uses them to preserve logical fuel/collection schedule when present.
- Decision reset requires an idle VM and invalidates retained roots by epoch.
- Heap storage is owned: runtime long strings use boxed bytes, compiled strings share immutable program bytes, and all guest edges use generation-checked handles. Explicit tracing retains transaction roots and sweeps weak edges at the existing instruction/manual/quota boundaries; there is no auxiliary arena collector or allocation-debt gate. Default local owners retain `Rc`/`RefCell` support. Opt-in `send` selects `Arc` owners: mutable clocks/output/resolvers require `Send`, entropy requires `Fn + Send + Sync`, payloads require `Send + Sync`, and retained-root registry locks never enclose callbacks. Runtime service/payload guards use nonblocking acquisition; cloned settings may read the immutable entropy provider concurrently while mutable providers reject conflicting entry. Payload read aliases remain valid. Guard poison recovery leaves the VM host-panic/reset policy unchanged.
- Heap transaction snapshots retain existing function handles and copy only mutable captures/environment; immutable function kind, host definition and module cohort stay owned by the retained function. Rollback, roots and logical quota accounting follow that separation.
- VM memory statistics report logical live/reserved/peak accounting, not operating-system memory. Owned-string admission includes string-index growth; tests compare it with the independent census. Removed arena headers are no longer charged, so earlier exact byte totals do not describe this storage layout.
- Production `unsafe` is forbidden by workspace policy.

## Side Effects

VM calls execute guest code, mutate heap and counters, allocate/collect, and may dispatch host callbacks or configured services. Guest transactions can roll back VM-owned guest state; external callback effects are outside that rollback.

## Verification

From the repository root, run `cargo test --locked -p rustia-vm` and repeat with
`--features send`. Compile the transferable lower-level boundary for Wasm with
`cargo check --locked -p rustia-vm --target wasm32-unknown-unknown --features send`.
