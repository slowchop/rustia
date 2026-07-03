#!/usr/bin/env -S node
"use strict";

const path = await import("node:path");

const defaultWorkloads = [
  {
    id: "dense_tables",
    path: "bench/workloads/luau/01_dense_tables.luau",
    unit: "build+sum+sort of 256-elem array",
  },
  {
    id: "sparse_tables",
    path: "bench/workloads/luau/02_sparse_tables.luau",
    unit: "fill+read 128 hash entries",
  },
  {
    id: "closures_upvalues",
    path: "bench/workloads/luau/04_closures_upvalues.luau",
    unit: "one upvalue-mutating closure call",
  },
  {
    id: "generic_iteration",
    path: "bench/workloads/luau/09_generic_iteration.luau",
    unit: "__iter(32)+pairs(5)+ipairs(32) pass",
  },
  {
    id: "stdlib_math_table",
    path: "bench/workloads/luau/10_stdlib_math_table.luau",
    unit: "math fastcalls + buffered push",
  },
];

function fail(message) {
  console.error(`measure-rustia-wasm: ${message}`);
  exit(1);
}

function exit(code) {
  if (globalThis.Deno) {
    Deno.exit(code);
  }
  process.exit(code);
}

function cwd() {
  if (globalThis.Deno) {
    return Deno.cwd();
  }
  return process.cwd();
}

function parseArgs(argv) {
  const options = {
    wasm: "demos/live-wasm/wasm-runner/target/wasm32-unknown-unknown/release/rustia_live_wasm.wasm",
    commit: "unknown",
    targetMs: 80,
    reps: 5,
    maxN: 1_000_000_000,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--wasm") {
      options.wasm = requiredValue(argv, ++index, arg);
    } else if (arg === "--commit") {
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

async function readBytes(filePath) {
  if (globalThis.Deno) {
    return await Deno.readFile(filePath);
  }
  if (globalThis.Bun) {
    return new Uint8Array(await Bun.file(filePath).arrayBuffer());
  }
  const fs = await import("node:fs/promises");
  return await fs.readFile(filePath);
}

async function readText(filePath) {
  return new TextDecoder().decode(await readBytes(filePath));
}

function resolvePath(filePath) {
  return path.isAbsolute(filePath) ? filePath : path.resolve(cwd(), filePath);
}

function runtimeInfo() {
  const host = hostInfo();
  if (globalThis.Deno) {
    return {
      name: "deno",
      family: "V8",
      version: Deno.version.deno,
      engine_version: Deno.version.v8,
      host,
    };
  }
  if (globalThis.Bun) {
    return {
      name: "bun",
      family: "JavaScriptCore",
      version: Bun.version,
      engine_version: "unknown",
      host,
    };
  }
  return {
    name: "node",
    family: "V8",
    version: process.versions.node,
    engine_version: process.versions.v8,
    host,
  };
}

function hostInfo() {
  if (globalThis.Deno) {
    return `${Deno.build.os}-${Deno.build.arch}`;
  }
  if (globalThis.process?.platform && globalThis.process?.arch) {
    return `${process.platform}-${process.arch}`;
  }
  return "unknown-host";
}

function targetName(runtime) {
  const family = runtime.family.toLowerCase().replace(/[^a-z0-9]+/g, "-");
  return `wasm32-${family}-${runtime.name}-${runtime.host}`;
}

function assertExports(exports) {
  const names = [
    "memory",
    "rustia_alloc",
    "rustia_dealloc",
    "rustia_bench_abi_version",
    "rustia_bench_prepare",
    "rustia_bench_run",
    "rustia_result_ptr",
    "rustia_result_len",
  ];
  for (const name of names) {
    if (exports[name] == null) {
      fail(`wasm export ${name} is missing`);
    }
  }
  if (exports.rustia_bench_abi_version() !== 1) {
    fail(`unsupported wasm bench ABI ${exports.rustia_bench_abi_version()}`);
  }
}

function writeString(exports, text) {
  const bytes = new TextEncoder().encode(text);
  const ptr = exports.rustia_alloc(bytes.length);
  new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, len: bytes.length };
}

function readLastResult(exports) {
  const ptr = exports.rustia_result_ptr();
  const len = exports.rustia_result_len();
  return new TextDecoder().decode(new Uint8Array(exports.memory.buffer, ptr, len));
}

function prepare(exports, source) {
  const input = writeString(exports, source);
  try {
    const code = exports.rustia_bench_prepare(input.ptr, input.len);
    if (code !== 0) {
      throw new Error(readLastResult(exports));
    }
  } finally {
    exports.rustia_dealloc(input.ptr, input.len);
  }
}

function runOnce(exports, n) {
  const start = performance.now();
  const checksum = exports.rustia_bench_run(n);
  const elapsedMs = performance.now() - start;
  if (!Number.isFinite(checksum)) {
    throw new Error(readLastResult(exports));
  }
  return { checksum, elapsedMs };
}

function calibrate(exports, options) {
  let n = 1;
  while (true) {
    const { elapsedMs } = runOnce(exports, n);
    if (elapsedMs >= options.targetMs * 0.75 || n >= options.maxN) {
      return n;
    }
    const scale = Math.max(2, Math.min(64, options.targetMs / Math.max(elapsedMs, 0.001)));
    n = Math.max(n + 1, Math.min(options.maxN, Math.ceil(n * scale)));
  }
}

function median(values) {
  const sorted = values.slice().sort((left, right) => left - right);
  return sorted[Math.floor(sorted.length / 2)];
}

function formatChecksum(value) {
  return `number:${value.toFixed(17)}`;
}

async function measureWorkload(exports, workload, options) {
  const source = await readText(resolvePath(workload.path));
  prepare(exports, source);
  const n = calibrate(exports, options);
  runOnce(exports, n);

  const samples = [];
  let checksum = 0;
  for (let rep = 0; rep < options.reps; rep += 1) {
    const result = runOnce(exports, n);
    checksum = result.checksum;
    samples.push(result.elapsedMs);
  }

  const minMs = Math.min(...samples);
  const medianMs = median(samples);
  return {
    workload: workload.id,
    unit: workload.unit,
    n,
    reps: options.reps,
    ns_per_unit_min: minMs * 1e6 / n,
    ns_per_unit_median: medianMs * 1e6 / n,
    units_per_sec: n / (minMs / 1000),
    checksum: formatChecksum(checksum),
  };
}

function argsForRuntime() {
  if (globalThis.Deno) {
    return Deno.args;
  }
  return process.argv.slice(2);
}

const options = parseArgs(argsForRuntime());
const runtime = runtimeInfo();
const wasmPath = resolvePath(options.wasm);
const wasmBytes = await readBytes(wasmPath);
const { instance } = await WebAssembly.instantiate(wasmBytes, {});
assertExports(instance.exports);

const rows = [];
for (const workload of defaultWorkloads) {
  rows.push(await measureWorkload(instance.exports, workload, options));
}

const value = {
  schema_version: 1,
  schema: "rustia.bench.wasm-runtime-results.v1",
  source: "script-bench-harness",
  timestamp_unix: Math.floor(Date.now() / 1000),
  commit: options.commit,
  target: targetName(runtime),
  runtime,
  wasm: {
    path: path.relative(cwd(), wasmPath),
    bytes: wasmBytes.byteLength,
    bench_abi_version: instance.exports.rustia_bench_abi_version(),
  },
  rows,
};

console.log(`${JSON.stringify(value, null, 2)}\n`);
