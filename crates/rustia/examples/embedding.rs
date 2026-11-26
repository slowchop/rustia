//! Pass Rust values and functions to Luau, then read a Luau table back into
//! a Rust struct with Serde. The last part resets script state and runs the
//! same compiled code again.
//!
//! From the repository root, run
//! `cargo run --locked -p rustia --example embedding`. It prints
//! `embedding example ok` when all the checks pass.
//!
//! If you copy this into your app, add Serde with its derive feature enabled
//! using `serde = { version = "1", features = ["derive"] }`.

use rustia::{FromRustia, Profile, Rustia, RustiaExtension};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Scenario {
    name: String,
    difficulty: u32,
    enemies: Vec<Enemy>,
}

#[derive(Debug, PartialEq, Deserialize, Serialize)]
struct Enemy {
    kind: String,
    hp: u32,
}

fn main() -> rustia::Result<()> {
    // Register host callbacks as persistent configuration for deterministic runs.
    let host = RustiaExtension::new("host")
        .fn_slot_read("bias", || 2.0)
        .fn_effect("score", |value: f64| value + 1.0);

    let mut rustia = Rustia::builder()
        .profile(Profile::DeterministicSim)
        .extension(host)
        .build()?;

    rustia.insert_global("difficulty", 3_i32)?;
    rustia.add_fn("clamp", |value: f64, lo: f64, hi: f64| value.clamp(lo, hi))?;
    rustia.add_fn("checked_bonus", |value: f64| -> rustia::Result<f64> {
        if value < 0.0 {
            return Err(rustia::Error::embed("negative bonus".to_owned()));
        }
        Ok(value + 1.0)
    })?;

    let value = rustia
        .load("return clamp(difficulty + host.bias() + host.score(5) + checked_bonus(1), 0, 12)")
        .source_name("embedding-inline.luau")
        .eval()?;
    assert_eq!(f64::from_rustia(value)?, 12.0);

    // Installation retains exports so Rust can call the script's functions later.
    let exports = rustia
        .load(
            "\
return {
    label = 'ai',
    decide = function(threat)
        return clamp(threat + host.bias(), 0, 10)
    end,
}
",
        )
        .source_name("ai_exports.luau")
        .install()?;
    assert_eq!(exports.get::<String>(&mut rustia, "label")?, "ai");

    let decide = exports.function(&mut rustia, "decide")?;
    let clamped: f64 = decide.call(&mut rustia, (9.0,))?;
    assert_eq!(clamped, 10.0);

    // Deserialize into Rust-owned structs, then send the data back to Luau.
    let scenario: Scenario = rustia
        .load(
            "\
return {
    name = 'arena',
    difficulty = 2,
    enemies = {
        { kind = 'scout', hp = 5 },
        { kind = 'brute', hp = 9 },
    },
}
",
        )
        .source_name("scenario.luau")
        .eval_serde()?;
    assert_eq!(
        scenario,
        Scenario {
            name: "arena".to_owned(),
            difficulty: 2,
            enemies: vec![
                Enemy {
                    kind: "scout".to_owned(),
                    hp: 5,
                },
                Enemy {
                    kind: "brute".to_owned(),
                    hp: 9,
                },
            ],
        }
    );

    rustia.insert_serde("scenario", &scenario)?;
    let value = rustia
        .load("return scenario.enemies[2].kind .. ':' .. tostring(scenario.difficulty)")
        .source_name("scenario_check.luau")
        .eval()?;
    assert_eq!(String::from_rustia(value)?, "brute:2");

    // Host-owned compiled code remains reusable after guest state is reset.
    let next_decision = Rustia::compile_module(
        "next_decision.luau",
        "return clamp(input + host.bias(), 0, 10)",
    )?;
    let prior_epoch = rustia.decision_epoch();
    rustia.reset_decision()?;
    assert_eq!(rustia.decision_epoch(), prior_epoch + 1);
    // Installed host functions and extensions survive; ordinary inputs do not.
    let value = rustia.eval("return difficulty == nil and host.bias() == 2")?;
    assert!(bool::from_rustia(value)?);
    rustia.insert_global("input", 9_i32)?;
    let value = rustia.install(&next_decision)?.value(&mut rustia)?;
    assert_eq!(f64::from_rustia(value)?, 10.0);

    println!("embedding example ok");
    Ok(())
}
