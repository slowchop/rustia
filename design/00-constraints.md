# rustia — design constraints (the contract every decision must satisfy)

> **Status: [FOUNDATION] (2026-06-30).** Grounded directly in `../grabby-aliens` source + design docs
> and gak's stated goal. This is the fixed target the design-exploration phase is scored against. It does
> NOT depend on the in-flight research workflow; the landscape/technique findings land in `10-landscape.md`.

## What rustia is

A from-scratch **Luau interpreter in Rust**, with JIT and other state-of-the-art (and better) execution
methods. The underlying goal: **replace koto** as the scripting layer in `../grabby-aliens/` (a
deterministic game sim). Net-new project; blank slate.

## Priorities (gak's stated order — ties break upward)

1. **FAST** — highest priority. At least as fast as koto and other wasm-compatible Rust scripting crates
   (piccolo, rhai, rune); ideally far faster. **JIT is a means, not a goal (gak, 2026-06-30): if a
   non-JIT approach is faster, take it.** This elevates interpreter-tier techniques (superinstructions,
   copy-and-patch, AOT-script→wasm, threaded dispatch) to first-class contenders — especially since
   JIT-to-native is unavailable in the browser anyway (see priority 3).
2. **Low memory.**
3. **Performant inside a Rust `wasm32-unknown-unknown` target** (browser/mobile mod path). Note: JIT to
   native machine code is unavailable in-browser, so wasm performance must come from the interpreter
   tier (and possibly wasm-emitting strategies — see research).
4. **Fully compatible with pinned Luau 0.709** (standalone luau.org surface, not Roblox) — see
   [`05-luau-target.md`](05-luau-target.md). ("latest Luau" was unstable; now pinned to the conformance oracle.)
5. **Excellent embedding API**, especially from Rust.

## Hard constraints inherited from grabby-aliens (non-negotiable)

These come from reading `ai_bakeoff/`, `scenario/src/lib.rs`, `view/src/console.rs`, and
`design/console-scripting-plan.md`. They are ADR-locked in grabby (determinism is ADR-001).

### C0 — Target & test matrix (gak, 2026-06-30: "Mac, Intel, arm, etc." + Firefox)
Determinism + correctness must hold across this FULL matrix (the cross-arch golden runs on all of it in CI):

| native (OS × arch) | browser/wasm engine |
|---|---|
| macOS **arm64** (Apple Silicon — dev machine) ✅ | **V8** — Chrome/Edge/Node/Deno ✅ measured |
| macOS **x86_64** (Intel Mac) — TODO | **SpiderMonkey** — **Firefox** — TODO (gak-requested) |
| Linux **x86_64** ✅ (Spike C, via docker) | **JavaScriptCore** — Safari + iOS/Android WKWebView — TODO |
| Linux **arm64** — TODO | |
| Windows **x86_64** (msvc) — TODO | |
| iOS **arm64** (native) + Android **arm64** (native) — TODO | |

Measured so far: the triple x86_64-linux + aarch64-mac + wasm32-in-V8 is bit-identical. The remaining native
arches (Intel Mac, Windows, ARM-Linux) and the two other browser engines (Firefox/SpiderMonkey, Safari/JSC)
are the validation gate at P1 — they could in principle re-open the value-layout choice (axis B), though the
libm + canonicalization discipline is arch-portable by construction.

### C1 — Determinism, bit-for-bit, across the full matrix above
- Same script + same seed/inputs → **identical bytes** on every native arch AND every browser engine in C0.
- **Maps are insertion-ordered** (koto guarantees this; grabby relies on it).
- Today grabby enforces a **"float-free boundary"**: the host↔script boundary passes only integers /
  **Q16.16 fixed-point**; all `f64` math lives in Rust. Enforced by (a) gating float ops out of the
  sim-side prelude and (b) a standing **cross-arch golden test** that runs a script and hashes the result.
- ✅ **RESOLVED (T1):** **Luau numbers are `f64`-only** (verified script-visible-f64 for pinned 0.709;
  `math.type==nil` — see [`05-luau-target.md`](05-luau-target.md)). Resolution: f64 internal + pure-Rust libm +
  per-op NaN canonicalization (bit-identical cross-arch, Spike C) + `bit32`/`buffer` as the exact-integer channel
  + grabby's Q16.16 multiply kept as a host fn. **No fixed-point primitive in the core Value.** The "integer
  subtype" is now a **tripwire**, not a tension: revisit only if upstream `FFlag::LuauIntegerType2` flips
  default-true in a version we adopt.

