#!/usr/bin/env js
"use strict";

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
  writeErr(`measure-rustia-wasm-sm: ${message}\n`);
  quit(1);
}

function writeOut(text) {
  if (typeof print === "function") {
    print(text);
  } else {
    console.log(text);
  }
}

function writeErr(text) {
  if (typeof printErr === "function") {
    printErr(text);
  } else if (typeof console !== "undefined" && console.error) {
    console.error(text.trimEnd());
  } else {
    print(text);
  }
}

function parseArgs(argv) {
  const options = {
    wasm: "bench/wasm/adapter-runner/target/wasm32-unknown-unknown/release/script_bench_wasm_adapter.wasm",
    commit: "unknown",
    runtimeVersion: "unknown",
    host: null,
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
    } else if (arg === "--runtime-version") {
      options.runtimeVersion = requiredValue(argv, ++index, arg);
    } else if (arg === "--host") {
      options.host = requiredValue(argv, ++index, arg);
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

function readBytes(filePath) {
  return read(filePath, "binary");
}

function readText(filePath) {
  return read(filePath);
}

function runtimeInfo(options) {
  return {
    name: "spidermonkey",
    family: "SpiderMonkey",
    version: options.runtimeVersion,
    engine_version: options.runtimeVersion,
    host: hostInfo(options.host),
  };
}

function hostInfo(override) {
  if (override) return override;
  if (typeof getBuildConfiguration === "function") {
    const config = getBuildConfiguration();
    const os = config.linux ? "linux" : config.osx ? "darwin" : config.windows ? "windows" : config.android ? "android" : "unknown-os";
    const arch = config.arm64 ? "arm64" : config.x64 ? "x64" : config.x86 ? "x86" : "unknown-arch";
    return `${os}-${arch}`;
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

function encodeUtf8(text) {
  const out = [];
  for (let index = 0; index < text.length; index += 1) {
    let code = text.charCodeAt(index);
    if (code >= 0xd800 && code <= 0xdbff && index + 1 < text.length) {
      const next = text.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        index += 1;
        code = 0x10000 + ((code - 0xd800) << 10) + (next - 0xdc00);
      }
    }
    if (code <= 0x7f) {
      out.push(code);
    } else if (code <= 0x7ff) {
      out.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
    } else if (code <= 0xffff) {
      out.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    } else {
      out.push(
        0xf0 | (code >> 18),
        0x80 | ((code >> 12) & 0x3f),
        0x80 | ((code >> 6) & 0x3f),
        0x80 | (code & 0x3f),
      );
    }
  }
  return new Uint8Array(out);
}

function decodeUtf8(bytes) {
  let text = "";
  for (let index = 0; index < bytes.length;) {
    const first = bytes[index++];
    let code;
    if (first < 0x80) {
      code = first;
    } else if ((first & 0xe0) === 0xc0) {
      code = ((first & 0x1f) << 6) | (bytes[index++] & 0x3f);
    } else if ((first & 0xf0) === 0xe0) {
      code = ((first & 0x0f) << 12) | ((bytes[index++] & 0x3f) << 6) | (bytes[index++] & 0x3f);
    } else {
      code = ((first & 0x07) << 18)
        | ((bytes[index++] & 0x3f) << 12)
        | ((bytes[index++] & 0x3f) << 6)
        | (bytes[index++] & 0x3f);
    }
    if (code <= 0xffff) {
      text += String.fromCharCode(code);
    } else {
      code -= 0x10000;
      text += String.fromCharCode(0xd800 + (code >> 10), 0xdc00 + (code & 0x3ff));
    }
  }
  return text;
}

function writeString(exports, text) {
  const bytes = encodeUtf8(text);
  const ptr = exports.bench_alloc(bytes.length);
  new Uint8Array(exports.memory.buffer, ptr, bytes.length).set(bytes);
  return { ptr, len: bytes.length };
}

function readLastResult(exports) {
  const ptr = exports.bench_result_ptr();
  const len = exports.bench_result_len();
  return decodeUtf8(new Uint8Array(exports.memory.buffer, ptr, len));
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

function nowMs() {
  return performance.now();
}

function runOnce(exports, n) {
  const start = nowMs();
  const checksum = exports.bench_run(n);
  const elapsedMs = nowMs() - start;
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

function measureWorkload(exports, adapter, workload, options) {
  const source = readText(workloadPath(adapter, workload));
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

function wasmImports() {
  const placeholder = {
    __wbg_new_0_3da9e97f24fc69be: () => new Date(),
    __wbg_getTime_d6f070c088c9b5ed: (value) => Number(value.getTime()),
    __wbindgen_object_drop_ref: () => {},
    __wbg_new_cc984128914cfc6f: () => ({}),
    __wbg_getTimezoneOffset_dc9862c79e5a81a3: (value) => Number(value.getTimezoneOffset()),
    __wbindgen_object_clone_ref: (value) => value,
    __wbg_get_78f252d074a84d0b: (object, key) => object && object[key],
    __wbg_now_390768da5ee9e776: () => performance.now(),
    __wbg_now_e7c6795a7f81e10f: (value) => Number((value || performance).now()),
    __wbg_performance_3fcf6e32a7e1ed0a: (value) => value && value.performance ? value.performance : performance,
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

const options = parseArgs(scriptArgs);
const runtime = runtimeInfo(options);
const wasmBytes = readBytes(options.wasm);
const instance = new WebAssembly.Instance(new WebAssembly.Module(wasmBytes), wasmImports());
assertExports(instance.exports);

const selectedAdapters = adapters.filter((adapter) => options.adapters.includes(adapter.id));
const adapterRuns = [];
for (const adapter of selectedAdapters) {
  const rows = [];
  for (const workload of defaultWorkloads) {
    rows.push(measureWorkload(instance.exports, adapter, workload, options));
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
      path: options.wasm,
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
    path: options.wasm,
    bytes: wasmBytes.byteLength,
    bench_abi_version: instance.exports.bench_abi_version(),
  },
  runs: adapterRuns,
};

writeOut(JSON.stringify(value, null, 2));
