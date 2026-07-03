#!/usr/bin/env js
"use strict";

const MODULES = [
  {
    style: "br_table",
    feature: "core br_table dispatch",
    wasm_base64: "AGFzbQEAAAABBgFgAX8BfwMCAQAHBwEDcnVuAAAKZAFiAQJ/AkADQCABIABPBEAMAgsCQAJAAkACQAJAIAFBA3EOAwABAgMLIAIgAWohAgwDCyACIAFBAXRzIQIMAgsgAiABayECDAELIAJBA2xBAWohAgsgAUEBaiEBDAALAAsgAgs=",
    wat: `(module
  (func (export "run") (param $n i32) (result i32)
    ;; One loop iteration equals one table dispatch.
    ;; Cases intentionally match the call_ref module below.
    (local $i i32) (local $acc i32)
    (block $done
      (loop $loop
        (if (i32.ge_u (local.get $i) (local.get $n))
          (then (br $done)))
        (block $dispatch
          (block $case3
            (block $case2
              (block $case1
                (block $case0
                  (br_table $case0 $case1 $case2 $case3
                    (i32.and (local.get $i) (i32.const 3))))
                (local.set $acc (i32.add (local.get $acc) (local.get $i)))
                (br $dispatch))
              (local.set $acc (i32.xor (local.get $acc) (i32.shl (local.get $i) (i32.const 1))))
              (br $dispatch))
            (local.set $acc (i32.sub (local.get $acc) (local.get $i)))
            (br $dispatch))
          (local.set $acc (i32.add (i32.mul (local.get $acc) (i32.const 3)) (i32.const 1))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $loop)))
    (local.get $acc)))`,
  },
  {
    style: "call_ref",
    feature: "typed function reference dispatch",
    wasm_base64: "AGFzbQEAAAABDAJgAn9/AX9gAX8BfwMGBQAAAAABBAQBcAAEBwcBA3J1bgAECQoBAEEACwQAAQIDClQFBwAgACABagsKACAAIAFBAXRzCwcAIAAgAWsLCgAgAEEDbEEBagssAQJ/A0AgASAASQRAIAIgASABQQNxJQD7FgAUACECIAFBAWohAQwBCwsgAgs=",
    wat: `(module
  (type $op (func (param i32 i32) (result i32)))
  (func $op0 (type $op) (param $a i32) (param $i i32) (result i32)
    (i32.add (local.get $a) (local.get $i)))
  (func $op1 (type $op) (param $a i32) (param $i i32) (result i32)
    (i32.xor (local.get $a) (i32.shl (local.get $i) (i32.const 1))))
  (func $op2 (type $op) (param $a i32) (param $i i32) (result i32)
    (i32.sub (local.get $a) (local.get $i)))
  (func $op3 (type $op) (param $a i32) (param $i i32) (result i32)
    (i32.add (i32.mul (local.get $a) (i32.const 3)) (i32.const 1)))
  (table $ops 4 funcref)
  (elem (i32.const 0) $op0 $op1 $op2 $op3)
  (func (export "run") (param $n i32) (result i32)
    ;; One loop iteration equals one typed table.get/ref.cast/call_ref dispatch.
    (local $i i32) (local $acc i32)
    (loop $loop
      (if (i32.lt_u (local.get $i) (local.get $n))
        (then
          (local.set $acc
            (call_ref $op
              (local.get $acc)
              (local.get $i)
              (ref.cast (ref $op)
                (table.get $ops
                  (i32.and (local.get $i) (i32.const 3))))))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $loop))))
    (local.get $acc)))`,
  },
];

function fail(message) {
  writeErr(`measure-dispatch-wasm: ${message}\n`);
  exit(1);
}

function exit(code) {
  if (typeof Deno !== "undefined") {
    Deno.exit(code);
  }
  if (typeof process !== "undefined" && process.exit) {
    process.exit(code);
  }
  if (typeof quit === "function") {
    quit(code);
  }
  throw new Error(`exit ${code}`);
}

function writeOut(text) {
  if (typeof console !== "undefined" && console.log) {
    console.log(text);
  } else if (typeof print === "function") {
    print(text);
  } else {
    throw new Error("no stdout writer available");
  }
}

function writeErr(text) {
  if (typeof console !== "undefined" && console.error) {
    console.error(text.trimEnd());
  } else if (typeof printErr === "function") {
    printErr(text);
  } else if (typeof print === "function") {
    print(text);
  }
}

