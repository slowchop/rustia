use rustia::{
    CompiledModule, Effects, Error, Exports, HostContext, HostModule, HostMut, HostSchema, Input,
    InvocationLimits, OpCapability, Profile, Result, Rustia, SlotWrites, Slots, VmError,
    VmErrorKind, VmSettings,
};
use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;

#[derive(Clone, Debug, Eq, PartialEq)]
enum RecordKind {
    Effect(i64),
    SlotWrite { slot: usize, value: i64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Record {
    sequence: u32,
    kind: RecordKind,
}

struct Host {
    input: i64,
    slots: Vec<i64>,
}

impl HostSchema for Host {
    type Input = i64;
    type Slots = [i64];
    type External = Host;
    type Effect = i64;
    type SlotWrite = (usize, i64);
    type Record = Record;
}

impl HostContext<Host> for Host {
    fn input(&self) -> &i64 {
        &self.input
    }

    fn slots(&self) -> &[i64] {
        &self.slots
    }

    fn external_mut(&mut self) -> &mut Host {
        self
    }

    fn record_effect(&self, sequence: u32, effect: i64) -> Record {
        Record {
            sequence,
            kind: RecordKind::Effect(effect),
        }
    }

    fn record_slot_write(&self, sequence: u32, write: (usize, i64)) -> Record {
        Record {
            sequence,
            kind: RecordKind::SlotWrite {
                slot: write.0,
                value: write.1,
            },
        }
    }

    fn record_bytes(&self, _record: &Record) -> u64 {
        16
    }
}

fn configured_vm() -> Rustia<Host> {
    let mut rustia = Rustia::for_host::<Host>(Profile::DeterministicSim);
    rustia
        .add_input("read_input", 2, |input: Input<'_, Host>, offset: i64| {
            *input.get() + offset
        })
        .unwrap();
    rustia
        .add_slot_read("read_slot", 2, |slots: Slots<'_, Host>, slot: usize| {
            slots.get()[slot]
        })
        .unwrap();
    rustia
        .add_effect(
            "emit",
            2,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> { effects.emit(value) },
        )
        .unwrap();
    rustia
        .add_slot_write(
            "write_slot",
            2,
            |mut writes: SlotWrites<'_, Host>, slot: usize, value: i64| -> Result<()> {
                writes.write((slot, value))
            },
        )
        .unwrap();
    rustia
}

trait TypedTestSetup {
    fn compile_and_run(&mut self, source: &str) -> Result<Exports>;
    fn install(&mut self, module: CompiledModule) -> Result<Exports>;
}

impl TypedTestSetup for Rustia<Host> {
    fn compile_and_run(&mut self, source: &str) -> Result<Exports> {
        let module = Rustia::<Host>::compile_module("typed-test.luau", source)?;
        self.install(module)
    }

    fn install(&mut self, module: CompiledModule) -> Result<Exports> {
        let mut context = Host {
            input: 0,
            slots: Vec::new(),
        };
        let outcome = self.invoke(&mut context, InvocationLimits::new(100_000), |invocation| {
            invocation.install(module)
        })?;
        let (exports, _, _) = outcome.into_parts();
        Ok(exports)
    }
}

#[test]
fn context_is_rebound_and_success_publishes_dense_ordered_records() {
    let mut rustia = configured_vm();
    let module = Rustia::<Host>::compile_module(
        "decision.luau",
        r#"
return {
    decide = function(offset)
        local value = read_input(offset)
        emit(value)
        write_slot(1, value + read_slot(1))
        return value
    end,
}
"#,
    )
    .unwrap();
    let exports = rustia.install(module).unwrap();
    let limits = InvocationLimits::new(10_000)
        .with_max_records(8)
        .with_max_record_bytes(128);

    let mut first_host = Host {
        input: 10,
        slots: vec![3, 4],
    };
    let first = rustia
        .invoke(&mut first_host, limits, |invocation| {
            let decide = exports.function_in(invocation, "decide")?;
            decide.call_in::<_, _, i64>(invocation, (5_i64,))
        })
        .unwrap();
    assert_eq!(*first.value(), 15);
    assert_eq!(
        first.records(),
        [
            Record {
                sequence: 0,
                kind: RecordKind::Effect(15),
            },
            Record {
                sequence: 1,
                kind: RecordKind::SlotWrite { slot: 1, value: 19 },
            },
        ]
    );
    let mut second_host = Host {
        input: -2,
        slots: vec![100, 7],
    };
    let second = rustia
        .invoke(&mut second_host, limits, |invocation| {
            let decide = exports.function_in(invocation, "decide")?;
            decide.call_in::<_, _, i64>(invocation, (1_i64,))
        })
        .unwrap();
    assert_eq!(*second.value(), -1);
    assert_eq!(second.records()[0].sequence, 0);
    assert_eq!(second.records()[0].kind, RecordKind::Effect(-1));
}

