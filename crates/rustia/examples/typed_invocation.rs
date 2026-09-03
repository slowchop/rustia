use rustia::{
    Effects, HostContext, HostModule, HostSchema, Input, InvocationLimits, Result, Rustia,
    RustiaBuilder, SlotWrites, Slots,
};

#[derive(Clone, Copy)]
struct AgentInput {
    target_x: i64,
}

#[derive(Debug, PartialEq)]
enum RecordPayload {
    Move { dx: i64 },
    SetSlot { slot: usize, value: i64 },
}

#[derive(Debug, PartialEq)]
struct EffectRecord {
    emitter: u64,
    sequence: u32,
    payload: RecordPayload,
}

struct AiSchema;

impl HostSchema for AiSchema {
    type Input = AgentInput;
    type Slots = [i64];
    type External = ();
    type Effect = RecordPayload;
    type SlotWrite = (usize, i64);
    type Record = EffectRecord;
}

// This value exists only for one invocation and may borrow tick-local data.
struct TickContext<'tick> {
    emitter: u64,
    input: &'tick AgentInput,
    slots: &'tick [i64],
    external: (),
}

impl HostContext<AiSchema> for TickContext<'_> {
    fn input(&self) -> &AgentInput {
        self.input
    }

    fn slots(&self) -> &[i64] {
        self.slots
    }

    fn external_mut(&mut self) -> &mut () {
        &mut self.external
    }

    fn record_effect(&self, sequence: u32, effect: RecordPayload) -> EffectRecord {
        EffectRecord {
            emitter: self.emitter,
            sequence,
            payload: effect,
        }
    }

    fn record_slot_write(&self, sequence: u32, (slot, value): (usize, i64)) -> EffectRecord {
        EffectRecord {
            emitter: self.emitter,
            sequence,
            payload: RecordPayload::SetSlot { slot, value },
        }
    }

    fn record_bytes(&self, _record: &EffectRecord) -> u64 {
        // Logical protocol bytes, not size_of::<EffectRecord>().
        24
    }
}

fn main() -> Result<()> {
    let mut vm = RustiaBuilder::deterministic().build_for_host::<AiSchema>()?;
    let host = HostModule::<AiSchema>::new("ai")
        .input("target_x", 2, |input: Input<'_, AiSchema>| {
            input.get().target_x
        })
        .slot_read("slot", 2, |slots: Slots<'_, AiSchema>, slot: usize| {
            slots.get()[slot]
        })
        .effect(
            "move",
            3,
            |mut effects: Effects<'_, AiSchema>, dx: i64| -> Result<()> {
                effects.emit(RecordPayload::Move { dx })
            },
        )
        .slot_write(
            "remember",
            3,
            |mut writes: SlotWrites<'_, AiSchema>, slot: usize, value: i64| -> Result<()> {
                writes.write((slot, value))
            },
        );
    vm.install_host_module(host)?;

    let module = Rustia::compile_module(
        "alien_ai.luau",
        r#"
return {
    decide = function()
        local target = ai.target_x()
        local dx = target - ai.slot(0)
        ai.move(dx)
        ai.remember(0, target)
        return dx
    end,
}
"#,
    )?;

    // Typed VMs execute guest code only inside an Invocation, including module
    // top-level code during installation.
    let mut bootstrap = TickContext {
        emitter: 0,
        input: &AgentInput { target_x: 0 },
        slots: &[],
        external: (),
    };
    let exports = vm
        .invoke(
            &mut bootstrap,
            InvocationLimits::new(10_000)
                .with_max_records(0)
                .with_max_record_bytes(0),
            |invocation| invocation.install(&module),
        )?
        .into_parts()
        .0;

    let input = AgentInput { target_x: 25 };
    let slots = [10];
    let mut context = TickContext {
        emitter: 7,
        input: &input,
        slots: &slots,
        external: (),
    };
    let mut lane = Vec::new();
    let receipt = vm.invoke_into(
        &mut context,
        InvocationLimits::new(10_000)
            .with_max_records(8)
            .with_max_record_bytes(8 * 24),
        &mut lane,
        |invocation| {
            let decide = exports.function_in(invocation, "decide")?;
            decide.call_in::<_, _, i64>(invocation, ())
        },
    )?;

    assert_eq!(*receipt.value(), 15);
    assert_eq!(receipt.record_count(), 2);
    assert_eq!(receipt.record_bytes(), 48);
    assert_eq!(lane[0].sequence, 0);
    assert_eq!(lane[1].sequence, 1);
    println!(
        "value={}, fuel={}, records={lane:?}",
        receipt.value(),
        receipt.fuel_used()
    );
    Ok(())
}
