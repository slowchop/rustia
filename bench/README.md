# script-bench-harness

`script-bench-harness` is a standalone benchmark harness for embedded scripting
engines. It lives in the rustia repo while rustia is early, but it is structured
to split into its own OSS project with a website, published result history, and
stats charts.

The harness writes one JSONL record per completed engine/workload measurement,
so history can be committed, served statically, or uploaded by CI without needing
a hosted metrics service.

The default build runs the in-repo `rustia` adapter over five workload families:

- `dense_tables`
- `sparse_tables`
- `closures_upvalues`
- `generic_iteration`
- `stdlib_math_table`

```sh
cargo run -p script-bench-harness --release -- --history bench/history/runs.jsonl
```

List all configured engine slots:

```sh
cargo run -p script-bench-harness -- --list
```

Engine slots are tracked for `rustia`, `boa`, `koto`, `mlua-lua54`,
`mlua-luau`, `rhai`, `roto`, `rquickjs`, `wasmi`, and `wasmtime`. Adapters are
feature-gated so heavyweight dependencies do not affect the normal workspace
build.

The matrix switch is present now:

```sh
cargo run -p script-bench-harness --features all-engines -- --list
```

In this first slice, `rustia` works through the default `rustia-engine` feature
and `koto` works with `--features koto`. The crate also builds with
`--no-default-features` so the standalone harness shell is not tied to rustia
internals. The other adapters intentionally report `adapter pending`; see
`ADAPTERS.md` for the dependency split required before wiring them in.

## History Schema

Each line in `history/runs.jsonl` is a JSON object:

```json
{
  "timestamp_unix": 0,
  "commit": "8c0211d",
  "target": "macos-aarch64",
  "engine": "rustia",
  "dialect": "luau",
  "workload": "dense_tables",
  "unit": "build+sum+sort of 256-elem array",
  "n": 1000,
  "reps": 9,
  "ns_per_unit_min": 1.0,
  "ns_per_unit_median": 1.1,
  "units_per_sec": 1000000.0,
  "checksum": "number:123"
}
```

The future website should treat JSONL as the append-only source of truth and
generate charts from `engine × target × workload` series.
