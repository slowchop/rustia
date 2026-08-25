//! Diagnostic-only census of which executor served a guest run.
//!
//! Every field is an exact count aggregated over the VM's currently loaded
//! programs. The counters exist only under `cfg(test)` or the default-off
//! `executor-diagnostics` feature, so production and benchmark builds carry no
//! instrumentation; see [`crate::Vm::executor_census`].
//!
//! This answers "which executor actually owns this workload" — the question
//! that decides whether an optimization measured in executed instructions can
//! translate into time, because attaching an optimized logical-unit schedule
//! deselects the HotIR and leaf-call plans entirely.

/// Aggregated executor counters across every loaded program.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecutorCensus {
    /// Total logical instructions the VM has charged, the denominator every
    /// executor share is measured against.
    pub executed_instructions: u64,
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
    /// Table inline-cache hits.
    pub table_inline_cache_hits: u64,
    /// Table inline-cache fills.
    pub table_inline_cache_fills: u64,
    /// Native-call inline-cache hits.
    pub native_call_inline_cache_hits: u64,
    /// Native-call inline-cache fills.
    pub native_call_inline_cache_fills: u64,
}
