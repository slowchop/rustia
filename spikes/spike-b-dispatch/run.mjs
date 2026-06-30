// Spike B wasm dispatch-tax runner. Works in both node (v25) and deno (both V8).
// Usage: node run.mjs   OR   deno run --allow-read run.mjs
import { readFileSync } from "node:fs";

const WASM_PATH = new URL(
  "./target/wasm32-unknown-unknown/release/spike_b_dispatch.wasm",
  import.meta.url
);

// performance.now() is global in both node>=16 and deno.
const now = () => performance.now();

const bytes = readFileSync(WASM_PATH);
const { instance } = await WebAssembly.instantiate(bytes, {});
const { bench, bench_threaded, dispatches_per_run, last_fuel } = instance.exports;

const perRun = dispatches_per_run();
const ITERS = 400000n; // ~5.2e9 dispatches, matches native run
const RUNS = 5;
let sink = 0;

function measure(fn) {
  // Warm up the JIT thoroughly (V8 tiers up to TurboFan after enough calls).
  for (let w = 0; w < 5; w++) sink += fn(2000n);
  let best = Infinity;
  let chk = 0,
    fuel = 0;
  for (let run = 0; run < RUNS; run++) {
    const t0 = now();
    chk = fn(ITERS);
    const t1 = now();
    fuel = last_fuel();
    const ms = t1 - t0;
    if (ms < best) best = ms;
    sink += chk;
  }
  const secs = best / 1000;
  return {
    total_dispatches: fuel,
    checksum: chk,
    best_elapsed_ms: best,
    ns_per_dispatch: Number(((secs * 1e9) / fuel).toFixed(4)),
    dispatch_per_sec: fuel / secs,
  };
}

const rt =
  typeof Deno !== "undefined"
    ? "deno " + Deno.version.deno + " (v8 " + Deno.version.v8 + ")"
    : "node " + process.version;

console.log(JSON.stringify(
  {
    runtime: rt,
    dispatches_per_run: perRun,
    iters: Number(ITERS),
    match_dispatch: measure(bench),
    fnptr_threaded: measure(bench_threaded),
    sink,
  },
  null,
  2
));
