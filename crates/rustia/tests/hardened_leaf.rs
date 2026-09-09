use rustia::{
    Effects, Error, FromRustia, GuestSurface, HostContext, HostModule, HostSchema, Input,
    InvocationLimits, OwnedValue, Profile, Result, Rustia, RustiaBuilder, VmErrorKind, VmSettings,
    RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION,
};
use std::num::NonZeroU64;
use std::panic::{catch_unwind, AssertUnwindSafe};

const MEMORY_LIMIT: usize = 4 * 1024 * 1024;

struct LeafSchema;

impl HostSchema for LeafSchema {
    type Input = ();
    type Slots = ();
    type External = ();
    type Effect = ();
    type SlotWrite = ();
    type Record = ();
}

impl HostContext<LeafSchema> for () {
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct TxRecord {
    sequence: u32,
    value: i64,
}

struct TxSchema;

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

fn leaf_vm() -> Rustia<LeafSchema> {
    let mut vm = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(MEMORY_LIMIT)
        .build_for_host::<LeafSchema>()
        .expect("hardened leaf VM");
    vm.install_host_module(HostModule::<LeafSchema>::new("leaf").pure("answer", 1, || 42i32))
        .expect("host module");
    vm
}

fn tx_vm(memory_limit_bytes: usize) -> Rustia<TxSchema> {
    let settings = VmSettings {
        deterministic_gc_interval: NonZeroU64::new(128).expect("non-zero interval"),
        memory_limit_bytes: Some(memory_limit_bytes),
        guest_surface: GuestSurface::HardenedLeaf,
        ..VmSettings::default()
    };
    let mut vm = RustiaBuilder::deterministic_leaf()
        .settings(settings)
        .build_for_host::<TxSchema>()
        .expect("transaction test VM");
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

fn invoke_string(vm: &mut Rustia<LeafSchema>, module: &rustia::CompiledModule) -> Result<String> {
    let mut context = ();
    vm.invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
        let exports = invocation.install(module)?;
        let decide = exports.function_in(invocation, "decide")?;
        decide.call_in::<_, _, String>(invocation, ())
    })
    .map(|outcome| outcome.into_parts().0)
}

#[test]
fn hardened_leaf_builder_requires_deterministic_profile_and_memory_limit() {
    let missing_limit = RustiaBuilder::deterministic_leaf()
        .build_for_host::<LeafSchema>()
        .err()
        .expect("memory limit is mandatory");
    assert!(matches!(
        missing_limit,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "missing logical-live memory limit"
                }
            )
    ));

    let settings = VmSettings {
        guest_surface: GuestSurface::HardenedLeaf,
        memory_limit_bytes: Some(MEMORY_LIMIT),
        ..VmSettings::default()
    };
    let wrong_profile = RustiaBuilder::standard()
        .settings(settings)
        .build_for_host::<LeafSchema>()
        .err()
        .expect("hardened leaves require DeterministicSim");
    assert!(matches!(
        wrong_profile,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "non-DeterministicSim profile"
                }
            )
    ));

    let context_free = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(MEMORY_LIMIT)
        .build()
        .err()
        .expect("hardened leaves require a typed host facade");
    assert!(matches!(
        context_free,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "context-free facade"
                }
            )
    ));

    let unchecked_settings = VmSettings {
        guest_surface: GuestSurface::HardenedLeaf,
        ..VmSettings::default()
    };
    let mut unchecked =
        Rustia::for_host_with_settings::<LeafSchema>(Profile::DeterministicSim, unchecked_settings);
    let entry = Rustia::compile_entry("unchecked-entry.luau", "return 1").expect("entry");
    let mut context = ();
    let mut lane = Vec::new();
    let error = unchecked
        .invoke_entry_into::<i32>(&mut context, InvocationLimits::new(100), &mut lane, &entry)
        .expect_err("the infallible constructor must fail closed");
    assert!(matches!(
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "missing logical-live memory limit"
                }
            )
    ));
    assert!(lane.is_empty());
    assert_eq!(unchecked.decision_epoch(), 1);

    assert_eq!(RUSTIA_HARDENED_LEAF_GUEST_ABI_VERSION, 1);
}

#[test]
fn failed_first_install_rolls_back_the_hardened_seal_checkpoint() {
    let mut vm = leaf_vm();
    let bad = Rustia::compile_module("bad-first-install.luau", "error('boom')")
        .expect("failing module compiles");
    let mut context = ();
    let error = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            invocation.install(&bad)
        })
        .expect_err("first installation fails");
    assert!(error.to_string().contains("boom"), "{error}");

    vm.reset_decision()
        .expect("rollback restores a valid pre-seal checkpoint");
    let good = Rustia::compile_module(
        "good-after-failure.luau",
        "return { decide = function() return 'recovered' end }",
    )
    .expect("recovery module compiles");
    assert_eq!(invoke_string(&mut vm, &good).unwrap(), "recovered");
}