function argsForRuntime() {
  if (typeof Deno !== "undefined") {
    return Deno.args;
  }
  if (typeof process !== "undefined" && Array.isArray(process.argv)) {
    return process.argv.slice(2);
  }
  if (typeof scriptArgs !== "undefined") {
    return scriptArgs;
  }
  return [];
}

function parseArgs(argv) {
  const options = {
    commit: "unknown",
    targetMs: 80,
    reps: 5,
    maxN: 1_000_000_000,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--commit") {
      options.commit = requiredValue(argv, ++index, arg);
    } else if (arg === "--target-ms") {
      options.targetMs = positiveNumber(requiredValue(argv, ++index, arg), arg);
    } else if (arg === "--reps") {
      options.reps = positiveInteger(requiredValue(argv, ++index, arg), arg);
    } else if (arg === "--max-n") {
      options.maxN = positiveInteger(requiredValue(argv, ++index, arg), arg);
    } else {
      fail(`unknown argument ${JSON.stringify(arg)}`);
    }
  }
  return options;
}

function requiredValue(argv, index, flag) {
  if (index >= argv.length) {
    fail(`${flag} requires a value`);
  }
  return argv[index];
}

function positiveNumber(value, label) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) {
    fail(`${label} must be a positive number`);
  }
  return number;
}

function positiveInteger(value, label) {
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) {
    fail(`${label} must be a positive integer`);
  }
  return number;
}

function runtimeInfo() {
  const host = hostInfo();
  if (typeof Deno !== "undefined") {
    return {
      name: "deno",
      family: "V8",
      version: Deno.version.deno,
      engine_version: Deno.version.v8,
      host,
    };
  }
  if (typeof Bun !== "undefined") {
    return {
      name: "bun",
      family: "JavaScriptCore",
      version: Bun.version,
      engine_version: "unknown",
      host,
    };
  }
  if (typeof process !== "undefined" && process.versions && process.versions.node) {
    return {
      name: "node",
      family: "V8",
      version: process.versions.node,
      engine_version: process.versions.v8 || "unknown",
      host,
    };
  }
  return {
    name: "spidermonkey",
    family: "SpiderMonkey",
    version: spiderMonkeyVersion(),
    engine_version: spiderMonkeyVersion(),
    host,
  };
}

function spiderMonkeyVersion() {
  if (typeof getBuildConfiguration === "function") {
    try {
      const config = getBuildConfiguration();
      if (config && typeof config === "object" && config.version) {
        return String(config.version);
      }
    } catch (_) {
      // Fall through to version().
    }
  }
  if (typeof version === "function") {
    try {
      return String(version());
    } catch (_) {
      return "unknown";
    }
  }
  return "unknown";
}

function hostInfo() {
  if (typeof Deno !== "undefined") {
    return `${Deno.build.os}-${Deno.build.arch}`;
  }
  if (typeof process !== "undefined" && process.platform && process.arch) {
    return `${process.platform}-${process.arch}`;
  }
  if (typeof navigator !== "undefined" && navigator.userAgent) {
    return navigator.userAgent.replace(/\s+/g, "-").slice(0, 80);
  }
  return "unknown-host";
}

function targetName(runtime) {
  const family = runtime.family.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  return `wasm32-dispatch-${family}-${runtime.name}-${runtime.host}`;
}

function base64Bytes(text) {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const values = Object.create(null);
  for (let index = 0; index < alphabet.length; index += 1) {
    values[alphabet[index]] = index;
  }
  let buffer = 0;
  let bits = 0;
  const out = [];
  for (const char of text.replace(/\s+/g, "")) {
    if (char === "=") {
      break;
    }
    const value = values[char];
    if (value == null) {
      fail(`invalid base64 character ${JSON.stringify(char)}`);
    }
    buffer = (buffer << 6) | value;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push((buffer >> bits) & 0xff);
    }
  }
  return new Uint8Array(out);
}

function compileModule(moduleSource) {
  const bytes = base64Bytes(moduleSource.wasm_base64);
  try {
    const module = new WebAssembly.Module(bytes);
    const instance = new WebAssembly.Instance(module, {});
    if (typeof instance.exports.run !== "function") {
      throw new Error("missing run export");
    }
    return { style: moduleSource.style, feature: moduleSource.feature, bytes, instance };
  } catch (error) {
    return {
      style: moduleSource.style,
      feature: moduleSource.feature,
      bytes,
      unsupported: String(error && error.message ? error.message : error),
    };
  }
}

