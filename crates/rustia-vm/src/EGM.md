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

- Core numeric helpers canonicalize arithmetic NaNs and table-key zero representations.
- Program logical-unit schedules must cover instructions with units of at least one; VM accounting uses them to preserve logical fuel/collection schedule when present.
- Decision reset requires an idle VM and invalidates retained roots by epoch.
- VM memory statistics report logical live/reserved/peak accounting, not operating-system memory.
- Production `unsafe` is forbidden by workspace policy.

## Side Effects

VM calls execute guest code, mutate heap and counters, allocate/collect, and may dispatch host callbacks or configured services. Guest transactions can roll back VM-owned guest state; external callback effects are outside that rollback.

## Verification

From the repository root, run `cargo test --locked -p rustia-vm`.
