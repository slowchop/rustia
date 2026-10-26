# Wasm Adapter ABI

## Overview

This Rust library implements the wasm-callable adapter ABI for Rustia, Koto, Rhai, and Wasmi. The owning crate is documented in [`../EGM.md`](../EGM.md).

## Interface

- Exported functions include `bench_alloc`, `bench_dealloc`, `bench_abi_version`, `bench_adapter_mask`, `bench_prepare`, `bench_prepare_rustia_limited`, `bench_run`, and result accessors ([`lib.rs`](lib.rs#L93)). These are internal wasm ABI symbols, not Rustia's public embedding API.
- `bench_prepare` validates the input bytes as UTF-8, prepares selected engine state, and returns status; `bench_run` reports a structured last-result message and returns NaN when no workload is prepared or execution fails ([`lib.rs`](lib.rs#L122)).

## Invariants

- `ci-bench` alone is authorized to produce timing evidence; this ABI library does not grant local timing authority ([repository policy](../../../../AGENTS.md#benchmark-policy)).
- ABI version is `1`; the adapter mask represents Rustia, Koto, Wasmi, and Rhai ([`lib.rs`](lib.rs#L9)).
- `bench_dealloc` requires the pointer and capacity returned by `bench_alloc`; input pointers passed to preparation must remain valid for the call ([`lib.rs`](lib.rs#L101)).
- Adapter state and last-result bytes are held in thread-local `RefCell`s ([`lib.rs`](lib.rs#L14)).

## Side Effects

- Calls allocate/deallocate buffers, retain prepared engine state, and execute guest workloads. Result text is stored for retrieval by the driver.

## Verification

Run from the repository root: `cargo check --locked --manifest-path bench/wasm/adapter-runner/Cargo.toml` checks the crate without a wasm execution or timing run.
