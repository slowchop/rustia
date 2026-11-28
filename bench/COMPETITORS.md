# Competitor platform checks

## Platform execution

This is correctness evidence from 2026-11-27 for the five added adapters and
standalone Lune and RustScript CLI runs. A runtime result requires a passing guest checksum
suite. A cross-check only establishes that Rust type-checked the test sources
for that target. A simulator or emulator result identifies that exact runtime.

| Engine | Linux ARM64 | macOS ARM64 | Web | Windows x86-64 | iOS ARM64 | Android ARM64 |
| --- | --- | --- | --- | --- | --- | --- |
| luau-rs | Executed | Executed | Executed, WebKit/Chrome/Firefox | Cross-check | Executed, simulator; device target cross-check | Executed, emulator |
| ulua | Executed | Executed | Disabled after a VM initialization crash | Cross-check | Executed, simulator; device target cross-check | Executed, emulator |
| Piccolo | Executed | Executed | Executed, WebKit/Chrome/Firefox | Cross-check | Executed, simulator; device target cross-check | Executed, emulator |
| Rune | Executed | Executed | Executed, WebKit/Chrome/Firefox | Cross-check | Executed, simulator; device target cross-check | Executed, emulator |
| Starlark | Executed | Executed | Executed, WebKit/Chrome/Firefox | Cross-check stopped at the missing MSVC assembler | Executed, simulator; device target cross-check | Executed, emulator |
| Lune 0.11.1 CLI | Executed | Executed | Unverified | Upstream release | Unverified | Unverified |
| RustScript 0.31.3 CLI | Smoke test, emulated x86-64 | Smoke test | Upstream playground | Upstream release | Unverified | Unverified |

Linux used a native ARM64 container. The wider native checksum suite passed all
18 configurations on macOS ARM64. On Linux ARM64, 17 passed. Luaur 0.1.8
failed to compile because its parser mixes signed and unsigned C character types
on that target. Browser execution used WebKit 27.2 and
Chrome 158.0.8127.38 under Playwright 1.64.1, and Firefox 161.0 with geckodriver
0.37.1. The browser harness used wasm-bindgen-test-runner 0.2.126. Android used
Android 15/API 35 with NDK 28.2.13676358 and hardware acceleration. No physical
mobile device executed these competitor workloads. Rust was pinned at 1.95.0.

A follow-up manual mobile suite passed on both the ARM64 iOS simulator and
Android 15 ARM64 emulator for Rustia, Boa, Koto, mlua Luau interpreter, Rhai,
Ruau, and Wasmi, along with the five adapters in the table. Android used
Boa's `jsvalue-enum` feature and the NDK's `libc++_shared.so` for mlua.
Luaur did not compile for the iOS simulator. rquickjs ships no bindings for
either tested mobile target with the harness's enabled features.

The existing Wasm library also executed closure or state-update checks in
headless WebKit 27.2 and Chromium 159 for Rustia, Koto, Wasmi, and Rhai. The
probe called each guest entry twice at `n = 3` and matched the expected value.

The released Lune 0.11.1 ARM64 executables passed all five Luau checksum
fixtures on macOS and on Ubuntu 24.04. The CLI test required each fixture as a
module, called the returned function twice with `n = 3`, and checked both
results against the same reference values as `check_luau`. It used no adapter.

The four-engine browser build without Starlark passed all checks. In the
five-engine browser build, luau-rs, Piccolo, Rune and Starlark passed in all
three browsers. ulua failed on VM initialization in all three with a misaligned
pointer in `ulua-vm` 0.1.7. Its default allocator requests alignment 1, while
the failing state access requires alignment 16 in this Wasm build. The harness disables ulua on browser Wasm until the pinned dependency guarantees
the required alignment. See the [upstream allocator](https://docs.rs/crate/ulua-vm/0.1.7/source/src/functions/l_alloc.rs).

The RustScript `pd-vm` 0.31.3 probe failed while compiling its published
`pd-host-function` 0.22.7 integration. The procedural macro rejected
`GuestResult` and `HostStateMut` in the VM's runtime builtins. This is a
reproduced build failure for that dependency combination. The separately
released `pd-vm-run` 0.31.3 passed a native RSS smoke script on macOS ARM64 and
emulated Ubuntu 24.04 x86-64. The script called the same captured closure twice
and checked an indexed array sum. The full comparison fixtures remain pending.
[RustScript 0.31.3 release](https://github.com/rustscript-lang/rustscript/releases/tag/0.31.3)

## Standalone platform probes

These commands and demos establish execution of the stated guest checks. They
are separate from the harness and provide no measurements.

| Engine | Executed environment | Check |
| --- | --- | --- |
| Steel 0.8.2 | macOS and Linux ARM64 | Released interpreter ran a closure with a captured value and returned 42. [Release](https://github.com/mattwparas/steel/releases/tag/v0.8.2) |
| Dyon 0.51.3 | macOS ARM64 | The published `dyonrun` evaluated a loop and printed 42. [CLI documentation](https://docs.rs/dyon/0.51.3/dyon/) |
| Duckscript 0.11.1 | Emulated Linux x86-64 | The released `duck` binary executed a variable assignment and printed it. [Release](https://github.com/sagiegurari/duckscript/releases/tag/0.11.1) |
| Gluon 0.18.4 | macOS ARM64 | The embedded VM ran a function application and returned 42. The 0.18.2 build failed with the dependency versions selected for that probe. [Embedding API](https://docs.rs/gluon/0.18.4/gluon/trait.ThreadExt.html) |
| RustPython hosted demo | Web, headless WebKit 27.2 | `rp.pyEval` executed a numeric Python expression and returned 42. This used the deployed demo's Wasm build. [Demo](https://rustpython.github.io/demo/) |
| Luna 4.0.2 | macOS ARM64 | The interpreter executed all five Lua 5.4 fixtures twice at `n = 3` and matched their checksums, using Rust 1.99.1. [Interpreter API](https://docs.rs/luna-core/4.0.2/luna_core/) |
| Mun 0.4.0 | Emulated Linux x86-64 | The prerelease compiler built a small library, then the CLI invoked its entry point and returned 42. [Release](https://github.com/mun-lang/mun/releases/tag/v0.4.0) |
