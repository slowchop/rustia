use rustia::{
    Effects, HostContext, HostModule, HostSchema, Input, InvocationLimits, Result, Rustia,
    RustiaBuilder, SlotWrites, Slots,
};
use std::hint::black_box;
use std::time::Instant;

const WARMUP_DECISIONS: usize = 2_000;
const MEASURED_DECISIONS: usize = 20_000;
const RECORD_BYTES: u64 = 16;
const MEMORY_LIMIT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
struct LeafInput {
    target: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeafCommand {
    Move(i32),
    SetSlot { slot: u8, value: i32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LeafRecord {
    sequence: u32,
    command: LeafCommand,
}

struct LeafSchema;

impl HostSchema for LeafSchema {
    type Input = LeafInput;
    type Slots = [i32];
    type External = ();
    type Effect = LeafCommand;
    type SlotWrite = (u8, i32);
    type Record = LeafRecord;
}

struct LeafContext<'decision> {
    input: &'decision LeafInput,
    slots: &'decision [i32],
    external: (),
}

impl HostContext<LeafSchema> for LeafContext<'_> {
    fn input(&self) -> &LeafInput {
        self.input
    }

    fn slots(&self) -> &[i32] {
        self.slots
    }

    fn external_mut(&mut self) -> &mut () {
        &mut self.external
    }

    fn record_effect(&self, sequence: u32, effect: LeafCommand) -> LeafRecord {
        LeafRecord {
            sequence,
            command: effect,
        }
    }

    fn record_slot_write(&self, sequence: u32, write: (u8, i32)) -> LeafRecord {
        LeafRecord {
            sequence,
            command: LeafCommand::SetSlot {
                slot: write.0,
                value: write.1,
            },
        }
    }

    fn record_bytes(&self, _record: &LeafRecord) -> u64 {
        RECORD_BYTES
    }
}

fn build_vm(hardened: bool) -> Result<Rustia<LeafSchema>> {
    let builder = if hardened {
        RustiaBuilder::deterministic_leaf().memory_limit_bytes(MEMORY_LIMIT_BYTES)
    } else {
        RustiaBuilder::deterministic()
    };
    let mut vm = builder.build_for_host::<LeafSchema>()?;
    vm.install_host_module(
        HostModule::<LeafSchema>::new("leaf")
            .input("target", 2, |input: Input<'_, LeafSchema>| {
                input.get().target
            })
            .slot_read("slot", 2, |slots: Slots<'_, LeafSchema>, slot: u8| {
                slots
                    .get()
                    .get(usize::from(slot))
                    .copied()
                    .ok_or_else(|| rustia::Error::Embed("slot index out of range".to_owned()))
            })
            .effect(
                "move",
                3,
                |mut effects: Effects<'_, LeafSchema>, delta: i32| -> Result<()> {
                    effects.emit(LeafCommand::Move(delta))
                },
            )
            .slot_write(
                "remember",
                3,
                |mut writes: SlotWrites<'_, LeafSchema>, slot: u8, value: i32| -> Result<()> {
                    writes.write((slot, value))
                },
            ),
    )?;
    Ok(vm)
}

fn run_module_decision(
    vm: &mut Rustia<LeafSchema>,
    module: &rustia::CompiledModule,
    lane: &mut Vec<LeafRecord>,
    target: i32,
) -> Result<i32> {
    lane.clear();
    let input = LeafInput { target };
    let slots = [target.wrapping_sub(3)];
    let mut context = LeafContext {
        input: &input,
        slots: &slots,
        external: (),
    };
    let receipt = vm.invoke_into(
        &mut context,
        InvocationLimits::new(10_000)
            .with_max_records(4)
            .with_max_record_bytes(4 * RECORD_BYTES),
        lane,
        |invocation| {
            let exports = invocation.install(module.clone())?;
            let decide = exports.function_in(invocation, "decide")?;
            decide.call_in::<_, _, i32>(invocation, ())
        },
    )?;
    let value = *receipt.value();
    vm.reset_decision()?;
    Ok(value)
}

fn run_entry_decision(
    vm: &mut Rustia<LeafSchema>,
    entry: &rustia::CompiledEntry,
    lane: &mut Vec<LeafRecord>,
    target: i32,
) -> Result<i32> {
    lane.clear();
    let input = LeafInput { target };
    let slots = [target.wrapping_sub(3)];
    let mut context = LeafContext {
        input: &input,
        slots: &slots,
        external: (),
    };
    let receipt = vm.invoke_entry_into(
        &mut context,
        InvocationLimits::new(10_000)
            .with_max_records(4)
            .with_max_record_bytes(4 * RECORD_BYTES),
        lane,
        entry,
    )?;
    Ok(*receipt.value())
}

fn percentile(sorted: &[u128], numerator: usize, denominator: usize) -> u128 {
    let index = sorted
        .len()
        .saturating_mul(numerator)
        .div_ceil(denominator)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted[index]
}

fn add_result(checksum: &mut i64, lane: &[LeafRecord], value: i32) {
    *checksum = checksum.wrapping_add(i64::from(value));
    for record in lane {
        *checksum = checksum.wrapping_add(i64::from(record.sequence));
        *checksum = checksum.wrapping_add(match record.command {
            LeafCommand::Move(delta) => i64::from(delta),
            LeafCommand::SetSlot { slot, value } => i64::from(slot) + i64::from(value),
        });
    }
}