#[test]
fn typed_invocations_exchange_rooted_live_values_without_snapshotting() {
    let mut rustia = configured_vm();
    let mut host = Host {
        input: 0,
        slots: vec![0],
    };
    let limits = InvocationLimits::new(100_000);

    let function = rustia
        .invoke(&mut host, limits, |invocation| {
            invocation.run_value(
                "live-function.luau",
                "return function(value) local result = { value = value }; result.self = result; return value, result end",
            )
        })
        .unwrap()
        .into_parts()
        .0
        .as_function(&rustia)
        .unwrap()
        .unwrap();
    let argument = rustia
        .invoke(&mut host, limits, |invocation| {
            invocation.run_value("live-argument.luau", "return { answer = 42 }")
        })
        .unwrap()
        .into_parts()
        .0;

    let results = rustia
        .invoke(&mut host, limits, |invocation| {
            function.call_values_in(invocation, std::slice::from_ref(&argument))
        })
        .unwrap()
        .into_parts()
        .0;
    assert!(argument.raw_equal(&mut rustia, &results[0]).unwrap());
    let result_table = results[1].as_table(&rustia).unwrap().unwrap();
    let result_value = result_table.get(&mut rustia, "value").unwrap();
    assert!(argument.raw_equal(&mut rustia, &result_value).unwrap());
}

#[test]
fn typed_exports_expose_rooted_live_values() {
    let mut rustia = configured_vm();
    let mut host = Host {
        input: 0,
        slots: vec![0],
    };
    let limits = InvocationLimits::new(100_000);
    let module = Rustia::<Host>::compile_module(
        "live-exports.luau",
        "local shared = { answer = 42 }; return { left = shared, right = shared }",
    )
    .unwrap();

    let exports = rustia
        .invoke(&mut host, limits, |invocation| invocation.install(module))
        .unwrap()
        .into_parts()
        .0;
    let (left, right) = rustia
        .invoke(&mut host, limits, |invocation| {
            Ok((
                exports.get_live_current_in(invocation, "left")?,
                exports.get_live_current_in(invocation, "right")?,
            ))
        })
        .unwrap()
        .into_parts()
        .0;

    assert!(left.raw_equal(&mut rustia, &right).unwrap());
}

#[test]
fn failed_host_call_rolls_back_its_suffix_but_pcall_keeps_completed_calls() {
    let mut rustia = configured_vm();
    rustia
        .add_effect(
            "emit_then_fail",
            2,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> {
                effects.emit(value)?;
                Err(Error::Embed("expected host failure".to_owned()))
            },
        )
        .unwrap();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    decide = function()
        emit(1)
        local ok = pcall(function() emit_then_fail(2) end)
        emit(3)
        return ok
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, bool>(invocation, ())
            },
        )
        .unwrap();
    assert!(!*outcome.value());
    assert_eq!(
        outcome.records(),
        [
            Record {
                sequence: 0,
                kind: RecordKind::Effect(1),
            },
            Record {
                sequence: 1,
                kind: RecordKind::Effect(3),
            },
        ]
    );
}

