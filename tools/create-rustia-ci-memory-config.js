#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_OUTPUT = /^bench\/cloud\/(linode|dedicated)-rustia-ci-memory-[0-9a-f]{7,40}\.json$/;
const DEDICATED_SCHEMA = "rustia.dedicated-memory-benchmark.v1";
// One source for the owned host's identity, shared with the history and matrix
// lanes; a second copy is a second thing to keep in step with the machine.
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

// The owned-host variant. Every measurement rule is untouched -- same groups,
// same samples, same bootstrap, same publication shape -- so a cohort measured
// here is comparable with one measured on the rented instance in everything but
// the machine. Two pinned cores, because the supervisor samples while the
// benchmark runs and sharing a core would make the supervisor part of what it
// measures; ci-bench isolates 4 and 5, which is exactly enough.
function toDedicated(config) {
  const base = JSON.parse(fs.readFileSync(path.join(ROOT, DEDICATED_HOST_CONFIG), "utf8"));
  const host = base.provider?.host;
  if (!host || typeof host.name !== "string") {
    fail(`${DEDICATED_HOST_CONFIG} does not carry a provider.host block`);
  }
  // The throughput lanes pin one core, so the shared host block says "4". This
  // protocol needs two and the machine has two: the isolated set is declared in
  // the operations policy, which is where the qualification lives and therefore
  // the honest place to widen from. Taking the second core from anywhere else
  // would be this lane asserting an isolation nobody checked.
  const policy = JSON.parse(
    fs.readFileSync(path.join(ROOT, "bench/operations-policy.json"), "utf8"),
  );
  const authority = Object.values(policy.runner_classes ?? {}).find(
    (entry) => entry.timing_authority === true,
  );
  if (!authority || authority.host !== host.name) {
    fail(`bench/operations-policy.json does not declare ${host.name} as the timing authority`);
  }
  const isolated = authority.qualification?.isolated_cpus;
  if (!Array.isArray(isolated) || isolated.length < 2) {
    fail(
      `bench/operations-policy.json isolates ${isolated?.length ?? 0} core(s) on ${host.name}; ` +
        "the memory protocol needs two, one for the benchmark and one for the supervisor that " +
        "samples it",
    );
  }
  const pinned = [...isolated].sort((left, right) => left - right).slice(0, 2);
  return {
    ...config,
    schema: DEDICATED_SCHEMA,
    provider: { host: { ...host, pinned_cpus: pinned.join(",") } },
    host_gate: {
      ...config.host_gate,
      benchmark_cpu: pinned[0],
      supervisor_cpu: pinned[1],
    },
  };
}

function parseOutput(argv) {
  const args = [...argv];
  let dedicated = false;
  const flag = args.indexOf("--dedicated");
  if (flag !== -1) {
    dedicated = true;
    args.splice(flag, 1);
  }
  if (args.length !== 2 || args[0] !== "--output") {
    fail(
      "usage: create-rustia-ci-memory-config.js [--dedicated] --output " +
        "bench/cloud/{linode,dedicated}-rustia-ci-memory-COMMIT.json",
    );
  }
  const relative = args[1].replaceAll("\\", "/");
  if (!SAFE_OUTPUT.test(relative)) fail("--output must be a safe per-commit bench/cloud JSON path");
  // The filename says which machine the config is for; it must agree with the
  // flag, or a reader takes it for the thing it is not.
  const named = relative.includes("/dedicated-rustia-ci-memory-");
  if (named !== dedicated) {
    fail(`--output name and ${dedicated ? "--dedicated" : "the rented lifecycle"} disagree`);
  }
  const output = path.resolve(ROOT, relative);
  if (path.relative(ROOT, output).startsWith("..")) fail("--output escapes the repository");
  return { relative, output, dedicated };
}


function createConfig(commit) {
  // Old commits remain reproducible as v1 configurations. Fresh execution of
  // those configs is refused by the runner; only committed v2 inputs freeze v2.
  const corrected = gitText(["ls-tree", "--name-only", commit, "--", "bench/memory-protocol.js"]) !== "";
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
        ...(corrected ? { memory_protocol: sourceHash(commit, "bench/memory-protocol.js") } : {}),
        memory_history_before: sourceHash(commit, "bench/memory/history/runs.jsonl"),
        memory_current_before: sourceHash(commit, "bench/memory/current-results.json"),
      },
    },
    measurement: {
      protocol: corrected ? "cross_engine_memory_v2" : "cross_engine_memory_v1",
      ...(corrected ? { warmup_rounds: 1, quiescence_ms: 100 } : {}),
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
    process.stderr.write(`create-rustia-ci-memory-config: ${error.message}\n`);
    process.exitCode = 1;
  }
}

// Exported so the runner's self-test can validate exactly what this writes.
module.exports = { createConfig, toDedicated, parseOutput };
