use rustia::{
    Error, Exports, Function, IntoRustia, InvocationLimits, OwnedValue, Result, Rustia, Table,
    VmErrorKind,
};
use std::cell::Cell;

struct ConversionProbe<'a>(&'a Cell<usize>);

impl IntoRustia for ConversionProbe<'_> {
    fn into_rustia(self) -> Result<OwnedValue> {
        self.0.set(self.0.get() + 1);
        Ok(OwnedValue::String("converted".repeat(64)))
    }
}

fn assert_invalid_handle(error: &Error, stale: bool, label: &str) {
    let kind = &error.as_vm_error().expect("typed handle error").kind;
    if stale {
        assert!(matches!(
            kind,
            VmErrorKind::StaleRetainedValue {
                retained_epoch: 0,
                current_epoch: 1,
            }
        ));
    } else {
        assert!(matches!(kind, VmErrorKind::RetainedValueFromDifferentVm));
    }
    assert_eq!(error.contexts().collect::<Vec<_>>(), [label]);
}

fn check_invalid_receiver(table: &Table, vm: &mut Rustia, stale: bool) {
    let conversions = Cell::new(0);
    let before = vm.memory_stats();
    let errors = [
        table.get(vm, ConversionProbe(&conversions)).unwrap_err(),
        table
            .set(
                vm,
                ConversionProbe(&conversions),
                ConversionProbe(&conversions),
            )
            .unwrap_err(),
        table.get(vm, "unallocated key".repeat(64)).unwrap_err(),
        table
            .set(vm, "unallocated key".repeat(64), vec![42_i32; 64])
            .unwrap_err(),
    ];

    assert_eq!(
        conversions.get(),
        0,
        "invalid receivers reject before conversion"
    );
    assert_eq!(
        vm.memory_stats(),
        before,
        "invalid receivers allocate no guest state"
    );
    for error in errors {
        assert_invalid_handle(&error, stale, "table handle");
    }
}

#[test]
fn foreign_table_receivers_reject_before_conversion_or_guest_allocation() {
    let mut owner = Rustia::default();
    let table = owner.create_table().unwrap();
    let mut other = Rustia::default();
    check_invalid_receiver(&table, &mut other, false);
}

#[test]
fn stale_table_receivers_reject_before_conversion_or_guest_allocation() {
    let mut vm = Rustia::default();
    let table = vm.create_table().unwrap();
    vm.reset_decision().unwrap();
    check_invalid_receiver(&table, &mut vm, true);
}

fn script_handles(vm: &mut Rustia) -> (Exports, Function) {
    let exports = vm
        .load("return { value = 42, read = function() return 42 end }")
        .install()
        .unwrap();
    let function = exports.function(vm, "read").unwrap();
    (exports, function)
}

fn check_function_and_exports_receivers(
    vm: &mut Rustia,
    exports: &Exports,
    function: &Function,
    stale: bool,
) {
    let conversions = Cell::new(0);
    let name = "unallocated export name".repeat(64);
    let argument = exports.as_value();
    let before = vm.memory_stats();
    let function_errors = [
        function
            .call::<_, i32>(vm, (ConversionProbe(&conversions),))
            .unwrap_err(),
        function
            .call::<_, i32>(vm, (vec![42_i32; 64],))
            .unwrap_err(),
        function
            .call_values(vm, std::slice::from_ref(&argument))
            .unwrap_err(),
    ];
    for error in function_errors {
        assert_invalid_handle(&error, stale, "function handle");
    }
    let export_errors = [
        exports.get::<i32>(vm, &name).unwrap_err(),
        exports.get_live(vm, &name).unwrap_err(),
        exports.function(vm, &name).unwrap_err(),
        exports.value(vm).unwrap_err(),
        exports.deserialize::<i32>(vm).unwrap_err(),
    ];
    for error in export_errors {
        assert_invalid_handle(&error, stale, "exports handle");
    }
    let outcome = vm
        .invoke(&mut (), InvocationLimits::new(1_000), |invocation| {
            let function_errors = [
                function
                    .call_in::<_, _, i32>(invocation, (ConversionProbe(&conversions),))
                    .unwrap_err(),
                function
                    .call_in::<_, _, i32>(invocation, (vec![42_i32; 64],))
                    .unwrap_err(),
                function
                    .call_values_in(invocation, std::slice::from_ref(&argument))
                    .unwrap_err(),
            ];
            for error in function_errors {
                assert_invalid_handle(&error, stale, "function handle");
            }
            let export_errors = [
                exports.get_in::<_, i32>(invocation, &name).unwrap_err(),
                exports.get_live_in(invocation, &name).unwrap_err(),
                exports.function_in(invocation, &name).unwrap_err(),
                exports.value_in(invocation).unwrap_err(),
            ];
            for error in export_errors {
                assert_invalid_handle(&error, stale, "exports handle");
            }
            assert_eq!(invocation.fuel_remaining(), 1_000);
            Ok(())
        })
        .unwrap();
    assert!(outcome.records().is_empty());
    assert_eq!(outcome.fuel_used(), 0);
    assert_eq!(conversions.get(), 0);
    assert_eq!(vm.memory_stats(), before);
}

#[test]
fn foreign_receivers_are_classified_by_ownership_even_when_epochs_differ() {
    let mut owner = Rustia::default();
    let (exports, function) = script_handles(&mut owner);
    let table = exports.as_table(&owner).unwrap().unwrap();
    let mut other = Rustia::default();
    other.reset_decision().unwrap();

    assert_invalid_handle(
        &exports.as_value().kind(&other).unwrap_err(),
        false,
        "value handle",
    );
    check_invalid_receiver(&table, &mut other, false);
    check_function_and_exports_receivers(&mut other, &exports, &function, false);
}

#[test]
fn stale_function_and_exports_receivers_reject_before_conversion_or_allocation() {
    let mut vm = Rustia::default();
    let (exports, function) = script_handles(&mut vm);
    vm.reset_decision().unwrap();
    check_function_and_exports_receivers(&mut vm, &exports, &function, true);
}
