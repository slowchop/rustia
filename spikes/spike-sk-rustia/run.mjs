// rustia walking-skeleton wasm runner. Works in node v25 and deno (both V8).
//   node run.mjs   OR   deno run --allow-read run.mjs
// Times the SAME per-agent VM realistic loop that the native harness times, plus the native floor
// and the fuel-off variant, all inside wasm32. Reports us/decision and the full-VM wasm tax vs native.
import { readFileSync } from "node:fs";

const WASM_PATH = new URL(
  "./target/wasm32-unknown-unknown/release/spike_sk_rustia.wasm",
  import.meta.url
);

const now = () => performance.now();
const bytes = readFileSync(WASM_PATH);
const { instance } = await WebAssembly.instantiate(bytes, {});
const ex = instance.exports;
const { bench_setup, bench_run_realistic, bench_run_realistic_nofuel, bench_run_native, bench_n } = ex;

const SIZES = [1000, 5000, 10000];
const RUNS = 5;

function measure(n, ticks, runFn) {
  // fresh world each measurement so paths are comparable; warm up the JIT + caches.
  bench_setup(n);
  for (let w = 0; w < 3; w++) runFn(10);
  let best = Infinity;
  let chk = 0n;
  for (let r = 0; r < RUNS; r++) {
    bench_setup(n);
    runFn(10); // warm this fresh world
    const t0 = now();
    chk = runFn(ticks);
    const t1 = now();
    const ms = t1 - t0;
    if (ms < best) best = ms;
  }
  const us_per_decision = (best * 1000) / (n * ticks);
  return { best_ms: Number(best.toFixed(3)), us_per_decision: Number(us_per_decision.toFixed(4)), checksum: "0x" + chk.toString(16) };
}

const rt =
  typeof Deno !== "undefined"
    ? "deno " + Deno.version.deno + " (v8 " + Deno.version.v8 + ")"
    : "node " + process.version;

const out = { runtime: rt, results: {} };
for (const n of SIZES) {
  const ticks = n >= 10000 ? 80 : 150;
  out.results[n] = {
    ticks,
    native: measure(n, ticks, bench_run_native),
    vm_realistic_fuel: measure(n, ticks, bench_run_realistic),
    vm_realistic_nofuel: measure(n, ticks, bench_run_realistic_nofuel),
  };
  const r = out.results[n];
  r.wasm_tax_vs_native_floor = Number(
    (r.vm_realistic_fuel.us_per_decision / r.native.us_per_decision).toFixed(2)
  );
}
console.log(JSON.stringify(out, null, 2));
