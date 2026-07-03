# Engine Adapter Plan

The standalone benchmark project tracks these engine slots from the start:

- `rustia`
- `boa`
- `koto`
- `mlua-lua54`
- `mlua-luau`
- `rhai`
- `roto`
- `rquickjs`
- `wasmi`
- `wasmtime`

Implemented adapters:

- `rustia`
- `boa`
- `koto`
- `mlua-lua54`
- `mlua-luau`
- `rhai`
- `roto`
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
the direct migration comparison, `mlua-lua54` is the stock Lua 5.4 mlua
comparison, `mlua-luau` is the Luau 0.709 oracle comparison, `rhai` tracks
another widely used Rust-native scripting baseline, `roto` is the statically
typed Cranelift-JIT scripting baseline, `wasmi` is the WASM interpreter
baseline, and `wasmtime` is the Wasmtime JIT baseline for the shared `.wat`
workload set.