#[test]
fn record_limit_is_hard_even_if_callback_and_guest_try_to_swallow_it() {
    let mut rustia = configured_vm();
    rustia
        .add_effect(
            "ignore_emit_error",
            2,
            |mut effects: Effects<'_, Host>, value: i64| {
                let _ = effects.emit(value);
            },
        )
        .unwrap();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    decide = function()
        ignore_emit_error(1)
        local ok = pcall(function() ignore_emit_error(2) end)
        return ok
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let mut lane = vec![Record {
        sequence: 99,
        kind: RecordKind::Effect(99),
    }];
    let error = rustia
        .invoke_into(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(1)
                .with_max_record_bytes(128),
            &mut lane,
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, bool>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error)
            if matches!(
                error.kind,
                VmErrorKind::HostResourceExhausted {
                    resource: "effect-record count",
                    limit: 1
                }
            )
    ));
    assert_eq!(lane.len(), 1);
    assert_eq!(lane[0].sequence, 99);
}

#[test]
fn a_completed_effect_inside_guest_pcall_survives_the_caught_guest_error() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    decide = function()
        local ok = pcall(function()
            emit(10)
            error("caught")
        end)
        emit(20)
        return ok
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, bool>(invocation, ())
            },
        )
        .unwrap();
    assert!(!*outcome.value());
    assert_eq!(
        outcome
            .records()
            .iter()
            .map(|record| &record.kind)
            .collect::<Vec<_>>(),
        [&RecordKind::Effect(10), &RecordKind::Effect(20)]
    );
}

#[test]
fn outer_guest_error_and_typed_return_error_publish_nothing() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    guest_error = function()
        emit(1)
        error("uncaught")
    end,
    wrong_return = function()
        emit(2)
        return {}
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let prefix = Record {
        sequence: 77,
        kind: RecordKind::Effect(77),
    };
    let mut lane = vec![prefix.clone()];
    let error = rustia
        .invoke_into(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            &mut lane,
            |invocation| {
                let function = exports.function_in(invocation, "guest_error")?;
                function.call_in::<_, _, ()>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("uncaught"));
    assert_eq!(lane.as_slice(), std::slice::from_ref(&prefix));

    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |invocation| {
                let function = exports.function_in(invocation, "wrong_return")?;
                let conversion = function.call_in::<_, _, i64>(invocation, ());
                assert!(conversion.is_err());
                Ok(())
            },
        )
        .unwrap();
    assert!(outcome.records().is_empty());
}

#[test]
fn outer_operation_panic_drops_records_and_leaves_the_vm_reusable() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run("return { decide = function() emit(9) end }")
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let limits = InvocationLimits::new(10_000)
        .with_max_records(2)
        .with_max_record_bytes(32);
    let mut lane = Vec::new();
    let unwind = catch_unwind(AssertUnwindSafe(|| {
        let _ = rustia.invoke_into::<(), _>(&mut host, limits, &mut lane, |invocation| {
            exports
                .function_in(invocation, "decide")?
                .call_in::<_, _, ()>(invocation, ())?;
            panic!("outer operation panic");
        });
    }));
    assert!(unwind.is_err());
    assert!(lane.is_empty());

    let outcome = rustia
        .invoke(&mut host, limits, |invocation| {
            exports
                .function_in(invocation, "decide")?
                .call_in::<_, _, ()>(invocation, ())
        })
        .unwrap();
    assert_eq!(
        outcome.records(),
        [Record {
            sequence: 0,
            kind: RecordKind::Effect(9),
        }]
    );
}

#[test]
fn byte_budget_is_checked_before_publication() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run("return { decide = function() emit(1); emit(2); return true end }")
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let error = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(31),
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, bool>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error)
            if matches!(
                error.kind,
                VmErrorKind::HostResourceExhausted {
                    resource: "effect-record bytes",
                    limit: 31
                }
            )
    ));
}

#[test]
fn fuel_is_shared_and_uncatchable_across_calls_in_one_invocation() {
    let mut rustia = configured_vm();
    rustia
        .add_effect(
            "expensive_emit",
            40,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> { effects.emit(value) },
        )
        .unwrap();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    once = function(value)
        expensive_emit(value)
        return value
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let error = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(70)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |invocation| {
                let once = exports.function_in(invocation, "once")?;
                let _: i64 = once.call_in(invocation, (1_i64,))?;
                let _: i64 = once.call_in(invocation, (2_i64,))?;
                Ok(())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::FuelExhausted { limit: 70 })
    ));
}

