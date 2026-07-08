# Benchmark Scoreboard

Latest generated benchmark scoreboard for rustia.

- Native source: [current-results.json](current-results.json)
- Wasm source: [wasm/current-results.json](wasm/current-results.json)
- History: `bench/history/runs.jsonl`
- Native rows: 100
- Native measurement window: 2026-07-03 01:53:44 UTC to 2026-07-07 23:48:18 UTC

How to read this:

- **ratio to rustia** is based on the total score for that target; higher is faster.
- **total score** is the sum of median ns/unit for all listed workloads, displayed in microseconds; lower is better.
- Missing wasm adapters are shown as `not supported` so unsupported engines are visible instead of silently absent.

## Native Targets

## linux-x86_64

Native/host benchmark results. Total score is the sum of per-workload medians; lower is better.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | commit | status |
|---:|---|---|---:|---:|---:|---|---|
| 1 | wasmtime | wasm | 90.5x | 1.045 us | 5/5 | 2c28c4a | measured |
| 2 | wasmi | wasm | 14.87x | 6.363 us | 5/5 | 2c28c4a | measured |
| 3 | mlua-luau | luau | 2.353x | 40.19 us | 5/5 | 2c28c4a | measured |
| 4 | mlua-lua54 | lua54 | 1.488x | 63.57 us | 5/5 | 2c28c4a | measured |
| 5 | rustia | luau | 1x | 94.6 us | 5/5 | dcfb26a | measured |
| 6 | rquickjs | javascript | 0.497x | 190.4 us | 5/5 | 2c28c4a | measured |
| 7 | koto | koto | 0.248x | 382.2 us | 5/5 | 2c28c4a | measured |
| 8 | rhai | rhai | 0.159x | 594.2 us | 5/5 | 2c28c4a | measured |
| 9 | boa | javascript | 0.085x | 1,119 us | 5/5 | 2c28c4a | measured |
| 10 | roto | roto | 0.06x | 1,567 us | 5/5 | 2c28c4a | measured |

## macos-aarch64

Native/host benchmark results. Total score is the sum of per-workload medians; lower is better.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | commit | status |
|---:|---|---|---:|---:|---:|---|---|
| 1 | wasmtime | wasm | 455x | 0.6 us | 5/5 | e48285b | measured |
| 2 | wasmi | wasm | 93.15x | 2.928 us | 5/5 | e48285b | measured |
| 3 | mlua-luau | luau | 12.67x | 21.53 us | 5/5 | e48285b | measured |
| 4 | mlua-lua54 | lua54 | 5.865x | 46.5 us | 5/5 | d4bb1ed | measured |
| 5 | rquickjs | javascript | 2.458x | 111 us | 5/5 | e48285b | measured |
| 6 | koto | koto | 1.721x | 158.5 us | 5/5 | e48285b | measured |
| 7 | rhai | rhai | 1.18x | 231.1 us | 5/5 | e48285b | measured |
| 8 | rustia | luau | 1x | 272.8 us | 5/5 | e48285b | measured |
| 9 | boa | javascript | 0.504x | 540.8 us | 5/5 | e48285b | measured |
| 10 | roto | roto | 0.214x | 1,272 us | 5/5 | 9f3b0d1 | measured |

## Wasm Targets

## wasm32-javascriptcore-bun-linux-x64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | commit | status |
|---:|---|---|---:|---:|---:|---|---|
| 1 | koto | koto | 1.882x | 436.9 us | 5/5 | 2c28c4a | measured |
| 2 | rhai | rhai | 1.148x | 716.1 us | 5/5 | 2c28c4a | measured |
| 3 | rustia | luau | 1x | 822.3 us | 5/5 | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not measured | not supported |

## wasm32-spidermonkey-spidermonkey-unknown-os-x64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | commit | status |
|---:|---|---|---:|---:|---:|---|---|
| 1 | koto | koto | 1.659x | 463 us | 5/5 | 2c28c4a | measured |
| 2 | rustia | luau | 1x | 768 us | 5/5 | 2c28c4a | measured |
| 3 | rhai | rhai | 0.951x | 807.9 us | 5/5 | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not measured | not supported |

## wasm32-v8-deno-linux-x86_64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | commit | status |
|---:|---|---|---:|---:|---:|---|---|
| 1 | koto | koto | 1.649x | 514.6 us | 5/5 | 2c28c4a | measured |
| 2 | rhai | rhai | 1.044x | 813 us | 5/5 | 2c28c4a | measured |
| 3 | rustia | luau | 1x | 848.5 us | 5/5 | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not measured | not supported |

## wasm32-v8-node-linux-x64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | commit | status |
|---:|---|---|---:|---:|---:|---|---|
| 1 | koto | koto | 1.625x | 456.5 us | 5/5 | 2c28c4a | measured |
| 2 | rhai | rhai | 1.029x | 720.3 us | 5/5 | 2c28c4a | measured |
| 3 | rustia | luau | 1x | 741.5 us | 5/5 | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not measured | not supported |

