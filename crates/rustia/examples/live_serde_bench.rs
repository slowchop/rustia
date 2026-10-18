use rustia::{Profile, Result, Rustia};
use serde::Deserialize;
use std::hint::black_box;
use std::time::{Duration, Instant};

#[derive(Deserialize, PartialEq)]
struct Scenario {
    name: String,
    difficulty: u32,
    enemies: Vec<Enemy>,
}

#[derive(Deserialize, PartialEq)]
struct Enemy {
    kind: String,
    hp: u32,
    position: (f64, f64),
}

fn main() -> Result<()> {
    let iterations = std::env::var("RUSTIA_LIVE_SERDE_BENCH_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2_000);
    let rounds = 6;

    let mut rustia = rustia::RustiaBuilder::standard()
        .profile(Profile::ReproducibleStandard)
        .build()
        .unwrap();
    let exports = rustia
        .load(
            r#"
        return {
            name = "arena",
            difficulty = 3,
            enemies = {
                { kind = "scout", hp = 5, position = { 1.25, 2.5 } },
                { kind = "brute", hp = 9, position = { 4.0, 8.0 } },
                { kind = "scout", hp = 6, position = { 16.0, 32.0 } },
            },
        }
        "#,
        )
        .install()?;

    let direct: Scenario = exports.deserialize(&mut rustia)?;
    let owned: Scenario = exports.value(&mut rustia)?.deserialize()?;
    assert!(
        direct == owned,
        "benchmark paths must deserialize identically"
    );

    let mut live_elapsed = Duration::ZERO;
    let mut owned_elapsed = Duration::ZERO;
    for _ in 0..rounds {
        live_elapsed += measure_live(&exports, &mut rustia, iterations)?;
        owned_elapsed += measure_owned(&exports, &mut rustia, iterations)?;
        owned_elapsed += measure_owned(&exports, &mut rustia, iterations)?;
        live_elapsed += measure_live(&exports, &mut rustia, iterations)?;
    }

    let samples = (rounds * iterations * 2) as f64;
    let live_ns = live_elapsed.as_nanos() as f64 / samples;
    let owned_ns = owned_elapsed.as_nanos() as f64 / samples;
    println!("live serde:  {live_ns:.1} ns/deserialize");
    println!("owned path:  {owned_ns:.1} ns/deserialize");
    println!("owned/live:  {:.3}x", owned_ns / live_ns);
    Ok(())
}

fn measure_live(
    exports: &rustia::Exports,
    rustia: &mut Rustia,
    iterations: usize,
) -> Result<Duration> {
    let started = Instant::now();
    for _ in 0..iterations {
        let value: Scenario = exports.deserialize(rustia)?;
        black_box(value);
    }
    Ok(started.elapsed())
}

fn measure_owned(
    exports: &rustia::Exports,
    rustia: &mut Rustia,
    iterations: usize,
) -> Result<Duration> {
    let started = Instant::now();
    for _ in 0..iterations {
        let value: Scenario = exports.value(rustia)?.deserialize()?;
        black_box(value);
    }
    Ok(started.elapsed())
}
