# Prior Art

`script-bench-harness` is a clean rewrite for rustia's embedded scripting
benchmark work. It does not vendor code, workloads, or benchmark source from
third-party benchmark projects.

## khvzak/script-bench-rs

`khvzak/script-bench-rs` is prior art for comparing Rust embedded scripting
engines, especially Rust-to-script interop workloads. rustia credits that
project for the comparable embedded-engine benchmarking direction.

The license review recorded in `design/80-bench-suite.md` found no `LICENSE`
file and no Cargo license metadata for that repository at the time of review,
so this harness intentionally avoids copying, adapting, or vendoring its code.
A future comparison job may run upstream `script-bench-rs` unmodified at a
pinned revision as an external datapoint.

## rustia bench-suite spikes

The workload families in this crate descend from rustia's in-repo
`spikes/bench-suite/` research and have been rewritten into per-engine dialects
under `bench/workloads/`.

## Policy

Any workload copied or translated from an external project must have explicit
license and provenance recorded here before it enters `bench/workloads/`.
