# Engine Adapter Plan

The standalone benchmark project tracks these engine slots from the start:

- `rustia`
- `boa`
- `koto`
- `luaur`
- `mlua-lua54`
- `mlua-luau`
- `rhai`
- `roto`
- `ruau`
- `rquickjs`
- `wasmi`
- `wasmtime`

Implemented adapters:

- `rustia`
- `boa`
- `koto`
- `luaur`
- `mlua-lua54`
- `mlua-luau`
- `rhai`
- `roto`
- `ruau`
- `rquickjs`
- `wasmi`
- `wasmtime`

All initially requested adapters are implemented.

Important dependency constraint: `mlua-lua54` and `mlua-luau` cannot be linked
into the same Rust binary because both `mlua-sys` builds use `links = "lua"`.
Run `mlua-lua54` separately with `--no-default-features --features mlua-lua54`.
The standalone OSS project should eventually handle those as separate adapter
crates or separate runner binaries that emit the same JSONL schema.

The feature flags keep heavyweight benchmark dependencies out of normal rustia
builds. `boa` and `rquickjs` are the JavaScript interpreter baselines, `koto` is
the direct migration comparison, `luaur` and `ruau` are pure-Rust Luau
implementations, `mlua-lua54` is the stock Lua 5.4 mlua comparison,
`mlua-luau` is the reference C++ Luau comparison, `rhai` tracks
another widely used Rust-native scripting baseline, `roto` is the statically
typed Cranelift-JIT scripting baseline, `wasmi` is the WASM interpreter
baseline, and `wasmtime` is the Wasmtime JIT baseline for the shared `.wat`
workload set.

The `wasmi` and `wasmtime` adapters are benchmark baselines for the shared `.wat`
workloads. They do not close rustia's browser-wasm pre-lock gate. That completed
gate is tracked separately in [`wasm-gate.json`](wasm-gate.json): the recurring
benchmark uses Node/V8, while accepted JavaScriptCore and SpiderMonkey results
remain retained as historical portability evidence.