#[test]
fn host_panic_rolls_back_poisons_and_reset_recovers_bindings() {
    let mut rustia = configured_vm();
    rustia
        .add_effect(
            "panic_after_emit",
            2,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> {
                effects.emit(value)?;
                panic!("deliberate host panic")
            },
        )
        .unwrap();
    let module = Rustia::<Host>::compile_module(
        "panic.luau",
        r#"
return {
    panic_now = function() panic_after_emit(5) end,
    healthy = function() emit(6); return 42 end,
}
"#,
    )
    .unwrap();
    let exports = rustia.install(module.clone()).unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let mut lane = Vec::new();
    let error = rustia
        .invoke_into(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            &mut lane,
            |invocation| {
                let function = exports.function_in(invocation, "panic_now")?;
                function.call_in::<_, _, ()>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::HostPanicked { .. })
    ));
    assert!(lane.is_empty());

    let poisoned = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |_invocation| Ok(()),
        )
        .unwrap_err();
    assert!(matches!(
        poisoned,
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));

    rustia.reset_decision().unwrap();
    let exports = rustia.install(module).unwrap();
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |invocation| {
                let healthy = exports.function_in(invocation, "healthy")?;
                healthy.call_in::<_, _, i64>(invocation, ())
            },
        )
        .unwrap();
    assert_eq!(*outcome.value(), 42);
    assert_eq!(outcome.records()[0].kind, RecordKind::Effect(6));
}

#[test]
fn suspended_coroutine_uses_the_context_of_the_resume_invocation() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run(
            r#"
local thread = coroutine.create(function()
    coroutine.yield("paused")
    return read_input(0)
end)
return {
    start = function()
        local ok, value = coroutine.resume(thread)
        return ok and value
    end,
    finish = function()
        local ok, value = coroutine.resume(thread)
        return ok and value
    end,
}
"#,
        )
        .unwrap();
    let limits = InvocationLimits::new(10_000)
        .with_max_records(8)
        .with_max_record_bytes(128);
    let mut first_host = Host {
        input: 11,
        slots: vec![],
    };
    let start = rustia
        .invoke(&mut first_host, limits, |invocation| {
            let function = exports.function_in(invocation, "start")?;
            function.call_in::<_, _, String>(invocation, ())
        })
        .unwrap();
    assert_eq!(start.value(), "paused");
    let mut second_host = Host {
        input: 22,
        slots: vec![],
    };
    let finish = rustia
        .invoke(&mut second_host, limits, |invocation| {
            let function = exports.function_in(invocation, "finish")?;
            function.call_in::<_, _, i64>(invocation, ())
        })
        .unwrap();
    assert_eq!(*finish.value(), 22);
}

#[test]
fn context_reaches_metamethods_and_accepts_unrelated_block_local_borrows() {
    struct BorrowedSchema;

    struct BorrowedHost<'a> {
        input: &'a i64,
        marker: Rc<Cell<usize>>,
        external: (),
    }

    impl HostSchema for BorrowedSchema {
        type Input = i64;
        type Slots = ();
        type External = ();
        type Effect = std::convert::Infallible;
        type SlotWrite = std::convert::Infallible;
        type Record = std::convert::Infallible;
    }

    impl HostContext<BorrowedSchema> for BorrowedHost<'_> {
        fn input(&self) -> &i64 {
            self.input
        }

        fn slots(&self) -> &() {
            &()
        }

        fn external_mut(&mut self) -> &mut () {
            &mut self.external
        }

        fn record_effect(
            &self,
            _sequence: u32,
            effect: std::convert::Infallible,
        ) -> std::convert::Infallible {
            match effect {}
        }

        fn record_slot_write(
            &self,
            _sequence: u32,
            write: std::convert::Infallible,
        ) -> std::convert::Infallible {
            match write {}
        }

        fn record_bytes(&self, record: &std::convert::Infallible) -> u64 {
            match *record {}
        }
    }

    let marker = Rc::new(Cell::new(0));
    let mut rustia = Rustia::for_host::<BorrowedSchema>(Profile::DeterministicSim);
    rustia
        .add_input("nested_input", 2, |input: Input<'_, BorrowedSchema>| {
            *input.get()
        })
        .unwrap();
    let module = Rustia::<BorrowedSchema>::compile_module(
        "borrowed-context.luau",
        r#"
local value = setmetatable({}, {
    __add = function() return nested_input() end,
})
return { decide = function() return value + value end }
"#,
    )
    .unwrap();
    let exports = {
        let source = 37;
        let mut host = BorrowedHost {
            input: &source,
            marker: Rc::clone(&marker),
            external: (),
        };
        let exports = rustia
            .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
                invocation.install(module)
            })
            .unwrap()
            .into_parts()
            .0;
        let outcome = rustia
            .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, i64>(invocation, ())
            })
            .unwrap();
        assert_eq!(*outcome.value(), 37);
        assert_eq!(host.marker.get(), 0);
        exports
    };

    {
        let unrelated_source = -11;
        let mut unrelated_host = BorrowedHost {
            input: &unrelated_source,
            marker: Rc::clone(&marker),
            external: (),
        };
        let outcome = rustia
            .invoke(
                &mut unrelated_host,
                InvocationLimits::new(10_000),
                |invocation| {
                    let decide = exports.function_in(invocation, "decide")?;
                    decide.call_in::<_, _, i64>(invocation, ())
                },
            )
            .unwrap();
        assert_eq!(*outcome.value(), -11);
    }
}

