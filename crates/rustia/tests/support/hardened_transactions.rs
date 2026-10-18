use rustia::{
    Effects, Error, FromRustia, HostContext, HostModule, HostSchema, Input, InvocationLimits,
    OwnedValue, Result, Rustia, VmErrorKind,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxRecord {
    sequence: u32,
    value: i64,
}

pub struct TxSchema;

struct PanickingReturn;

impl FromRustia for PanickingReturn {
    fn from_rustia(_value: OwnedValue) -> Result<Self> {
        panic!("deliberate FromRustia panic")
    }
}

impl HostSchema for TxSchema {
    type Input = i64;
    type Slots = ();
    type External = ();
    type Effect = i64;
    type SlotWrite = ();
    type Record = TxRecord;
}

struct TxContext {
    input: i64,
    external: (),
}

impl HostContext<TxSchema> for TxContext {
    fn input(&self) -> &i64 {
        &self.input
    }

    fn slots(&self) -> &() {
        &()
    }

    fn external_mut(&mut self) -> &mut () {
        &mut self.external
    }

    fn record_effect(&self, sequence: u32, value: i64) -> TxRecord {
        TxRecord { sequence, value }
    }

    fn record_slot_write(&self, sequence: u32, _write: ()) -> TxRecord {
        TxRecord {
            sequence,
            value: i64::MIN,
        }
    }

    fn record_bytes(&self, _record: &TxRecord) -> u64 {
        16
    }
}

pub fn configure_vm(mut vm: Rustia<TxSchema>) -> Rustia<TxSchema> {
    vm.install_host_module(
        HostModule::<TxSchema>::new("leaf")
            .input("input", 2, |input: Input<'_, TxSchema>| *input.get())
            .effect(
                "emit",
                2,
                |mut effects: Effects<'_, TxSchema>, value: i64| -> Result<()> {
                    effects.emit(value)
                },
            )
            .pure("panic_now", 2, || -> i64 {
                panic!("deliberate hardened host panic")
            }),
    )
    .expect("transaction host module");
    vm
}

fn assert_entry_failure_resets(
    vm: &mut Rustia<TxSchema>,
    context: &mut TxContext,
    lane: &mut Vec<TxRecord>,
    limits: InvocationLimits,
    entry: &rustia::CompiledEntry,
) -> Error {
    let expected_lane = lane.clone();
    let prior_epoch = vm.decision_epoch();
    let error = vm
        .invoke_entry_into::<i64>(context, limits, lane, entry)
        .expect_err("entry must fail");
    assert_eq!(*lane, expected_lane, "failure published staged records");
    assert_eq!(
        vm.decision_epoch(),
        prior_epoch + 1,
        "failure did not advance the reset epoch"
    );
    error
}

pub fn hardened_entry_lifecycle_is_atomic_and_recovers_from_every_failure_class(
    mut vm: Rustia<TxSchema>,
) {
    let mut context = TxContext {
        input: 7,
        external: (),
    };
    let prefix = TxRecord {
        sequence: 99,
        value: 99,
    };
    let mut lane = vec![prefix.clone()];
    let ordinary_limits = InvocationLimits::new(100_000)
        .with_max_records(8)
        .with_max_record_bytes(128);

    let guest_error =
        Rustia::compile_entry("guest-error-entry.luau", "leaf.emit(1)\nerror('boom')")
            .expect("guest-error entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        ordinary_limits,
        &guest_error,
    );
    assert!(error.to_string().contains("boom"), "{error}");

    let wrong_return = Rustia::compile_entry("wrong-return-entry.luau", "leaf.emit(2)\nreturn {}")
        .expect("wrong-return entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        ordinary_limits,
        &wrong_return,
    );
    assert!(error.to_string().contains("expected number"), "{error}");

    let record_overflow = Rustia::compile_entry(
        "record-overflow-entry.luau",
        "leaf.emit(3)\nleaf.emit(4)\nreturn 0",
    )
    .expect("record-overflow entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        InvocationLimits::new(100_000)
            .with_max_records(1)
            .with_max_record_bytes(128),
        &record_overflow,
    );
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HostResourceExhausted {
                    resource: "effect-record count",
                    limit: 1
                }
            )
    ));

    let byte_overflow = Rustia::compile_entry("byte-overflow-entry.luau", "leaf.emit(5)\nreturn 0")
        .expect("byte-overflow entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        InvocationLimits::new(100_000)
            .with_max_records(1)
            .with_max_record_bytes(15),
        &byte_overflow,
    );
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(
                error.kind(),
                VmErrorKind::HostResourceExhausted {
                    resource: "effect-record bytes",
                    limit: 15
                }
            )
    ));

    let fuel = Rustia::compile_entry("fuel-entry.luau", "leaf.emit(7)\nwhile true do end")
        .expect("fuel entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        InvocationLimits::new(64)
            .with_max_records(1)
            .with_max_record_bytes(16),
        &fuel,
    );
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error)
            if matches!(error.kind(), VmErrorKind::FuelExhausted { limit: 64 })
    ));

    let host_panic = Rustia::compile_entry(
        "host-panic-entry.luau",
        "leaf.emit(5)\nreturn leaf.panic_now()",
    )
    .expect("host-panic entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        ordinary_limits,
        &host_panic,
    );
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind(), VmErrorKind::HostPanicked { .. })
    ));

    let panicking_conversion = Rustia::compile_entry(
        "panicking-conversion-entry.luau",
        "local scratch = { 1, 2, 3 }\nleaf.emit(6)\nreturn scratch[1]",
    )
    .expect("panicking-conversion entry");
    let expected_lane = lane.clone();
    let prior_epoch = vm.decision_epoch();
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _ = vm.invoke_entry_into::<PanickingReturn>(
            &mut context,
            ordinary_limits,
            &mut lane,
            &panicking_conversion,
        );
    }));
    assert!(unwind.is_err(), "custom conversion panic must resume");
    assert_eq!(lane, expected_lane, "conversion panic published records");
    assert_eq!(
        vm.decision_epoch(),
        prior_epoch + 1,
        "conversion panic skipped decision reset"
    );

    let out_of_memory = Rustia::compile_entry(
        "oom-entry.luau",
        r#"
leaf.emit(8)
local scratch = {}
for i = 1, 20000 do
    scratch[i] = tostring(i) .. string.rep("x", 64)
end
return #scratch
"#,
    )
    .expect("out-of-memory entry");
    let error = assert_entry_failure_resets(
        &mut vm,
        &mut context,
        &mut lane,
        InvocationLimits::new(2_000_000)
            .with_max_records(1)
            .with_max_record_bytes(16),
        &out_of_memory,
    );
    assert!(matches!(
        error.kind(),
        rustia::ErrorKind::Vm(error) if matches!(error.kind(), VmErrorKind::OutOfMemory)
    ));

    let healthy = Rustia::compile_entry("healthy-entry.luau", "leaf.emit(9)\nreturn 42")
        .expect("healthy entry");
    let prior_epoch = vm.decision_epoch();
    let receipt = vm
        .invoke_entry_into::<i64>(&mut context, ordinary_limits, &mut lane, &healthy)
        .expect("VM must recover after every failed entry");
    assert_eq!(*receipt.value(), 42);
    assert_eq!(
        lane,
        [
            prefix,
            TxRecord {
                sequence: 0,
                value: 9
            }
        ]
    );
    assert_eq!(vm.decision_epoch(), prior_epoch + 1);
}

