// axis-H vertical slice wasm runner (node v25 / V8). Reproduces the cross-arch golden and measures the
// HONEST general-script us in wasm (run / run+snapshot / run+snapshot+serde), all inside wasm32.
//   node run.mjs
import { readFileSync } from "node:fs";

const WASM_PATH = new URL(
  "./target/wasm32-unknown-unknown/release/spike_h_slice.wasm",
  import.meta.url
);

const now = () => performance.now();
const bytes = readFileSync(WASM_PATH);
const { instance } = await WebAssembly.instantiate(bytes, {});
const ex = instance.exports;
const { bench_setup, bench_run, bench_run_noserde, bench_run_only, bench_run_zerocopy, bench_golden } = ex;

bench_setup();

// ---- golden ----
const golden = bench_golden();
const GOLDEN_NATIVE = 0xa6816bc158ca5244n;
console.log("golden (node/V8) = 0x" + BigInt.asUintN(64, golden).toString(16).padStart(16, "0"));
console.log("golden (native ) = 0x" + GOLDEN_NATIVE.toString(16).padStart(16, "0"));
console.log("MATCH            = " + (BigInt.asUintN(64, golden) === GOLDEN_NATIVE ? "YES (bit-identical)" : "NO"));

// ---- general-script us ----
const ITERS = 2_000_000;
const RUNS = 5;
function measure(fn) {
  bench_setup();
  for (let w = 0; w < 3; w++) fn(ITERS / 10); // warm + JIT
  let best = Infinity;
  let chk = 0n;
  for (let r = 0; r < RUNS; r++) {
    bench_setup();
    fn(ITERS / 10);
    const t0 = now();
    chk = fn(ITERS);
    const t1 = now();
    if (t1 - t0 < best) best = t1 - t0;
  }
  return { us_per_run: Number(((best * 1000) / ITERS).toFixed(4)), checksum: "0x" + BigInt.asUintN(64, chk).toString(16) };
}

const run_only = measure(bench_run_only);
const run_snap = measure(bench_run_noserde);
const run_full = measure(bench_run);
const run_zc = measure(bench_run_zerocopy);

console.log("\n=== HONEST GENERAL-SCRIPT NUMBER (wasm32 / node V8) ===");
console.log("run only (VM exec + gc-arena heap)     : " + run_only.us_per_run + " us   (" + (run_only.us_per_run * 1000 / 68).toFixed(2) + " ns/dispatch over 68)");
console.log("run + snapshot(exports -> OwnedValue)  : " + run_snap.us_per_run + " us   (snapshot = " + (run_snap.us_per_run - run_only.us_per_run).toFixed(4) + " us)");
console.log("run + snapshot + serde -> ScenarioSpec : " + run_full.us_per_run + " us   (serde = " + (run_full.us_per_run - run_snap.us_per_run).toFixed(4) + " us)");
console.log("  -> marshalling+serde overhead        : " + (run_full.us_per_run - run_only.us_per_run).toFixed(4) + " us (" + (((run_full.us_per_run - run_only.us_per_run) / run_full.us_per_run) * 100).toFixed(0) + "% of full)");
console.log("run + ZERO-COPY deserialize_exports    : " + run_zc.us_per_run + " us   (saves " + (run_full.us_per_run - run_zc.us_per_run).toFixed(4) + " us vs snapshot path)");

const node_v = process.version;
console.log(JSON.stringify({ runtime: "node " + node_v, golden: "0x" + BigInt.asUintN(64, golden).toString(16), run_only, run_snap, run_full }, null, 2));
