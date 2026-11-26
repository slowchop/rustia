//! Run the same Luau decision three times with a limit on how many commands
//! it can issue. The middle run goes over the limit, so the host receives none
//! of its commands. The third run shows that you can keep using the interpreter.
//!
//! From the repository root, run
//! `cargo run --locked -p rustia --example hardened_decision`. It prints
//! `success: 7; rejected: record limit; recovered: 9; epochs: 3`.

use rustia::{
    Effects, HostContext, HostModule, HostSchema, Input, InvocationLimits, Result, Rustia,
    RustiaBuilder, VmErrorKind,
};
use std::convert::Infallible;

#[derive(Debug, PartialEq)]
struct Command {
    sequence: u32,
    target: i32,
}

struct DecisionSchema;

impl HostSchema for DecisionSchema {
    type Input = i32;
    type Slots = ();
    type External = ();
    type Effect = i32;
    type SlotWrite = Infallible;
    type Record = Command;
}

// Borrow fresh application input for one call; do not capture it in callbacks.
struct DecisionContext<'input> {
    target: &'input i32,
    external: (),
}

impl HostContext<DecisionSchema> for DecisionContext<'_> {
    fn input(&self) -> &i32 {
        self.target
    }

    fn slots(&self) -> &() {
        &()
    }

    fn external_mut(&mut self) -> &mut () {
        &mut self.external
    }

    fn record_effect(&self, sequence: u32, target: i32) -> Command {
        Command { sequence, target }
    }

    fn record_slot_write(&self, _sequence: u32, write: Infallible) -> Command {
        match write {}
    }

    fn record_bytes(&self, _record: &Command) -> u64 {
        // Logical protocol bytes: a u32 sequence and an i32 target.
        8
    }
}

fn main() -> Result<()> {
    // Hardened leaves require an explicit accounted-memory limit. Register
    // their host interface before the first entry seals the environment.
    let mut vm = RustiaBuilder::deterministic_leaf()
        .memory_limit_bytes(2 * 1024 * 1024)
        .build_for_host::<DecisionSchema>()?;
    vm.install_host_module(
        HostModule::<DecisionSchema>::new("agent")
            .input("target", 2, |input: Input<'_, DecisionSchema>| *input.get())
            .effect(
                "move",
                2,
                |mut effects: Effects<'_, DecisionSchema>, target: i32| -> Result<()> {
                    effects.emit(target)
                },
            ),
    )?;

    // Compile once. The same host-owned entry survives every decision reset.
    let entry = Rustia::compile_entry(
        "decision.luau",
        r#"
local target = agent.target()
agent.move(target)
if target < 0 then
    agent.move(0)
end
return target
"#,
    )?;
    let limits = InvocationLimits::new(1_000)
        .with_max_records(1)
        .with_max_record_bytes(8);
    let mut commands = Vec::new();

    let mut context = DecisionContext {
        target: &7,
        external: (),
    };
    let receipt = vm.invoke_entry_into::<i32>(&mut context, limits, &mut commands, &entry)?;
    assert_eq!(*receipt.value(), 7);
    assert_eq!(
        commands,
        [Command {
            sequence: 0,
            target: 7
        }]
    );
    assert_eq!(vm.decision_epoch(), 1);
    // Apply successful commands to application state here, then clear the lane.
    commands.clear();

    // This input stages one command, then exceeds the one-record budget.
    // Neither command is published, and the entry lifecycle resets on failure.
    context.target = &-1;
    let error = vm
        .invoke_entry_into::<i32>(&mut context, limits, &mut commands, &entry)
        .expect_err("two commands must exceed the one-record budget");
    assert!(matches!(
        error.as_vm_error().map(|error| error.kind()),
        Some(VmErrorKind::HostResourceExhausted {
            resource: "effect-record count",
            limit: 1,
        })
    ));
    assert!(commands.is_empty());
    assert_eq!(vm.decision_epoch(), 2);

    // No manual reset or recompilation is needed after the rejected decision.
    context.target = &9;
    let receipt = vm.invoke_entry_into::<i32>(&mut context, limits, &mut commands, &entry)?;
    assert_eq!(*receipt.value(), 9);
    assert_eq!(
        commands,
        [Command {
            sequence: 0,
            target: 9
        }]
    );
    assert_eq!(vm.decision_epoch(), 3);
    println!("success: 7; rejected: record limit; recovered: 9; epochs: 3");
    Ok(())
}
