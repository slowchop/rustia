# Compiler Source

## Overview

[lib.rs](./lib.rs) contains the compiler API and lowering passes. [builtins.rs](./builtins.rs) contains recognized global/library names that select specialized call evaluation order. The compiler exposes the same rule to other emitters through `defers_builtin_lookup`. [o2.rs](./o2.rs) contains default-disabled optimization rewrites and logical receipts. [source_bundle.rs](./source_bundle.rs) contains the experimental in-memory source identity and static module-graph boundary. [stateless_helper.rs](./stateless_helper.rs) applies conservative pure-function admission to a reachable graph without defining host fallback policy.

## Interface

- `compile(&Chunk) -> Result<Program>` uses default options.
- `compile_with_options` accepts source metadata and PUC constructor-order settings.
- `compile_with_options_and_receipts` also returns `ProgramReceipts` for rewritten code.
- `CompileError` carries an optional lexer span; `BuiltinEnvironment` separates public and conformance compilation modes.
- `ExperimentalO2Options::reachability_exact()` selects the documented corpus-green rewrite track; `ALL` includes dead cleanup-clear elision, which the source marks as blocked on root-liveness work.

- [compiled_call.rs](./compiled_call.rs) lowers a numeric closure with at most one numeric argument and one host-supplied capture to a backend-independent numeric expression tree. The generated Rust backend consumes that tree. The source byte and token limits apply before recursive parsing, and lowering checks that the closure actually reads or writes the declared capture. A closure may perform multiple straight-line writes to that capture and return one numeric value or no values. Bundle-based generation returns the frozen source and entry graph digests, compiler ABI, backend, and digest of the emitted Rust bytes.
- Preparing a numeric call compiles ordinary bytecode from the same admitted entry. Each instance roots an ordinary VM closure. An embedder can instead bind an existing closure by matching its numeric bytecode and keep its current capture. A host invocation can share its remaining fuel with the compiled call. At a healthy, idle boundary the VM counts the straight-line numeric instructions and advances its logical clock when the backend commits a result. A mismatched receipt, unsupported operand (including raw noncanonical NaNs), active transaction, or call that can exceed its fuel or memory allowance or cross a collection boundary uses the interpreter on that same closure.
- Stateless helpers with one numeric argument can use the same scalar lowering for a returned arithmetic expression. This backend also admits the standard `math.abs` operation; it has no setup, captures, or module resolver.
- [hosted_rust.rs](./hosted_rust.rs) lowers a wider checked Luau subset to Rust source against a caller-supplied value and operation adapter. It supports ordinary modules and isolated callbacks. The caller supplies admitted capture names and the available global set. Rustia applies the same builtin lookup rule as its interpreter compiler. The output depends on the adapter for values, calls, operators, captures and module resolution. It does not establish VM instruction metering.
- [compiled_host_call.rs](./compiled_host_call.rs) emits native/Wasm-portable Rust for a zero-argument callback with setup captures, text-literal calls or methods, and bounded branching. The embedding retains setup, handle resolution and host dispatch. Generated control flow does not charge interpreter instruction fuel.
- [callback_source.rs](./callback_source.rs) extracts an embedder-selected module callback or named factory export. It preserves source lines, reports candidate capture names and only nominates direct literal imports that are safe to analyze for specialization. Path resolution, backend selection, interpreted initialization and artifact publication remain caller decisions.
- Hosts can gate each syntactically supported setup capture by its namespace, member and exact literal argument bytes. The compiler validates the capture syntax and reports denied declarations with a source span.
- [source_bundle.rs](./source_bundle.rs) accepts immutable canonical module keys and source bytes without file I/O. Its sorted path/NUL/decimal-length/NUL/source SHA-256 encoding preserves the accepted-source identity already shared by the live compiler and host. It separately fingerprints a dependency-first graph of top-level literal `require` declarations. Embedders may supply their resolver policy; graph ordering, validation, diagnostics, and identity stay compiler-owned.
- `SourceBundle::stateless_function_graph[_with]` additionally requires every reachable module to return a function whose file-scope declarations are only literal scalars and direct imports. Function bodies may use locals, those immutable scalars, admitted imports, pure scalar builtins, and the bounded `math` operations implemented by the generated-Rust consumer. Mutable captures, host effects, indirect calls, nested closures, stateful initialization, and broader control flow reject with module identity and a source span.

## Invariants

- Source lines for calls follow the callee end or method name. Indexed stores retain the target location across RHS evaluation. Iterator callback lines identify the loop header.
- [repeat_condition.rs](./repeat_condition.rs) validates skipped bindings using the default Luau 0.709 compiler rules. A direct reference in `until` is checked lexically. A nested function contributes its surviving captures. Captured folded values reach the condition even on a jump over their declarations.
- Type function declarations emit no bytecode, constants or function prototypes for their erased bodies.
- Capture discovery is a whole-tree prepass so assignment sites account for captures and source-later global writes.
- Each compiled capture retains its lexical name at the matching upvalue-descriptor index, including transitive captures and direct immutable values. Source provenance stays in `Program::source_name` and function debug metadata. Names do not change capture execution or cell sharing.
- Capture interning consults existing descriptors after lexical local-name maps
  are restored. Repeated branch/loop reads and transitive closures reuse one
  capture slot for the same parent binding, without merging shadowed bindings.
- O2 is all-off by default. Rewrites must preserve evaluation/error order and logical schedule.
- Do not treat the experimental option fields as a stable public compiler contract.
- Do not treat the source-bundle ABI labels or graph representation as a stable public API. Nested or dynamic `require` calls remain runtime resolver work and are not silently included in the static graph.
- Stateless admission is conservative compiler policy, not proof that an arbitrary backend implements the admitted syntax. A host may keep an unsupported function in the interpreter; admission does not choose fallback or artifact activation.

- The interpreter compiler ignores validated function attributes. Explicit instantiation evaluates its operand once to a single result. As a method receiver or callee, that value is read before the arguments. A grouped local receiver still follows Luau’s late register read. Writes through instantiated globals still invalidate affected builtin fast paths.

## Side Effects

Compilation is in-memory and does not execute the resulting program or perform host I/O.

## Verification

From the repository root, run `cargo test --locked -p rustia-compiler`.
`tests/repeat_condition.rs` covers skipped binding identity, source spans, constant captures and literal loadstring recovery.
`tests/capture_metadata.rs` covers names, source provenance, shadowing, transitive captures, detached chunks, and execution equivalence with names removed.
It also covers branch restoration and repeated scoped reads which previously
duplicated names and could exhaust the distinct-upvalue limit.

Run `cargo test --locked -p rustia-compiler --test compiled_call` for source diagnostic and generated-code equivalence.
Run `cargo test --locked -p rustia-compiler --test source_bundle` for source-bundle and module-graph identity, traversal, resolver, and diagnostic coverage.
Run `cargo test --locked -p rustia-compiler --test stateless_helper` for transitive pure-helper admission and state/effect rejection.
