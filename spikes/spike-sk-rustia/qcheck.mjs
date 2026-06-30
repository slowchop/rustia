import { readFileSync } from "node:fs";
const p = new URL("./target/wasm32-unknown-unknown/release/spike_sk_rustia.wasm", import.meta.url);
const { instance } = await WebAssembly.instantiate(readFileSync(p), {});
const h = instance.exports.bench_q16_golden();
const golden = 0x31c6ec2734ea6fa2n;
const hu = h < 0n ? h + (1n << 64n) : h; // to unsigned
console.log("wasm Q16 hash = 0x" + hu.toString(16) + "  golden = 0x" + golden.toString(16) + "  MATCH=" + (hu === golden));
