# CLAUDE.md — how to work in rustia (read before writing code)

rustia is a from-scratch, **deterministic** Luau interpreter in Rust (replaces koto in grabby; public-bound at
full Luau parity). **This is an LLM-first codebase** — written and maintained mostly by agents, read and
contributed to by humans. Optimise for *both*: code an agent can reason about locally and a human can trust.

**Start every session here:** read [`design/CURRENT.md`](design/CURRENT.md) (the single canonical state), then
the relevant [`design/`](design/) docs. The design docs + the pinned Luau **0.709** oracle are the SPEC; code
serves them.

## The 10 principles (follow these; CI enforces the mechanical ones)

1. **The design docs + the oracle are the spec — cite them, keep them in sync.** Ground decisions in
   `design/` + the conformance table + the pinned-Luau-0.709 oracle, never in vibes. A behavioural choice links
   to its design doc / conformance row / oracle test. When you change behaviour, update the code, the design
   doc, the conformance row, and the test **in the same change** — never let them drift.

2. **Determinism is the prime directive — by construction, enforced mechanically.** libm-only transcendentals
   (clippy `disallowed-methods`); deny FMA / relaxed-SIMD / fast-math at the build; seedless hashing only
   (never `RandomState`/`DefaultHasher`); instruction-fuel, never wall-clock; per-op NaN/`-0` canonicalisation;
   insertion-ordered maps. Every nondeterminism hazard is gated by the cross-arch golden and (for JIT'd code)
   the JIT≡interpreter equivalence oracle. A lint/test that *can't* be forgotten beats a sentence in a doc.

3. **Measure, don't assert — perf is a number or it didn't happen.** No "this should be faster." Perf claims
   come from the benchmark suite (a CI gate vs reference Luau + koto, per-workload). Add a tier/optimisation
   (JIT, inline cache, …) only when a measurement demands it. "Done" means tested + measured, not hoped.

4. **KISS — one core, earn every abstraction.** One portable engine (no per-target forks); no bespoke ops in
   the public core; no new layer/tier/trait until a measurement or a second real caller justifies it. Prefer
   the simplest thing that passes the golden + the bench. Strangler: small, always-green steps.

5. **Mechanical enforcement over good intentions (the LLM-first lever).** rustfmt, `clippy -D warnings`, the
   determinism lints, the golden test, the bench gate, the parser fuzz-vs-`full_moon`, and `miri` on every
   `unsafe` — all in CI. If a rule matters, make it a check, not a comment. Agents forget; CI doesn't.

6. **Local reasoning — explicit state, no spooky action at a distance.** No hidden globals / thread-locals /
   statics that affect behaviour; pass VM state explicitly. Host effects go through the typed effect-queue,
   never `&World` aliasing. A function must be understandable from its signature + body — that's how both
   agents and humans reason, and it's what makes the Send-not-Sync, one-VM-per-lane model sound.

7. **Greppable structure, honest names.** One concept per file; small self-contained modules named for what
   they hold; the module tree mirrors the design's crate layout; no mega-files, deep nesting, or
   definition-hiding macros. Names map to the design vocabulary and Luau/Lua terms (so cross-reference is
   trivial). Rust idiom: `snake_case` fns, `CamelCase` types, `SCREAMING_SNAKE` consts; opcodes and `Value`
   variants named consistently. A name should tell the truth about cost and effects (`get` is cheap; `compile`
   is not).

8. **Every `unsafe` and every invariant is documented and asserted.** A `// SAFETY:` comment on every `unsafe`
   block stating the invariant it relies on (especially the gc-arena `Send` wrapper and the `'gc` lifetime
   rules); `debug_assert!` invariants where cheap; soundness-critical ones get `miri`/`loom` in CI. The two
   profiles only *restrict* — a test asserts anything that runs under `DeterministicSim` produces identical
   results under `ReproducibleStandard`.

9. **Never panic on guest input — errors are structured Results.** Script/compile errors → a typed
   `Result`/`ErrorKind` with source span + the byte-exact Luau message. `panic!`/`unwrap` are reserved for
   *engine* bugs (a broken internal invariant), never for anything a script or embedder can trigger. Fuel and
   recursion exhaustion are distinct, `pcall`-uncatchable errors — never a silent `nil`.

10. **Reproducible by default — pin everything.** Pinned `rust-toolchain.toml`, pinned deps, the Luau oracle
    pinned to 0.709 (with the documented re-pin policy + the `LuauIntegerType2` tripwire), the fuel cost table
    versioned (fusion-invariant). Build reproducibility underwrites runtime determinism. Every "magic" choice —
    a constant, a tie-break, an iteration order — is documented and golden-tested, never incidental.

## Coherence rule (the meta-principle)
Code is ground truth for "what's BUILT"; `design/` is intent; they must agree. Touch an area → reconcile its
doc + tests in the same change. `design/CURRENT.md` wins on any disagreement and must be kept current.

## Where things live (public vs private boundary)
- **This repo `rustia` (public-bound):** code + `design/` (the spec) + public GitHub Issues. **Nothing secret
  or infra-identifying ever lands here** (no runner hostnames/IPs/config, no credentials).
- **`private/` — the `rustia-ops` private sibling repo, git-ignored mount** (separate git history → the
  self-hosted CI/infra can never appear in public history by construction). Holds private notes, the CI-runner
  setup, and private tasks as plain markdown. Present only when cloned. Put anything not-for-public **here**.
- **Claude memory** (`MEMORY.md` + `memory/*.md`, outside the repo) — cross-session agent context.
- **Tasks:** public bugs/features = GitHub Issues on this repo; private/infra tasks = files (or Issues) in
  `rustia-ops`. **No Linear** (the private repo covers it).
- Self-hosted CI is referenced in public workflows ONLY by a generic label (e.g. `[self-hosted, bench]`).
