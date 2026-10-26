# VM Source

## Overview

[lib.rs](./lib.rs) exports selected engine types. [vm.rs](./vm.rs) integrates the heap, services, settings, bytecode execution, resource accounting, transactions, reset, and standard libraries. Internal modules hold heap and dispatch implementation details.

## Interface

- Public modules: `bytecode`, `diagnostic`, `error`, `extension`, `instruction_facts`, `liveness`, `number`, `profile`, and `vm`.
- `Value` is the sealed runtime value; `Program`, `Function`, `Instruction`, and `Opcode` describe bytecode.
- `Vm` exposes explicit `run`, call, retention, transaction, reset, and settings/service APIs. `RetainedValue` is checked against VM ownership and decision epoch.
- [`vm/EGM.md`](./vm/EGM.md) documents the private call/host-object/require implementation.
- Source roles: [bytecode.rs](./bytecode.rs) (program/opcodes), [diagnostic.rs](./diagnostic.rs) and [error.rs](./error.rs) (runtime reporting), [extension.rs](./extension.rs) (host operations), [instruction_facts.rs](./instruction_facts.rs) and [liveness.rs](./liveness.rs) (analysis), [number.rs](./number.rs) (numeric rules), [profile.rs](./profile.rs) (engine policy), [value.rs](./value.rs) (runtime value), and [vm.rs](./vm.rs) (execution integration).
- Internal storage/execution modules: [heap.rs](./heap.rs), [host_object.rs](./host_object.rs), [hot_ir.rs](./hot_ir.rs), [register_blocks.rs](./register_blocks.rs), [tape.rs](./tape.rs), and [executor_census.rs](./executor_census.rs) (diagnostic-only counters). They are not the facade API.

## Invariants

- Plain dispatch attempts the predecoded tape before fetching canonical bytecode. Completed tape runs fall directly into canonical admission at the returned PC, without retrying tape eligibility. Loader-established tape boundaries preserve scalar calls and HotIR-owned backedges. Guard exits refresh roots and enter scalar semantics without retrying fast admission or repeating accounting.

- Recognized array-iterator leaves validate the table handle and reject metatables before the existing boundary calculation. The guard precedes index mutation; general length and lookup APIs are unchanged.
- Core numeric helpers canonicalize arithmetic NaNs and table-key zero representations.
- String-index conversion accepts the half-open signed-64 range and truncates accepted fractions toward zero; `2^63` is rejected ([numeric-index contract](../../../design/95-numeric-index-contract.md)). The nine dedicated string-opcode bodies share a non-inlined helper within `vm.rs`, preserving register access, fuel charges, and error locations.
- Successful scalar leaf calls skip caller-register synchronization only when loader analysis of actual bytecode proves the caller cannot attach local capture cells. `GetLocalCell`, `SetLocalCell`, or `LoadFunction` keeps the scan; unknown frames fall back conservatively. The proof is read with the existing caller-site cache lookup on eligible scalar calls, rather than at every frame entry. Optional capture hints do not establish this proof ([VM design](../../../design/58-vm-core2.md)).
- Iterator steps reuse a resolved function kind after built-in rejection through existing shared executors with iterator-specific entry glue; no guest execution or heap mutation crosses that reuse, and invalid handles/nonfunctions keep ordinary fallback and error precedence.
- Scalar leaf-call admission borrows function kind and captures from one generation-checked arena access. It copies only the selected capture pair before cache mutation, without caching capture values or changing fallback diagnostics.
- Native and opcode unary math entries validate their own call shape, then share value coercion and the numeric operation. Native calls do not copy arguments into a temporary register vector or fabricate an opcode instruction.
- Program logical-unit schedules must cover instructions with units of at least one; VM accounting uses them to preserve logical fuel/collection schedule when present.
- Decision reset requires an idle VM and invalidates retained roots by epoch.
- VM memory statistics report logical live/reserved/peak accounting, not operating-system memory.
- Production `unsafe` is forbidden by workspace policy.

## Side Effects

VM calls execute guest code, mutate heap and counters, allocate/collect, and may dispatch host callbacks or configured services. Guest transactions can roll back VM-owned guest state; external callback effects are outside that rollback.

## Verification

From the repository root, run `cargo test --locked -p rustia-vm`.
