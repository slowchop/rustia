#!/usr/bin/env -S node
"use strict";

import { readFile } from "node:fs/promises";

const wasmPath =
  process.argv[2] ??
  "bench/wasm/adapter-runner/target/wasm32-unknown-unknown/release/script_bench_wasm_adapter.wasm";
const wasmBytes = await readFile(wasmPath);
const { instance } = await WebAssembly.instantiate(wasmBytes, wasmImports());
const exports = instance.exports;
for (const name of [
  "memory",
  "bench_alloc",
  "bench_dealloc",
  "bench_prepare_rustia_limited",
  "bench_run",
  "bench_result_ptr",
  "bench_result_len",
]) {
  if (exports[name] == null) throw new Error(`missing wasm export ${name}`);
}

const source = `
return function(_n)
    local state = {}
    if pcall(function() state[1] = string.rep("x", 1_000_000) end) then return 1 end
    if state[1] ~= nil then return 2 end
    if pcall(function() return table.create(100_000, 1) end) then return 3 end
    if pcall(function() return buffer.create(1_000_000) end) then return 4 end
    state[2] = 42
    return state[2]
end
`;
const input = new TextEncoder().encode(source);
const ptr = exports.bench_alloc(input.length);
new Uint8Array(exports.memory.buffer, ptr, input.length).set(input);
const pagesBefore = exports.memory.buffer.byteLength / 65_536;
try {
  const status = exports.bench_prepare_rustia_limited(ptr, input.length, 512 * 1024);
  if (status !== 0) throw new Error(readLastResult(exports));
} finally {
  exports.bench_dealloc(ptr, input.length);
}
const result = exports.bench_run(1);
if (result !== 42) {
  throw new Error(`limited wasm workload returned ${result}: ${readLastResult(exports)}`);
}
const pagesAfter = exports.memory.buffer.byteLength / 65_536;
console.log(
  JSON.stringify({
    status: "ok",
    target: "wasm32-unknown-unknown",
    memory_limit_bytes: 512 * 1024,
    result,
    linear_memory_pages_before: pagesBefore,
    linear_memory_pages_after: pagesAfter,
  }),
);

function readLastResult(wasmExports) {
  const resultPtr = wasmExports.bench_result_ptr();
  const resultLen = wasmExports.bench_result_len();
  return new TextDecoder().decode(
    new Uint8Array(wasmExports.memory.buffer, resultPtr, resultLen),
  );
}

function wasmImports() {
  const placeholder = {
    __wbg_new_0_3da9e97f24fc69be: () => new Date(),
    __wbg_getTime_d6f070c088c9b5ed: (value) => Number(value.getTime()),
    __wbindgen_object_drop_ref: () => {},
    __wbg_new_cc984128914cfc6f: () => ({}),
    __wbg_getTimezoneOffset_dc9862c79e5a81a3: (value) =>
      Number(value.getTimezoneOffset()),
    __wbindgen_object_clone_ref: (value) => value,
    __wbg_get_78f252d074a84d0b: (object, key) => object?.[key],
    __wbg_now_390768da5ee9e776: () => performance.now(),
    __wbg_now_e7c6795a7f81e10f: (value) =>
      Number((value ?? performance).now()),
    __wbg_performance_3fcf6e32a7e1ed0a: (value) =>
      value?.performance ?? performance,
    __wbg___wbindgen_is_undefined_c05833b95a3cf397: (value) =>
      value === undefined,
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
  return {
    __wbindgen_placeholder__: placeholder,
    __wbindgen_externref_xform__: {
      __wbindgen_externref_table_set_null: () => {},
      __wbindgen_externref_table_grow: () => 0,
    },
  };
}