pub fn mixed_hardened_failures_reset_to_an_exact_memory_plateau(mut vm: Rustia<TxSchema>) {
    let guest_error = Rustia::compile_entry(
        "mixed-guest-error.luau",
        r#"
local input = leaf.input()
local scratch = {}
for i = 1, 8 + (input % 5) do
    scratch[i] = { value = input + i, label = string.rep("x", 12 + (i % 3)) }
end
leaf.emit(#scratch)
error("mixed failure")
"#,
    )
    .expect("mixed guest error");
    let wrong_return = Rustia::compile_entry(
        "mixed-wrong-return.luau",
        "local scratch = { leaf.input(), string.rep('y', 24) }\nleaf.emit(1)\nreturn scratch",
    )
    .expect("mixed wrong return");
    let record_overflow = Rustia::compile_entry(
        "mixed-record-overflow.luau",
        "leaf.emit(1)\nleaf.emit(2)\nreturn 0",
    )
    .expect("mixed record overflow");
    let fuel = Rustia::compile_entry(
        "mixed-fuel.luau",
        "local scratch = { leaf.input() }\nleaf.emit(1)\nwhile true do end",
    )
    .expect("mixed fuel");
    let prefix = TxRecord {
        sequence: 99,
        value: 99,
    };
    let mut lane = vec![prefix];
    let ordinary_limits = InvocationLimits::new(100_000)
        .with_max_records(4)
        .with_max_record_bytes(64);
    let overflow_limits = InvocationLimits::new(100_000)
        .with_max_records(1)
        .with_max_record_bytes(64);
    let fuel_limits = InvocationLimits::new(80)
        .with_max_records(1)
        .with_max_record_bytes(16);
    let mut plateau = None;

    for decision in 0..300 {
        let mut context = TxContext {
            input: decision,
            external: (),
        };
        let (entry, limits) = match decision % 4 {
            0 => (&guest_error, ordinary_limits),
            1 => (&wrong_return, ordinary_limits),
            2 => (&record_overflow, overflow_limits),
            _ => (&fuel, fuel_limits),
        };
        let _ = assert_entry_failure_resets(&mut vm, &mut context, &mut lane, limits, entry);
        let stats = vm.memory_stats();
        if decision == 49 {
            plateau = Some(stats);
        } else if decision > 49 {
            assert_eq!(
                Some(stats),
                plateau,
                "mixed failure memory changed after decision {decision}"
            );
        }
    }
    assert_eq!(vm.decision_epoch(), 300);
}

pub fn hardened_checkpoint_reset_reaches_an_exact_heterogeneous_memory_plateau(
    mut vm: Rustia<TxSchema>,
) {
    let entry = Rustia::compile_entry(
        "hardened-memory-plateau.luau",
        r#"
local input = leaf.input()
local count = 8 + (input % 5)
local scratch = {}
for i = 1, count do
    scratch[i] = {
        label = string.rep("x", 16 + ((i + input) % 4)),
        value = input + i,
    }
end
leaf.emit(count)
return scratch[1].value
"#,
    )
    .expect("plateau entry");
    let limits = InvocationLimits::new(100_000)
        .with_max_records(2)
        .with_max_record_bytes(32);
    let mut lane = Vec::with_capacity(2);
    let mut plateau = None;

    for decision in 0..600 {
        lane.clear();
        let mut context = TxContext {
            input: decision,
            external: (),
        };
        let receipt = vm
            .invoke_entry_into::<i64>(&mut context, limits, &mut lane, &entry)
            .expect("heterogeneous decision");
        assert_eq!(*receipt.value(), decision + 1);
        assert_eq!(
            lane,
            [TxRecord {
                sequence: 0,
                value: 8 + (decision % 5),
            }]
        );
        let stats = vm.memory_stats();
        if decision == 49 {
            plateau = Some(stats);
        } else if decision > 49 {
            assert_eq!(
                Some(stats),
                plateau,
                "hardened VM memory changed after the warm plateau at decision {decision}"
            );
        }
    }
    assert_eq!(vm.decision_epoch(), 600);
}
