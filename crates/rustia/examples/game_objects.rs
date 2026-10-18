//! A deterministic decision reads a snapshot and requests changes. The game
//! applies those requests only after the complete invocation succeeds.

use rustia::{
    Effects, HostContext, HostSchema, Input, InvocationLimits, ObjectType, Result, Rustia,
    RustiaBuilder, SlotWrites, Slots,
};

struct Actor {
    id: u64,
}
struct Game;

#[derive(Debug)]
enum Change {
    Move { actor: u64, destination: i64 },
    Remember(i64),
}

impl HostSchema for Game {
    type Input = i64;
    type Slots = i64;
    type External = ();
    type Effect = Change;
    type SlotWrite = i64;
    type Record = (u32, Change);
}

// These borrows last for one decision; the VM never owns the game state.
struct Turn<'a> {
    target: &'a i64,
    last_goal: &'a i64,
    external: (),
}

impl HostContext<Game> for Turn<'_> {
    fn input(&self) -> &i64 {
        self.target
    }
    fn slots(&self) -> &i64 {
        self.last_goal
    }
    fn external_mut(&mut self) -> &mut () {
        &mut self.external
    }
    fn record_effect(&self, sequence: u32, change: Change) -> (u32, Change) {
        (sequence, change)
    }
    fn record_slot_write(&self, sequence: u32, value: i64) -> (u32, Change) {
        (sequence, Change::Remember(value))
    }
    fn record_bytes(&self, _: &(u32, Change)) -> u64 {
        24
    }
}

fn main() -> Result<()> {
    let mut lua = RustiaBuilder::deterministic()
        .memory_limit_bytes(2 * 1024 * 1024)
        .build_for_host::<Game>()?;
    lua.register_object_type(
        ObjectType::<Actor, Game>::new("Actor")
            .input("target", 1, |_: &Actor, input: Input<'_, Game>| {
                *input.get()
            })
            .slot_read("last_goal", 1, |_: &Actor, slots: Slots<'_, Game>| {
                *slots.get()
            })
            .effect(
                "move_to",
                2,
                |actor: &Actor, mut effects: Effects<'_, Game>, destination: i64| {
                    effects.emit(Change::Move {
                        actor: actor.id,
                        destination,
                    })
                },
            )
            .slot_write(
                "remember",
                2,
                |_: &Actor, mut writes: SlotWrites<'_, Game>, value: i64| writes.write(value),
            ),
    )?;

    let module = Rustia::compile_module(
        "decision.luau",
        r#"
        local function decide(actor)
            local target = actor:target()
            if target ~= actor:last_goal() then
                actor:move_to(target)
                actor:remember(target)
            end
        end
        return {
            accept = decide,
            reject = function(actor)
                decide(actor)
                error("cancel this decision")
            end,
        }
    "#,
    )?;

    let mut position = 0;
    let mut last_goal = 0;
    for (entry, target, accepted) in [
        ("accept", 8, true),
        ("reject", 99, false),
        ("accept", 8, true),
    ] {
        let actor = lua.create_object(Actor { id: 7 })?;
        let mut turn = Turn {
            target: &target,
            last_goal: &last_goal,
            external: (),
        };
        let outcome = lua.invoke(
            &mut turn,
            InvocationLimits::new(10_000)
                .with_max_records(2)
                .with_max_record_bytes(48),
            |call| {
                let exports = call.install(&module)?;
                exports
                    .function_in(call, entry)?
                    .call_values_in(call, &[actor.as_value()])?;
                Ok(())
            },
        );
        assert_eq!(outcome.is_ok(), accepted);
        match outcome {
            Ok(outcome) => {
                // This is the application commit. A failed invocation publishes
                // no records, so it cannot partially change the game.
                for (_, change) in outcome.records() {
                    match change {
                        Change::Move { actor, destination } => {
                            assert_eq!(*actor, 7);
                            position = *destination;
                        }
                        Change::Remember(value) => last_goal = *value,
                    }
                }
            }
            Err(error) => {
                assert!(error.to_string().contains("cancel this decision"));
                println!("Decision cancelled: {error}");
            }
        }
        lua.reset_decision()?;
        assert_eq!((position, last_goal), (8, 8));
    }
    println!("Actor 7 stays at position 8; the cancelled move to 99 was not applied.");
    Ok(())
}
