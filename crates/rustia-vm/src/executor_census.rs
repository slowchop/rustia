//! Diagnostic-only census of which executor served a guest run.
//!
//! Every field is an exact count aggregated over the VM's currently loaded
//! programs. The counters exist only under `cfg(test)` or the default-off
//! `executor-diagnostics` feature, so production and benchmark builds carry no
//! instrumentation; see [`crate::Vm::executor_census`].
//!
//! This answers "which executor actually owns this workload" — the question
//! that decides whether an optimization measured in executed instructions can
//! translate into time, because a prototype whose own slice of the attached
//! logical-unit schedule is non-identity loses its HotIR loops (keyed on the
//! prototype containing the loop) and its leaf-call plan (keyed on the callee
//! the plan replays); a rewrite in a different prototype deselects nothing
//! (`FunctionInlineCaches::new` and `recognize_leaf_call_plan`, per
//! prototype since 2026-09-15; see `design/58-vm-core2.md` §C2.3).
//!
//! Leaf-call plans are served on two paths and counted separately: the `Call`
//! opcode's inline cache (`closure_call_fast_entries`) and the dynamic
//! `call_value` path (`leaf_call_value_entries`) that generic-for iterator
//! steps, host `call_function`, `pcall`/`xpcall`, and coroutine starts take.
//! Both charge the plan's fixed unit count.

/// Aggregated executor counters across every loaded program.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutorCensus {
    /// Total logical instructions the VM has charged, the denominator every
    /// executor share is measured against.
    pub executed_instructions: u64,
    /// Times the VM rebuilt the current frame's GC root vector.
    pub gc_root_refreshes: u64,
    /// Values copied into frame root vectors by those refreshes. This is the
    /// work a C2.4 window scan would replace with a borrow.
    pub gc_root_values_copied: u64,
    /// Root refreshes taken on the tape's guard-exit path, which refreshes
    /// unconditionally rather than through the predicate.
    pub gc_root_refreshes_tape_exit: u64,
    /// Times the plain-mode tape was entered at a function or block boundary.
    pub tape_entries: u64,
    /// Source instructions the tape executed without returning to the scalar
    /// dispatcher.
    pub tape_instructions: u64,
    /// Fused four/six-operation receipts the tape committed.
    pub tape_receipts: u64,
    /// Tape entries that ended in a guard side-exit to the scalar dispatcher.
    pub tape_guard_exits: u64,
    /// Times a recognized HotIR loop was entered.
    pub hot_ir_entries: u64,
    /// Loop iterations HotIR completed without per-instruction dispatch.
    pub hot_ir_iterations: u64,
    /// HotIR entries that ended in a guard side-exit.
    pub hot_ir_guard_exits: u64,
    /// Calls served by the closure-call fast entry.
    pub closure_call_fast_entries: u64,
    /// Closure-call prototype PIC hits.
    pub closure_call_prototype_hits: u64,
    /// Closure-call prototype PIC fills.
    pub closure_call_cache_fills: u64,
    /// Closure-call fast entries that side-exited.
    pub closure_call_guard_exits: u64,
    /// Leaf-plan replays served through the dynamic `call_value` path
    /// (generic-for iterator steps, host `call_function`, `pcall`/`xpcall`,
    /// coroutine starts) rather than the `Call` opcode's inline cache. Both
    /// paths charge the plan's fixed unit count. Attributed to the callee's
    /// program (the path has no caller site), whereas
    /// `closure_call_fast_entries` is attributed to the caller's.
    pub leaf_call_value_entries: u64,
    /// Table inline-cache hits.
    pub table_inline_cache_hits: u64,
    /// Table inline-cache fills.
    pub table_inline_cache_fills: u64,
    /// Native-call inline-cache hits.
    pub native_call_inline_cache_hits: u64,
    /// Native-call inline-cache fills.
    pub native_call_inline_cache_fills: u64,
}