#[test]
fn pure_host_panic_is_caught_rolls_back_and_poisons() {
    let mut rustia = configured_vm();
    rustia
        .add_pure("pure_panic", 2, || -> i64 { panic!("pure callback panic") })
        .unwrap();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    decide = function()
        emit(1)
        pcall(pure_panic)
        emit(2)
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let mut lane = Vec::new();
    let error = rustia
        .invoke_into(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            &mut lane,
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, ()>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error)
            if matches!(
                &error.kind,
                VmErrorKind::HostPanicked { function }
                    if function == "pure_panic"
            )
    ));
    assert!(lane.is_empty());
    assert!(matches!(
        rustia
            .invoke(
                &mut host,
                InvocationLimits::new(10_000),
                |_invocation| Ok(()),
            )
            .unwrap_err(),
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));
}

#[test]
fn direct_host_panic_poisoning_is_enforced_by_the_vm_core() {
    let mut rustia = Rustia::new(Profile::DeterministicSim);
    rustia
        .add_fn("panic_now", || -> i64 { panic!("direct callback panic") })
        .unwrap();
    assert!(matches!(
        rustia.run::<()>("panic_now()").unwrap_err(),
        Error::Vm(ref error)
            if matches!(error.kind, VmErrorKind::HostPanicked { .. })
    ));
    assert!(matches!(
        rustia.run::<i64>("return 1").unwrap_err(),
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));
    assert!(matches!(
        rustia
            .invoke(&mut (), InvocationLimits::new(0), |_invocation| Ok(()))
            .unwrap_err(),
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));
    rustia.reset_decision().unwrap();
    assert_eq!(rustia.run::<i64>("return 1").unwrap(), 1);
    rustia
        .invoke(&mut (), InvocationLimits::new(0), |_invocation| Ok(()))
        .unwrap();
}

