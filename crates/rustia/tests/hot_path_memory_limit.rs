//! Regression coverage for the HotIR/tape memory-limit-overshoot finding
//! (design/58-vm-core2.md SS C2.3, "fast-tier memory-limit overshoot",
//! found 2026-09-15): a numeric loop admitted into HotIR or the tape's
//! dense-array-append lane is not re-checked against `memory_limit_bytes`
//! until it yields back to the scalar dispatcher at the next
//! deterministic-collection boundary, so growth inside the loop can outrun
//! the configured limit by up to one `deterministic_gc_interval` window
//! before `OutOfMemory` fires. These tests pin that bound so a future change
//! to fast-tier admission can't silently widen it into an unbounded
//! overshoot.
//!
//! Only reachable through the unmetered `Rustia::eval`/`exec`/`install`
//! surface: every typed `invoke`/`invoke_entry_into` call always threads an
//! `InvocationLimits` fuel budget into `fuel.state.fuel`
//! (`crates/rustia/src/invocation.rs::Invocation::new`), and both HotIR and
//! the tape require `fuel.state.fuel.is_none()` to be admitted at all -- so
//! the hardened-leaf profile, which mandates a memory limit, never actually
//! hits this path.

use rustia::{ErrorKind, RustiaBuilder, VmSettings};
use std::num::NonZeroU64;

fn headroom_limit(extra: usize) -> usize {
    let mut baseline = RustiaBuilder::standard().build().unwrap();
    baseline.memory_stats().live_bytes.saturating_add(extra)
}

fn assert_out_of_memory(result: &Result<i64, rustia::Error>) {
    assert!(
        matches!(
            result,
            Err(e) if matches!(&e.kind, ErrorKind::Vm(v) if matches!(v.kind, rustia::VmErrorKind::OutOfMemory))
        ),
        "expected OutOfMemory, got {result:?}"
    );
}

fn overshoot_for_interval(interval: u64, source: &str) -> usize {
    let limit = headroom_limit(8 * 1024);
    let settings = VmSettings {
        memory_limit_bytes: Some(limit),
        deterministic_gc_interval: NonZeroU64::new(interval).unwrap(),
        ..VmSettings::default()
    };
    let mut vm = RustiaBuilder::standard()
        .settings(settings)
        .build()
        .unwrap();
    let result: Result<i64, _> = vm.eval(source);
    let after = vm.memory_stats().live_bytes;
    assert_out_of_memory(&result);
    after.saturating_sub(limit)
}

const NUMERIC_APPEND: &str = "local t = {} for i = 1, 5000000 do t[i] = i end return #t";

#[test]
fn hot_ir_numeric_array_append_overshoot_tracks_the_gc_interval_not_the_loop_length() {
    // A pure-numeric `t[i] = i` for-loop is exactly the array_mutation shape
    // that promotes to HotIR (design/58-vm-core2.md SS C2.3's tracked
    // workload); HotIR is a default-on fast tier, not gated by
    // ExperimentalO2Options, so this needs no O2 opt-in.
    let tight = overshoot_for_interval(8, NUMERIC_APPEND);
    let default = overshoot_for_interval(10_000, NUMERIC_APPEND);
    assert!(
        tight < 2_000,
        "a tight deterministic-collection interval should keep the overshoot near the \
         scalar dispatcher's per-instruction precision, got {tight} bytes"
    );
    assert!(
        default < 200_000,
        "the default interval's overshoot should stay within a small multiple of one \
         interval's worth of array growth, not grow toward the 5,000,000-iteration loop's \
         full working set (~80 MB uncapped); got {default} bytes"
    );
    assert!(
        default > tight,
        "overshoot should grow with the deterministic-collection interval, confirming the \
         mechanism is the interval-bounded re-check gap and not some unrelated slack"
    );
}

#[test]
fn tape_dense_array_set_overshoot_is_bounded_the_same_way() {
    // Values that disqualify HotIR (a call-free but non-numeric assignment)
    // still reach the general tape's TapeOp::DenseArraySet lane, which has
    // the identical gap: try_write_numeric_array_slot never consults
    // memory_limit_bytes.
    let source =
        "local t = {} local s = string.rep('x', 32) for i = 1, 5000000 do t[i] = s end return #t";
    let tight = overshoot_for_interval(8, source);
    let default = overshoot_for_interval(10_000, source);
    assert!(tight < 2_000, "got {tight} bytes");
    assert!(default < 200_000, "got {default} bytes");
    assert!(default > tight, "default={default} tight={tight}");
}

#[test]
fn scalar_dispatch_stays_within_one_instruction_of_the_limit() {
    // Reference point: forcing the scalar dispatcher (any fuel budget at all
    // disqualifies both HotIR and the tape) enforces the limit every
    // instruction via `maybe_collect` -> `enforce_memory_limit`, so the
    // overshoot here is the bug's absence, not its bounded form.
    let limit = headroom_limit(8 * 1024);
    let mut vm = RustiaBuilder::standard()
        .memory_limit_bytes(limit)
        .fuel(100_000_000)
        .build()
        .unwrap();
    let result: Result<i64, _> = vm.eval(NUMERIC_APPEND);
    let after = vm.memory_stats().live_bytes;
    assert_out_of_memory(&result);
    assert!(
        after.saturating_sub(limit) < 200,
        "scalar dispatch should catch the limit within about one instruction, got {} bytes over",
        after.saturating_sub(limit)
    );
}

#[test]
fn hardened_leaf_invocations_never_reach_the_fast_tiers_that_have_this_gap() {
    // The one profile that mandates memory_limit_bytes always sets fuel too
    // (InvocationLimits is required by every invoke*/invoke_entry_into
    // call), so it can never admit HotIR or the tape and never observes this
    // overshoot; this test pins that safety property directly rather than by
    // inference.
    use rustia::{HostContext, HostSchema, InvocationLimits};

    struct Schema;
    impl HostSchema for Schema {
        type Input = ();
        type Slots = ();
        type External = ();
        type Effect = ();
        type SlotWrite = ();
        type Record = ();
    }
    impl HostContext<Schema> for () {
        fn input(&self) -> &() {
            self
        }
        fn slots(&self) -> &() {
            self
        }
        fn external_mut(&mut self) -> &mut () {
            self
        }
        fn record_effect(&self, _sequence: u32, _effect: ()) {}
        fn record_slot_write(&self, _sequence: u32, _write: ()) {}
        fn record_bytes(&self, _record: &()) -> u64 {
            0
        }
    }

    let limit = headroom_limit(8 * 1024);
    let mut vm = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(limit)
        .build_for_host::<Schema>()
        .expect("hardened leaf VM");
    let module = rustia::Rustia::compile_module(
        "hot-loop.luau",
        "return { decide = function() local t = {} for i = 1, 5000000 do t[i] = i end return #t end }",
    )
    .expect("module compiles");
    let mut context = ();
    let error = vm
        .invoke(
            &mut context,
            InvocationLimits::new(100_000_000),
            |invocation| {
                let exports = invocation.install(&module)?;
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, i64>(invocation, ())
            },
        )
        .expect_err("the loop must still hit the memory limit");
    assert!(matches!(
        error,
        rustia::Error { kind: ErrorKind::Vm(ref inner), .. }
            if matches!(inner.kind, rustia::VmErrorKind::OutOfMemory)
    ));
    let after = vm.memory_stats().live_bytes;
    assert!(
        after.saturating_sub(limit) < 200,
        "a hardened-leaf invocation always carries fuel, so it must stay on the scalar \
         dispatcher and enforce the limit tightly; got {} bytes over",
        after.saturating_sub(limit)
    );
}
