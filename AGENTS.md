# AGENTS.md — how to work in rustia (read before writing code)

rustia is a from-scratch, **deterministic** Luau interpreter in Rust (replaces koto in grabby).
Public source access is separate from package publication and supported-release acceptance; full Luau parity
is not implied by source visibility. **This is an LLM-first codebase** — written and maintained mostly by agents, read and
reviewed by humans. Issues and PRs are publicly readable. Only repository collaborators can open either. Optimise for *both*: code an agent can reason about locally and a human can trust.

**Start every session here:** read [`design/CURRENT.md`](design/CURRENT.md) (the single canonical state), then
the relevant [`design/`](design/) docs. The design docs + the pinned Luau **0.709** oracle are the SPEC; code
serves them.

## Development starts from GitHub Issues

- Maintainers use the issue workflow below. Security reports follow [SECURITY.md](SECURITY.md).
- The [GitHub issue tracker](https://github.com/slowchop/rustia/issues) is the primary
  development backlog for source bugs, features, documentation and maintenance.
  Search open and closed issues before starting; reuse an existing issue or create
  one focused issue with the problem, evidence, scope and acceptance criteria.
- Read the selected issue, linked design and existing PRs before editing. Record
  the branch/PR and current progress on the issue so another session can see work
  already in progress. Split independent follow-ups into linked issues rather
  than leaving actionable work only in chat, agent memory or a report.
- PRs link their issues. Use `Closes #N` only when the whole issue is resolved;
  use `Refs #N` for partial work and keep the remaining acceptance items open.
  Merge only after independent review and applicable CI pass, then verify both
  the merged artifact and the issue state. Do not close a tracker for one child fix.
- Issues own task status, priority and coordination. `design/CURRENT.md` owns
  implemented state and active constraints; design docs and the pinned oracle own
  behavior. An issue does not override those contracts or authorize restricted
  publication, benchmark, infrastructure or repository-setting actions.
- Keep CURRENT's backlog section as links into Issues, not a second task list.
  Private infrastructure and sensitive client work stay in `rustia-ops`.

## EGM code maps

- Before editing a file, read every existing ancestor `EGM.md`, starting at the repository root and
  continuing through the file's directory. Read linked child maps when the change crosses their boundaries.
- Maintain affected `EGM.md` maps alongside code changes: update file/module roles, interfaces, invariants,
  side effects, verification commands, and parent links when those facts change. Create or remove maps as
  appropriate when source directories are added, moved, or removed.
- `EGM.md` describes the code as built and provides navigation; it does not replace the spec or grant new
  authority. Keep `design/CURRENT.md`, the relevant design documents, and the pinned oracle as the authority
  described above. Report code/design disagreements explicitly rather than silently resolving them in a map.
- Preserve human-facing `README.md` files. EGM bootstrap and drift maintenance target `EGM.md`; ignored,
  vendored, generated, and test-only trees do not need their own maps.

## Benchmark policy

- This checkout selects `public-hosted` correctness in `design/public-source-profile.json`.
  The GitHub `public` event starts the required suites when the source repository becomes public.
  Use the standard hosted runners and keep private runners and credentials outside this repository.
  Public per-commit/history measurement, landscape refresh and benchmark publication remain paused.

- A separate private maintainer bridge is available only for manually dispatched,
  reviewed exact-cohort runtime comparisons on `ci-bench`. The #33 harness-lifetime
  repeatability controls use a separately reviewed, immutable private protocol.
  Keep all outputs private and
  review raw data independently before accepting a comparison. Bridge availability is
  not a performance result; private comparisons do not update public history or authorize public claims.
- Never run performance benchmarks on the developer's local machine. This includes benchmark examples or binaries, `cargo bench`, timing or profiling scripts, and ad hoc performance comparisons.
- Local compilation, correctness tests, and static validation are allowed, but their durations must not be treated or reported as performance evidence.
- Obtain measurements through the separate private maintainer bridge; pushing a source PR
  here does not run benchmarks. Only authorized `ci-bench` results count as performance evidence.
- If that measurement service is unavailable, report the benchmark as pending; never substitute a local run.

## Pull request delivery

- Enable auto-merge once a pull request is ready and let required checks and reviews gate it; do not leave a fully green pull request open for separate approval.
- Verify all applicable hosted checks and independent review before merging.
  Verify the repository settings directly before relying on protection or auto-merge.
  Historic benchmark publication permissions apply only to the private historic repository.

- Use a merge commit, not squash or rebase, because generated benchmark-operations artifacts derive their state from the reviewed commit history and must retain those commit identities on `main`.

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
   This guarantee is an explicitly selected contract, not a global ban on ordinary embedding:
   `RustiaBuilder::live()` is a first-class, intentionally non-reproducible environment, and `standard()`
   permits explicit nondeterministic providers and host capabilities. Only construction paths that promise
   determinism must reject nondeterminism; do not cripple the shared advanced API for `live()` users.

3. **Measure, don't assert — perf is a number or it didn't happen.** No "this should be faster." Perf claims
   come from the benchmark suite (a CI gate vs reference Luau + koto, per-workload). Add a tier/optimisation
   (JIT, inline cache, …) only when a measurement demands it. "Done" means tested + measured, not hoped.

4. **KISS — one core, earn every abstraction.** One portable engine (no per-target forks); no bespoke ops in
   the public core; no new layer/tier/trait until a measurement or a second real caller justifies it. Prefer
   the simplest thing that passes the golden + the bench. Strangler: small, always-green steps.

5. **Mechanical enforcement over good intentions (the LLM-first lever).** rustfmt, `clippy -D warnings`, the
   determinism lints, `unsafe_code = "forbid"` for the public workspace, the golden test, the bench gate, and
   bounded frontend/VM fuzz smoke and differential checks against the pinned Luau 0.709 oracle — all in CI
   (see [the fuzz harness](fuzz/README.md) and [oracle workflow](.github/workflows/error-fuzz.yml)).
   Independent-frontend comparison such as `full_moon` remains planned, not an implemented CI gate.
   If a rule matters, make it a check, not a comment. Agents forget; CI doesn't.

6. **Local reasoning — explicit state, no spooky action at a distance.** No hidden globals / thread-locals /
   statics that affect behaviour; pass VM state explicitly. Host effects go through the typed effect-queue,
   never `&World` aliasing. A function must be understandable from its signature + body — that's how both
   agents and humans reason, and it's what makes the thread-affine, one-VM-per-lane model sound.

7. **Greppable structure, honest names.** One concept per file; small self-contained modules named for what
   they hold; the module tree mirrors the design's crate layout; no mega-files, deep nesting, or
   definition-hiding macros. Names map to the design vocabulary and Luau/Lua terms (so cross-reference is
   trivial). Rust idiom: `snake_case` fns, `CamelCase` types, `SCREAMING_SNAKE` consts; opcodes and `Value`
   variants named consistently. A name should tell the truth about cost and effects (`get` is cheap; `compile`
   is not).

8. **No unsafe code in the public workspace; invariants are documented and asserted.** Production crates and
   the benchmark harness inherit `unsafe_code = "forbid"`. Experimental spikes or standalone FFI demos may use
   `unsafe` only with an explicit local reason and `SAFETY:` comments. Use `debug_assert!` for cheap invariants.
   `DeterministicSim` is a strict subset of `ReproducibleStandard` — a test asserts anything that runs under
   the former produces identical results under the latter. `live()` deliberately adds nondeterministic
   services without creating a separate semantic engine or weakening the stricter construction paths.

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

## Pre-release API development

- Until Rustia's first supported release, prefer the cleanest coherent long-term API over compatibility with
  controlled clients. Treat necessary completeness as part of the feature and migrate every active client in the
  same rollout. Existing public-looking names are design candidates during `0.0.x`, not reasons to preserve a
  confusing split API.

## Where things live (public vs private boundary)
- **`slowchop/rustia`** holds the experimental source release and its design documents.
  Keep credentials, private infrastructure and sensitive client details out of source and hosted content.
- **`slowchop/rustia-historic`** retains the original evidence and stays private.

- **`private/` — the `rustia-ops` private sibling repo, git-ignored mount** (separate git history → the
  self-hosted CI/infra can never appear in public history by construction). Holds private notes, the CI-runner
  setup, and private tasks as plain markdown. Present only when cloned. Put anything not-for-public **here**.
- **Claude memory** (`MEMORY.md` + `memory/*.md`, outside the repo) — cross-session agent context.
- **Tasks:** source development uses GitHub Issues in `slowchop/rustia`; private/infra tasks use files or Issues in
  `rustia-ops`. Only collaborators create issues and PRs. **No Linear**.
- The source workflows use only standard GitHub-hosted runners. Private operations and benchmark runners stay isolated.

## Before pushing

Install the checkout-local guard with `just install-hooks`. Use `just check-quick`
while iterating. For final delivery, prefer `just validated-push`: it runs the
full `just release-preflight` inventory once, rechecks the exact clean HEAD, and
immediately pushes only that commit. Ordinary `git push` still runs the full
hook. A focused test or one stage is not a substitute. Preserve existing hooks
when installing. Do not bypass failures or regenerate evidence inside validation.
See `design/97-validation-workflow.md`.
