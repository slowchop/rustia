#!/usr/bin/env -S node
"use strict";

const path = await import("node:path");

const adapters = [
  {
    id: "rustia",
    abi: 1,
    dialect: "luau",
    sourceDir: "bench/workloads/luau",
    extension: "luau",
  },
  {
    id: "koto",
    abi: 2,
    dialect: "koto",
    sourceDir: "bench/workloads/koto",
    extension: "koto",
  },
  {
    id: "wasmi",
    abi: 3,
    dialect: "wasm",
    sourceDir: "bench/workloads/wasm",
    extension: "wat",
  },
  {
    id: "rhai",
    abi: 4,
    dialect: "rhai",
    sourceDir: "bench/workloads/rhai",
    extension: "rhai",
  },
];

const defaultWorkloads = [
  {
    id: "dense_tables",
    fileStem: "01_dense_tables",
    unit: "build+sum+sort of 256-elem array",
  },
  {
    id: "sparse_tables",
    fileStem: "02_sparse_tables",
    unit: "fill+read 128 hash entries",
  },
  {
    id: "closures_upvalues",
    fileStem: "04_closures_upvalues",
    unit: "one upvalue-mutating closure call",
  },
  {
    id: "generic_iteration",
    fileStem: "09_generic_iteration",
    unit: "__iter(32)+pairs(5)+ipairs(32) pass",
  },
  {
    id: "stdlib_math_table",
    fileStem: "10_stdlib_math_table",
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
    wasm: "bench/wasm/adapter-runner/target/wasm32-unknown-unknown/release/script_bench_wasm_adapter.wasm",
    commit: "unknown",
    targetMs: 80,
    reps: 5,
    maxN: 1_000_000_000,
    adapters: ["rustia", "koto", "rhai"],
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
    } else if (arg === "--adapters") {
      options.adapters = requiredValue(argv, ++index, arg)
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
    } else {
      fail(`unknown argument ${JSON.stringify(arg)}`);
    }
  }

  const known = new Set(adapters.map((adapter) => adapter.id));
  for (const adapter of options.adapters) {
    if (!known.has(adapter)) {
      fail(`unknown adapter ${JSON.stringify(adapter)}`);
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
    "bench_alloc",
    "bench_dealloc",
    "bench_abi_version",
    "bench_adapter_mask",
    "bench_prepare",
    "bench_run",
    "bench_result_ptr",
    "bench_result_len",
  ];
  for (const name of names) {
    if (exports[name] == null) {
      fail(`wasm export ${name} is missing`);
    }
  }
  if (exports.bench_abi_version() !== 1) {
    fail(`unsupported wasm bench ABI ${exports.bench_abi_version()}`);
  }
}

function writeString(exports, text) {
  const bytes = new TextEncoder().encode(text);
  const ptr = exports.bench_alloc(bytes.length);
  new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, len: bytes.length };
}

function readLastResult(exports) {
  const ptr = exports.bench_result_ptr();
  const len = exports.bench_result_len();
  return new TextDecoder().decode(new Uint8Array(exports.memory.buffer, ptr, len));
}

function prepare(exports, adapter, source) {
  const input = writeString(exports, source);
  try {
    const code = exports.bench_prepare(adapter.abi, input.ptr, input.len);
    if (code !== 0) {
      throw new Error(readLastResult(exports));
    }
  } finally {
    exports.bench_dealloc(input.ptr, input.len);
  }
}

function runOnce(exports, n) {
  const start = performance.now();
  const checksum = exports.bench_run(n);
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

function workloadPath(adapter, workload) {
  return `${adapter.sourceDir}/${workload.fileStem}.${adapter.extension}`;
}

async function measureWorkload(exports, adapter, workload, options) {
  const source = await readText(resolvePath(workloadPath(adapter, workload)));
  prepare(exports, adapter, source);
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
    engine: adapter.id,
    dialect: adapter.dialect,
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
const { instance } = await WebAssembly.instantiate(wasmBytes, wasmImports());
assertExports(instance.exports);

const selectedAdapters = adapters.filter((adapter) => options.adapters.includes(adapter.id));
const adapterRuns = [];
for (const adapter of selectedAdapters) {
  const rows = [];
  for (const workload of defaultWorkloads) {
    rows.push(await measureWorkload(instance.exports, adapter, workload, options));
  }
  adapterRuns.push({
    schema_version: 1,
    schema: "rustia.bench.wasm-runtime-results.v1",
    source: "script-bench-harness",
    timestamp_unix: Math.floor(Date.now() / 1000),
    commit: options.commit,
    target: targetName(runtime),
    runtime,
    adapter: {
      id: adapter.id,
      dialect: adapter.dialect,
      abi: adapter.abi,
    },
    wasm: {
      path: path.relative(cwd(), wasmPath),
      bytes: wasmBytes.byteLength,
      bench_abi_version: instance.exports.bench_abi_version(),
    },
    rows,
  });
}

const value = {
  schema_version: 1,
  schema: "rustia.bench.wasm-runtime-results-batch.v1",
  source: "script-bench-harness",
  runtime,
  wasm: {
    path: path.relative(cwd(), wasmPath),
    bytes: wasmBytes.byteLength,
    bench_abi_version: instance.exports.bench_abi_version(),
  },
  runs: adapterRuns,
};

console.log(`${JSON.stringify(value, null, 2)}\n`);

function wasmImports() {
  const placeholder = {
    __wbg_new_0_3da9e97f24fc69be: () => new Date(),
    __wbg_getTime_d6f070c088c9b5ed: (value) => Number(value.getTime()),
    __wbindgen_object_drop_ref: () => {},
    __wbg_new_cc984128914cfc6f: () => ({}),
    __wbg_getTimezoneOffset_dc9862c79e5a81a3: (value) => Number(value.getTimezoneOffset()),
    __wbindgen_object_clone_ref: (value) => value,
    __wbg_get_78f252d074a84d0b: (object, key) => object?.[key],
    __wbg_now_390768da5ee9e776: () => performance.now(),
    __wbg_now_e7c6795a7f81e10f: (value) => Number((value ?? performance).now()),
    __wbg_performance_3fcf6e32a7e1ed0a: (value) => value?.performance ?? performance,
    __wbg___wbindgen_is_undefined_c05833b95a3cf397: (value) => value === undefined,
    __wbg_static_accessor_GLOBAL_4ef717fb391d88b7: () => globalThis,
    __wbg_static_accessor_GLOBAL_THIS_8d1badc68b5a74f4: () => globalThis,
    __wbg_static_accessor_WINDOW_f2829a2234d7819e: () => undefined,
    __wbg_static_accessor_SELF_146583524fe1469b: () => globalThis,
    __wbindgen_describe: () => {},
    __wbindgen_describe_cast: () => {},
    __wbg___wbindgen_throw_344f42d3211c4765: () => {
      throw new Error("wasm-bindgen throw");
    },
    __wbg___wbindgen_debug_string_c25d447a39f5578f: () => 0,
  };
  const externref = {
    __wbindgen_externref_table_set_null: () => {},
    __wbindgen_externref_table_grow: () => 0,
  };
  return {
    __wbindgen_placeholder__: placeholder,
    __wbindgen_externref_xform__: externref,
  };
}