fn run_cohort(
    samples: &mut Vec<u128>,
    checksum: &mut i64,
    lane: &mut Vec<LeafRecord>,
    next_target: &mut i32,
    decisions: usize,
    mut run: impl FnMut(&mut Vec<LeafRecord>, i32) -> Result<i32>,
) -> Result<()> {
    for _ in 0..decisions {
        let target = *next_target;
        *next_target = next_target.wrapping_add(1);
        let started = Instant::now();
        let value = run(lane, target)?;
        samples.push(started.elapsed().as_nanos());
        add_result(checksum, lane, value);
    }
    Ok(())
}

fn print_report(label: &str, samples: &mut [u128], lane_capacity: usize, checksum: i64) {
    samples.sort_unstable();
    black_box(checksum);
    println!(
        "path={label} decisions={} p50_ns={} p95_ns={} p99_ns={} max_ns={} lane_capacity={} checksum={checksum}",
        samples.len(),
        percentile(samples, 50, 100),
        percentile(samples, 95, 100),
        percentile(samples, 99, 100),
        samples.last().copied().unwrap_or_default(),
        lane_capacity,
    );
}

fn validate_semantics(
    module: &rustia::CompiledModule,
    entry: &rustia::CompiledEntry,
) -> Result<()> {
    let mut module_vm = build_vm(false)?;
    let mut entry_vm = build_vm(true)?;
    let mut module_lane = Vec::with_capacity(4);
    let mut entry_lane = Vec::with_capacity(4);
    let module_value = run_module_decision(&mut module_vm, module, &mut module_lane, 9)?;
    let entry_value = run_entry_decision(&mut entry_vm, entry, &mut entry_lane, 9)?;
    let expected = [
        LeafRecord {
            sequence: 0,
            command: LeafCommand::Move(3),
        },
        LeafRecord {
            sequence: 1,
            command: LeafCommand::SetSlot { slot: 0, value: 9 },
        },
    ];
    assert_eq!(module_value, 3, "module path returned the wrong value");
    assert_eq!(entry_value, 3, "entry path returned the wrong value");
    assert_eq!(
        module_lane, expected,
        "module path recorded the wrong effects"
    );
    assert_eq!(
        entry_lane, expected,
        "entry path recorded the wrong effects"
    );
    assert_eq!(
        module_lane, entry_lane,
        "the benchmark paths do not implement the same decision"
    );
    Ok(())
}

fn main() -> Result<()> {
    let module = Rustia::<LeafSchema>::compile_module(
        "leaf_lifecycle_module_bench.luau",
        r#"
return {
    decide = function()
        local target = leaf.target()
        local delta = target - leaf.slot(0)
        leaf.move(delta)
        leaf.remember(0, target)
        return delta
    end,
}
"#,
    )?;
    let entry = Rustia::<LeafSchema>::compile_entry(
        "leaf_lifecycle_entry_bench.luau",
        r#"
local target = leaf.target()
local delta = target - leaf.slot(0)
leaf.move(delta)
leaf.remember(0, target)
return delta
"#,
    )?;

    validate_semantics(&module, &entry)?;

    let mut module_vm = build_vm(false)?;
    let mut entry_vm = build_vm(true)?;
    let mut module_lane = Vec::with_capacity(4);
    let mut entry_lane = Vec::with_capacity(4);

    for decision in 0..WARMUP_DECISIONS {
        black_box(run_module_decision(
            &mut module_vm,
            &module,
            &mut module_lane,
            decision as i32,
        )?);
        black_box(run_entry_decision(
            &mut entry_vm,
            &entry,
            &mut entry_lane,
            decision as i32,
        )?);
    }

    let cohort = MEASURED_DECISIONS / 2;
    let mut module_samples = Vec::with_capacity(MEASURED_DECISIONS);
    let mut entry_samples = Vec::with_capacity(MEASURED_DECISIONS);
    let mut module_checksum = 0i64;
    let mut entry_checksum = 0i64;
    let mut module_target = 0i32;
    let mut entry_target = 0i32;

    // ABBA balances the process-level order while retaining an individual
    // duration for every complete decision lifecycle.
    run_cohort(
        &mut module_samples,
        &mut module_checksum,
        &mut module_lane,
        &mut module_target,
        cohort,
        |lane, target| run_module_decision(&mut module_vm, &module, lane, target),
    )?;
    run_cohort(
        &mut entry_samples,
        &mut entry_checksum,
        &mut entry_lane,
        &mut entry_target,
        cohort,
        |lane, target| run_entry_decision(&mut entry_vm, &entry, lane, target),
    )?;
    run_cohort(
        &mut entry_samples,
        &mut entry_checksum,
        &mut entry_lane,
        &mut entry_target,
        MEASURED_DECISIONS - cohort,
        |lane, target| run_entry_decision(&mut entry_vm, &entry, lane, target),
    )?;
    run_cohort(
        &mut module_samples,
        &mut module_checksum,
        &mut module_lane,
        &mut module_target,
        MEASURED_DECISIONS - cohort,
        |lane, target| run_module_decision(&mut module_vm, &module, lane, target),
    )?;
    assert_eq!(
        module_checksum, entry_checksum,
        "the measured benchmark paths diverged"
    );

    print_report(
        "module",
        &mut module_samples,
        module_lane.capacity(),
        module_checksum,
    );
    print_report(
        "hardened_entry",
        &mut entry_samples,
        entry_lane.capacity(),
        entry_checksum,
    );
    Ok(())
}
