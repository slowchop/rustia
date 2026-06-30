// spike-wasmjit V8 harness. Runs in node v25 and deno (both V8).
//   node harness.mjs   OR   deno run --allow-read harness.mjs
//
// All four paths measured in wasm-in-V8:
//   (1) INTERPRETER  : run_interp   (real big-match register VM executing the kernel bytecode)
//   (2) STATIC-AOT   : run_aot      (kernel as a plain Rust fn -> wasm32; the steady-state ceiling)
//   (3) RUNTIME-GEN  : codegen @ runtime -> compile+instantiate -> run.   Two emitters:
//                        gen_loop = naive (action loop + branchless select for the curve kind)
//                        gen_opt  = unrolled + kind-specialized (AOT-to-wasm style)
//   (4) SHARED-HEAP  : codegen that IMPORTS the host memory; inputs round-trip through that heap.
// then: steady-state speedup, total fixed cost (codegen+compile+instantiate), and the crossover.
import { readFileSync } from "node:fs";

const HOST_WASM = new URL(
  "./target/wasm32-unknown-unknown/release/spike_wasmjit.wasm",
  import.meta.url
);

const isDeno = typeof Deno !== "undefined";
const rt = isDeno
  ? "deno " + Deno.version.deno + " (v8 " + Deno.version.v8 + ")"
  : "node " + process.version;
const now = () => performance.now();

const SEED = 0x12345678; // 305419896, positive i32
const ITERS = 2_000_000; // steady-state op count per timed call (fits i32)
const WARMS = 5; // warmup calls (tier-up to TurboFan)
const RUNS = 9; // timed repetitions, take the best (min)
const FIXED_K = 4000; // repetitions for fixed-cost (codegen/compile/instantiate) means

const hostBytes = readFileSync(HOST_WASM);
const host = (await WebAssembly.instantiate(hostBytes, {})).instance;
const HX = host.exports;

function nsop(fn) {
  for (let w = 0; w < WARMS; w++) fn(Math.floor(ITERS / 8), SEED);
  let best = Infinity;
  let chk = 0n;
  for (let r = 0; r < RUNS; r++) {
    const t0 = now();
    chk = fn(ITERS, SEED);
    const t1 = now();
    if (t1 - t0 < best) best = t1 - t0;
  }
  return { ns_op: +((best * 1e6) / ITERS).toFixed(4), best_ms: +best.toFixed(3), chk: "0x" + chk.toString(16) };
}

function microK(thunk, K) {
  for (let i = 0; i < 16; i++) thunk();
  const t0 = now();
  for (let i = 0; i < K; i++) thunk();
  const t1 = now();
  const mean_us = ((t1 - t0) * 1000) / K;
  let best_us = Infinity;
  const N2 = Math.min(K, 500);
  for (let i = 0; i < N2; i++) {
    const a = now();
    thunk();
    const b = now();
    if ((b - a) * 1000 < best_us) best_us = (b - a) * 1000;
  }
  return { mean_us: +mean_us.toFixed(4), best_us: +best_us.toFixed(4) };
}

async function microKAsync(thunk, K) {
  for (let i = 0; i < 16; i++) await thunk();
  const t0 = now();
  for (let i = 0; i < K; i++) await thunk();
  const t1 = now();
  return { mean_us: +(((t1 - t0) * 1000) / K).toFixed(4) };
}

function readGen(packed) {
  const ptr = Number(packed >> 32n);
  const len = Number(packed & 0xffffffffn);
  return { bytes: new Uint8Array(HX.memory.buffer, ptr, len).slice(), ptr, len };
}

