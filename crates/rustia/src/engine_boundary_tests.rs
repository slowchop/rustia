//! Internal controls remain covered without making them application settings.
use crate::{HostSchema, InvocationLimits, Profile, Rustia, RustiaBuilder, VmErrorKind};
use rustia_vm::VmSettings;
use std::num::NonZeroU64;

#[path = "../tests/support/hardened_transactions.rs"]
mod hardened_transactions;
#[path = "../tests/support/mixed_heap_scenario.rs"]
mod mixed_heap_scenario;
#[path = "../tests/support/quota_precision.rs"]
mod quota_precision;

#[test]
fn numeric_and_non_numeric_append_keep_precision_across_collection_cadences() {
    for source in [
        quota_precision::NUMERIC_APPEND,
        quota_precision::STRING_APPEND,
    ] {
        for interval in [1, 8, 128, 1_000, 10_000] {
            quota_precision::assert_append_matches_scalar(
                source,
                &interval.to_string(),
                |profile, fuel, limit| {
                    RustiaBuilder::standard()
                        .profile(profile)
                        .raw_settings(VmSettings {
                            memory_limit_bytes: Some(limit),
                            deterministic_gc_interval: NonZeroU64::new(interval).unwrap(),
                            initial_fuel: fuel,
                            enable_memory_tracking: true,
                            ..VmSettings::default()
                        })
                        .build()
                        .unwrap()
                },
            );
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn mixed_heap_pressure_preserves_the_short_collection_cadence() {
    let vm = RustiaBuilder::standard()
        .raw_settings(VmSettings {
            deterministic_gc_interval: NonZeroU64::new(128).unwrap(),
            memory_limit_bytes: Some(mixed_heap_scenario::MEMORY_LIMIT_BYTES),
            ..VmSettings::default()
        })
        .build()
        .expect("mixed-heap pressure VM with internal cadence");
    mixed_heap_scenario::run_pressure_scenario(vm, mixed_heap_scenario::PlateauChecks::WavePhases);
}

#[test]
fn an_unchecked_hardened_leaf_fails_closed_without_a_memory_limit() {
    struct Schema;
    impl HostSchema for Schema {
        type Input = ();
        type Slots = ();
        type External = ();
        type Effect = ();
        type SlotWrite = ();
        type Record = ();
    }
    impl crate::HostContext<Schema> for () {
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

    let mut vm = Rustia::for_host_with_settings::<Schema>(
        Profile::DeterministicSim,
        VmSettings {
            guest_surface: rustia_vm::GuestSurface::HardenedLeaf,
            ..VmSettings::default()
        },
    );
    let entry = Rustia::compile_entry("unchecked-entry.luau", "return 1").expect("entry");
    let mut context = ();
    let mut records = Vec::new();
    let error = vm
        .invoke_entry_into::<i32>(
            &mut context,
            InvocationLimits::new(100),
            &mut records,
            &entry,
        )
        .expect_err("the unchecked internal constructor must fail closed");
    assert!(matches!(
        error.kind(),
        crate::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HardenedLeafViolation {
                    feature: "missing logical-live memory limit"
                }
            )
    ));
    assert!(records.is_empty());
    assert_eq!(vm.decision_epoch(), 1);
}

#[test]
fn hardened_transactions_preserve_short_collection_cadence_coverage() {
    let make_vm = |limit| {
        hardened_transactions::configure_vm(
            RustiaBuilder::deterministic_leaf()
                .raw_settings(VmSettings {
                    deterministic_gc_interval: NonZeroU64::new(128).unwrap(),
                    memory_limit_bytes: Some(limit),
                    guest_surface: rustia_vm::GuestSurface::HardenedLeaf,
                    ..VmSettings::default()
                })
                .build_for_host::<hardened_transactions::TxSchema>()
                .expect("short-cadence hardened transaction VM"),
        )
    };
    hardened_transactions::hardened_entry_lifecycle_is_atomic_and_recovers_from_every_failure_class(
        make_vm(512 * 1024),
    );
    hardened_transactions::mixed_hardened_failures_reset_to_an_exact_memory_plateau(make_vm(
        4 * 1024 * 1024,
    ));
    hardened_transactions::hardened_checkpoint_reset_reaches_an_exact_heterogeneous_memory_plateau(
        make_vm(4 * 1024 * 1024),
    );
}
