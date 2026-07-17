# rustia — 10 design improvements (conformant, adopted)

> **Status: [IMPROVEMENTS v1 — adopted] (2026-06-30).** From a 6-lens proposer + adversarial-synthesis
> workflow over the design + the [conformance table](50-conformance.md). Each was verified against the five
> gates (full-Luau public · the Standard/Deterministic two-profile model · public-clean/no-bespoke-in-public ·
> thread-affine safe-Rust VM · the locked axes) — **all 10 hold; none survives by violating an ideal.** Most live in the
> UNSPECIFIED/IMPL-DEFINED bucket (free to choose) or *are what real Luau already does*; the rest are pure
> host-side/tooling guardrails. They **refine** the locked axes, never abandon them. Raw: `50-conformance-raw.json`.
> For the newer performance-specific ranking and C++ Luau comparison, see
> [`56-performance-research.md`](56-performance-research.md). This file remains the conformance-vetted improvement
> list; the performance note reorders and extends the speed work.

## The 10 (ranked by leverage)

**1. Dual-part table (dense array part + insertion-ordered hash part).** *(speed #1 · memory #2 · compat #4)*
Replace the flat IndexMap with **Luau's own two-part shape**: a contiguous `Vec<Value>` array part for the
dense `1..n` prefix (no key, no hash) + the insertion-ordered hash part. `pairs()`/`next()`/generalized-for =
array-ascending then hash-insertion-order. Logical `t[k] = nil` deletion preserves the entry as a stable
tombstone so `next(t, k)` can resume; physical removals preserve relative order and rebuild the deterministic
side index. Never use `swap_remove`. **Why better:** ~16 B/elem vs ~40 B+ on the common shape (memory),
`t[i]`/`ipairs`/`#t` become array
ops (kills the ~8% range/iterator cost + integer-key hashing), and it makes `pairs()` order *match real Luau on
every proper sequence* — closing the single biggest cross-category compat risk. **Conformant:** hash-part order
is UNSPECIFIED in Luau, so pinning it is a within-spec strengthening; byte-match claimed only on proper
sequences (where `#`/order are SPECIFIED); we do *not* clone `luaH_getn`'s capacity-dependent border. **Cost:**
large (it's the core table type) — but it *replaces* the planned IndexMap, and is the rep Luau uses anyway.

**2. String model: per-Program `Arc` constant-intern pool + atom ids + short-string inlining.** *(speed #1 ·
memory #2)* Intern every compile-time string constant into the immutable `Arc<Program>` (shared by all VMs
running it) with a `u32` atom id + precomputed seedless hash; inline strings ≤~13 B directly in the 16-B
`Value`. Constant-key lookups probe via the cached hash; method dispatch keys on the atom id; equality of two
interned operands is an integer compare. **Why better:** attacks the measured ~20% clone/drop + ~3.3% lookup +
method dispatch at once (this is what Luau does); the **cross-VM shared pool is the biggest RSS lever** in the
N-VM model (literals stored 1× per process, not N×). **Conformant:** eq/hash unify *by bytes* (atom fast-path
only when both sides are interned; else byte compare) → Luau semantics byte-identical; only **compile-time
constants** go in the immutable `Arc` (Sync-blessed), runtime strings stay per-VM `Gc` → the live VM remains
thread-affine; stays in the 16-B layout. **Cost:** medium-high; sequence behind the sealed `Value` newtype pre-P1.
(Starts resolving the open const-pool/`Gc`-string seam.)

**Implementation update (2026-07-17):** the immutable `Arc<ConstantStorage>` is now the sole owner of compiled
constant bytes and precomputed string hashes. Program clones and separate VMs retain typed descriptors into that
same storage, so long compiled atoms do not allocate a second byte vector. Binary compiled literals and ordinary
runtime results up to 13 bytes live directly in the sealed 16-byte `Value`; an equal runtime-first long atom
upgrades to compiled shared storage. Short table keys atomize only when a successful non-nil insertion needs
identity, while misses and failed/nil writes do not grow the atom table.

**3. mutation-safe generic FASTCALL + GETIMPORT.** *(speed #1 — strongest on the sim hot path)* Re-introduce
Luau's import and builtin-call families without assuming the Standard environment is frozen. `GETIMPORT`
resolves one-to-three-component descriptors through shape/key-guarded locations and reads the live value at
every component; it never caches a final `Value`. Generic fixed-result `FASTCALL` guards the exact current
function handle and native function kind, directly dispatches only on a match, and otherwise takes the ordinary
resumable call path. The compiler admits unshadowed standard-library fields and binding-scoped aliases; lexical
shadowing does not inherit an outer prediction. **Why better:** stable `math.*` and alias-heavy calls avoid repeated lookup/function-kind
work while retaining monkey-patching, host replacement, metamethod, arity-error, and yield behavior.
**Conformant:** the guards make mutation a deoptimization rather than a semantic change; no builtin result or
callee value is folded into mutable Standard bytecode. `GETIMPORT` costs 1/3/5 logical fuel for one/two/three
components, reserving the total atomically before lookup, and `FASTCALL` costs the same one base fuel as `CALL`,
with native variable-work fuel charged once on all paths. **Cost:** medium (reuses the locked
generic-superinstruction machinery).

This mutation guarantee applies to calls lowered through the guarded import/call pair. Pre-existing dedicated
math/string opcodes still use their existing source-level safeenv gates; making those opcodes observe arbitrary
host-time replacement is separate hardening work.

**4. Borrowing internal host-fn ABI (zero-copy inside `mutate`) + typed `add_fn` sugar.** *(speed #1 · memory
#2 · DX #5)* Add an internal `Fn(&Ctx<'gc>, &[Value<'gc>]) -> Result<Value<'gc>>` convention for the stdlib +
op-hook ops (zero-copy on live values), exposed to embedders only via typed sugar:
`vm.add_fn("clamp", |x:f64, lo:f64, hi:f64| …)` — args extracted off the register window via `FromValue`, no
per-call `Vec<OwnedValue>`. **Why better:** the measured value-boundary cost is the deep-copy, not serde
(1.47 vs 1.01 µs); the stdlib is called constantly, so paying `OwnedValue` round-trips on every builtin is pure
waste — collapses the ~4.7% host-call cost and gives the ergonomic typed API. **Conformant:** the public
`Box<dyn Fn(&[OwnedValue])->Result>` ABI is **retained as the `'gc` firewall** for embedder code; typed sugar
yields owned Rust scalars (brand never leaks); runs on the owning thread inside `mutate` → thread-affine VM intact.
**Cost:** medium (one internal trait + ~8 arity impls; public ABI unchanged).

**5. Enforce libm-only transcendentals in-tree (clippy `disallowed-methods` + deny FMA/relaxed-SIMD).** *(determinism #4 · wasm #3)* Hard-forbid `f64::{sin,cos,…,powf,…}` everywhere (route through the `libm` crate);
whitelist the correctly-rounded ops (`sqrt`/`floor`/`ceil`/`trunc`/`abs`/`copysign`/`nearest`); deny
`+fma`/`+relaxed-simd`/fast-math at the build level. **Why better:** closes a silent landmine — native
`x.sin()` binds to the *system* libm (glibc vs macOS vs musl, last-ULP-divergent); a grep-able ban guarantees
native and wasm run identical source, which is exactly what Spike-C's bit-identity *silently depended on*.
**Conformant:** transcendental ULPs are UNSPECIFIED → enforcing one impl is conformant; only *enforces* locked
axis E. **Cost:** low (CI config). **← cheap insurance for the core determinism bet.**

**6. Pin float→int conversion to saturating `as` (= wasm `trunc_sat`).** *(determinism #4 · wasm #3 · compat
#4)* Lock out-of-range/NaN float→int (for `%d`/`%x`, `buffer`, `bit32`) to Rust's saturating `as i64`/`as u64`
(NaN→0), which lowers to the single non-trapping `i64.trunc_sat_f64` opcode. **Why better:** the trapping
alternative would **abort the whole wasm module — taking down the iOS WKWebView page** — on a NaN/overflow;
saturating is one branchless opcode and bit-identical across engines. **Conformant:** in-range floats still
truncate-toward-zero exactly like Luau (SPECIFIED, untouched); out-of-range is C-UB in reference Luau →
UNSPECIFIED → free to pick the deterministic option. **Cost:** low.

**7. Capability-typed, fuel-declaring contract on the op-extension hook.** *(public-clean · determinism #4 ·
DX #5)* Each op registered through the hook (the seam that keeps grabby's bespoke `OP_UTIL` a thin private
delta) must declare: (1) its **fuel cost** (folded into the versioned cost table → fuel stays fusion-invariant);
(2) a purity/capability tag (Pure / ReadsHostMutableState / Effectful); (3) profile-eligibility — and a
`Deterministic` VM **refuses at registration** to install any non-deterministic-safe op. **Why better:** the
public/private split is only as safe as the hook contract; without it a private op could read wall-clock or
mischarge fuel and *silently* break the determinism the *public* engine advertises. Makes the private fork a
**verifiable** thin delta. **Conformant:** no script-observable change; strengthens the split + axis-G
fuel-ABI. **Cost:** low.

**8. Two-class f64 ingress (buffer/pack store RAW bytes; canon only when bytes become a *number*).** *(determinism #4 · compat #4)* Name the canon point as an invariant: a NaN/`-0` is canonicalized **when it becomes
a number `Value`**, never when it's byte-reinterpreted — so `buffer`/`string.pack` memory stores the exact
bytes (`writeu32`→`readu32`, `pack`→`unpack` round-trip arbitrary bytes losslessly); `readf64`/unpack-`d`
canonicalize on the *read* (producing a number). **Why better:** a naïve "canon at every f64 ingress" reading
would corrupt buffer/pack byte-fidelity and break Luau parity — an easy-to-write differential test and a real
landmine; stating the boundary now prevents the whole bug class. **Conformant:** byte storage is SPECIFIED-
lossless (matched exactly); NaN-bits *as a number* are UNSPECIFIED (canon is within-spec). **Cost:** low if
designed in when `buffer`/`pack` land (P4).

**9. Frozen, `Arc`-shared sandboxed stdlib/global base environment.** *(memory #2 · wasm #3 · DX #5 · Send fit)*
Build the immutable stdlib (builtins, shared number/string metatables, interned method atoms) **once per
process** as a frozen `Arc`-shared structure (Luau's `luaL_sandbox`); each VM points at it via a readonly
child-env import and allocates per-VM heap only for what it mutates (Standard global writes copy-on-write into
the child). **Why better:** in the N-VM model the dominant per-VM cost is materializing the stdlib (paid on
every hot-reload + console/mod VM) → sharing makes spawn ~free, cuts steady-state RSS (priority #2), and shrinks
wasm startup/high-water on mobile (priority #3). **Conformant:** a readonly shared sandbox *is* Luau's behavior;
Standard layers mutable globals via COW (base never mutated); the frozen image holds only **non-`Gc`**
fn-pointers/`Arc` (never shared `Gc`) → the live VM remains thread-affine. **Cost:** medium (child-env indirection + COW
seam; couple with #2 at P1).

**10. Structured error (kind + span + chunkname + traceback) with one byte-exact Luau-message renderer + typed
capability-removed errors.** *(DX #5 · compat #4 · LSP)* Replace `VmError::Runtime(String)` with a structured
error carrying kind / source span / chunkname / name-enrichment / traceback (cheap under the stackless
trampoline — the frame stack *is* the traceback). Two renderers: rich `Display` for host/LSP, and
`.luau_message()` returning the **byte-exact** Luau string (the only thing `pcall`/`error` observe). Gated
builtins in `Deterministic` raise a distinct `ErrorKind::CapabilityRemoved{feature,profile}`. **Why better:**
one renderer collapses the conformance corpus's *dozens* of exact-string rows (conf-3/4/5/8/10) into a single
differential-tested function, while embedders + the future luau-lsp get spans/traceback for free; turns the
confusing "works in Standard, dies in Deterministic" into a precise diagnostic. **Conformant:** the
`pcall`-visible string is exactly Luau's; extra structure is host-side only (never script-observable); fuel
exhaustion stays a distinct, `pcall`-**uncatchable** kind (axis G); capability-removed errors are ordinary
catchable errors → Deterministic stays a pure restriction. **Cost:** medium (needs compiler span/name metadata,
already required by conf-8).

## Strong runners-up (several worth doing later)
> **PROMOTED per review M3:** the two below are no longer "P4 runner-ups" — **hot/cold opcode split → P1**
> (de-risks the JSC/SpiderMonkey baseline tiers; design-in, not retrofit). Loaded-program cache metadata now uses
> a 4-byte per-PC site map and lazily allocated 16-byte atom/slot/shape records for existing global/index paths
> and `GETIMPORT` lanes. Records revalidate shape/key and read the live value, never mutate shared `Program`, and
> charge the same logical fuel on hits and misses. Dedicated `GETTABLEKS`/`SETTABLEKS`/`NAMECALL` specialization
> is now implemented as the first broader field/method cache slice.
- **Broader field/namecall inline caches** — **implemented** (M3). `GETTABLEKS`/`SETTABLEKS` cover compile-time
  string-key reads and writes, while live receiver and table-valued-`__index` lanes accelerate `NAMECALL` lookup
  without caching a final callee. Full Luau `__namecall` context remains separate conformance work.
- **Front-end crate split → bytecode-only deterministic-sim wasm** — real size/hygiene win; overlaps #9; strong runner-up.
- **Hot/cold opcode split for JSC/SpiderMonkey baseline tiers** — **promoted to P1** (M3); validated by disassembly + the P1 cross-engine measurement.
- **wasm SIMD `f32x4` for vector ops** — clean wasm speedup but `vector` is P4 and workload-specific.
- **`memory.grow` failure → catchable OOM + per-VM arena reset** — browser-robustness (contains an iOS module abort to one `pcall`); pair with #9.
- **Unify the two serde Deserializers + add the `Serialize` direction** (closes C4's round-trip want); **generalized retain/fetch handle registry** (must exist for "call a script fn from Rust"); **first-class hot-reload** (compile-without-teardown, error isolation); **port `lnumprint` formatting shell** (must-do P2 byte-parity guidance — Luau *is* Schubfach; the risk is the `[-5,21]` thresholds/exponent/`-0`/`nan` shell); **"present-but-raises beats nil"** for gated builtins (feature-detection like `os.time and os.time()` silently diverges under `nil`) — folded into #10.

## Net effect on the locked axes (all refined, none abandoned)
- **A** dispatch ← adds generic FASTCALL/GETIMPORT (#3) + the capability-typed hook contract (#7).
- **B** value ← shared compiled strings and 13-byte inline strings stay within the 16-B `Value` contract (#2);
  the array part (#1) cuts value memory.
- **C** determinism ← libm enforcement teeth (#5), float→int rule (#6), the canon-point invariant (#8).
- **D** GC/tables ← the flat map becomes a dual-part table with stable tombstones and a deterministic side index
  (#1); shared compiled storage, inline short values, and indexed VM-local atoms resolve the planned
  const-pool/`Gc`-string seam (#2);
  frozen shared env remains future work (#9).
- **E/F/G/H** ← stable logical-deletion/physical-compaction rules (#1), fuel-ABI on the hook (#7), borrowing ABI
  + typed sugar + structured errors (#4, #10).
