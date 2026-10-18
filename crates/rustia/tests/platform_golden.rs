//! One fixed transcript for Linux x86-64, macOS arm64, and Node/V8 Wasm.
//! Numeric cases retain the existing VM golden; RNG values come from the
//! pinned math_random_seed_sequence conformance fixture. No timing is taken.

use rustia::{Profile, Result, Rustia, RustiaBuilder};

const GOLDEN: &str = concat!(
    "numbers=7ff8000000000000,7ff8000000000000,fff0000000000000,3ff0000000000000,",
    "8000000000000000,c000000000000000,7ff0000000000000,7ff8000000000000\n",
    "libm=3ff6a09e667f3bcd,3fdeaee8744b05f0,3fe62e42fefa39ef\n",
    "state=first:1;second:2;third:3;|6|782827,551456,107634|305419896|pause:8:9|true"
);

fn transcript(lua: &mut Rustia) -> Result<String> {
    let cases = [
        ("left / right", 0.0, 0.0),
        ("left - right", f64::INFINITY, f64::INFINITY),
        ("left / right", 1.0, -0.0),
        ("left % right", -3.0, 2.0),
        ("left % right", -0.0, 3.0),
        ("left // right", -3.0, 2.0),
        ("left // right", 5.0, 0.0),
        ("left ^ right", -2.0, 0.5),
    ];
    let mut numbers = Vec::new();
    for (expression, left, right) in cases {
        lua.insert_global("left", left)?;
        lua.insert_global("right", right)?;
        let result: f64 = lua.eval(format!("return {expression}"))?;
        numbers.push(format!("{:016x}", result.to_bits()));
    }
    let libm: Vec<f64> = lua.eval("return {math.sqrt(2), math.sin(0.5), math.log(2)}")?;
    let libm = libm
        .into_iter()
        .map(|value| format!("{:016x}", value.to_bits()))
        .collect::<Vec<_>>()
        .join(",");
    let state: String = lua.eval(
        r#"
        local ordered = {}
        ordered.first = 1; ordered.second = 2; ordered.third = 3
        local keys = ''
        for key, value in pairs(ordered) do
            keys = keys .. key .. ':' .. value .. ';'
        end
        local total = 0
        for _, value in ipairs({1, 2, 3}) do total = total + value end
        math.randomseed(0)
        local random = math.random(1000000) .. ',' .. math.random(1000000)
            .. ',' .. math.random(1000000)
        local bytes = buffer.create(4)
        buffer.writeu32(bytes, 0, 0x12345678)
        local worker = coroutine.create(function()
            local input = coroutine.yield('pause')
            return input + 1
        end)
        local ok, first = coroutine.resume(worker)
        assert(ok)
        local resumed, last = coroutine.resume(worker, 8)
        assert(resumed)
        local cycle = {}; cycle.self = cycle
        return keys .. '|' .. total .. '|' .. random .. '|'
            .. buffer.readu32(bytes, 0) .. '|' .. first .. ':8:' .. last .. '|'
            .. tostring(cycle.self == cycle)
        "#,
    )?;
    Ok(format!(
        "numbers={}\nlibm={libm}\nstate={state}",
        numbers.join(",")
    ))
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn private_platform_transcript_matches_before_and_after_reset() -> Result<()> {
    for profile in [Profile::DeterministicSim, Profile::ReproducibleStandard] {
        let mut lua = RustiaBuilder::standard()
            .profile(profile)
            .without_stdout()
            .fuel(100_000)
            .memory_limit_bytes(2 * 1024 * 1024)
            .build()?;
        for _ in 0..2 {
            assert_eq!(transcript(&mut lua)?, GOLDEN, "{profile:?}");
            lua.reset_decision()?;
        }
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    wasm_bindgen_test::console_log!(
        "RUSTIA_PRIVATE_PLATFORM_GOLDEN {}",
        GOLDEN.replace('\n', "|")
    );
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    println!(
        "RUSTIA_PRIVATE_PLATFORM_GOLDEN {}",
        GOLDEN.replace('\n', "|")
    );
    Ok(())
}