#[test]
fn host_service_panic_rolls_back_records_poisons_and_leaves_reset_usable() {
    let mut rustia = Rustia::builder()
        .profile(Profile::DeterministicSim)
        .stdout(|_| panic!("stdout service panic"))
        .build_for_host::<Host>()
        .unwrap();
    rustia
        .add_effect(
            "emit",
            2,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> { effects.emit(value) },
        )
        .unwrap();
    let module = Rustia::<Host>::compile_module(
        "service-panic.luau",
        r#"
return {
    decide = function()
        emit(1)
        print("panic")
        emit(2)
    end,
}
"#,
    )
    .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let installed = rustia
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            invocation.install(module)
        })
        .unwrap();
    let (exports, _, _) = installed.into_parts();
    let mut lane = Vec::new();
    let error = rustia
        .invoke_into(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(32),
            &mut lane,
            |invocation| {
                exports
                    .function_in(invocation, "decide")?
                    .call_in::<_, _, ()>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(matches!(
        &error,
        Error::Vm(error)
            if matches!(
                &error.kind,
                VmErrorKind::HostPanicked { function }
                    if function == "service::stdout"
            )
    ));
    assert!(lane.is_empty());
    assert!(matches!(
        rustia
            .invoke(&mut host, InvocationLimits::new(0), |_invocation| Ok(()))
            .unwrap_err(),
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));
    let mut poison_probe = Vec::new();
    assert!(matches!(
        rustia
            .invoke_into(
                &mut host,
                InvocationLimits::new(0).with_max_records(4),
                &mut poison_probe,
                |_invocation| Ok(()),
            )
            .unwrap_err(),
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::VmPoisoned)
    ));
    assert_eq!(poison_probe.capacity(), 0);
    rustia.reset_decision().unwrap();
    rustia
        .invoke(&mut host, InvocationLimits::new(0), |_invocation| Ok(()))
        .unwrap();
}

#[test]
fn typed_builder_preserves_profile_validation() {
    let mut rustia = Rustia::builder()
        .profile(Profile::DeterministicSim)
        .fuel(7)
        .build_for_host::<Host>()
        .unwrap();
    assert!(matches!(
        rustia.add_external(
            "external",
            1,
            |mut host: HostMut<'_, Host>| host.get_mut().input
        ),
        Err(Error::Vm(ref error))
            if matches!(
                error.kind,
                VmErrorKind::CapabilityRemoved {
                    feature: "host function reading external mutable state",
                    operation: Some(ref operation),
                    profile: Profile::DeterministicSim,
                } if operation == "external"
            )
    ));
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let outcome = rustia
        .invoke(&mut host, InvocationLimits::new(100), |invocation| {
            Ok(invocation.fuel_remaining())
        })
        .unwrap();
    assert_eq!(*outcome.value(), 7);
}

#[test]
fn invocation_run_compiles_source_with_the_vm_profile() {
    let mut rustia = Rustia::for_host::<Host>(Profile::Puc51Compat);
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };

    let outcome = rustia
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            invocation.run::<f64>("puc-invocation.luau", "local t = {1, [1] = 9} return t[1]")
        })
        .unwrap();
    assert_eq!(*outcome.value(), 1.0);
}

#[test]
fn typed_host_modules_are_inspectable_atomic_and_persist_across_reset() {
    let mut rustia = Rustia::for_host::<Host>(Profile::DeterministicSim);
    let rejected = HostModule::<Host>::new("rejected")
        .input("input", 2, |input: Input<'_, Host>| *input.get())
        .external("mutate", 2, |mut host: HostMut<'_, Host>| {
            host.get_mut().input += 1;
        });
    let error = rustia.install_host_module(rejected).unwrap_err();
    assert!(matches!(
        &error,
        Error::Vm(error)
            if matches!(
                &error.kind,
                VmErrorKind::CapabilityRemoved {
                    feature: "host function reading external mutable state",
                    operation: Some(operation),
                    profile: Profile::DeterministicSim,
                } if operation == "rejected.mutate"
            )
    ));

    let module = HostModule::<Host>::new("world")
        .pure("double", 1, |value: i64| value * 2)
        .input("input", 2, |input: Input<'_, Host>| *input.get())
        .effect(
            "emit",
            3,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> { effects.emit(value) },
        );
    assert_eq!(module.namespace(), "world");
    let metadata = module
        .functions()
        .map(|function| {
            (
                function.name.to_owned(),
                function.fuel_cost,
                function.capability,
                function.parameter_count,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        metadata,
        [
            ("double".to_owned(), 1, OpCapability::Pure, 1),
            (
                "input".to_owned(),
                2,
                OpCapability::ReadsDeterministicInput,
                0,
            ),
            (
                "emit".to_owned(),
                3,
                OpCapability::EmitsDeterministicEffect,
                1,
            ),
        ]
    );
    rustia.install_host_module(module).unwrap();

    let source = r#"
return {
    decide = function()
        local value = world.double(world.input())
        world.emit(value)
        return value
    end,
}
"#;
    let exports = rustia.compile_and_run(source).unwrap();
    let mut host = Host {
        input: 21,
        slots: Vec::new(),
    };
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(32),
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, i64>(invocation, ())
            },
        )
        .unwrap();
    assert_eq!(*outcome.value(), 42);
    assert_eq!(outcome.records()[0].kind, RecordKind::Effect(42));
    rustia.reset_decision().unwrap();
    let exports = rustia.compile_and_run(source).unwrap();
    host.input = 7;
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(32),
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, i64>(invocation, ())
            },
        )
        .unwrap();
    assert_eq!(*outcome.value(), 14);
}

