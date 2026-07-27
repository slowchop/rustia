#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_OUTPUT = /^bench\/cloud\/linode-rustia-ci-memory-[0-9a-f]{7,40}\.json$/;

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
    fail("usage: create-rustia-ci-memory-config.js --output bench/cloud/linode-rustia-ci-memory-COMMIT.json");
  }
  const relative = argv[1].replaceAll("\\", "/");
  if (!SAFE_OUTPUT.test(relative)) fail("--output must be a safe per-commit bench/cloud JSON path");
  const output = path.resolve(ROOT, relative);
  if (path.relative(ROOT, output).startsWith("..")) fail("--output escapes the repository");
  return { relative, output };
}

function createConfig(commit) {
  const workloads = [
    "dense_tables",
    "sparse_tables",
    "closures_upvalues",
    "generic_iteration",
    "stdlib_math_table",
  ].map((id) => ({ id, n: 128, epochs: 16 }));
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
      max_runtime_minutes: 42,
    },
    source: {
      benchmark_commit: commit,
      expected_sha256: {
        bench_manifest: sourceHash(commit, "bench/Cargo.toml"),
        native_harness: sourceHash(commit, "bench/src/main.rs"),
        cargo_lock: sourceHash(commit, "Cargo.lock"),
        toolchain: sourceHash(commit, "rust-toolchain.toml"),
        memory_supervisor: sourceHash(commit, "bench/run-memory-supervisor.js"),
        memory_analyzer: sourceHash(commit, "bench/analyze-memory-samples.js"),
        memory_cohort: sourceHash(commit, "bench/run-memory-cohort.js"),
        memory_publisher: sourceHash(commit, "bench/publish-memory-analysis.js"),
        memory_history_before: sourceHash(commit, "bench/memory/history/runs.jsonl"),
        memory_current_before: sourceHash(commit, "bench/memory/current-results.json"),
      },
    },
    measurement: {
      protocol: "cross_engine_memory_v1",
      cohort_id: `rustia-ci-${commit.slice(0, 12)}`,
      samples: 9,
      bootstrap_repetitions: 20000,
      bootstrap_seed: 1381323604,
      growth_thresholds: {
        absolute_kib_per_epoch: 4,
        proportional_percent_per_epoch: 0.05,
      },
      sample_interval_ms: 10,
      sample_timeout_ms: 600000,
      groups: [
        {
          binary: "all",
          engines: ["rustia"],
          workloads,
        },
        {
          binary: "all",
          engines: ["rustia"],
          workloads: [
            { id: "memory_burst", n: 1, epochs: 16 },
            { id: "memory_churn", n: 1, epochs: 40 },
            { id: "memory_retained_control", n: 1, epochs: 40 },
            { id: "todo_steady", n: 128, epochs: 40 },
            { id: "todo_history", n: 128, epochs: 40 },
            { id: "todo_retained_control", n: 128, epochs: 40 },
          ],
        },
      ],
    },
    host_gate: {
      benchmark_cpu: 3,
      supervisor_cpu: 2,
      idle_window_seconds: 10,
      minimum_idle_percent: 99,
      maximum_load_1m: 0.1,
      maximum_idle_steal_ticks: 0,
      maximum_run_steal_percent: 0.1,
      wait_timeout_seconds: 300,
    },
    publication: {
      mode: "rustia_ci_memory",
      rows: 11,
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
  process.stderr.write(`create-rustia-ci-memory-config: ${error.message}\n`);
  process.exitCode = 1;
}
