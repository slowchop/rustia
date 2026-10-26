# rustia-cli Source

## Overview

[main.rs](./main.rs) owns command-line argument parsing, source acquisition, VM configuration, module path resolution, single-run execution, REPL behavior, and user-facing exit/error handling.

## Interface

- `main()` maps help, version, usage, I/O, and Rustia outcomes to output and exit status.
- `run_cli()` selects a source run or persistent REPL.
- `read_source()` handles a file, inline source, stdin, and invalid missing input.
- `configured_rustia()` applies profile, optional fuel, stdout, and the non-deterministic filesystem resolver.
- `cli_module_candidates()` orders extension and `init` candidates.

## Invariants

- The CLI does not install filesystem resolution for `DeterministicSim`.
- Extensionless module lookup checks `.luau`, `.lua`, `init.luau`, then `init.lua`.
- A shebang line is replaced with spaces, preserving source offsets and line breaks.
- REPL commands share one VM; separate script invocation constructs one VM.

## Side Effects

Filesystem and stdin reads, canonical path resolution, module reads, stdout/stderr writes, and REPL prompts are direct process effects.

## Verification

From the repository root, run `cargo test --locked -p rustia-cli`.
