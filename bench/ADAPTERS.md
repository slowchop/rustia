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
- `koto`
- `mlua-luau`

The remaining adapters are pending.

Important dependency constraint: `mlua-lua54` and `mlua-luau` cannot be linked
into the same Rust binary because both `mlua-sys` builds use `links = "lua"`.
The standalone OSS project should handle those as separate adapter crates or
separate runner binaries that emit the same JSONL schema.

Most current feature flags are intentionally dependency-free placeholders. They
let the CLI expose the public engine matrix without forcing normal rustia builds
to download heavyweight benchmark dependencies. `koto` is the first optional
comparison engine wired into the harness, and `mlua-luau` is the Luau 0.709
oracle comparison.
