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

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | closures | dense | generic | sparse | stdlib math | commit | status |
|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| 1 | wasmtime | wasm | 90.5x | 1.045 us | 5/5 | 0.028 us | 0.458 us | 0.101 us | 0.452 us | 0.006 us | 2c28c4a | measured |
| 2 | wasmi | wasm | 14.87x | 6.363 us | 5/5 | 0.157 us | 2.827 us | 0.407 us | 2.951 us | 0.021 us | 2c28c4a | measured |
| 3 | mlua-luau | luau | 2.353x | 40.19 us | 5/5 | 0.053 us | 17.6 us | 3.827 us | 18.5 us | 0.216 us | 2c28c4a | measured |
| 4 | mlua-lua54 | lua54 | 1.488x | 63.57 us | 5/5 | 0.039 us | 39.09 us | 2.665 us | 21.51 us | 0.259 us | 2c28c4a | measured |
| 5 | rustia | luau | 1x | 94.6 us | 5/5 | 0.321 us | 35.81 us | 17.97 us | 39.98 us | 0.512 us | dcfb26a | measured |
| 6 | rquickjs | javascript | 0.497x | 190.4 us | 5/5 | 0.401 us | 102.2 us | 4.799 us | 82.48 us | 0.509 us | 2c28c4a | measured |
| 7 | koto | koto | 0.248x | 382.2 us | 5/5 | 0.285 us | 247 us | 5.742 us | 127.7 us | 1.407 us | 2c28c4a | measured |
| 8 | rhai | rhai | 0.159x | 594.2 us | 5/5 | 1.27 us | 292.9 us | 10.02 us | 287.3 us | 2.711 us | 2c28c4a | measured |
| 9 | boa | javascript | 0.085x | 1,119 us | 5/5 | 2.107 us | 766.3 us | 32.8 us | 314.5 us | 3.418 us | 2c28c4a | measured |
| 10 | roto | roto | 0.06x | 1,567 us | 5/5 | 0.005 us | 1,498 us | 1.561 us | 66.66 us | 0.183 us | 2c28c4a | measured |

## macos-aarch64

Native/host benchmark results. Total score is the sum of per-workload medians; lower is better.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | closures | dense | generic | sparse | stdlib math | commit | status |
|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| 1 | wasmtime | wasm | 455x | 0.6 us | 5/5 | 0.016 us | 0.194 us | 0.079 us | 0.307 us | 0.003 us | e48285b | measured |
| 2 | wasmi | wasm | 93.15x | 2.928 us | 5/5 | 0.09 us | 1.28 us | 0.163 us | 1.388 us | 0.007 us | e48285b | measured |
| 3 | mlua-luau | luau | 12.67x | 21.53 us | 5/5 | 0.022 us | 9.898 us | 2.133 us | 9.387 us | 0.091 us | e48285b | measured |
| 4 | mlua-lua54 | lua54 | 5.865x | 46.5 us | 5/5 | 0.018 us | 30.91 us | 1.433 us | 14.01 us | 0.137 us | d4bb1ed | measured |
| 5 | rquickjs | javascript | 2.458x | 111 us | 5/5 | 0.253 us | 58.06 us | 2.947 us | 49.48 us | 0.233 us | e48285b | measured |
| 6 | koto | koto | 1.721x | 158.5 us | 5/5 | 0.123 us | 100.4 us | 2.345 us | 55.04 us | 0.641 us | e48285b | measured |
| 7 | rhai | rhai | 1.18x | 231.1 us | 5/5 | 0.527 us | 108.2 us | 3.491 us | 117.8 us | 1.052 us | e48285b | measured |
| 8 | rustia | luau | 1x | 272.8 us | 5/5 | 0.429 us | 136.9 us | 17.76 us | 116.4 us | 1.276 us | e48285b | measured |
| 9 | boa | javascript | 0.504x | 540.8 us | 5/5 | 1.018 us | 381.7 us | 14.42 us | 141.9 us | 1.78 us | e48285b | measured |
| 10 | roto | roto | 0.214x | 1,272 us | 5/5 | 0.004 us | 1,219 us | 1.16 us | 51.95 us | 0.141 us | 9f3b0d1 | measured |

## Wasm Targets

## wasm32-javascriptcore-bun-linux-x64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | closures | dense | generic | sparse | stdlib math | commit | status |
|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| 1 | koto | koto | 1.882x | 436.9 us | 5/5 | 0.358 us | 282.6 us | 6.98 us | 145.4 us | 1.547 us | 2c28c4a | measured |
| 2 | rhai | rhai | 1.148x | 716.1 us | 5/5 | 1.519 us | 366.1 us | 10.56 us | 334.4 us | 3.548 us | 2c28c4a | measured |
| 3 | rustia | luau | 1x | 822.3 us | 5/5 | 1.655 us | 440.7 us | 68.8 us | 307.3 us | 3.8 us | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |

## wasm32-spidermonkey-spidermonkey-unknown-os-x64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | closures | dense | generic | sparse | stdlib math | commit | status |
|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| 1 | koto | koto | 1.659x | 463 us | 5/5 | 0.395 us | 280.6 us | 7.814 us | 172.4 us | 1.706 us | 2c28c4a | measured |
| 2 | rustia | luau | 1x | 768 us | 5/5 | 1.343 us | 401.7 us | 57.74 us | 303.7 us | 3.492 us | 2c28c4a | measured |
| 3 | rhai | rhai | 0.951x | 807.9 us | 5/5 | 1.654 us | 409 us | 10.54 us | 382.5 us | 4.168 us | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |

## wasm32-v8-deno-linux-x86_64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | closures | dense | generic | sparse | stdlib math | commit | status |
|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| 1 | koto | koto | 1.649x | 514.6 us | 5/5 | 0.406 us | 315.5 us | 7.873 us | 189 us | 1.786 us | 2c28c4a | measured |
| 2 | rhai | rhai | 1.044x | 813 us | 5/5 | 1.681 us | 414.9 us | 11.01 us | 381.3 us | 4.146 us | 2c28c4a | measured |
| 3 | rustia | luau | 1x | 848.5 us | 5/5 | 1.418 us | 414.3 us | 59.55 us | 369.8 us | 3.437 us | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |

## wasm32-v8-node-linux-x64

Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.

| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | closures | dense | generic | sparse | stdlib math | commit | status |
|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|---|
| 1 | koto | koto | 1.625x | 456.5 us | 5/5 | 0.358 us | 300.7 us | 7.184 us | 146.6 us | 1.669 us | 2c28c4a | measured |
| 2 | rhai | rhai | 1.029x | 720.3 us | 5/5 | 1.378 us | 363 us | 11.03 us | 341.2 us | 3.677 us | 2c28c4a | measured |
| 3 | rustia | luau | 1x | 741.5 us | 5/5 | 1.142 us | 381.1 us | 49.9 us | 306.4 us | 2.934 us | 2c28c4a | measured |
| - | mlua-luau | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | mlua-lua54 | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | roto | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | rquickjs | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | boa | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmi | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |
| - | wasmtime | not supported | not supported | not supported | 0/5 | not supported | not supported | not supported | not supported | not supported | not measured | not supported |