#[test]
fn mutable_external_context_is_profile_gated() {
    let mut deterministic = Rustia::for_host::<Host>(Profile::DeterministicSim);
    let rejected = deterministic.add_external("increment", 2, |mut host: HostMut<'_, Host>| {
        host.get_mut().input += 1;
        host.get_mut().input
    });
    assert!(matches!(
        rejected.unwrap_err(),
        Error::Vm(ref error)
            if matches!(error.kind, VmErrorKind::CapabilityRemoved { .. })
    ));

    let mut standard = Rustia::for_host::<Host>(Profile::ReproducibleStandard);
    standard
        .add_external("increment", 2, |mut host: HostMut<'_, Host>| {
            host.get_mut().input += 1;
            host.get_mut().input
        })
        .unwrap();
    let exports = standard
        .compile_and_run("return { decide = function() return increment() end }")
        .unwrap();
    let mut host = Host {
        input: 5,
        slots: Vec::new(),
    };
    let outcome = standard
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            exports
                .function_in(invocation, "decide")?
                .call_in::<_, _, i64>(invocation, ())
        })
        .unwrap();
    assert_eq!(*outcome.value(), 6);
    assert_eq!(host.input, 6);
}

#[test]
fn exact_limits_publish_without_growing_a_preallocated_lane_and_report_usage() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run("return { decide = function() emit(1); emit(2); return 7 end }")
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let mut lane = Vec::with_capacity(3);
    lane.push(Record {
        sequence: 99,
        kind: RecordKind::Effect(99),
    });
    let allocation = lane.as_ptr();
    let receipt = rustia
        .invoke_into(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(32),
            &mut lane,
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, i64>(invocation, ())
            },
        )
        .unwrap();
    assert_eq!(*receipt.value(), 7);
    assert_eq!(receipt.record_count(), 2);
    assert_eq!(receipt.record_bytes(), 32);
    assert!(receipt.fuel_used() > 0);
    assert_eq!(lane.as_ptr(), allocation);
    assert_eq!(lane.len(), 3);
    assert_eq!(lane[1].sequence, 0);
    assert_eq!(lane[2].sequence, 1);
}

#[test]
fn a_soft_failure_in_a_later_rust_call_rolls_back_only_that_call() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    first = function() emit(1) end,
    second = function() emit(2); error("second failed") end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(4)
                .with_max_record_bytes(64),
            |invocation| {
                exports
                    .function_in(invocation, "first")?
                    .call_in::<_, _, ()>(invocation, ())?;
                let error = exports
                    .function_in(invocation, "second")?
                    .call_in::<_, _, ()>(invocation, ())
                    .unwrap_err();
                assert!(error.to_string().contains("second failed"));
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(
        outcome.records(),
        [Record {
            sequence: 0,
            kind: RecordKind::Effect(1),
        }]
    );
}

#[test]
fn invocation_run_conversion_failure_rolls_back_module_main_records() {
    let mut rustia = configured_vm();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let outcome = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(4)
                .with_max_record_bytes(64),
            |invocation| {
                let conversion = invocation.run::<i64>("bad-main.luau", "emit(9); return {}");
                assert!(conversion.is_err());
                Ok(())
            },
        )
        .unwrap();
    assert!(outcome.records().is_empty());
}

