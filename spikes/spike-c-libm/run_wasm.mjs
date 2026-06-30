import { readFileSync } from "node:fs";

const path = "target/wasm32-unknown-unknown/release/spike_c_libm.wasm";
const bytes = readFileSync(path);
const { instance } = await WebAssembly.instantiate(bytes, {});
const { wasm_n, wasm_hash } = instance.exports;

const fnNames = ["sin","cos","exp","ln","pow","sqrt","fmod","tan","atan2","coreops"];
const sampleLabels = ["0.0/0.0","sqrt(-1)","ln(-1)","-0.0","+inf","-inf","inf-inf","0*inf","pow(0,0)","pow(-1,0.5)","f64::min(NaN,1)","f64::max(NaN,1)"];

const n = wasm_n();
const h = (i) => wasm_hash(i); // returns BigInt
const hex = (b) => "0x" + BigInt.asUintN(64, b).toString(16).padStart(16, "0");

console.log("=== spike-c libm bit-identity report ===");
console.log("target_arch=wasm32 (wasm32-unknown-unknown, run in node " + process.version + ")");
console.log("--- per-function hashes (canon / raw) ---");
for (let i = 0; i < 10; i++) {
  const c = h(i * 2), r = h(i * 2 + 1);
  console.log(`${fnNames[i].padStart(7)}: canon=${hex(c)} raw=${hex(r)}`);
}
console.log("--- combined ---");
console.log("combined_canon=" + hex(h(20)));
console.log("combined_raw  =" + hex(h(21)));
console.log("--- determinism-hostile sample bit patterns (canon_bits / raw_bits) ---");
for (let s = 0; s < 12; s++) {
  const c = h(22 + s * 2), r = h(22 + s * 2 + 1);
  console.log(`${sampleLabels[s].padStart(16)}: canon=${hex(c)} raw=${hex(r)}`);
}
console.log("n_values=" + n);