// Build/measure a runtime-generated path. codegenName/runName are host export + module export.
// importsFn(memory) supplies instantiation imports (shared variant needs the host memory).
async function genPath(codegenName, runName, sharedBase) {
  const codegen = HX[codegenName];
  // (a) codegen time (runs INSIDE wasm-in-V8 — the in-browser scenario)
  const cg = microK(() => codegen(), FIXED_K);
  const { bytes, len } = readGen(codegen());
  // (b) compile + instantiate, both ways
  let mod = null;
  const cmpSync = microK(() => { mod = new WebAssembly.Module(bytes); }, FIXED_K);
  const imports = sharedBase != null ? { env: { memory: HX.memory } } : {};
  const instSync = microK(() => { new WebAssembly.Instance(mod, imports); }, FIXED_K);
  const instAsyncBytes = await microKAsync(async () => { await WebAssembly.instantiate(bytes, imports); }, 800);
  const instAsyncMod = await microKAsync(async () => { await WebAssembly.instantiate(mod, imports); }, 800);
  // (c) run
  const inst = new WebAssembly.Instance(mod, imports);
  const fn = sharedBase != null
    ? (it, sd) => inst.exports[runName](it, sd, sharedBase)
    : inst.exports[runName];
  const run = nsop(fn);
  const fixed_us = +(cg.mean_us + cmpSync.mean_us + instSync.mean_us).toFixed(4);
  return {
    ...run, bytes: len,
    codegen_us: cg, compile_sync_us: cmpSync, instantiate_sync_us: instSync,
    instantiate_async_bytes_us: instAsyncBytes, instantiate_async_module_us: instAsyncMod,
    fixed_cost_us: fixed_us,
  };
}

const out = { runtime: rt, seed: SEED, iters: ITERS, results: {} };

// proof the module is GENERATED AT RUNTIME (not a prebuilt .wasm): codegen writes bytes into the
// HOST module's own linear memory; we read them back and confirm the \0asm magic + version.
{
  const heapStart = HX.__heap_base ? HX.__heap_base.value : 0;
  const packed = HX.codegen_run();
  const { bytes, ptr, len } = readGen(packed);
  const magic = [...bytes.slice(0, 4)].map((b) => b.toString(16).padStart(2, "0")).join(" ");
  const version = bytes[4] | (bytes[5] << 8) | (bytes[6] << 16) | (bytes[7] << 24);
  out.gen_proof = {
    bytes_ptr_in_host_heap: ptr,
    heap_base: heapStart,
    ptr_above_heap_base: ptr >= heapStart,
    len,
    wasm_magic: magic, // expect "00 61 73 6d"
    wasm_version: version, // expect 1
    valid_module: WebAssembly.validate(bytes),
  };
}

// (1) interpreter, (2) static-AOT
out.results.interp = nsop(HX.run_interp);
out.results.aot = nsop(HX.run_aot);

// (3) runtime-generated: looped (naive) and unrolled (AOT-to-wasm style)
out.results.gen_loop = await genPath("codegen_run", "run_gen", null);
out.results.gen_opt = await genPath("codegen_run_opt", "run_gen", null);

// (4) shared-heap: looped and unrolled, importing the host linear memory
const base = HX.scratch_ptr();
out.results.shared_loop = await genPath("codegen_shared", "run_shared", base);
out.results.shared_opt = await genPath("codegen_shared_opt", "run_shared", base);
out.results.shared_base = base;

// checksum agreement across ALL paths
const cks = {
  interp: out.results.interp.chk,
  aot: out.results.aot.chk,
  gen_loop: out.results.gen_loop.chk,
  gen_opt: out.results.gen_opt.chk,
  shared_loop: out.results.shared_loop.chk,
  shared_opt: out.results.shared_opt.chk,
};
out.checksums = { ...cks, ALL_IDENTICAL: new Set(Object.values(cks)).size === 1 };

// crossover analysis. Primary = the unrolled "AOT-to-wasm" path we'd actually ship.
const interp = out.results.interp.ns_op;
const aot = out.results.aot.ns_op;
function crossover(path) {
  const saving = interp - path.ns_op;
  return {
    ns_op: path.ns_op,
    speedup_vs_interp: +(interp / path.ns_op).toFixed(1),
    ratio_vs_aot: +(path.ns_op / aot).toFixed(3),
    fixed_cost_us: path.fixed_cost_us,
    per_op_saving_ns: +saving.toFixed(3),
    crossover_ops: Math.ceil((path.fixed_cost_us * 1000) / saving),
  };
}
out.analysis = {
  interp_ns_op: interp,
  aot_ns_op: aot,
  steady_speedup_interp_over_aot: +(interp / aot).toFixed(1),
  gen_opt: crossover(out.results.gen_opt),
  gen_loop: crossover(out.results.gen_loop),
  shared_opt: crossover(out.results.shared_opt),
};

console.log(JSON.stringify(out, null, 2));
