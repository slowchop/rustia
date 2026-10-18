//! Memory-limited execution must enforce its quota at scalar instruction
//! boundaries, regardless of fuel or deterministic collection interval.
//! HotIR, the executable tape, and leaf calls stay on scalar dispatch under
//! a memory limit (design/58-vm-core2.md §C2.3).
//!
//! Compare unmetered evaluation with a fuel-forced scalar reference on the
//! numeric append shape recognized by HotIR and a non-numeric append shape
//! that previously used the tape's dense-array write lane.

use rustia::{ErrorKind, Profile, Rustia, RustiaBuilder};

mod support {
    pub mod quota_precision;
}
use support::quota_precision::{
    assert_append_matches_scalar, headroom_limit, NUMERIC_APPEND, STRING_APPEND,
};

fn quota_vm(profile: Profile, fuel: Option<u64>, limit: usize) -> Rustia {
    let mut builder = RustiaBuilder::standard()
        .profile(profile)
        .memory_limit_bytes(limit)
        .memory_tracking(true);
    if let Some(fuel) = fuel {
        builder = builder.fuel(fuel);
    }
    builder.build().unwrap()
}

#[test]
fn numeric_array_append_matches_scalar_quota_precision_without_fuel() {
    assert_append_matches_scalar(NUMERIC_APPEND, "default", quota_vm);
}

#[test]
fn non_numeric_array_append_matches_scalar_quota_precision_without_fuel() {
    assert_append_matches_scalar(STRING_APPEND, "default", quota_vm);
}

#[test]
fn hardened_leaf_invocations_keep_single_instruction_quota_precision() {
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

    let limit = headroom_limit(Profile::ReproducibleStandard, 8 * 1024);
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
        error.kind(),
        ErrorKind::Vm(inner)
            if matches!(inner.kind(), rustia::VmErrorKind::OutOfMemory)
    ));
    let after = vm.memory_stats().live_bytes();
    assert!(
        after.saturating_sub(limit) < 200,
        "a hardened-leaf invocation always carries fuel, so it must stay on the scalar \
         dispatcher and enforce the limit tightly; got {} bytes over",
        after.saturating_sub(limit)
    );
}