function nowMs() {
  if (typeof performance !== "undefined" && performance.now) {
    return performance.now();
  }
  if (typeof dateNow === "function") {
    return dateNow();
  }
  return Date.now();
}

function runOnce(compiled, n) {
  const start = nowMs();
  const checksum = compiled.instance.exports.run(n) | 0;
  const elapsedMs = nowMs() - start;
  if (!Number.isFinite(elapsedMs) || elapsedMs < 0) {
    throw new Error(`${compiled.style} timer returned invalid elapsed time ${elapsedMs}`);
  }
  return { checksum, elapsedMs };
}

function calibrate(compiled, options) {
  let n = 1024;
  while (true) {
    const { elapsedMs } = runOnce(compiled, n);
    if (elapsedMs >= options.targetMs * 0.75 || n >= options.maxN) {
      return n;
    }
    const scale = Math.max(2, Math.min(64, options.targetMs / Math.max(elapsedMs, 0.001)));
    const next = Math.min(options.maxN, Math.ceil(n * scale));
    n = Math.max(n + 1, next);
  }
}

function median(values) {
  const sorted = values.slice().sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)];
}

function measureStyle(compiled, options) {
  const n = calibrate(compiled, options);
  runOnce(compiled, n);
  const samples = [];
  let checksum = 0;
  for (let rep = 0; rep < options.reps; rep += 1) {
    const result = runOnce(compiled, n);
    checksum = result.checksum;
    samples.push(result.elapsedMs);
  }
  const minMs = Math.min(...samples);
  const medianMs = median(samples);
  return {
    style: compiled.style,
    feature: compiled.feature,
    n,
    reps: options.reps,
    wasm_bytes: compiled.bytes.byteLength,
    ns_per_dispatch_min: minMs * 1e6 / n,
    ns_per_dispatch_median: medianMs * 1e6 / n,
    dispatches_per_sec: n / (minMs / 1000),
    checksum: `i32:${checksum}`,
  };
}

function comparison(rows) {
  const baseline = rows.find((row) => row.style === "br_table");
  const candidate = rows.find((row) => row.style === "call_ref");
  if (!baseline || !candidate) {
    return {
      status: "unsupported",
      baseline: "br_table",
      candidate: "call_ref",
    };
  }
  return {
    status: "measured",
    baseline: "br_table",
    candidate: "call_ref",
    candidate_vs_baseline_min: candidate.ns_per_dispatch_min / baseline.ns_per_dispatch_min,
    candidate_vs_baseline_median: candidate.ns_per_dispatch_median / baseline.ns_per_dispatch_median,
  };
}

const options = parseArgs(argsForRuntime());
const runtime = runtimeInfo();
const compiled = MODULES.map(compileModule);
const skipped = compiled
  .filter((moduleResult) => moduleResult.unsupported)
  .map((moduleResult) => ({
    style: moduleResult.style,
    feature: moduleResult.feature,
    wasm_bytes: moduleResult.bytes.byteLength,
    reason: moduleResult.unsupported,
  }));

let rows = [];
let verification = null;
if (skipped.length === 0) {
  const verificationN = 1000;
  const verificationRows = compiled.map((moduleResult) => ({
    style: moduleResult.style,
    checksum: `i32:${moduleResult.instance.exports.run(verificationN) | 0}`,
  }));
  const checksums = new Set(verificationRows.map((row) => row.checksum));
  if (checksums.size !== 1) {
    fail(
      `dispatch modules disagree on fixed checksum: ${
        verificationRows.map((row) => `${row.style}=${row.checksum}`).join(", ")
      }`,
    );
  }
  verification = {
    n: verificationN,
    checksum: verificationRows[0].checksum,
  };
  rows = compiled.map((moduleResult) => measureStyle(moduleResult, options));
}

const result = {
  schema_version: 1,
  schema: "rustia.bench.wasm-dispatch-results.v1",
  source: "script-bench-harness",
  timestamp_unix: Math.floor(Date.now() / 1000),
  commit: options.commit,
  target: targetName(runtime),
  runtime,
  microbench: {
    id: "call_ref_vs_br_table",
    label: "call_ref vs br_table dispatch",
    dispatches_per_unit: 1,
    styles: MODULES.map((moduleSource) => ({
      style: moduleSource.style,
      feature: moduleSource.feature,
      wasm_bytes: base64Bytes(moduleSource.wasm_base64).byteLength,
    })),
  },
  rows,
  skipped,
  verification,
  comparison: comparison(rows),
};

writeOut(JSON.stringify(result, null, 2));