#[test]
fn transactional_resource_errors_are_hard_without_changing_legacy_pcall() {
    let mut legacy = Rustia::new(Profile::DeterministicSim);
    legacy
        .add_fn("oom", || -> Result<()> {
            Err(Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))
        })
        .unwrap();
    let caught: bool = legacy.run("local ok = pcall(oom); return ok").unwrap();
    assert!(!caught);

    let mut rustia = configured_vm();
    rustia
        .add_pure("oom", 2, || -> Result<()> {
            Err(Error::Vm(VmError::new(VmErrorKind::OutOfMemory)))
        })
        .unwrap();
    let exports = rustia
        .compile_and_run("return { decide = function() pcall(oom); emit(1) end }")
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let error = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(32),
            |invocation| {
                exports
                    .function_in(invocation, "decide")?
                    .call_in::<_, _, ()>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error) if matches!(error.kind, VmErrorKind::OutOfMemory)
    ));
}

#[test]
fn resumed_protected_coroutine_cannot_swallow_a_hard_yield_boundary() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run(
            r#"
local object = setmetatable({}, {
    __index = function()
        coroutine.yield("illegal")
    end,
})
local thread = coroutine.create(function()
    local ok, value = pcall(function()
        coroutine.yield("paused")
        return object.missing
    end)
    return ok, value
end)
return {
    resume = function()
        return coroutine.resume(thread)
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let first = rustia
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            exports
                .function_in(invocation, "resume")?
                .call_in::<_, _, (bool, String)>(invocation, ())
        })
        .unwrap();
    assert_eq!(first.value(), &(true, "paused".to_owned()));
    let error = rustia
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            exports
                .function_in(invocation, "resume")?
                .call_in::<_, _, ()>(invocation, ())
        })
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error)
            if matches!(error.kind, VmErrorKind::YieldAcrossBoundary)
    ));
}

#[test]
fn resumed_xpcall_coroutine_cannot_swallow_a_hard_yield_boundary() {
    let mut rustia = configured_vm();
    let exports = rustia
        .compile_and_run(
            r#"
local object = setmetatable({}, {
    __index = function()
        coroutine.yield("illegal")
    end,
})
local thread = coroutine.create(function()
    local ok, value = xpcall(function()
        coroutine.yield("paused")
        return object.missing
    end, function(error)
        return error
    end)
    return ok, value
end)
return {
    resume = function()
        return coroutine.resume(thread)
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: Vec::new(),
    };
    let first = rustia
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            exports
                .function_in(invocation, "resume")?
                .call_in::<_, _, (bool, String)>(invocation, ())
        })
        .unwrap();
    assert_eq!(first.value(), &(true, "paused".to_owned()));
    let error = rustia
        .invoke(&mut host, InvocationLimits::new(10_000), |invocation| {
            exports
                .function_in(invocation, "resume")?
                .call_in::<_, _, ()>(invocation, ())
        })
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Vm(ref error)
            if matches!(error.kind, VmErrorKind::YieldAcrossBoundary)
    ));
}

#[test]
fn deterministic_oom_escapes_pcall_and_discards_the_outer_journal() {
    let settings = VmSettings {
        memory_limit_bytes: Some(1_000_000),
        ..VmSettings::default()
    };
    let mut rustia = Rustia::for_host_with_settings::<Host>(Profile::DeterministicSim, settings);
    rustia
        .add_effect(
            "emit",
            2,
            |mut effects: Effects<'_, Host>, value: i64| -> Result<()> { effects.emit(value) },
        )
        .unwrap();
    let exports = rustia
        .compile_and_run(
            r#"
return {
    decide = function()
        emit(1)
        pcall(function()
            local values = {}
            for i = 1, 20000 do
                values[i] = string.rep("x", 100) .. i
            end
        end)
        emit(2)
        return true
    end,
}
"#,
        )
        .unwrap();
    let mut host = Host {
        input: 0,
        slots: vec![],
    };
    let error = rustia
        .invoke(
            &mut host,
            InvocationLimits::new(10_000_000)
                .with_max_records(8)
                .with_max_record_bytes(128),
            |invocation| {
                let decide = exports.function_in(invocation, "decide")?;
                decide.call_in::<_, _, bool>(invocation, ())
            },
        )
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::Vm(ref error) if matches!(error.kind, VmErrorKind::OutOfMemory)
        ),
        "unexpected deterministic OOM error: {error:?}"
    );
}
