#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_OUTPUT = /^bench\/cloud\/linode-cross-engine-matrix-[0-9a-f]{7,40}\.json$/;

function fail(message) {
  throw new Error(message);
}

function gitText(args) {
  return execFileSync("git", args, {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }).trim();
}

function gitBytes(commit, relative) {
  return execFileSync("git", ["show", `${commit}:${relative}`], {
    cwd: ROOT,
    encoding: null,
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 64 * 1024 * 1024,
  });
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function sourceHash(commit, relative) {
  return sha256(gitBytes(commit, relative));
}

function parseOutput(argv) {
  if (argv.length !== 2 || argv[0] !== "--output") {
    fail(
      "usage: create-linode-matrix-config.js --output bench/cloud/linode-cross-engine-matrix-COMMIT.json",
    );
  }
  const relative = argv[1].replaceAll("\\", "/");
  if (!SAFE_OUTPUT.test(relative)) fail("--output must be a safe per-commit bench/cloud JSON path");
  const output = path.resolve(ROOT, relative);
  if (path.relative(ROOT, output).startsWith("..")) fail("--output escapes the repository");
  return { relative, output };
}

// The measurement protocol, host gate, and publication contract mirror the
// last operator-frozen cross-engine matrix. Every source identity is re-pinned
// to the exact commit being measured; `history_before` must equal both that
// commit's history blob and the clean working tree, so a run can only be
// frozen from an unmodified checkout of the measured commit.
function createConfig(commit) {
  return {
    schema: "rustia.linode-fixed-benchmark.v2",
    provider: {
      region: "ap-southeast",
      image: "linode/ubuntu24.04",
      size: {
        id: "g6-dedicated-4",
        description: "Dedicated 8GB",
        class: "dedicated",
        vcpus: 4,
        memory_mib: 8192,
        disk_gib: 160,
        max_hourly_price_usd: 0.108,
      },
      max_runtime_minutes: 55,
    },
    source: {
      benchmark_commit: commit,
      expected_sha256: {
        bench_manifest: sourceHash(commit, "bench/Cargo.toml"),
        native_harness: sourceHash(commit, "bench/src/main.rs"),
        cargo_lock: sourceHash(commit, "Cargo.lock"),
        toolchain: sourceHash(commit, "rust-toolchain.toml"),
        wasm_gate: sourceHash(commit, "tools/run-wasm-gate.js"),
        wasm_dispatch_gate: sourceHash(commit, "tools/run-wasm-dispatch-gate.js"),
        wasm_adapter_manifest: sourceHash(commit, "bench/wasm/adapter-runner/Cargo.toml"),
        wasm_measure_module: sourceHash(commit, "bench/wasm/measure-rustia-wasm.mjs"),
        wasm_dispatch_measure: sourceHash(commit, "bench/wasm/measure-dispatch-wasm.js"),
        history_before: sourceHash(commit, "bench/history/runs.jsonl"),
      },
    },
    measurement: {
      protocol: "cross_engine_matrix_v4",
      native_target: "linux-x86_64",
      native_engines: [
        "rustia",
        "boa",
        "koto",
        "luaur",
        "mlua-luau",
        "mlua-luau-jit",
        "rhai",
        "roto",
        "ruau",
        "rquickjs",
        "wasmi",
        "wasmtime",
        "mlua-lua54",
      ],
      workloads: [
        "dense_tables",
        "sparse_tables",
        "closures_upvalues",
        "generic_iteration",
        "stdlib_math_table",
      ],
      native_target_ms: 120,
      native_repetitions: 5,
      wasm_runtimes: ["node"],
      wasm_adapters: ["rustia", "koto", "rhai"],
      wasm_target_ms: 80,
      wasm_repetitions: 5,
      dispatch_target_ms: 80,
      dispatch_repetitions: 5,
      runtime_artifacts: {
        node: {
          version: "22.23.1",
          url: "https://nodejs.org/download/release/v22.23.1/node-v22.23.1-linux-x64.tar.xz",
          sha256: "9749e988f437343b7fa832c69ded82a312e41a03116d766797ac14f6f9eee578",
          archive: "tar",
          binary_relative_path: "node-v22.23.1-linux-x64/bin/node",
        },
      },
    },
    host_gate: {
      benchmark_cpu: 3,
      cooldown_seconds: 30,
      idle_window_seconds: 10,
      minimum_idle_percent: 99,
      maximum_load_1m: 0.1,
      maximum_steal_ticks: 0,
      maximum_timed_steal_percent: 0.1,
      maximum_timed_steal_ticks: 1,
      maximum_timed_attempts_per_group: 3,
      wait_timeout_seconds: 300,
    },
    publication: {
      mode: "matrix",
      native_rows: 65,
      wasm_rows: 15,
      dispatch_runtimes: 1,
    },
  };
}

function main() {
  const { relative, output } = parseOutput(process.argv.slice(2));
  const commit = gitText(["rev-parse", "HEAD"]);
  if (!FULL_COMMIT.test(commit)) fail("HEAD did not resolve to a full lowercase commit");
  if (fs.existsSync(output)) fail(`refusing to overwrite ${relative}`);
  const config = createConfig(commit);
  fs.writeFileSync(output, `${JSON.stringify(config, null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
  process.stdout.write(`${relative}\n`);
}

try {
  main();
} catch (error) {
  process.stderr.write(`create-linode-matrix-config: ${error.message}\n`);
  process.exitCode = 1;
}