#[test]
fn hardened_leaf_surface_is_allowlisted_frozen_and_reset_stable() {
    let mut vm = leaf_vm();
    let module = Rustia::compile_module(
        "surface.luau",
        r#"
return {
    decide = function()
        local forbidden = {
            debug,
            getfenv,
            setfenv,
            setmetatable,
            getmetatable,
            newproxy,
            rawequal,
            rawget,
            rawset,
            rawlen,
            loadstring,
            collectgarbage,
            gcinfo,
            print,
            os,
            buffer,
            vector,
            coroutine,
            math.random,
            math.randomseed,
            _G,
            RTTI,
        }
        for _, value in forbidden do
            if value ~= nil then
                return "forbidden-present"
            end
        end
        if not table.isfrozen(table)
            or not table.isfrozen(string)
            or not table.isfrozen(math)
            or not table.isfrozen(utf8)
            or not table.isfrozen(bit32)
        then
            return "mutable-library"
        end
        local scratch = { -4, 7 }
        table.insert(scratch, leaf.answer())
        return tostring(math.abs(scratch[1]) + scratch[2] + scratch[3])
    end,
}
"#,
    )
    .expect("surface module");

    assert!(matches!(
        vm.insert_global("decision_input", 7),
        Err(Error { kind: rustia::ErrorKind::Vm(ref error), .. })
            if matches!(
                &error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "ordinary global inputs"
                }
            )
    ));
    assert_eq!(
        invoke_string(&mut vm, &module).expect("first decision"),
        "53"
    );
    vm.reset_decision().expect("decision reset");
    assert_eq!(
        invoke_string(&mut vm, &module).expect("second decision"),
        "53"
    );
    assert!(matches!(
        vm.install_host_module(HostModule::<LeafSchema>::new("late").pure("value", 1, || 1i32)),
        Err(Error { kind: rustia::ErrorKind::Vm(ref error), .. }) if matches!(&error.kind, VmErrorKind::GuestEnvironmentSealed)
    ));

    for (source_name, source) in [
        ("mutate-table.luau", "table.insert = nil; return {}"),
        ("mutate-string.luau", "string.byte = nil; return {}"),
        ("mutate-math.luau", "math.abs = nil; return {}"),
        ("mutate-utf8.luau", "utf8.len = nil; return {}"),
        ("mutate-bit32.luau", "bit32.band = nil; return {}"),
    ] {
        let mut vm = leaf_vm();
        let module = Rustia::compile_module(source_name, source).expect("mutation module");
        let mut context = ();
        let error = vm
            .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
                invocation.install(&module)
            })
            .expect_err("standard-library tables must be frozen");
        assert!(
            error.to_string().contains("readonly table"),
            "{source_name}: {error}"
        );
    }
}

#[test]
fn hardened_leaf_stateless_entry_runs_without_retained_exports() {
    let mut vm = leaf_vm();
    let entry = Rustia::compile_entry(
        "entry.luau",
        r#"
local scratch = { 1, 2, 3 }
return leaf.answer() + scratch[1] + scratch[2] + scratch[3]
"#,
    )
    .expect("stateless entry");

    let mut lane = Vec::new();
    for expected_epoch in 1..=3 {
        let mut context = ();
        let value = vm
            .invoke_entry_into::<i32>(
                &mut context,
                InvocationLimits::new(20_000),
                &mut lane,
                &entry,
            )
            .expect("entry invocation")
            .into_parts()
            .0;
        assert_eq!(value, 48);
        assert_eq!(vm.decision_epoch(), expected_epoch);
    }

    let nested = Rustia::compile_entry(
        "nested-entry.luau",
        "local one = 1\nlocal two = 2\nlocal three = 3\nreturn (function() return one + two + three end)()",
    )
    .err()
    .expect("nested functions are not stateless entries");
    assert!(nested
        .to_string()
        .contains("stateless entry contains nested functions or captured upvalues"));
    assert!(
        nested.to_string().starts_with("nested-entry.luau:4:"),
        "{nested}"
    );

    let located = Rustia::compile_entry(
        "located-policy-error.luau",
        "local ok = 1\nforbidden_global = 2\nreturn ok",
    )
    .err()
    .expect("global mutation must fail entry compilation");
    assert!(
        located
            .to_string()
            .starts_with("located-policy-error.luau:2:"),
        "{located}"
    );

    let mut standard = RustiaBuilder::deterministic()
        .build_for_host::<LeafSchema>()
        .expect("standard typed VM");
    let standard_entry = Rustia::compile_entry("standard-entry.luau", "return 1").expect("entry");
    let mut context = ();
    let mut lane = Vec::new();
    let error = standard
        .invoke_entry_into::<i32>(
            &mut context,
            InvocationLimits::new(100),
            &mut lane,
            &standard_entry,
        )
        .expect_err("hardened entries must reject an ordinary surface");
    assert!(matches!(
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
                VmErrorKind::HardenedLeafViolation {
                    feature: "non-hardened guest surface"
                }
            )
    ));
    assert_eq!(standard.decision_epoch(), 1);
}

