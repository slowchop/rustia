#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_OUTPUT = /^bench\/cloud\/(linode|dedicated)-cross-engine-matrix-[0-9a-f]{7,40}\.json$/;
const DEDICATED_SCHEMA = "rustia.dedicated-cross-engine-matrix.v1";
// The owned host's identity has exactly one source in the repository. The
// history lane already reads it from here; the matrix reading it from anywhere
// else would let the two lanes disagree about which machine they measured on.
const DEDICATED_HOST_CONFIG = "bench/cloud/dedicated-core2-tape-sparse.json";

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
  const args = [...argv];
  let dedicated = false;
  const dedicatedIndex = args.indexOf("--dedicated");
  if (dedicatedIndex !== -1) {
    dedicated = true;
    args.splice(dedicatedIndex, 1);
  }
  if (args.length !== 2 || args[0] !== "--output") {
    fail(
      "usage: create-linode-matrix-config.js [--dedicated] --output " +
        "bench/cloud/{linode,dedicated}-cross-engine-matrix-COMMIT.json",
    );
  }
  const relative = args[1].replaceAll("\\", "/");
  if (!SAFE_OUTPUT.test(relative)) fail("--output must be a safe per-commit bench/cloud JSON path");
  // The filename says which machine the config is for, so it must agree with
  // the flag: a dedicated config sitting under a linode- name would be read by
  // a human as the thing it is not.
  const named = relative.includes("/dedicated-cross-engine-matrix-");
  if (named !== dedicated) {
    fail(`--output name and ${dedicated ? "--dedicated" : "the rented lifecycle"} disagree`);
  }
  const output = path.resolve(ROOT, relative);
  if (path.relative(ROOT, output).startsWith("..")) fail("--output escapes the repository");
  return { relative, output, dedicated };
}

// The owned-host variant of the same protocol. Everything about the measurement
// is untouched -- same engines, workloads, repetitions, runtime artifacts and
// publication dimensions -- so a cohort measured here is comparable with one
// measured on the rented instance in every respect except the machine. Only the
// host description and the steal ceilings change, and the ceilings tighten:
// steal is structurally zero on bare metal, so a non-zero reading is a fault
// rather than a tolerance.
function toDedicated(config) {
  const base = JSON.parse(fs.readFileSync(path.join(ROOT, DEDICATED_HOST_CONFIG), "utf8"));
  const host = base.provider?.host;
  if (!host || typeof host.name !== "string") {
    fail(`${DEDICATED_HOST_CONFIG} does not carry a provider.host block`);
  }
  return {
    ...config,
    schema: DEDICATED_SCHEMA,
    provider: {
      host: {
        ...host,
        // Tighter than the shared host block's general ceiling, and tighter than
        // the workflow's job timeout on purpose: whichever bound is smaller is
        // the one that produces the error message, and the runner's names the
        // phase it was in. A job killed at its timeout says only that it ran
        // out of time.
        max_runtime_minutes: 90,
      },
    },
    host_gate: {
      ...config.host_gate,
      benchmark_cpu: base.host_gate.benchmark_cpu,
      cooldown_seconds: base.host_gate.cooldown_seconds,
      maximum_timed_steal_percent: 0,
      maximum_timed_steal_ticks: 0,
    },
  };
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
  const { relative, output, dedicated } = parseOutput(process.argv.slice(2));
  const commit = gitText(["rev-parse", "HEAD"]);
  if (!FULL_COMMIT.test(commit)) fail("HEAD did not resolve to a full lowercase commit");
  if (fs.existsSync(output)) fail(`refusing to overwrite ${relative}`);
  const config = dedicated ? toDedicated(createConfig(commit)) : createConfig(commit);
  fs.writeFileSync(output, `${JSON.stringify(config, null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
  process.stdout.write(`${relative}\n`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`create-linode-matrix-config: ${error.message}\n`);
    process.exitCode = 1;
  }
}

// Exported so the runner's self-test can validate exactly what this writes,
// rather than a hand-built fixture that only resembles it.
module.exports = { createConfig, toDedicated, parseOutput };
