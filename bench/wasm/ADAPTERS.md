# Wasm Target Adapter Status

The wasm target bench measures adapters that both compile to
`wasm32-unknown-unknown` and execute the default workload set inside JS
`WebAssembly` runtimes.

Currently measured:

- `rustia` - production rustia crate compiled to wasm.
- `koto` - pure Rust Koto adapter compiled to wasm.
- `rhai` - Rhai adapter compiled to wasm with deterministic hashing features.

The active recurring runtime is Node/V8. The accepted Deno/V8,
Bun/JavaScriptCore-family, and SpiderMonkey `js` shell measurements are kept in
[`history/`](history/) as portability evidence instead of being repeated in
every public benchmark run.

Not currently measured:

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

Those exclusions are not permanent product decisions. They are the current
evidence-backed wasm target status for this benchmark harness.
