//! Memory-limited execution must enforce its quota at scalar instruction
//! boundaries, regardless of fuel or deterministic collection interval.
//! HotIR, the executable tape, and leaf calls stay on scalar dispatch under
//! a memory limit (design/58-vm-core2.md §C2.3).
//!
//! Compare unmetered evaluation with a fuel-forced scalar reference on the
//! numeric append shape recognized by HotIR and a non-numeric append shape
//! that previously used the tape's dense-array write lane.

use rustia::{ErrorKind, Profile, RustiaBuilder, VmMemoryStats, VmSettings};
use std::num::NonZeroU64;

fn headroom_limit(profile: Profile, extra: usize) -> usize {
    let mut baseline = RustiaBuilder::standard().profile(profile).build().unwrap();
    baseline.memory_stats().live_bytes.saturating_add(extra)
}

fn assert_out_of_memory(result: &Result<i64, rustia::Error>) {
    assert!(
        matches!(
            result,
            Err(e) if matches!(e.kind(), ErrorKind::Vm(v) if matches!(v.kind, rustia::VmErrorKind::OutOfMemory))
        ),
        "expected OutOfMemory, got {result:?}"
    );
}

fn append_until_out_of_memory(
    profile: Profile,
    interval: u64,
    source: &str,
    fuel: Option<u64>,
) -> (usize, VmMemoryStats) {
    let limit = headroom_limit(profile, 8 * 1024);
    let settings = VmSettings {
        memory_limit_bytes: Some(limit),
        deterministic_gc_interval: NonZeroU64::new(interval).unwrap(),
        initial_fuel: fuel,
        enable_memory_tracking: true,
        ..VmSettings::default()
    };
    let mut vm = RustiaBuilder::standard()
        .profile(profile)
        .settings(settings)
        .build()
        .unwrap();
    let result: Result<i64, _> = vm.eval(source);
    assert_out_of_memory(&result);
    (limit, vm.memory_stats())
}

const NUMERIC_APPEND: &str = "local t = {} for i = 1, 5000000 do t[i] = i end return #t";
const STRING_APPEND: &str =
    "local t = {} local s = string.rep('x', 32) for i = 1, 5000000 do t[i] = s end return #t";

fn assert_append_matches_scalar(source: &str) {
    for profile in [
        Profile::ReproducibleStandard,
        Profile::DeterministicSim,
        Profile::LuauCompat,
        Profile::Puc51Compat,
    ] {
        for interval in [1, 8, 128, 1_000, 10_000] {
            let (limit, unmetered) = append_until_out_of_memory(profile, interval, source, None);
            // A fuel budget independently forces scalar dispatch. It is large
            // enough that memory, rather than fuel, is the failing resource.
            let (_, scalar) =
                append_until_out_of_memory(profile, interval, source, Some(100_000_000));
            assert_eq!(
                unmetered.live_bytes, scalar.live_bytes,
                "quota failure must leave the same live memory: {profile:?}, interval {interval}"
            );
            assert_eq!(
                unmetered.peak_live_bytes, scalar.peak_live_bytes,
                "no unchecked allocation window: {profile:?}, interval {interval}"
            );
            let peak = unmetered.peak_live_bytes.expect("tracking is enabled");
            assert!(
                peak.saturating_sub(limit) < 200,
                "quota enforcement must retain single-instruction precision: \
                 {profile:?}, interval {interval}, peak {peak}, limit {limit}"
            );
        }
    }
}

#[test]
fn numeric_array_append_matches_scalar_quota_precision_without_fuel() {
    assert_append_matches_scalar(NUMERIC_APPEND);
}

#[test]
fn non_numeric_array_append_matches_scalar_quota_precision_without_fuel() {
    assert_append_matches_scalar(STRING_APPEND);
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
