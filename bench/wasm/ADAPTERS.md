# Wasm Target Adapter Status

The retained Wasm benchmark protocol covers adapters that compile to
`wasm32-unknown-unknown` and execute the default workload set inside JS
`WebAssembly` runtimes.

Adapters measured in the recorded cohort:

- `rustia` - production rustia crate compiled to wasm.
- `koto` - pure Rust Koto adapter compiled to wasm.
- `rhai` - Rhai adapter compiled to wasm with deterministic hashing features.

The last recurring cohort used Node/V8. Public benchmark measurement is paused.
The accepted Deno/V8, Bun/JavaScriptCore-family and SpiderMonkey `js` shell
measurements are preserved in [history](history/) as portability evidence.

The rustia wasm32 quota/recovery check also executes a 300-decision reset
lifecycle. It verifies reusable compiled modules, restored host bindings,
cleared decision inputs, deterministic RNG restart, epoch-stale handles, and an
exact post-warmup VM-owned-memory plateau inside Node/V8.

Adapters excluded from that cohort:

- `boa` - compile probe reaches `getrandom`'s wasm support gate.
- `luaur` - native adapter added; wasm adapter compatibility has not yet been
  qualified under the benchmark gate.
- `mlua-lua54` - `lua-src` does not build Lua for `wasm32-unknown-unknown`.
- `mlua-luau` - Luau C++ build lacks the target C++ standard library for this
  target in the current toolchain.
- `roto` - Cranelift/region virtual-memory dependencies are native-only here.
- `ruau` - native adapter added; wasm adapter compatibility has not yet been
  qualified under the benchmark gate.
- `rquickjs` - QuickJS C build lacks the target C standard library/bindings.
- `wasmi` - compiles to wasm, but running the default workload set as
  wasm-in-wasm overflows the JS/Wasm stack in the current runtimes.
- `wasmtime` - native virtual-memory/JIT dependencies do not compile for
  `wasm32-unknown-unknown`.

These exclusions describe the toolchain and workloads used for that cohort.
