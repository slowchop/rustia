# rustia — pinned Luau target (the conformance anchor)

> **Status: [PINNED] (2026-06-30).** Resolves review finding **C1**. rustia targets a *specific* Luau version,
> not "latest," so "full parity" is a stable, testable claim. Chosen to equal our local conformance oracle, so
> **spec == oracle** — which is what kills the C2 conformance-table drift. Authoritative; see [`CURRENT.md`](CURRENT.md).

## The pin
- **Luau release `0.709`**, commit **`b968ef742741bb2b703afc3b3c53f06608c87481`** (released 2026-02-21).
- **Why 0.709:** it is *bit-identical* to what our local oracle builds — `mlua 0.11.6` → `mlua-sys 0.10.0` →
  `luau0-src 0.18.3+luau709` (checksum `e73f7d752fe2ef0a41f132b83d0be154c6ae0ed84967b4df80a16901866be075`).
  Pinning the spec to the oracle means every conformance row can be mechanically verified against a running
  binary. The oracle and optional benchmark adapters use mlua. Rustia itself implements the interpreter in Rust.

## Surface
- **Standalone luau.org language + stdlib** as registered in `linit.cpp` at 0.709: `base` globals, `coroutine`,
  `table`, `os`, `string`, `math`, `debug`, `utf8`, `bit32`, `buffer`, `vector`.
- **Explicitly NOT the Roblox engine API**: no `game`/`workspace`/`script`, no `Instance`/`DataModel`, no Roblox
  task scheduler or Roblox `typeof` extensions. `require()`/module resolution is host policy, not part of the
  conformance surface.

## Configuration
- **Vector width = 3** script-visible `f32` lanes (`LUA_VECTOR_SIZE` default 3; mlua `luau-vector4` NOT enabled).
- **FFlags = compiled defaults, override nothing.** The only three script-visible-semantics FFlags referenced
  in the 0.709 VM all default **false**, and neither the luau CLI nor mlua flips them, so oracle == flags-off:
  `LuauStacklessPcall` (pcall/xpcall + yield/error edges), `UseNewTraceback` (traceback string format),
  `LuauMathSeedEncode` (default seed of *unseeded* `math.random` — irrelevant; we always seed explicitly).
  These are the VM defaults. One imported **parser** fixture uses a different configuration:
  the [upstream test harness](../conformance/upstream/luau-0.709/tests/Conformance.test.cpp)
  enables `LuauExplicitTypeInstantiationSyntax`. Rustia accepts this syntax on calls and value expressions by default. The shared parser comparison enables
  the same flag in the pinned compiler. The [case inventory](../conformance/upstream/luau-0.709/parser-cases.tsv)
  maps each upstream parser test to executable evidence or a reason it is outside the standalone target.
  The [lexer inventory](../conformance/upstream/luau-0.709/lexer-cases.tsv) records which token boundaries,
  decoded values and first diagnostics Rustia exposes for each of the 28 upstream lexer cases.
  The [compiler inventory](../conformance/upstream/luau-0.709/compiler-cases.tsv) maps all 200 declarations
  to guest behavior or an explicit exclusion. Luau can reuse equivalent closures at optimization levels 1 and 2.
  Rustia creates a fresh function on each evaluation. Using those functions as table keys can distinguish
  the engines. Luau describes the reuse heuristic as optional in its [closure caching documentation](https://luau.org/performance/#closure-caching).

## Numbers are f64-only (verified, not assumed)
At 0.709, **`f64` (IEEE-754 double) is the only script-level number type.** Oracle-proven:
`type(3)==type(3.0)==type(7//2)==type(0x10)==type(bit32.bor(1,2))==type(math.floor(3.7))=="number"`;
`math.type==nil` (Lua 5.3's int/float discriminator is deliberately absent); `2^53+1` prints `9007199254740992`
and `2^63` prints `9223372036854776000` (rounds to nearest double — impossible if a script-visible int64
existed). The reviewer correctly spotted `int64`/`LUA_TINTEGER`/`LBC_CONSTANT_INTEGER` in *current* (0.727)
source, but they are **default-OFF**, gated behind **`FFlag::LuauIntegerType2 = false`** + a new `i`-suffix
literal + bytecode v8 (default compile target is v7) — so even at 0.727 with default flags, `type(3)=="number"`.
Three things sometimes conflated as "integers" all stay pure f64 at script level: `LOP_LOADN`'s 16-bit immediate
(codegen; executes `setnvalue` → double), the `lua_Integer`/`lua_Unsigned` C-API typedefs (boundary conversion
only), and `bit32`/`buffer` integer semantics (operate on doubles reinterpreted as `uint32`, read back as
doubles). **Conclusion: the "f64-only" design decision is CORRECT for the pinned target.**

## Re-pin policy + tripwire
- **Re-pin only when mlua bumps its vendored Luau**, and re-run the full conformance suite at each bump.
- **Tripwire:** watch upstream `FFlag::LuauIntegerType2` (and the bytecode-v8 default-target flip). If integers
  become script-visible by default in a version we adopt, revisit the f64-only decision and the number model.
- **Secondary oracle:** build the standalone `luau` CLI from `luau-lang/luau` at tag 0.709 via cmake, to confirm
  mlua doesn't perturb defaults (notably: `Lua::new()` does *not* sandbox by default, so `getfenv`/`setfenv`/
  `table.maxn` presence is from unconditional stdlib registration — confirm under `luaL_sandbox` too).
