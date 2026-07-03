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
    let host = RustiaExtension::new("host")
        .fn_slot_read("bias", || 2.0)
        .fn_effect("score", |value: f64| value + 1.0);

    let mut rustia = Rustia::builder()
        .profile(Profile::DeterministicSim)
        .extension(host)
        .build()?;

    rustia.insert_global("difficulty", 3_i32)?;
    rustia.add_fn("clamp", |value: f64, lo: f64, hi: f64| value.clamp(lo, hi))?;

    let value = rustia.run_owned_named(
        "embedding-inline.luau",
        "return clamp(difficulty + host.bias() + host.score(5), 0, 12)",
    )?;
    assert_eq!(f64::from_rustia(value)?, 11.0);

    let exports = rustia.compile_and_run_exports_named(
        "ai_exports.luau",
        "\
return {
    label = 'ai',
    decide = function(threat)
        return clamp(threat + host.bias(), 0, 10)
    end,
}
",
    )?;
    assert_eq!(exports.get::<String>(&mut rustia, "label")?, "ai");

    let decide = exports.function(&mut rustia, "decide")?;
    let clamped: f64 = decide.call(&mut rustia, (9.0,))?;
    assert_eq!(clamped, 10.0);

    let scenario_exports = rustia.compile_and_run_exports_named(
        "scenario.luau",
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
    )?;
    let scenario: Scenario = scenario_exports.deserialize(&mut rustia)?;
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
    let value = rustia.run_owned_named(
        "scenario_check.luau",
        "return scenario.enemies[2].kind .. ':' .. tostring(scenario.difficulty)",
    )?;
    assert_eq!(String::from_rustia(value)?, "brute:2");

    println!("embedding example ok");
    Ok(())
}