### C2 — `Send + Sync`
- Values and the VM must be `Send + Sync` (koto's non-default `arc` feature; grabby builds with it).
- Execution model is **N independent VMs across a thread pool**, NOT one shared VM. Per-borrow tax of the
  Arc path was ~5–10% in koto — budget for, and try to beat, that.

### C3 — Steppable VM with deterministic limits
- **Fuel** (instruction budget, monotonic — NOT wall-clock) and a **recursion/stack-depth cap**, both
  deterministic. koto exposes `with_execution_limit(Duration)`; grabby wants a deterministic fuel budget
  instead. rustia should make fuel + recursion limits first-class and cheap.

### C4 — serde bridge
- Deserialize script values into Rust structs: grabby does
  `koto::serde::from_koto_value(koto.exports().clone()) -> ScenarioSpec`. rustia needs an equivalent
  `serde::Deserialize` path from a script value (and ideally `Serialize` the other way).

## API surface to match (koto 0.16.1, as used in grabby)

The equivalent rustia API must cover at least:

| koto call | what it does | rustia must offer |
|---|---|---|
| `Koto::default()` / `Koto::with_settings(KotoSettings::default().with_execution_limit(d))` | construct VM, set limits | a VM handle + settings (fuel/recursion/limits) |
| `koto.prelude().add_fn("name", \|ctx\| { match ctx.args() { … } })` | register a native Rust fn; arg pattern-match | native-fn registration with ergonomic arg extraction |
| `koto.prelude().insert("name", value)` | inject a host value/constant | inject host values into the global env |
| `koto.compile_and_run(script)` -> `Result` | compile + execute a chunk | compile + run, with a useful error type |
| `koto.exports().get("name")` / `.clone()` | read a script-exported binding | read exported globals (`export {…}` ⇒ exports map) |
| call an exported function with `&[KValue]` args | invoke script fn from Rust | call a script function from Rust with native args |
| `KValue::Number(KNumber::I64 \| F64)`, maps, lists, strings | the value type | a `Value` type with the same shape (mind T1 re: numbers) |
| `koto::serde::from_koto_value(…)` | serde deserialize | serde bridge (see C4) |

Insertion-ordered map literals via `export { k: v, … }` must round-trip through serde deterministically.

## Performance bar / the prize

- grabby's Phase-0 bake-off **measured koto at ~4–5 µs/decision (~30–40× the ~0.14 µs Rust-native floor)**;
  removing all host calls saved only ~26%, so the cost is the **interpreted VM itself**, not the FFI
  boundary. That forced the per-agent-per-tick AI hot-path OUT of script into hand-written Rust.
- **Target:** thousands of agents @ 50 Hz, deterministic, within a ≲5 ms AI budget. koto blew that bar at
  ~900–1250 agents.
- **The architectural prize:** if rustia gets within a *small multiple* of native (not 30–40×), the
  scripted per-agent AI hot-path becomes viable again — that, not merely matching koto, is the real win.
- We must be able to **re-run grabby's bake-off methodology** against rustia (port `ai_bakeoff`'s
  `KotoSelector` to a `RustiaSelector`) to prove the number.

## DX requirements

- **Hot-reload = re-eval** (no separate compile/build step in the loop); fast compile of scripts.
- The Rust **embedding API** is a first-class deciding axis (compile time, API surface, ergonomics).
- Luau compatibility buys us the existing **luau-lsp / IDE tooling** for free — a stated DX win; preserve it
  (i.e. don't diverge from Luau syntax/semantics in ways that break the LSP).

## Scoring rubric for design-exploration (apply to every candidate direction)

Each candidate direction at each axis is scored against:
**(1) native speed · (2) wasm32 speed · (3) memory · (4) determinism (incl. cross-arch + T1) ·
(5) Luau compatibility · (6) Send+Sync fit · (7) embedding-API/DX · (8) implementation complexity/risk.**
Speed dominates ties, then determinism (it's ADR-locked — a fast-but-nondeterministic option is
disqualified for the sim path), then wasm, then the rest.