#[test]
fn hardened_leaf_failures_do_not_retain_rich_diagnostics() {
    let mut vm = leaf_vm();
    let entry = Rustia::compile_entry("leaf-failure.luau", "error('stop')").unwrap();
    let mut lane = Vec::new();
    let error = vm
        .invoke_entry_into::<i32>(&mut (), InvocationLimits::new(20_000), &mut lane, &entry)
        .err()
        .unwrap();
    assert!(error.diagnostic().is_none());
    assert_eq!(vm.diagnostic_source_bytes(), 0);
    assert!(!error.report().to_string().contains("called from:"));
}

#[test]
fn compiled_entry_is_shareable_across_hardened_vms_and_module_handles_go_stale() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<rustia::CompiledEntry>();

    let entry = Rustia::compile_entry(
        "shared-entry.luau",
        "local scratch = { 1, 2, 3 }\nreturn leaf.answer() + #scratch",
    )
    .expect("shared entry");
    let mut first = leaf_vm();
    let mut second = leaf_vm();
    for (vm, shared) in [(&mut first, entry.clone()), (&mut second, entry)] {
        let mut context = ();
        let mut lane = Vec::new();
        let receipt = vm
            .invoke_entry_into::<i32>(
                &mut context,
                InvocationLimits::new(20_000),
                &mut lane,
                &shared,
            )
            .expect("shared entry invocation");
        assert_eq!(*receipt.value(), 45);
        assert_eq!(vm.decision_epoch(), 1);
    }

    let mut vm = leaf_vm();
    let module = Rustia::compile_module(
        "hardened-compatibility-module.luau",
        "return { decide = function() return leaf.answer() end }",
    )
    .expect("compatibility module");
    let mut context = ();
    let exports = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            invocation.install(&module)
        })
        .expect("module install")
        .into_parts()
        .0;
    let function = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            exports.function_in(invocation, "decide")
        })
        .expect("function lookup")
        .into_parts()
        .0;
    vm.reset_decision().expect("hardened reset");
    let error = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            function.call_in::<_, _, i32>(invocation, ())
        })
        .expect_err("stale hardened function must reject");
    assert!(matches!(
        error,
        Error { kind: rustia::ErrorKind::Embed(ref message), .. } if message.contains("stale function handle")
    ));
}

#[test]
fn hardened_leaf_rejects_global_upvalue_and_host_module_mutation() {
    let cases = [
        (
            "global mutation",
            "global-write.luau",
            "forbidden_global = 1; return {}",
        ),
        (
            "captured upvalues",
            "upvalue.luau",
            "local hidden = 1; return { decide = function() return hidden end }",
        ),
    ];

    for (feature, name, source) in cases {
        let mut vm = leaf_vm();
        let module = Rustia::compile_module(name, source).expect("policy-tested source compiles");
        let mut context = ();
        let error = vm
            .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
                invocation.install(&module)
            })
            .expect_err("policy violation must reject before guest execution");
        assert!(
            matches!(
                error,
                Error { kind: rustia::ErrorKind::Vm(ref error), .. }
                    if matches!(
                        &error.kind,
                        VmErrorKind::HardenedLeafViolation {
                            feature: actual
                        } if *actual == feature
                    )
            ),
            "{name}: {error}"
        );
    }

    let mut vm = leaf_vm();
    let module = Rustia::compile_module(
        "host-module-write.luau",
        "leaf.answer = function() return 0 end; return {}",
    )
    .expect("host-module mutation source compiles");
    let mut context = ();
    let error = vm
        .invoke(&mut context, InvocationLimits::new(20_000), |invocation| {
            invocation.install(&module)
        })
        .expect_err("host module table must be frozen");
    assert!(error.to_string().contains("readonly table"), "{error}");
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

#[test]
fn hardened_entry_lifecycle_is_atomic_and_recovers_from_every_failure_class() {
    let mut vm = tx_vm(512 * 1024);
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
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
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
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(
                &error.kind,
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
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. }
            if matches!(&error.kind, VmErrorKind::FuelExhausted { limit: 64 })
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
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. } if matches!(&error.kind, VmErrorKind::HostPanicked { .. })
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
        error,
        Error { kind: rustia::ErrorKind::Vm(ref error), .. } if matches!(&error.kind, VmErrorKind::OutOfMemory)
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

#[test]
fn mixed_hardened_failures_reset_to_an_exact_memory_plateau() {
    let mut vm = tx_vm(MEMORY_LIMIT);
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

#[test]
fn hardened_checkpoint_reset_reaches_an_exact_heterogeneous_memory_plateau() {
    let mut vm = tx_vm(MEMORY_LIMIT);
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

#[test]
fn ordinary_deterministic_surface_remains_compatible() {
    let mut vm = RustiaBuilder::deterministic()
        .build()
        .expect("ordinary deterministic VM");
    let result: String = vm
        .eval(
            r#"
return table.concat({
    type(debug),
    type(getfenv),
    type(rawset),
    type(buffer),
    type(vector),
    type(coroutine),
    type(math.random),
    type(_G),
}, ",")
"#,
        )
        .expect("ordinary deterministic surface");
    assert_eq!(
        result,
        "table,function,function,table,table,table,function,table"
    );
}
