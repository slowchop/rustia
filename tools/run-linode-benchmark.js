#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const zlib = require("node:zlib");
const { spawn, spawnSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const DEFAULT_CONFIG = path.join(REPO, "bench/cloud/linode-dense-ed65077.json");
const REMOTE_RUNNER = path.join(REPO, "tools/run-linux-fixed-benchmark.js");
const MATRIX_REMOTE_RUNNER = path.join(REPO, "tools/run-linux-matrix-benchmark.js");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/linode");
const RUNS_ROOT = path.join(STATE_ROOT, "runs");
const TOKEN_FILE = path.join(STATE_ROOT, "token");
const COORDINATOR_LOCK = path.join(STATE_ROOT, "coordinator.lock");
const API_HOST = "api.linode.com";
const API_TIMEOUT_MS = 30 * 1000;
const CREATE_RECONCILE_TIMEOUT_MS = 2 * 60 * 1000;
const DELETE_CONFIRM_TIMEOUT_MS = 4 * 60 * 1000;
const CLEANUP_RESERVE_MS = 8 * 60 * 1000;
const FORCE_KILL_DELAY_MS = 10 * 1000;
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/;
const SAFE_WORKLOAD_FILE_COMPONENT = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const RUN_ID = /^\d{8}T\d{6}Z-[0-9a-f]{8}$/;
const MAX_EXPORT_ARCHIVE_BYTES = 256 * 1024 * 1024;
const MAX_EXPORT_UNCOMPRESSED_BYTES = 512 * 1024 * 1024;
const MAX_EXPORT_ENTRIES = 10_000;
const FIXED_THROUGHPUT_PROTOCOL = "fixed_throughput_v1";
const VM_STRING_MEMORY_PROTOCOL = "vm_string_memory_v1";
const CROSS_ENGINE_MATRIX_PROTOCOL = "cross_engine_matrix_v3";
const MAX_TIMED_ATTEMPTS_PER_GROUP = 3;
const ALLOWED_PROTOCOLS = new Set([
  FIXED_THROUGHPUT_PROTOCOL,
  VM_STRING_MEMORY_PROTOCOL,
  CROSS_ENGINE_MATRIX_PROTOCOL,
]);
const MATRIX_NATIVE_TARGET = "linux-x86_64";
const MATRIX_NATIVE_ENGINES = Object.freeze([
  "rustia",
  "boa",
  "koto",
  "mlua-luau",
  "rhai",
  "roto",
  "rquickjs",
  "wasmi",
  "wasmtime",
  "mlua-lua54",
]);
const MATRIX_WORKLOADS = Object.freeze([
  "dense_tables",
  "sparse_tables",
  "closures_upvalues",
  "generic_iteration",
  "stdlib_math_table",
]);
const MATRIX_WASM_RUNTIMES = Object.freeze(["node"]);
const MATRIX_WASM_ADAPTERS = Object.freeze(["rustia", "koto", "rhai"]);
const MATRIX_RUNTIME_FAMILIES = Object.freeze({
  node: "V8",
});
const VM_STRING_SCENARIOS = Object.freeze([
  "empty",
  "short_unloaded",
  "short_loaded",
  "long_unloaded",
  "long_loaded",
]);
const VM_STRING_EVIDENCE_FILES = Object.freeze([
  "analysis.json",
  "gate.jsonl",
  "host.json",
  "manifest.json",
  "protocol.json",
  "provider.json",
  "remote-run.log",
  "samples.jsonl",
  "warmup-samples.jsonl",
]);
const VM_STRING_HASHED_EVIDENCE_FILES = Object.freeze(
  VM_STRING_EVIDENCE_FILES.filter((file) => file !== "manifest.json"),
);

let activeChild = null;
let activeRequest = null;
let activeForceKillTimer = null;
let interruptedSignal = null;
let cleanupInProgress = false;

class LinodeApiError extends Error {
  constructor(message, status = null, response = null) {
    super(message);
    this.name = "LinodeApiError";
    this.status = status;
    this.response = response;
  }
}

function fail(message) {
  throw new Error(message);
}

function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode !== null) return;
  child.kill(signal);
  if (!activeForceKillTimer) {
    activeForceKillTimer = setTimeout(() => {
      if (child.exitCode === null) child.kill("SIGKILL");
    }, FORCE_KILL_DELAY_MS);
  }
}

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    interruptedSignal = interruptedSignal || signal;
    if (cleanupInProgress) return;
    if (activeRequest) activeRequest.destroy(new Error(`interrupted by ${signal}`));
    if (activeChild && activeChild.exitCode === null) terminateChild(activeChild, signal);
  });
}

function throwIfInterrupted() {
  if (interruptedSignal && !cleanupInProgress) {
    fail(`cloud benchmark interrupted by ${interruptedSignal}`);
  }
}

function requireObject(value, location) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${location} must be an object`);
  }
  return value;
}

function requireString(value, location) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${location} must be a non-empty string`);
  }
  return value;
}

function requireInteger(value, location, minimum = 0) {
  if (!Number.isSafeInteger(value) || value < minimum) {
    fail(`${location} must be an integer >= ${minimum}`);
  }
  return value;
}

function requireNumber(value, location, minimum = 0) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < minimum) {
    fail(`${location} must be a finite number >= ${minimum}`);
  }
  return value;
}

function requireExactKeys(value, expected, location) {
  const actual = Object.keys(value).sort();
  const wanted = expected.slice().sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(`${location} keys must be ${wanted.join(", ")}; got ${actual.join(", ")}`);
  }
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function protocolKind(config) {
  const measurement = requireObject(config.measurement, "config.measurement");
  const kind = measurement.protocol === undefined ? FIXED_THROUGHPUT_PROTOCOL : measurement.protocol;
  if (!ALLOWED_PROTOCOLS.has(kind)) {
    fail(`config.measurement.protocol must be one of ${[...ALLOWED_PROTOCOLS].join(", ")}`);
  }
  return kind;
}

function isVmStringMemory(config) {
  return protocolKind(config) === VM_STRING_MEMORY_PROTOCOL;
}

function isCrossEngineMatrix(config) {
  return protocolKind(config) === CROSS_ENGINE_MATRIX_PROTOCOL;
}

function benchmarkCommit(config) {
  return isCrossEngineMatrix(config)
    ? config.source.benchmark_commit
    : config.source.candidate_commit;
}

function remoteRunnerForConfig(config) {
  return isCrossEngineMatrix(config) ? MATRIX_REMOTE_RUNNER : REMOTE_RUNNER;
}

function requireFrozenArray(value, frozen, location) {
  if (canonicalJson(value) !== canonicalJson(frozen)) {
    fail(`${location} must be exactly ${frozen.join(", ")}`);
  }
}

function validateRuntimeArtifacts(value, location) {
  const artifacts = requireObject(value, location);
  requireExactKeys(artifacts, MATRIX_WASM_RUNTIMES, location);
  for (const runtime of MATRIX_WASM_RUNTIMES) {
    const artifact = requireObject(artifacts[runtime], `${location}.${runtime}`);
    requireExactKeys(
      artifact,
      ["version", "url", "sha256", "archive", "binary_relative_path"],
      `${location}.${runtime}`,
    );
    requireString(artifact.version, `${location}.${runtime}.version`);
    const url = requireString(artifact.url, `${location}.${runtime}.url`);
    let parsed;
    try {
      parsed = new URL(url);
    } catch (error) {
      fail(`${location}.${runtime}.url must be an absolute URL: ${error.message}`);
    }
    if (parsed.protocol !== "https:") fail(`${location}.${runtime}.url must use HTTPS`);
    if (!SHA256.test(requireString(artifact.sha256, `${location}.${runtime}.sha256`))) {
      fail(`${location}.${runtime}.sha256 must be a lowercase SHA-256`);
    }
    if (!["tar", "zip"].includes(artifact.archive)) {
      fail(`${location}.${runtime}.archive must be tar or zip`);
    }
    const binary = requireString(
      artifact.binary_relative_path,
      `${location}.${runtime}.binary_relative_path`,
    );
    if (path.isAbsolute(binary) || binary.split(/[\\/]+/).includes("..")) {
      fail(`${location}.${runtime}.binary_relative_path must be a safe relative path`);
    }
  }
}

function validateConfig(value) {
  const root = requireObject(value, "config");
  const kind = protocolKind(root);
  requireExactKeys(
    root,
    kind === VM_STRING_MEMORY_PROTOCOL || kind === CROSS_ENGINE_MATRIX_PROTOCOL
      ? ["schema", "provider", "source", "measurement", "host_gate", "publication"]
      : ["schema", "provider", "source", "measurement", "qualification", "host_gate", "publication"],
    "config",
  );
  if (root.schema !== "rustia.linode-fixed-benchmark.v2") {
    fail("unsupported Linode benchmark config schema");
  }

  const provider = requireObject(root.provider, "config.provider");
  requireExactKeys(
    provider,
    ["region", "image", "size", "max_runtime_minutes"],
    "config.provider",
  );
  requireString(provider.region, "config.provider.region");
  requireString(provider.image, "config.provider.image");
  requireInteger(provider.max_runtime_minutes, "config.provider.max_runtime_minutes", 1);
  const size = requireObject(provider.size, "config.provider.size");
  requireExactKeys(
    size,
    ["id", "description", "class", "vcpus", "memory_mib", "disk_gib", "max_hourly_price_usd"],
    "config.provider.size",
  );
  requireString(size.id, "config.provider.size.id");
  requireString(size.description, "config.provider.size.description");
  requireString(size.class, "config.provider.size.class");
  requireInteger(size.vcpus, "config.provider.size.vcpus", 2);
  requireInteger(size.memory_mib, "config.provider.size.memory_mib", 1);
  requireInteger(size.disk_gib, "config.provider.size.disk_gib", 1);
  requireNumber(size.max_hourly_price_usd, "config.provider.size.max_hourly_price_usd", 0.001);
  if (provider.max_runtime_minutes >= 60) {
    fail("config.provider.max_runtime_minutes must remain below Linode's first whole billing hour");
  }

  const source = requireObject(root.source, "config.source");
  requireExactKeys(
    source,
    kind === CROSS_ENGINE_MATRIX_PROTOCOL
      ? ["benchmark_commit", "expected_sha256"]
      : ["baseline_commit", "candidate_commit", "harness_commit", "expected_sha256"],
    "config.source",
  );
  if (kind === CROSS_ENGINE_MATRIX_PROTOCOL) {
    if (!FULL_COMMIT.test(requireString(source.benchmark_commit, "config.source.benchmark_commit"))) {
      fail("config.source.benchmark_commit must be a lowercase full commit");
    }
  } else {
    for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
      if (!FULL_COMMIT.test(requireString(source[field], `config.source.${field}`))) {
        fail(`config.source.${field} must be a lowercase full commit`);
      }
    }
    if (source.baseline_commit === source.candidate_commit) {
      fail("baseline and candidate commits must differ");
    }
  }
  const expected = requireObject(source.expected_sha256, "config.source.expected_sha256");
  requireExactKeys(
    expected,
    kind === CROSS_ENGINE_MATRIX_PROTOCOL
      ? [
          "bench_manifest",
          "native_harness",
          "cargo_lock",
          "toolchain",
          "wasm_gate",
          "wasm_dispatch_gate",
          "wasm_adapter_manifest",
          "wasm_measure_module",
          "wasm_dispatch_measure",
          "history_before",
        ]
      : kind === VM_STRING_MEMORY_PROTOCOL
      ? [
          "baseline_original_bench_manifest",
          "candidate_original_bench_manifest",
          "overlay_bench_manifest",
          "sampler",
          "analyzer",
          "cargo_lock",
          "toolchain",
        ]
      : [
          "baseline_original_harness",
          "overlay_harness",
          "workload",
          "cargo_lock",
          "toolchain",
          "analyzer",
          "history_before",
        ],
    "config.source.expected_sha256",
  );
  for (const [field, hash] of Object.entries(expected)) {
    if (!SHA256.test(requireString(hash, `config.source.expected_sha256.${field}`))) {
      fail(`config.source.expected_sha256.${field} must be a lowercase SHA-256`);
    }
  }
  const measurement = requireObject(root.measurement, "config.measurement");
  if (kind === CROSS_ENGINE_MATRIX_PROTOCOL) {
    requireExactKeys(
      measurement,
      [
        "protocol",
        "native_target",
        "native_engines",
        "workloads",
        "native_target_ms",
        "native_repetitions",
        "wasm_runtimes",
        "wasm_adapters",
        "wasm_target_ms",
        "wasm_repetitions",
        "dispatch_target_ms",
        "dispatch_repetitions",
        "runtime_artifacts",
      ],
      "config.measurement",
    );
    if (measurement.native_target !== MATRIX_NATIVE_TARGET) {
      fail(`config.measurement.native_target must be ${MATRIX_NATIVE_TARGET}`);
    }
    requireFrozenArray(measurement.native_engines, MATRIX_NATIVE_ENGINES, "config.measurement.native_engines");
    requireFrozenArray(measurement.workloads, MATRIX_WORKLOADS, "config.measurement.workloads");
    requireFrozenArray(measurement.wasm_runtimes, MATRIX_WASM_RUNTIMES, "config.measurement.wasm_runtimes");
    requireFrozenArray(measurement.wasm_adapters, MATRIX_WASM_ADAPTERS, "config.measurement.wasm_adapters");
    for (const field of ["native_target_ms", "wasm_target_ms", "dispatch_target_ms"]) {
      requireInteger(measurement[field], `config.measurement.${field}`, 1);
    }
    for (const field of ["native_repetitions", "wasm_repetitions", "dispatch_repetitions"]) {
      if (requireInteger(measurement[field], `config.measurement.${field}`, 1) !== 5) {
        fail(`config.measurement.${field} must remain frozen at 5`);
      }
    }
    validateRuntimeArtifacts(measurement.runtime_artifacts, "config.measurement.runtime_artifacts");
  } else if (kind === VM_STRING_MEMORY_PROTOCOL) {
    requireExactKeys(
      measurement,
      [
        "protocol",
        "vm_count",
        "accepted_rounds",
        "warmup_rounds",
        "round_retry_limit",
        "scenarios",
        "bootstrap_repetitions",
        "bootstrap_seed",
      ],
      "config.measurement",
    );
    if (root.schema !== "rustia.linode-fixed-benchmark.v2") {
      fail("vm_string_memory_v1 is supported only by the Linode v2 lifecycle");
    }
    const frozenIntegers = {
      vm_count: 512,
      accepted_rounds: 16,
      warmup_rounds: 1,
      round_retry_limit: 3,
      bootstrap_repetitions: 100000,
      bootstrap_seed: 1381323604,
    };
    for (const [field, frozen] of Object.entries(frozenIntegers)) {
      if (requireInteger(measurement[field], `config.measurement.${field}`, field === "bootstrap_seed" ? 0 : 1) !== frozen) {
        fail(`config.measurement.${field} must remain frozen at ${frozen}`);
      }
    }
    if (canonicalJson(measurement.scenarios) !== canonicalJson(VM_STRING_SCENARIOS)) {
      fail(`config.measurement.scenarios must be exactly ${VM_STRING_SCENARIOS.join(", ")}`);
    }
  } else {
  requireExactKeys(
    measurement,
    [
      ...(measurement.protocol === undefined ? [] : ["protocol"]),
      "workload",
      "workload_path",
      "fixed_n",
      "repetitions",
      "expected_checksum",
      "run_order",
      "bootstrap_repetitions",
      "bootstrap_seed",
      "material_regression_threshold_percent",
      "expected_legs_per_group",
    ],
    "config.measurement",
  );
  requireString(measurement.workload, "config.measurement.workload");
  const workloadPath = requireString(measurement.workload_path, "config.measurement.workload_path");
  if (path.isAbsolute(workloadPath) || workloadPath.split("/").includes("..")) {
    fail("config.measurement.workload_path must be repository-relative");
  }
  requireInteger(measurement.fixed_n, "config.measurement.fixed_n", 1);
  requireInteger(measurement.repetitions, "config.measurement.repetitions", 1);
  requireString(measurement.expected_checksum, "config.measurement.expected_checksum");
  requireInteger(measurement.bootstrap_repetitions, "config.measurement.bootstrap_repetitions", 1000);
  requireInteger(measurement.bootstrap_seed, "config.measurement.bootstrap_seed", 0);
  requireNumber(
    measurement.material_regression_threshold_percent,
    "config.measurement.material_regression_threshold_percent",
    0,
  );
  const expectedLegs = requireInteger(
    measurement.expected_legs_per_group,
    "config.measurement.expected_legs_per_group",
    1,
  );
  if (!Array.isArray(measurement.run_order) || measurement.run_order.length !== expectedLegs * 2) {
    fail("config.measurement.run_order must contain exactly two groups of expected legs");
  }
  const seenFiles = new Set();
  const counts = { baseline: 0, candidate: 0 };
  for (const [index, rawLeg] of measurement.run_order.entries()) {
    const leg = requireObject(rawLeg, `config.measurement.run_order[${index}]`);
    requireExactKeys(leg, ["variant", "file"], `config.measurement.run_order[${index}]`);
    if (!(leg.variant in counts)) {
      fail(`config.measurement.run_order[${index}].variant must be baseline or candidate`);
    }
    if (!SAFE_FILE.test(requireString(leg.file, `config.measurement.run_order[${index}].file`))) {
      fail(`config.measurement.run_order[${index}].file must be a safe JSON basename`);
    }
    if (seenFiles.has(leg.file)) fail(`duplicate run-order file ${leg.file}`);
    seenFiles.add(leg.file);
    counts[leg.variant] += 1;
  }
  if (counts.baseline !== expectedLegs || counts.candidate !== expectedLegs) {
    fail("run order must contain the declared number of baseline and candidate legs");
  }

  }

  if (kind === FIXED_THROUGHPUT_PROTOCOL) {
    const qualification = requireObject(root.qualification, "config.qualification");
    requireExactKeys(
      qualification,
      ["candidate_pilot_legs", "max_median_spread_percent"],
      "config.qualification",
    );
    requireInteger(qualification.candidate_pilot_legs, "config.qualification.candidate_pilot_legs", 2);
    requireNumber(
      qualification.max_median_spread_percent,
      "config.qualification.max_median_spread_percent",
      0,
    );
  }

  const gate = requireObject(root.host_gate, "config.host_gate");
  requireExactKeys(
    gate,
    [
      "benchmark_cpu",
      "cooldown_seconds",
      "idle_window_seconds",
      "minimum_idle_percent",
      "maximum_load_1m",
      "maximum_steal_ticks",
      "maximum_timed_steal_percent",
      ...(kind === CROSS_ENGINE_MATRIX_PROTOCOL
        ? ["maximum_timed_steal_ticks", "maximum_timed_attempts_per_group"]
        : []),
      "wait_timeout_seconds",
    ],
    "config.host_gate",
  );
  requireInteger(gate.benchmark_cpu, "config.host_gate.benchmark_cpu", 0);
  requireInteger(gate.cooldown_seconds, "config.host_gate.cooldown_seconds", 1);
  requireInteger(gate.idle_window_seconds, "config.host_gate.idle_window_seconds", 1);
  requireNumber(gate.minimum_idle_percent, "config.host_gate.minimum_idle_percent", 0);
  if (gate.minimum_idle_percent > 100) fail("config.host_gate.minimum_idle_percent must be <= 100");
  requireNumber(gate.maximum_load_1m, "config.host_gate.maximum_load_1m", 0);
  requireInteger(gate.maximum_steal_ticks, "config.host_gate.maximum_steal_ticks", 0);
  requireNumber(gate.maximum_timed_steal_percent, "config.host_gate.maximum_timed_steal_percent", 0);
  if (gate.maximum_steal_ticks !== 0 || gate.maximum_timed_steal_percent !== 0.1) {
    fail("Linode v2 steal budget must remain zero while idle and 0.1% during a timed leg");
  }
  if (kind === CROSS_ENGINE_MATRIX_PROTOCOL) {
    if (
      requireInteger(
        gate.maximum_timed_steal_ticks,
        "config.host_gate.maximum_timed_steal_ticks",
        0,
      ) !== 1
    ) {
      fail("config.host_gate.maximum_timed_steal_ticks must be one for the matrix protocol");
    }
    const maximumTimedAttempts = requireInteger(
      gate.maximum_timed_attempts_per_group,
      "config.host_gate.maximum_timed_attempts_per_group",
      1,
    );
    if (maximumTimedAttempts > MAX_TIMED_ATTEMPTS_PER_GROUP) {
      fail(
        `config.host_gate.maximum_timed_attempts_per_group must be <= ${MAX_TIMED_ATTEMPTS_PER_GROUP}`,
      );
    }
  }
  const waitTimeout = requireInteger(
    gate.wait_timeout_seconds,
    "config.host_gate.wait_timeout_seconds",
    1,
  );
  if (kind === CROSS_ENGINE_MATRIX_PROTOCOL && waitTimeout !== 300) {
    fail("config.host_gate.wait_timeout_seconds must be 300 for the matrix protocol");
  }

  const publication = requireObject(root.publication, "config.publication");
  if (kind === CROSS_ENGINE_MATRIX_PROTOCOL) {
    requireExactKeys(
      publication,
      ["mode", "native_rows", "wasm_rows", "dispatch_runtimes"],
      "config.publication",
    );
    if (publication.mode !== "matrix") fail("cross_engine_matrix_v3 publication.mode must be matrix");
    const expectedCounts = {
      native_rows: MATRIX_NATIVE_ENGINES.length * MATRIX_WORKLOADS.length,
      wasm_rows: MATRIX_WASM_RUNTIMES.length * MATRIX_WASM_ADAPTERS.length * MATRIX_WORKLOADS.length,
      dispatch_runtimes: MATRIX_WASM_RUNTIMES.length,
    };
    for (const [field, wanted] of Object.entries(expectedCounts)) {
      if (requireInteger(publication[field], `config.publication.${field}`, 1) !== wanted) {
        fail(`config.publication.${field} must remain frozen at ${wanted}`);
      }
    }
  } else if (kind === VM_STRING_MEMORY_PROTOCOL) {
    requireExactKeys(publication, ["mode"], "config.publication");
    if (publication.mode !== "evidence_only") {
      fail("vm_string_memory_v1 publication.mode must be evidence_only");
    }
  } else {
    requireExactKeys(publication, ["mode", "required_verdict", "final_measurements"], "config.publication");
    if (publication.mode !== "acceptance") fail("Linode coordinator is acceptance-only");
    if (publication.required_verdict !== "material_regression_excluded") {
      fail("config.publication.required_verdict must be material_regression_excluded");
    }
    requireInteger(publication.final_measurements, "config.publication.final_measurements", 3);
  }
  return root;
}

function pathIsInside(directory, candidate) {
  const relative = path.relative(directory, candidate);
  return relative !== "" && relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function resolveConfigFile(argument, { repo = REPO, defaultConfig = DEFAULT_CONFIG } = {}) {
  let file;
  if (argument === undefined) {
    file = path.resolve(defaultConfig);
  } else {
    const relative = requireString(argument, "Linode benchmark config path");
    if (path.isAbsolute(relative) || relative.split(/[\\/]+/).includes("..")) {
      fail("Linode benchmark config path must be repository-relative and contain no .. segments");
    }
    file = path.resolve(repo, relative);
  }

  const resolvedRepo = fs.realpathSync(repo);
  if (!pathIsInside(path.resolve(repo), file)) {
    fail("Linode benchmark config path must resolve inside the repository");
  }

  let metadata;
  try {
    metadata = fs.lstatSync(file);
  } catch (error) {
    fail(`cannot inspect Linode benchmark config ${path.relative(repo, file)}: ${error.message}`);
  }
  if (metadata.isSymbolicLink()) fail("Linode benchmark config must not be a symbolic link");
  if (!metadata.isFile()) fail("Linode benchmark config must be a regular file");

  let realFile;
  try {
    realFile = fs.realpathSync(file);
  } catch (error) {
    fail(`cannot resolve Linode benchmark config ${path.relative(repo, file)}: ${error.message}`);
  }
  if (!pathIsInside(resolvedRepo, realFile)) {
    fail("Linode benchmark config path must resolve inside the repository");
  }
  return file;
}

function loadConfig(file = DEFAULT_CONFIG) {
  try {
    return validateConfig(JSON.parse(fs.readFileSync(file, "utf8")));
  } catch (error) {
    fail(`cannot load ${path.relative(REPO, file)}: ${error.message}`);
  }
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function workloadFileComponent(workload) {
  const value = requireString(workload, "benchmark workload");
  if (SAFE_WORKLOAD_FILE_COMPONENT.test(value)) return value;
  const slug = value
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[-._]+|[-._]+$/g, "")
    .slice(0, 48)
    .replace(/[-._]+$/g, "");
  return `${slug || "workload"}-${sha256(Buffer.from(value, "utf8")).slice(0, 12)}`;
}

function finalSidecarName(config, index) {
  requireInteger(index, "final sidecar index", 1);
  const file = `final-${workloadFileComponent(config.measurement.workload)}-${index}.json`;
  if (!SAFE_FILE.test(file)) fail("generated final sidecar name is not a safe JSON basename");
  return file;
}

function rotatedMemoryScenarios(round, scenarios = VM_STRING_SCENARIOS) {
  requireInteger(round, "memory round", 1);
  // Consecutive rounds invert the first build while retaining the same scenario
  // rotation. This makes every scenario's baseline/candidate position sum exact
  // within each pair instead of relying on the 40-round joint cycle.
  const rotation = Math.floor((round - 1) / 2) % scenarios.length;
  return [...scenarios.slice(rotation), ...scenarios.slice(0, rotation)];
}

function memoryRoundOrder(round, scenarios = VM_STRING_SCENARIOS) {
  const firstBuildCycle = [
    "baseline",
    "candidate",
    "candidate",
    "baseline",
    "candidate",
    "baseline",
    "baseline",
    "candidate",
  ];
  const firstRole = firstBuildCycle[(round - 1) % firstBuildCycle.length];
  const secondRole = firstRole === "baseline" ? "candidate" : "baseline";
  const firstScenarios = rotatedMemoryScenarios(round, scenarios);
  return [
    ...firstScenarios.map((scenario) => ({ role: firstRole, scenario })),
    ...[...firstScenarios].reverse().map((scenario) => ({ role: secondRole, scenario })),
  ];
}

function memorySampleId(prefix, round, role, scenario) {
  if (!/^(warmup|round)$/.test(prefix)) fail("memory sample prefix is invalid");
  if (!/^(baseline|candidate)$/.test(role) || !VM_STRING_SCENARIOS.includes(scenario)) {
    fail("memory sample identity is invalid");
  }
  return `${prefix}-${String(round).padStart(2, "0")}-${role}-${scenario.replaceAll("_", "-")}`;
}

function addFreshMemoryProcessId(processIds, pid, location) {
  requireInteger(pid, `${location} process ID`, 1);
  if (processIds.has(pid)) {
    fail(`${location} reuses process ID ${pid}; every sample must be a fresh process`);
  }
  processIds.add(pid);
}

function memoryEvidenceRelativePath(commit) {
  if (!FULL_COMMIT.test(commit)) fail("memory evidence commit must be a lowercase full commit");
  return path.join("bench/evidence/vm-string-memory", commit);
}

function memoryPromotionLayout(config, downloadDirectory) {
  if (!isVmStringMemory(config)) fail("memory promotion layout requires vm_string_memory_v1");
  const relativeEvidence = memoryEvidenceRelativePath(config.source.candidate_commit);
  return {
    evidence_source: path.join(downloadDirectory, relativeEvidence.replace(/^bench\//, "")),
    evidence_destination: path.join(REPO, relativeEvidence),
    publication_source: null,
    history_destination: null,
  };
}

function sha256File(file) {
  return sha256(fs.readFileSync(file));
}

function quoteArg(value) {
  return /^[A-Za-z0-9_./:=+,-]+$/.test(value) ? value : JSON.stringify(value);
}

function commandText(command, args) {
  return [command, ...args].map(quoteArg).join(" ");
}

async function run(command, args, options = {}) {
  throwIfInterrupted();
  const {
    cwd = REPO,
    capture = true,
    allowFailure = false,
    input = null,
    timeoutMs = 0,
    announce = false,
  } = options;
  if (announce) process.stderr.write(`+ ${commandText(command, args)}\n`);
  return await new Promise((resolve, reject) => {
    const stdout = [];
    const stderr = [];
    const child = spawn(command, args, {
      cwd,
      env: process.env,
      stdio: [input === null ? "inherit" : "pipe", capture ? "pipe" : "inherit", capture ? "pipe" : "inherit"],
    });
    activeChild = child;
    let timedOut = false;
    let timer = null;
    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true;
        terminateChild(child);
      }, timeoutMs);
    }
    if (capture) {
      child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
      child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
    }
    child.on("error", (error) => {
      if (timer) clearTimeout(timer);
      if (activeForceKillTimer) clearTimeout(activeForceKillTimer);
      activeForceKillTimer = null;
      if (activeChild === child) activeChild = null;
      reject(new Error(`cannot run ${command}: ${error.message}`));
    });
    child.on("close", (status, signal) => {
      if (timer) clearTimeout(timer);
      if (activeForceKillTimer) clearTimeout(activeForceKillTimer);
      activeForceKillTimer = null;
      if (activeChild === child) activeChild = null;
      const result = {
        status: status === null ? 1 : status,
        signal,
        stdout: Buffer.concat(stdout),
        stderr: Buffer.concat(stderr),
        timedOut,
      };
      if (result.status !== 0 && !allowFailure) {
        const detail = result.stderr.toString("utf8").trim();
        const suffix = timedOut ? " (timed out)" : signal ? ` (signal ${signal})` : "";
        reject(new Error(`${commandText(command, args)} failed with status ${result.status}${suffix}${detail ? `: ${detail}` : ""}`));
        return;
      }
      resolve(result);
    });
    if (input !== null) child.stdin.end(input);
  });
}

function outputText(result) {
  return result.stdout.toString("utf8").trim();
}

function findExecutable(name) {
  for (const directory of (process.env.PATH || "").split(path.delimiter)) {
    const candidate = path.join(directory || ".", name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch {
      // Keep searching.
    }
  }
  return null;
}

function requireExecutables(names) {
  const missing = names.filter((name) => !findExecutable(name));
  if (missing.length > 0) fail(`missing required commands: ${missing.join(", ")}`);
}

function apiErrorDetail(response) {
  if (!response) return "no response body";
  if (Array.isArray(response.errors)) {
    const detail = response.errors
      .map((entry) => {
        if (!entry || typeof entry !== "object") return String(entry);
        const reason = typeof entry.reason === "string" ? entry.reason : "API error";
        const field = typeof entry.field === "string" ? ` (${entry.field})` : "";
        return `${reason}${field}`;
      })
      .join("; ");
    if (detail) return detail.slice(0, 2000);
  }
  return "unrecognized API error response";
}

async function apiRequest(token, method, apiPath, options = {}) {
  throwIfInterrupted();
  if (!/^\/v4\//.test(apiPath)) fail("Linode API path must stay under /v4/");
  const {
    body = null,
    acceptedStatuses = null,
    timeoutMs = API_TIMEOUT_MS,
    unauthenticated = false,
  } = options;
  if (unauthenticated) {
    const publicCatalogPath = [
      /^\/v4\/linode\/types\/[^/?]+$/,
      /^\/v4\/images\/[^/?]+$/,
      /^\/v4\/regions\/[^/?]+$/,
      /^\/v4\/regions\/[^/?]+\/availability$/,
    ].some((pattern) => pattern.test(apiPath));
    if (method !== "GET" || !publicCatalogPath || body !== null) {
      fail("unauthenticated Linode requests are limited to exact public catalog GETs");
    }
  } else if (typeof token !== "string" || token.length === 0) {
    fail("Linode token is unavailable");
  }
  const requestBody = body === null ? null : Buffer.from(JSON.stringify(body));
  return await new Promise((resolve, reject) => {
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      if (activeRequest === request) activeRequest = null;
      callback(value);
    };
    const request = https.request(
      {
        hostname: API_HOST,
        port: 443,
        method,
        path: apiPath,
        headers: {
          Accept: "application/json",
          ...(unauthenticated ? {} : { Authorization: `Bearer ${token}` }),
          "User-Agent": "rustia-linode-benchmark/1",
          ...(requestBody === null
            ? {}
            : { "Content-Type": "application/json", "Content-Length": requestBody.length }),
        },
      },
      (response) => {
        const chunks = [];
        let size = 0;
        response.on("data", (chunk) => {
          size += chunk.length;
          if (size > 32 * 1024 * 1024) {
            response.destroy(new Error("Linode API response exceeded 32 MiB"));
            return;
          }
          chunks.push(Buffer.from(chunk));
        });
        response.on("error", (error) => finish(reject, new LinodeApiError(`Linode API response failed: ${error.message}`)));
        response.on("end", () => {
          const bytes = Buffer.concat(chunks);
          let data = null;
          if (bytes.length > 0) {
            try {
              data = JSON.parse(bytes.toString("utf8"));
            } catch (error) {
              finish(reject, new LinodeApiError(`Linode API returned invalid JSON: ${error.message}`, response.statusCode));
              return;
            }
          }
          const status = response.statusCode || 0;
          const accepted = acceptedStatuses
            ? acceptedStatuses.includes(status)
            : status >= 200 && status < 300;
          if (!accepted) {
            finish(reject, new LinodeApiError(`Linode API ${method} ${apiPath} returned ${status}: ${apiErrorDetail(data)}`, status, data));
            return;
          }
          finish(resolve, { status, data, headers: response.headers });
        });
      },
    );
    activeRequest = request;
    request.setTimeout(timeoutMs, () => request.destroy(new Error("Linode API request timed out")));
    request.on("error", (error) => {
      finish(reject, new LinodeApiError(`Linode API ${method} ${apiPath} failed: ${error.message}`));
    });
    if (requestBody !== null) request.write(requestBody);
    request.end();
  });
}

function writeBytesAtomic(file, bytes, mode = 0o600) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomBytes(6).toString("hex")}`;
  fs.writeFileSync(temporary, bytes, { flag: "wx", mode });
  fs.chmodSync(temporary, mode);
  fs.renameSync(temporary, file);
}

function writeJsonAtomic(file, value, mode = 0o600) {
  writeBytesAtomic(file, Buffer.from(`${JSON.stringify(value, null, 2)}\n`), mode);
}

function readStoredToken({ allowMissing = false } = {}) {
  let stat;
  try {
    stat = fs.lstatSync(TOKEN_FILE);
  } catch (error) {
    if (allowMissing && error && error.code === "ENOENT") return null;
    fail("Linode token is not configured; run the auth command from an interactive terminal");
  }
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`refusing non-regular token file ${TOKEN_FILE}`);
  if ((stat.mode & 0o077) !== 0) fail(`Linode token file permissions must be 0600: ${TOKEN_FILE}`);
  const token = fs.readFileSync(TOKEN_FILE, "utf8").trim();
  if (!token || /\s/.test(token)) fail(`Linode token file is empty or malformed: ${TOKEN_FILE}`);
  return token;
}

async function readHiddenToken() {
  if (!process.stdin.isTTY || !process.stderr.isTTY || typeof process.stdin.setRawMode !== "function") {
    fail("auth requires an interactive TTY; the token is never accepted in argv or environment variables");
  }
  process.stderr.write("Linode personal access token (input hidden): ");
  const input = process.stdin;
  const wasRaw = Boolean(input.isRaw);
  input.setEncoding("utf8");
  input.setRawMode(true);
  input.resume();
  return await new Promise((resolve, reject) => {
    let secret = "";
    const signals = ["SIGINT", "SIGTERM", "SIGHUP"];
    const signalHandlers = new Map();
    const cleanup = () => {
      input.off("data", onData);
      input.off("end", onEnd);
      input.off("error", onError);
      for (const signal of signals) process.off(signal, signalHandlers.get(signal));
      try {
        input.setRawMode(wasRaw);
      } catch {
        // The terminal may already have disappeared.
      }
      input.pause();
      process.stderr.write("\n");
    };
    const abort = (message) => {
      cleanup();
      reject(new Error(message));
    };
    const onEnd = () => abort("token entry ended before a token was submitted");
    const onError = (error) => abort(`token entry failed: ${error.message}`);
    for (const signal of signals) {
      signalHandlers.set(signal, () => abort(`token entry interrupted by ${signal}`));
    }
    const onData = (chunk) => {
      for (const character of chunk) {
        if (character === "\r" || character === "\n") {
          cleanup();
          if (!secret || /\s/.test(secret)) reject(new Error("token cannot be empty or contain whitespace"));
          else resolve(secret);
          return;
        }
        if (character === "\u0003") {
          abort("token entry interrupted");
          return;
        }
        if (character === "\u007f" || character === "\b") {
          secret = secret.slice(0, -1);
        } else if (character >= " ") {
          secret += character;
        }
      }
    };
    input.on("data", onData);
    input.once("end", onEnd);
    input.once("error", onError);
    for (const signal of signals) process.once(signal, signalHandlers.get(signal));
  });
}

function safeTimestamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function roundedHourlyCharge(hourlyPrice, seconds) {
  requireNumber(hourlyPrice, "hourly price", 0);
  requireNumber(seconds, "billable seconds", 0);
  if (seconds === 0) return 0;
  return Number((Math.ceil(seconds / 3600) * hourlyPrice).toFixed(6));
}

function requireApiObject(response, description) {
  return requireObject(response && response.data, description);
}

async function listLinodes(token, request = apiRequest) {
  const all = [];
  let page = 1;
  let pages = 1;
  do {
    const response = await request(token, "GET", `/v4/linode/instances?page=${page}&page_size=500`);
    const envelope = requireApiObject(response, "Linode instance list response");
    if (!Array.isArray(envelope.data)) fail("Linode instance list response.data must be an array");
    all.push(...envelope.data);
    pages = requireInteger(envelope.pages, "Linode instance list response.pages", 1);
    page += 1;
  } while (page <= pages);
  return all;
}

async function preflightAccount(token, request = apiRequest) {
  const settings = requireApiObject(
    await request(token, "GET", "/v4/account/settings"),
    "Linode account settings",
  );
  if (settings.backups_enabled !== false) {
    fail(
      "Linode account-wide backups_enabled must be false before billing; otherwise the " +
        "ephemeral instance would incur an additional backup charge",
    );
  }
  if (settings.interfaces_for_new_linodes === "linode_only") {
    fail(
      "Linode account settings require linode-only interfaces, but this coordinator pins " +
        "legacy_config so firewall_id=-1 can explicitly prevent a default firewall attachment",
    );
  }
  return settings;
}

async function preflightAccountRegion(token, config, request = apiRequest) {
  const regionId = encodeURIComponent(config.provider.region);
  const availability = requireApiObject(
    await request(token, "GET", `/v4/account/availability/${regionId}`),
    "Linode account region availability",
  );
  if (!Array.isArray(availability.available) || !Array.isArray(availability.unavailable)) {
    fail("Linode account region availability must contain available and unavailable arrays");
  }
  const malformed = [...availability.available, ...availability.unavailable].some(
    (service) => typeof service !== "string" || service.length === 0,
  );
  if (malformed) fail("Linode account region availability contains a malformed service name");
  const available = availability.available.filter((service) => service === "Linodes").length;
  const unavailable = availability.unavailable.filter((service) => service === "Linodes").length;
  if (available !== 1 || unavailable !== 0) {
    fail(
      `Linode instances are not available to this account in ${config.provider.region}; ` +
        "select an account-enabled region before billing",
    );
  }
  return availability;
}

function effectiveTypePrice(type, region) {
  const regional = Array.isArray(type.region_prices)
    ? type.region_prices.filter((entry) => entry && entry.id === region)
    : [];
  if (regional.length > 1) fail(`Linode type ${type.id} has duplicate regional prices for ${region}`);
  const price = regional.length === 1 ? regional[0] : type.price;
  requireObject(price, `Linode type ${type.id} effective price`);
  return {
    hourly: requireNumber(price.hourly, `Linode type ${type.id} hourly price`, 0),
    monthly:
      typeof price.monthly === "number" && Number.isFinite(price.monthly)
        ? price.monthly
        : null,
    regional: regional.length === 1,
  };
}

function validateProviderCatalog(config, responses) {
  const wanted = config.provider.size;
  const type = requireApiObject(responses.type, "Linode type");
  const image = requireApiObject(responses.image, "Linode image");
  const region = requireApiObject(responses.region, "Linode region");
  const availability = responses.availability && responses.availability.data;
  if (!Array.isArray(availability)) fail("Linode region availability must be an array");

  const typeChecks = [
    ["id", type.id, wanted.id],
    ["description", type.label, wanted.description],
    ["class", type.class, wanted.class],
    ["vCPU count", type.vcpus, wanted.vcpus],
    ["memory MiB", type.memory, wanted.memory_mib],
    ["disk MiB", type.disk, wanted.disk_gib * 1024],
  ];
  for (const [field, actual, expected] of typeChecks) {
    if (String(actual) !== String(expected)) {
      fail(`Linode public type ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  const price = effectiveTypePrice(type, config.provider.region);
  if (price.hourly > wanted.max_hourly_price_usd) {
    fail(
      `Linode type ${wanted.id} costs $${price.hourly}/h in ${config.provider.region}, ` +
        `above the configured $${wanted.max_hourly_price_usd}/h maximum`,
    );
  }

  if (
    image.id !== config.provider.image ||
    image.is_public !== true ||
    image.status !== "available"
  ) {
    fail(
      `Linode image ${config.provider.image} must resolve exactly as a public, available image`,
    );
  }
  if (region.id !== config.provider.region || region.status !== "ok") {
    fail(`Linode region ${config.provider.region} is not an exact, healthy public region match`);
  }
  const availabilityMatches = availability.filter(
    (entry) =>
      entry &&
      entry.region === config.provider.region &&
      entry.plan === wanted.id,
  );
  if (availabilityMatches.length !== 1 || availabilityMatches[0].available !== true) {
    fail(
      `Linode type ${wanted.id} must have one available catalog row in ${config.provider.region}; ` +
        `found ${availabilityMatches.length}`,
    );
  }
  return { type, image, region, availability: availabilityMatches[0], price };
}

async function resolveProviderPlan(token, config, request = apiRequest) {
  const typeId = encodeURIComponent(config.provider.size.id);
  const imageId = encodeURIComponent(config.provider.image);
  const regionId = encodeURIComponent(config.provider.region);
  const [type, image, region, availability] = await Promise.all([
    request(null, "GET", `/v4/linode/types/${typeId}`, { unauthenticated: true }),
    request(null, "GET", `/v4/images/${imageId}`, { unauthenticated: true }),
    request(null, "GET", `/v4/regions/${regionId}`, { unauthenticated: true }),
    request(null, "GET", `/v4/regions/${regionId}/availability`, { unauthenticated: true }),
  ]);
  return validateProviderCatalog(config, { type, image, region, availability });
}

async function verifyDeleteAuthority(token, request = apiRequest, { announce = true } = {}) {
  const sentinel = 2_147_483_647;
  if ((await listLinodes(token, request)).some((entry) => entry && entry.id === sentinel)) {
    fail(`refusing absent-ID probe because Linode ${sentinel} exists on the account`);
  }
  let response;
  try {
    response = await request(token, "DELETE", `/v4/linode/instances/${sentinel}`, {
      acceptedStatuses: [404],
    });
  } catch (error) {
    fail(
      "Linode delete authority could not be proven before billing; the token needs " +
        `linodes:read_write (${error.message})`,
    );
  }
  if (response.status !== 404) fail("absent-ID Linode delete probe did not return 404");
  if (announce) process.stdout.write("Pre-billing absent-ID probe confirmed Linode delete authority.\n");
}

async function gitBlob(commit, file) {
  return (await run("git", ["show", `${commit}:${file}`])).stdout;
}

async function verifySource(config, configFile = DEFAULT_CONFIG) {
  const relativeConfig = path.relative(REPO, configFile);
  if (relativeConfig.startsWith("..") || path.isAbsolute(relativeConfig)) {
    fail("Linode benchmark config must be inside the repository");
  }
  const trackedStatus = outputText(
    await run("git", ["status", "--porcelain=v1", "--untracked-files=no"]),
  );
  if (trackedStatus.length > 0) fail("tracked worktree changes must be committed before a cloud benchmark");
  const sourceCommits = isCrossEngineMatrix(config)
    ? [config.source.benchmark_commit]
    : [config.source.baseline_commit, config.source.candidate_commit, config.source.harness_commit];
  for (const commit of sourceCommits) await run("git", ["cat-file", "-e", `${commit}^{commit}`]);
  const selectedRemoteRunner = remoteRunnerForConfig(config);
  for (const file of [
    "tools/run-linode-benchmark.js",
    path.relative(REPO, selectedRemoteRunner),
    relativeConfig,
  ]) {
    await run("git", ["ls-files", "--error-unmatch", file]);
  }

  const source = config.source;
  const expected = source.expected_sha256;
  const checks = isCrossEngineMatrix(config) ? [
    ["benchmark manifest", sha256(await gitBlob(source.benchmark_commit, "bench/Cargo.toml")), expected.bench_manifest],
    ["native harness", sha256(await gitBlob(source.benchmark_commit, "bench/src/main.rs")), expected.native_harness],
    ["Cargo.lock", sha256(await gitBlob(source.benchmark_commit, "Cargo.lock")), expected.cargo_lock],
    ["toolchain", sha256(await gitBlob(source.benchmark_commit, "rust-toolchain.toml")), expected.toolchain],
    ["wasm gate", sha256(await gitBlob(source.benchmark_commit, "tools/run-wasm-gate.js")), expected.wasm_gate],
    [
      "wasm dispatch gate",
      sha256(await gitBlob(source.benchmark_commit, "tools/run-wasm-dispatch-gate.js")),
      expected.wasm_dispatch_gate,
    ],
    [
      "wasm adapter manifest",
      sha256(await gitBlob(source.benchmark_commit, "bench/wasm/adapter-runner/Cargo.toml")),
      expected.wasm_adapter_manifest,
    ],
    [
      "wasm module measurement",
      sha256(await gitBlob(source.benchmark_commit, "bench/wasm/measure-rustia-wasm.mjs")),
      expected.wasm_measure_module,
    ],
    [
      "wasm dispatch measurement",
      sha256(await gitBlob(source.benchmark_commit, "bench/wasm/measure-dispatch-wasm.js")),
      expected.wasm_dispatch_measure,
    ],
    [
      "benchmark commit history",
      sha256(await gitBlob(source.benchmark_commit, "bench/history/runs.jsonl")),
      expected.history_before,
    ],
    ["current history", sha256File(path.join(REPO, "bench/history/runs.jsonl")), expected.history_before],
  ] : isVmStringMemory(config) ? [
    [
      "baseline original bench manifest",
      sha256(await gitBlob(source.baseline_commit, "bench/Cargo.toml")),
      expected.baseline_original_bench_manifest,
    ],
    [
      "candidate original bench manifest",
      sha256(await gitBlob(source.candidate_commit, "bench/Cargo.toml")),
      expected.candidate_original_bench_manifest,
    ],
    [
      "overlay bench manifest",
      sha256(await gitBlob(source.harness_commit, "bench/Cargo.toml")),
      expected.overlay_bench_manifest,
    ],
    [
      "vm string memory sampler",
      sha256(await gitBlob(source.harness_commit, "bench/src/bin/vm-string-memory.rs")),
      expected.sampler,
    ],
    [
      "vm string memory analyzer",
      sha256(await gitBlob(source.harness_commit, "bench/analyze-vm-string-memory.js")),
      expected.analyzer,
    ],
    ...[source.baseline_commit, source.candidate_commit, source.harness_commit].flatMap((commit, index) => [
      [`source ${index + 1} Cargo.lock`, null, expected.cargo_lock, commit, "Cargo.lock"],
      [`source ${index + 1} toolchain`, null, expected.toolchain, commit, "rust-toolchain.toml"],
    ]),
  ] : [
    [
      "baseline original harness",
      sha256(await gitBlob(source.baseline_commit, "bench/src/main.rs")),
      expected.baseline_original_harness,
    ],
    [
      "candidate overlay harness",
      sha256(await gitBlob(source.harness_commit, "bench/src/main.rs")),
      expected.overlay_harness,
    ],
    [
      "baseline workload",
      sha256(await gitBlob(source.baseline_commit, config.measurement.workload_path)),
      expected.workload,
    ],
    [
      "candidate workload",
      sha256(await gitBlob(source.candidate_commit, config.measurement.workload_path)),
      expected.workload,
    ],
    ["baseline Cargo.lock", sha256(await gitBlob(source.baseline_commit, "Cargo.lock")), expected.cargo_lock],
    ["candidate Cargo.lock", sha256(await gitBlob(source.candidate_commit, "Cargo.lock")), expected.cargo_lock],
    [
      "baseline toolchain",
      sha256(await gitBlob(source.baseline_commit, "rust-toolchain.toml")),
      expected.toolchain,
    ],
    [
      "candidate toolchain",
      sha256(await gitBlob(source.candidate_commit, "rust-toolchain.toml")),
      expected.toolchain,
    ],
    [
      "analyzer",
      sha256(await gitBlob(source.harness_commit, "bench/analyze-fixed-samples.js")),
      expected.analyzer,
    ],
    ["current history", sha256File(path.join(REPO, "bench/history/runs.jsonl")), expected.history_before],
  ];
  for (const check of checks) {
    const [name, immediate, wanted, commit, file] = check;
    const actual = immediate === null ? sha256(await gitBlob(commit, file)) : immediate;
    check[1] = actual;
    if (actual !== wanted) fail(`${name} SHA-256 is ${actual}, expected ${wanted}`);
  }
  return {
    orchestration_commit: outputText(await run("git", ["rev-parse", "HEAD"])),
    config_path: relativeConfig,
    config_sha256: sha256File(configFile),
    coordinator_sha256: sha256File(__filename),
    ...(isCrossEngineMatrix(config)
      ? { remote_runner_path: path.relative(REPO, selectedRemoteRunner) }
      : {}),
    remote_runner_sha256: sha256File(selectedRemoteRunner),
    verified_source_hashes: Object.fromEntries(checks.map(([name, actual]) => [name, actual])),
  };
}

function sanitizedProviderPlan(config, provider, source) {
  const price = provider.price;
  return {
    schema: config.schema,
    provider: "linode",
    protocol_mode: config.publication.mode,
    region: {
      id: provider.region.id,
      label: provider.region.label,
      country: provider.region.country,
      status: provider.region.status,
    },
    image: {
      id: provider.image.id,
      label: provider.image.label,
      vendor: provider.image.vendor,
      created: provider.image.created,
      updated: provider.image.updated,
      status: provider.image.status,
      is_public: provider.image.is_public,
    },
    size: {
      id: provider.type.id,
      description: provider.type.label,
      class: provider.type.class,
      vcpus: provider.type.vcpus,
      memory_mib: provider.type.memory,
      disk_gib: provider.type.disk / 1024,
      price_hourly_usd: price.hourly,
      price_monthly_usd: price.monthly,
      regional_price: price.regional,
    },
    billable_lifetime_limit_minutes: config.provider.max_runtime_minutes,
    billing_increment_minutes: 60,
    planned_compute_cost_ceiling_usd: roundedHourlyCharge(
      price.hourly,
      config.provider.max_runtime_minutes * 60,
    ),
    cleanup_reserve_minutes: CLEANUP_RESERVE_MS / 60_000,
    billing_caveat:
      "Linode usage is rounded up to a whole hour; deletion, not shutdown, stops further accrual, and an API outage can delay confirmed deletion",
    source,
    comparison: isCrossEngineMatrix(config)
      ? {
          protocol: CROSS_ENGINE_MATRIX_PROTOCOL,
          benchmark_commit: config.source.benchmark_commit,
          native_target: config.measurement.native_target,
          native_engines: config.measurement.native_engines,
          workloads: config.measurement.workloads,
          native_repetitions: config.measurement.native_repetitions,
          wasm_runtimes: config.measurement.wasm_runtimes,
          wasm_adapters: config.measurement.wasm_adapters,
          wasm_repetitions: config.measurement.wasm_repetitions,
          dispatch_repetitions: config.measurement.dispatch_repetitions,
          maximum_timed_steal_ticks: config.host_gate.maximum_timed_steal_ticks,
          maximum_timed_attempts_per_group: config.host_gate.maximum_timed_attempts_per_group,
          wait_timeout_seconds: config.host_gate.wait_timeout_seconds,
          result_shape: {
            native_rows: config.publication.native_rows,
            wasm_rows: config.publication.wasm_rows,
            dispatch_runtimes: config.publication.dispatch_runtimes,
          },
        }
      : isVmStringMemory(config)
      ? {
          protocol: VM_STRING_MEMORY_PROTOCOL,
          baseline_commit: config.source.baseline_commit,
          candidate_commit: config.source.candidate_commit,
          harness_overlay_commit: config.source.harness_commit,
          vm_count: config.measurement.vm_count,
          warmup_rounds: config.measurement.warmup_rounds,
          accepted_rounds: config.measurement.accepted_rounds,
          scenarios: config.measurement.scenarios,
        }
      : {
          baseline_commit: config.source.baseline_commit,
          candidate_commit: config.source.candidate_commit,
          harness_overlay_commit: config.source.harness_commit,
          workload: config.measurement.workload,
          fixed_n: config.measurement.fixed_n,
          repetitions: config.measurement.repetitions,
          order: config.measurement.run_order,
        },
  };
}

async function buildPlan(config, { print = true, configFile = DEFAULT_CONFIG } = {}) {
  requireExecutables(["git", "node", "scp", "ssh", "ssh-keygen", "tar"]);
  const token = readStoredToken();
  await preflightAccount(token);
  await preflightAccountRegion(token, config);
  const source = await verifySource(config, configFile);
  const provider = await resolveProviderPlan(token, config);
  const plan = sanitizedProviderPlan(config, provider, source);
  if (print) process.stdout.write(`${JSON.stringify(plan, null, 2)}\n`);
  return { token, plan, provider, source };
}

function processExists(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return null;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error && error.code === "ESRCH") return false;
    return true;
  }
}

function processFingerprint(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return null;
  const result = spawnSync("ps", ["-p", String(pid), "-o", "lstart=", "-o", "command="], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  });
  if (result.status !== 0) return null;
  const fingerprint = String(result.stdout || "").trim();
  return fingerprint || null;
}

function lockOwnerIsLive(owner) {
  if (processExists(owner.pid) === false) return false;
  const current = processFingerprint(owner.pid);
  if (owner.process_fingerprint && current) return owner.process_fingerprint === current;
  return true;
}

function acquireCoordinatorLock() {
  fs.mkdirSync(STATE_ROOT, { recursive: true, mode: 0o700 });
  fs.chmodSync(STATE_ROOT, 0o700);
  const token = crypto.randomBytes(16).toString("hex");
  const pending = `${COORDINATOR_LOCK}.pending-${token}`;
  fs.mkdirSync(pending, { mode: 0o700 });
  writeJsonAtomic(path.join(pending, "owner.json"), {
    schema: "rustia.linode-benchmark-lock.v1",
    pid: process.pid,
    token,
    process_fingerprint: processFingerprint(process.pid),
    acquired_at: new Date().toISOString(),
  });
  try {
    fs.renameSync(pending, COORDINATOR_LOCK);
  } catch (error) {
    fs.rmSync(pending, { recursive: true, force: true });
    if (!error || !["EEXIST", "ENOTEMPTY"].includes(error.code)) throw error;
    let owner;
    try {
      owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
    } catch {
      fail(`Linode coordinator lock exists without readable ownership at ${COORDINATOR_LOCK}`);
    }
    if (!lockOwnerIsLive(owner)) {
      fail(`stale Linode coordinator lock for PID ${owner.pid}; run the unlock command`);
    }
    fail(`Linode benchmark coordinator PID ${owner.pid} is already active`);
  }
  return token;
}

function releaseCoordinatorLock(token) {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot verify Linode coordinator lock ownership: ${error.message}`);
  }
  if (owner.pid !== process.pid || owner.token !== token) {
    fail("refusing to remove a Linode coordinator lock owned by another process");
  }
  fs.rmSync(COORDINATOR_LOCK, { recursive: true });
}

function unlockStaleCoordinator() {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot read Linode coordinator lock ownership: ${error.message}`);
  }
  if (lockOwnerIsLive(owner)) fail(`refusing to unlock live Linode coordinator PID ${owner.pid}`);
  const quarantine = `${COORDINATOR_LOCK}.stale-${crypto.randomBytes(8).toString("hex")}`;
  fs.renameSync(COORDINATOR_LOCK, quarantine);
  const moved = JSON.parse(fs.readFileSync(path.join(quarantine, "owner.json"), "utf8"));
  if (moved.pid !== owner.pid || moved.token !== owner.token) {
    fail(`stale lock ownership changed during quarantine; retained at ${quarantine}`);
  }
  fs.rmSync(quarantine, { recursive: true });
  process.stdout.write(`Removed stale Linode coordinator lock for PID ${owner.pid}.\n`);
}

function readState(directory) {
  const state = JSON.parse(fs.readFileSync(path.join(directory, "state.json"), "utf8"));
  state.__directory = directory;
  return state;
}

function saveState(state) {
  const copy = { ...state };
  delete copy.__directory;
  writeJsonAtomic(path.join(state.__directory, "state.json"), copy);
}

function listStates() {
  if (!fs.existsSync(RUNS_ROOT)) return [];
  return fs
    .readdirSync(RUNS_ROOT, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && fs.existsSync(path.join(RUNS_ROOT, entry.name, "state.json")))
    .map((entry) => readState(path.join(RUNS_ROOT, entry.name)))
    .sort((left, right) => String(left.created_at).localeCompare(String(right.created_at)));
}

function incompleteStates() {
  return listStates().filter((state) => !state.linode_destroyed_at);
}

function stateForRunId(runId) {
  const states = incompleteStates();
  if (runId) {
    const matches = states.filter((state) => state.run_id === runId);
    if (matches.length !== 1) fail(`no active owned run has id ${runId}`);
    return matches[0];
  }
  if (states.length !== 1) {
    fail(
      states.length === 0
        ? "no active owned Linode benchmark run exists"
        : `more than one active run exists; pass one of: ${states.map((state) => state.run_id).join(", ")}`,
    );
  }
  return states[0];
}

function completedStateForRunId(runId) {
  if (!RUN_ID.test(runId)) fail("a valid explicit Linode run id is required for offline promotion");
  const matches = listStates().filter((state) => state.run_id === runId);
  if (matches.length !== 1) fail(`no uniquely owned Linode run has id ${runId}`);
  return matches[0];
}

function startBillingWindow(state, config) {
  const started = Date.now();
  state.billing_window_started_at = new Date(started).toISOString();
  state.billing_deadline_at = new Date(started + config.provider.max_runtime_minutes * 60 * 1000).toISOString();
  saveState(state);
}

function remainingWorkTimeout(state, requestedMs, label) {
  const deadline = Date.parse(state.billing_deadline_at || "");
  if (!Number.isFinite(deadline)) fail("owned run has no valid billing deadline");
  const remaining = deadline - Date.now() - CLEANUP_RESERVE_MS;
  if (remaining <= 0) fail(`${label}: billable lifetime reached the reserved cleanup window`);
  return Math.max(1, Math.min(requestedMs, remaining));
}

function matchingLinodesByLabel(linodes, label) {
  if (!Array.isArray(linodes)) fail("Linode list must be an array");
  return linodes.filter((entry) => entry && entry.label === label);
}

async function reconcileLinodeByLabel(state, token, request = apiRequest) {
  const matches = matchingLinodesByLabel(await listLinodes(token, request), state.linode_label);
  if (matches.length > 1) {
    fail(`refusing to adopt or delete ${matches.length} Linodes labelled ${state.linode_label}`);
  }
  if (matches.length === 1) {
    const id = requireInteger(matches[0].id, "reconciled Linode ID", 1);
    state.linode_id = id;
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileLinodeAfterCreateAttempt(state, token, request = apiRequest) {
  if (state.linode_absence_confirmed_at) return null;
  const attemptedAt = Date.parse(state.linode_create_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile a Linode without a create-attempt timestamp");
  const finishedAt = Date.parse(state.linode_create_finished_at || "");
  const requestDeadline = Date.parse(state.linode_create_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = Math.max(Date.now(), uncertaintyEndedAt + CREATE_RECONCILE_TIMEOUT_MS);
  while (Date.now() < deadline) {
    try {
      const linode = await reconcileLinodeByLabel(state, token, request);
      if (linode) return linode;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      if (/refusing to adopt or delete/.test(error.message)) throw error;
      // Only a successful list after the uncertainty window proves absence.
    }
    const delay = Math.min(5000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  const linode = await reconcileLinodeByLabel(state, token, request);
  if (linode) return linode;
  state.linode_absence_confirmed_at = new Date().toISOString();
  saveState(state);
  return null;
}

async function getOwnedLinode(state, token, { allowMissing = false, request = apiRequest } = {}) {
  if (!state.linode_id) return null;
  const response = await request(token, "GET", `/v4/linode/instances/${state.linode_id}`, {
    acceptedStatuses: [200, 404],
  });
  if (response.status === 404) {
    if (allowMissing) return null;
    fail(`owned Linode ${state.linode_id} was not found`);
  }
  const linode = requireApiObject(response, `Linode ${state.linode_id}`);
  if (linode.id !== state.linode_id || linode.label !== state.linode_label) {
    fail(`Linode ${state.linode_id} does not match owned label ${state.linode_label}`);
  }
  return linode;
}

function verifyOwnedLinodeIdentity(linode, state) {
  const checks = [
    ["ID", linode.id, state.linode_id],
    ["label", linode.label, state.linode_label],
    ["region", linode.region, state.plan.region.id],
    ["type", linode.type, state.plan.size.id],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`refusing to delete owned Linode: ${field} is ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
}

async function deleteLinode(state, token, request = apiRequest) {
  if (state.linode_destroyed_at) return;
  if (!state.linode_id) {
    const reconciled = state.linode_create_attempted_at
      ? await reconcileLinodeAfterCreateAttempt(state, token, request)
      : await reconcileLinodeByLabel(state, token, request);
    if (!reconciled) {
      state.linode_destroyed_at = new Date().toISOString();
      state.cleanup_note = state.linode_create_attempted_at
        ? "post-attempt grace window and a final successful list confirmed the unique Linode label absent"
        : "no create was attempted and a successful list confirmed the unique Linode label absent";
      saveState(state);
      return;
    }
  }

  let existing = await getOwnedLinode(state, token, { allowMissing: true, request });
  if (!existing) {
    const linodes = await listLinodes(token, request);
    const listed = linodes.find((entry) => entry && entry.id === state.linode_id);
    const matchingLabels = matchingLinodesByLabel(linodes, state.linode_label);
    if (!listed && matchingLabels.length === 0) {
      state.linode_destroyed_at = new Date().toISOString();
      saveState(state);
      return;
    }
    if (!listed) {
      fail(
        `refusing cleanup: owned Linode ID ${state.linode_id} is absent but label ` +
          `${state.linode_label} exists under ${matchingLabels.length} other ID(s)`,
      );
    }
    existing = listed;
  }
  verifyOwnedLinodeIdentity(existing, state);

  try {
    await request(token, "DELETE", `/v4/linode/instances/${existing.id}`, {
      acceptedStatuses: [200, 204, 404],
    });
  } catch {
    // A read below, never the DELETE response, decides whether deletion completed.
  }

  const deadline = Date.now() + DELETE_CONFIRM_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const linodes = await listLinodes(token, request);
      const byId = linodes.find((entry) => entry && entry.id === existing.id);
      const byLabel = matchingLinodesByLabel(linodes, state.linode_label);
      if (!byId && byLabel.length === 0) {
        state.linode_destroyed_at = new Date().toISOString();
        saveState(state);
        return;
      }
      if (byLabel.length > 1) {
        fail(`refusing cleanup: duplicate Linode label ${state.linode_label} appeared`);
      }
      if (!byId || byId.label !== state.linode_label) {
        fail(`refusing cleanup: owned Linode ID/label identity changed`);
      }
      verifyOwnedLinodeIdentity(byId, state);
      await request(token, "DELETE", `/v4/linode/instances/${existing.id}`, {
        acceptedStatuses: [200, 204, 404],
      });
    } catch (error) {
      if (/refusing cleanup|refusing to delete/.test(error.message)) throw error;
      // API failure is not deletion proof; retry within the bounded confirmation window.
    }
    const delay = Math.min(4000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  fail(
    `could not confirm deletion of owned Linode ${existing.id}; run ` +
      `\`node tools/run-linode-benchmark.js destroy ${state.run_id}\` and do not assume billing stopped`,
  );
}

async function cleanupOwned(state, token) {
  await deleteLinode(state, token);
  if (state.linode_destroyed_at) {
    for (const file of [state.private_key_path, state.public_key_path]) {
      if (file && fs.existsSync(file)) fs.rmSync(file);
    }
  }
}

function sshOptions(state) {
  return [
    "-i",
    state.private_key_path,
    "-o",
    "BatchMode=yes",
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-o",
    `UserKnownHostsFile=${state.known_hosts_path}`,
  ];
}

function isPublicIpv4(address) {
  if (!net.isIPv4(address)) return false;
  const octets = address.split(".").map(Number);
  if (octets[0] === 10 || octets[0] === 127) return false;
  if (octets[0] === 169 && octets[1] === 254) return false;
  if (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) return false;
  if (octets[0] === 192 && octets[1] === 168) return false;
  return true;
}

function publicIpv4(linode) {
  if (!linode || !Array.isArray(linode.ipv4)) return null;
  const address = linode.ipv4.find((entry) => typeof entry === "string" && isPublicIpv4(entry));
  return address || null;
}

async function waitForLinode(state, token) {
  const deadline = Date.now() + remainingWorkTimeout(state, 5 * 60 * 1000, "Linode activation");
  while (Date.now() < deadline) {
    const linode = await getOwnedLinode(state, token, { allowMissing: true });
    const address = publicIpv4(linode);
    if (linode && linode.status === "running" && address) {
      state.public_ipv4 = address;
      state.linode_created_at = linode.created || new Date().toISOString();
      saveState(state);
      return linode;
    }
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`Linode ${state.linode_id} did not become running with a public IPv4 address`);
}

async function waitForSsh(state) {
  const deadline = Date.now() + remainingWorkTimeout(state, 5 * 60 * 1000, "SSH readiness");
  while (Date.now() < deadline) {
    const result = await run("ssh", [...sshOptions(state), `root@${state.public_ipv4}`, "true"], {
      allowFailure: true,
      timeoutMs: 15 * 1000,
    });
    if (result.status === 0) return;
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`SSH did not become ready on owned Linode ${state.linode_id}`);
}

function verifyLinodeMatchesPlan(linode, plan) {
  const specs = requireObject(linode.specs, "activated Linode specs");
  const checks = [
    ["type", linode.type, plan.size.id],
    ["region", linode.region, plan.region.id],
    ["image", linode.image, plan.image.id],
    ["vCPU count", specs.vcpus, plan.size.vcpus],
    ["memory MiB", specs.memory, plan.size.memory_mib],
    ["disk MiB", specs.disk, plan.size.disk_gib * 1024],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`activated Linode ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if (!linode.backups || linode.backups.enabled !== false) {
    fail("activated Linode unexpectedly has paid backups enabled");
  }
  if (linode.interface_generation !== "legacy_config") {
    fail(
      `activated Linode interface generation was ${JSON.stringify(linode.interface_generation)}, ` +
        'expected "legacy_config"',
    );
  }
  if (linode.maintenance_policy !== "linode/power_off_on") {
    fail(
      `activated Linode maintenance policy was ${JSON.stringify(linode.maintenance_policy)}, ` +
        'expected "linode/power_off_on"',
    );
  }
}

function safeLinodeMetadata(linode, plan, state) {
  return {
    provider: "linode",
    linode_id: linode.id,
    linode_label: linode.label,
    created_at: linode.created,
    status: linode.status,
    region: linode.region,
    size_slug: linode.type,
    image: linode.image,
    vcpus: linode.specs.vcpus,
    memory_mib: linode.specs.memory,
    disk_gib: linode.specs.disk / 1024,
    public_ipv4: state.public_ipv4,
    backups_enabled: linode.backups.enabled,
    interface_generation: linode.interface_generation,
    maintenance_policy: linode.maintenance_policy,
    catalog_plan: plan,
  };
}

function createLinodeBody(config, label, publicKey) {
  if (typeof publicKey !== "string" || !/^ssh-ed25519 [A-Za-z0-9+/]+={0,3}(?:\s+\S.*)?$/.test(publicKey)) {
    fail("generated public key is not a single valid ssh-ed25519 line");
  }
  return {
    region: config.provider.region,
    type: config.provider.size.id,
    label,
    image: config.provider.image,
    authorized_keys: [publicKey],
    backups_enabled: false,
    booted: true,
    interface_generation: "legacy_config",
    firewall_id: -1,
    private_ip: false,
    maintenance_policy: "linode/power_off_on",
  };
}

function verifyProviderPlanUnchanged(original, refreshed) {
  const fields = [
    ["region", original.region.id, refreshed.region.id],
    ["image", original.image.id, refreshed.image.id],
    ["type", original.size.id, refreshed.size.id],
    ["class", original.size.class, refreshed.size.class],
    ["vCPUs", original.size.vcpus, refreshed.size.vcpus],
    ["memory", original.size.memory_mib, refreshed.size.memory_mib],
    ["disk", original.size.disk_gib, refreshed.size.disk_gib],
    ["hourly price", original.size.price_hourly_usd, refreshed.size.price_hourly_usd],
  ];
  for (const [field, before, after] of fields) {
    if (String(before) !== String(after)) {
      fail(`Linode provider ${field} changed between plan and pre-billing check`);
    }
  }
}

function tarAsciiField(header, start, length, location) {
  const field = header.subarray(start, start + length);
  const nul = field.indexOf(0);
  const bytes = nul === -1 ? field : field.subarray(0, nul);
  for (const byte of bytes) {
    if (byte < 0x20 || byte > 0x7e) fail(`${location} contains non-ASCII or control bytes`);
  }
  return bytes.toString("ascii");
}

function tarOctalField(header, start, length, location) {
  const field = header.subarray(start, start + length);
  if ((field[0] & 0x80) !== 0) fail(`${location} uses an unsupported base-256 number`);
  const text = field.toString("ascii").replace(/[\0 ]+$/g, "").replace(/^ +/g, "");
  if (text === "") return 0;
  if (!/^[0-7]+$/.test(text)) fail(`${location} is not a valid octal number`);
  const value = Number.parseInt(text, 8);
  if (!Number.isSafeInteger(value) || value < 0) fail(`${location} is outside the safe integer range`);
  return value;
}

function safeArchivePath(rawName, type) {
  if (rawName === "" || rawName.includes("\\") || !/^[A-Za-z0-9._/-]+$/.test(rawName)) {
    fail(`unsafe archive path ${JSON.stringify(rawName)}`);
  }
  if (path.posix.isAbsolute(rawName)) fail(`archive path is absolute: ${JSON.stringify(rawName)}`);
  const parts = rawName.split("/");
  if (parts.includes("..")) fail(`archive path contains a parent component: ${JSON.stringify(rawName)}`);
  while (parts[0] === ".") parts.shift();
  while (parts[parts.length - 1] === "") parts.pop();
  if (parts.some((part) => part === "" || part === ".")) {
    fail(`archive path is not canonical: ${JSON.stringify(rawName)}`);
  }
  const canonical = parts.length === 0 ? "." : parts.join("/");
  if (canonical === "." && type !== "directory") fail("archive root entry must be a directory");
  return canonical;
}

function inspectTarBytes(bytes, location = "export archive") {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_EXPORT_UNCOMPRESSED_BYTES) {
    fail(`${location} has an invalid uncompressed size`);
  }
  if (bytes.length % 512 !== 0) fail(`${location} is not aligned to 512-byte tar records`);
  const entries = [];
  const seen = new Map();
  let offset = 0;
  let zeroBlocks = 0;
  let totalPayloadBytes = 0;
  while (offset < bytes.length) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      zeroBlocks += 1;
      offset += 512;
      if (zeroBlocks >= 2) {
        if (!bytes.subarray(offset).every((byte) => byte === 0)) {
          fail(`${location} contains data after its end-of-archive marker`);
        }
        break;
      }
      continue;
    }
    if (zeroBlocks !== 0) fail(`${location} contains an entry after a partial end marker`);
    if (entries.length >= MAX_EXPORT_ENTRIES) fail(`${location} contains too many entries`);
    const recordedChecksum = tarOctalField(header, 148, 8, `${location} header checksum`);
    let computedChecksum = 0;
    for (let index = 0; index < header.length; index += 1) {
      computedChecksum += index >= 148 && index < 156 ? 0x20 : header[index];
    }
    if (recordedChecksum !== computedChecksum) fail(`${location} contains a header with a bad checksum`);
    const name = tarAsciiField(header, 0, 100, `${location} entry name`);
    const magic = header.subarray(257, 263);
    const version = header.subarray(263, 265);
    let prefix = "";
    if (magic.equals(Buffer.from("ustar\0", "binary")) && version.equals(Buffer.from("00", "ascii"))) {
      prefix = tarAsciiField(header, 345, 155, `${location} entry prefix`);
    } else {
      fail(`${location} entry ${JSON.stringify(name)} is not exact POSIX ustar format`);
    }
    const rawName = prefix ? `${prefix}/${name}` : name;
    const typeFlag = header[156];
    const type = typeFlag === 0 || typeFlag === 0x30 ? "file" : typeFlag === 0x35 ? "directory" : null;
    if (!type) fail(`${location} entry ${JSON.stringify(rawName)} has a forbidden tar type`);
    const canonical = safeArchivePath(rawName, type);
    if (seen.has(canonical)) fail(`${location} contains duplicate path ${JSON.stringify(canonical)}`);
    const mode = tarOctalField(header, 100, 8, `${location} entry mode`);
    if ((mode & 0o7000) !== 0) fail(`${location} entry ${JSON.stringify(rawName)} has special mode bits`);
    const size = tarOctalField(header, 124, 12, `${location} entry size`);
    if (type === "directory" && size !== 0) {
      fail(`${location} directory ${JSON.stringify(rawName)} has a non-zero payload`);
    }
    totalPayloadBytes += size;
    if (totalPayloadBytes > MAX_EXPORT_UNCOMPRESSED_BYTES) fail(`${location} payload is too large`);
    const next = offset + 512 + Math.ceil(size / 512) * 512;
    if (next > bytes.length) fail(`${location} entry ${JSON.stringify(rawName)} extends past EOF`);
    const entry = { path: canonical, type, size };
    seen.set(canonical, entry);
    entries.push(entry);
    offset = next;
  }
  if (zeroBlocks < 2) fail(`${location} lacks the two-block end-of-archive marker`);
  if (entries.length === 0) fail(`${location} contains no entries`);
  for (const entry of entries) {
    if (entry.path === ".") continue;
    const parts = entry.path.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const parent = parts.slice(0, index).join("/");
      const parentEntry = seen.get(parent);
      if (parentEntry && parentEntry.type !== "directory") {
        fail(`${location} places ${JSON.stringify(entry.path)} below a non-directory`);
      }
    }
  }
  return entries;
}

function inspectArchive(archive) {
  const stat = fs.lstatSync(archive);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
    fail("export archive must be a single regular file");
  }
  if (stat.size <= 0 || stat.size > MAX_EXPORT_ARCHIVE_BYTES) fail("export archive compressed size is invalid");
  let bytes;
  try {
    bytes = zlib.gunzipSync(fs.readFileSync(archive), { maxOutputLength: MAX_EXPORT_UNCOMPRESSED_BYTES });
  } catch (error) {
    fail(`cannot safely decompress export archive: ${error.message}`);
  }
  return inspectTarBytes(bytes);
}

function validateExtractedTree(destination, archiveEntries) {
  const root = fs.lstatSync(destination);
  if (!root.isDirectory() || root.isSymbolicLink()) fail("extraction destination is not a real directory");
  const actual = new Map();
  const visit = (directory, prefix = "") => {
    for (const name of fs.readdirSync(directory).sort()) {
      const relative = prefix ? `${prefix}/${name}` : name;
      const absolute = path.join(directory, name);
      const stat = fs.lstatSync(absolute);
      if (stat.isSymbolicLink()) fail(`extracted export contains symlink ${JSON.stringify(relative)}`);
      if (stat.isDirectory()) {
        actual.set(relative, "directory");
        visit(absolute, relative);
      } else if (stat.isFile() && stat.nlink === 1) {
        actual.set(relative, "file");
      } else {
        fail(`extracted export contains unsafe entry ${JSON.stringify(relative)}`);
      }
    }
  };
  visit(destination);
  const expected = new Map(
    archiveEntries.filter((entry) => entry.path !== ".").map((entry) => [entry.path, entry.type]),
  );
  if (canonicalJson(Object.fromEntries(actual)) !== canonicalJson(Object.fromEntries(expected))) {
    fail("extracted export tree does not exactly match the validated archive listing");
  }
}

async function extractArchive(archive, destination) {
  const entries = inspectArchive(archive);
  if (fs.existsSync(destination)) fail("refusing to extract an export over an existing path");
  fs.mkdirSync(destination, { recursive: false, mode: 0o700 });
  try {
    await run("tar", ["-xzf", archive, "-C", destination], { capture: false, announce: true });
    validateExtractedTree(destination, entries);
  } catch (error) {
    fs.rmSync(destination, { recursive: true, force: true });
    throw error;
  }
}

async function verifyPromotionCheckout(config, state) {
  const current = await verifySource(config, state.source_config_path || DEFAULT_CONFIG);
  const planned = state.plan && state.plan.source;
  if (!planned || JSON.stringify(current) !== JSON.stringify(planned)) {
    fail("local orchestration checkout changed while the Linode run was in flight; retaining export only");
  }
}

async function verifyRecoveryCheckout(config, state) {
  const planned = requireObject(state.plan && state.plan.source, "state.plan.source");
  if (!FULL_COMMIT.test(planned.orchestration_commit)) {
    fail("recorded Linode orchestration commit is invalid");
  }
  const configFile = path.join(REPO, planned.config_path);
  if (
    path.relative(REPO, configFile).startsWith("..") ||
    path.isAbsolute(path.relative(REPO, configFile)) ||
    path.resolve(state.source_config_path || "") !== configFile
  ) {
    fail("recorded Linode config path is outside or inconsistent with the repository");
  }
  const recordedRemoteRunner = planned.remote_runner_path || "tools/run-linux-fixed-benchmark.js";
  const originalFiles = [
    [planned.config_path, planned.config_sha256, "config"],
    ["tools/run-linode-benchmark.js", planned.coordinator_sha256, "coordinator"],
    [recordedRemoteRunner, planned.remote_runner_sha256, "remote runner"],
  ];
  for (const [file, wanted, label] of originalFiles) {
    if (!SHA256.test(wanted) || sha256(await gitBlob(planned.orchestration_commit, file)) !== wanted) {
      fail(`recorded ${label} does not match the original Linode orchestration commit`);
    }
  }
  const ancestry = await run(
    "git",
    ["merge-base", "--is-ancestor", planned.orchestration_commit, "HEAD"],
    { allowFailure: true },
  );
  if (ancestry.status !== 0) {
    fail("offline promotion requires a checkout descended from the recorded orchestration commit");
  }
  if (sha256File(state.config_path) !== planned.config_sha256) {
    fail("saved Linode protocol snapshot no longer matches the recorded plan");
  }
  if (sha256File(state.runner_path) !== planned.remote_runner_sha256) {
    fail("saved Linode runner snapshot no longer matches the recorded plan");
  }
  const current = await verifySource(config, configFile);
  for (const field of ["config_path", "config_sha256", "remote_runner_sha256"]) {
    if (current[field] !== planned[field]) {
      fail(`current ${field} is incompatible with the recorded Linode run`);
    }
  }
  if (JSON.stringify(current.verified_source_hashes) !== JSON.stringify(planned.verified_source_hashes)) {
    fail("current verified sources are incompatible with the recorded Linode run");
  }
}

function evidenceFiles(directory, relative = "") {
  const files = [];
  for (const entry of fs.readdirSync(path.join(directory, relative), { withFileTypes: true })) {
    const child = path.posix.join(relative.split(path.sep).join(path.posix.sep), entry.name);
    const full = path.join(directory, ...child.split("/"));
    const stat = fs.lstatSync(full);
    if (stat.isSymbolicLink()) fail(`accepted Linode evidence contains symlink ${child}`);
    if (stat.isDirectory()) files.push(...evidenceFiles(directory, path.join(relative, entry.name)));
    else if (stat.isFile() && stat.nlink === 1) files.push(child);
    else fail(`accepted Linode evidence contains non-regular entry ${child}`);
  }
  return files.sort();
}

function requireJsonEqual(actual, expected, location) {
  if (canonicalJson(actual) !== canonicalJson(expected)) fail(`${location} does not match the frozen value`);
}

function loadJsonRegular(file, location) {
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
    fail(`${location} must be a single regular file`);
  }
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`${location} is not valid JSON: ${error.message}`);
  }
}

function requireIsoTimestamp(value, location) {
  requireString(value, location);
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds) || new Date(milliseconds).toISOString() !== value) {
    fail(`${location} must be an exact ISO-8601 UTC timestamp`);
  }
  return milliseconds;
}

function validateAcceptedStatus(status, requiredVerdict) {
  requireObject(status, "accepted status");
  requireExactKeys(
    status,
    ["schema", "status", "detail", "started_at", "finished_at", "verdict"],
    "accepted status",
  );
  if (status.schema !== "rustia.bench.cloud-run-status.v1") fail("accepted status has the wrong schema");
  if (status.status !== "accepted_and_published") fail("accepted status is not accepted_and_published");
  if (status.detail !== "material regression excluded and three actual final-binary history rows appended") {
    fail("accepted status has an unexpected detail");
  }
  if (status.verdict !== requiredVerdict) fail("accepted status has the wrong verdict");
  const started = requireIsoTimestamp(status.started_at, "accepted status.started_at");
  const finished = requireIsoTimestamp(status.finished_at, "accepted status.finished_at");
  if (finished < started) fail("accepted status finished before it started");
}

function validateMemoryEvidenceStatus(status) {
  requireObject(status, "memory evidence status");
  requireExactKeys(
    status,
    ["schema", "status", "detail", "started_at", "finished_at"],
    "memory evidence status",
  );
  if (status.schema !== "rustia.bench.cloud-run-status.v1" || status.status !== "evidence_complete") {
    fail("memory evidence status has the wrong schema or terminal state");
  }
  if (status.detail !== "vm string memory evidence completed; public throughput history was not changed") {
    fail("memory evidence status has an unexpected detail");
  }
  const started = requireIsoTimestamp(status.started_at, "memory evidence status.started_at");
  const finished = requireIsoTimestamp(status.finished_at, "memory evidence status.finished_at");
  if (finished < started) fail("memory evidence status finished before it started");
}

function expectedTerminalStatus(config) {
  if (isCrossEngineMatrix(config)) return "matrix_complete";
  return isVmStringMemory(config) ? "evidence_complete" : "accepted_and_published";
}

function strictUtf8(bytes, location) {
  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) fail(`${location} is not valid UTF-8`);
  return text;
}

function numbersClose(left, right) {
  if (typeof left !== "number" || !Number.isFinite(left) || typeof right !== "number" || !Number.isFinite(right)) {
    return false;
  }
  const scale = Math.max(1, Math.abs(left), Math.abs(right));
  return Math.abs(left - right) <= scale * 1e-12;
}

function matchesHistoryPrecision(serialized, raw) {
  return (
    typeof serialized === "number" &&
    Number.isFinite(serialized) &&
    typeof raw === "number" &&
    Number.isFinite(raw) &&
    Math.abs(serialized - raw) <= 0.500001e-6
  );
}

function quantileSorted(values, probability) {
  if (values.length === 1) return values[0];
  const position = (values.length - 1) * probability;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  if (lower === upper) return values[lower];
  const fraction = position - lower;
  return values[lower] * (1 - fraction) + values[upper] * fraction;
}

function validateCpuDelta(deltaValue, location) {
  const delta = requireObject(deltaValue, location);
  requireExactKeys(delta, ["total_ticks", "idle_ticks", "idle_percent", "steal_ticks"], location);
  requireInteger(delta.total_ticks, `${location}.total_ticks`, 1);
  requireInteger(delta.idle_ticks, `${location}.idle_ticks`, 0);
  requireInteger(delta.steal_ticks, `${location}.steal_ticks`, 0);
  requireNumber(delta.idle_percent, `${location}.idle_percent`, 0);
  if (delta.idle_ticks > delta.total_ticks || delta.idle_percent > 100) fail(`${location} has impossible data`);
  const expectedIdle = Number(((delta.idle_ticks / delta.total_ticks) * 100).toFixed(6));
  if (delta.idle_percent !== expectedIdle) fail(`${location}.idle_percent does not match its tick counts`);
  return delta;
}

function timedStealGatePasses(config, delta) {
  const withinPercentage = (
    delta.steal_ticks * 100 <=
    config.host_gate.maximum_timed_steal_percent * delta.total_ticks
  );
  const tickLimit = config.host_gate.maximum_timed_steal_ticks;
  return withinPercentage || (Number.isInteger(tickLimit) && delta.steal_ticks <= tickLimit);
}

function validateGateLog(config, gateFile, declaredRecords) {
  const text = strictUtf8(fs.readFileSync(gateFile), "gate.jsonl");
  if (!text.endsWith("\n") || text.includes("\r")) fail("gate.jsonl is not canonical JSONL");
  const lines = text.slice(0, -1).split("\n");
  if (lines.length !== declaredRecords || lines.some((line) => line === "")) {
    fail("gate.jsonl record count does not match the manifest");
  }
  const records = lines.map((line, index) => {
    let record;
    try {
      record = JSON.parse(line);
    } catch (error) {
      fail(`gate.jsonl record ${index + 1} is invalid JSON: ${error.message}`);
    }
    if (line !== JSON.stringify(record)) fail(`gate.jsonl record ${index + 1} is not canonical JSON`);
    return requireObject(record, `gate.jsonl record ${index + 1}`);
  });
  const expectedLabels = [
    ...Array.from({ length: config.qualification.candidate_pilot_legs }, (_, index) => `qualification-${index + 1}`),
    ...config.measurement.run_order.map((leg, index) => `accepted-${index + 1}-${leg.file}`),
    ...Array.from({ length: config.publication.final_measurements }, (_, index) => `publication-${index + 1}`),
  ];
  let recordIndex = 0;
  for (const label of expectedLabels) {
    let passedPre = false;
    while (
      recordIndex < records.length &&
      records[recordIndex].label === label &&
      records[recordIndex].phase === "pre_leg_idle_window"
    ) {
      const record = records[recordIndex];
      requireExactKeys(
        record,
        ["phase", "label", "timestamp", "cpu", "load_1m", "cpu_delta", "contaminants", "passed"],
        `gate pre-record ${recordIndex + 1}`,
      );
      requireIsoTimestamp(record.timestamp, `gate pre-record ${recordIndex + 1}.timestamp`);
      if (record.cpu !== config.host_gate.benchmark_cpu) fail("gate pre-record has the wrong CPU");
      requireNumber(record.load_1m, `gate pre-record ${recordIndex + 1}.load_1m`, 0);
      const delta = validateCpuDelta(record.cpu_delta, `gate pre-record ${recordIndex + 1}.cpu_delta`);
      if (!Array.isArray(record.contaminants)) fail("gate pre-record contaminants is not an array");
      const shouldPass =
        record.load_1m <= config.host_gate.maximum_load_1m &&
        delta.idle_percent >= config.host_gate.minimum_idle_percent &&
        delta.steal_ticks <= config.host_gate.maximum_steal_ticks &&
        record.contaminants.length === 0;
      if (record.passed !== shouldPass) fail("gate pre-record has a false passed flag");
      passedPre = record.passed;
      recordIndex += 1;
      if (passedPre) break;
    }
    if (!passedPre) fail(`gate label ${label} lacks a final passing pre-idle record`);
    if (
      recordIndex >= records.length ||
      records[recordIndex].label !== label ||
      records[recordIndex].phase !== "during_leg"
    ) {
      fail(`gate label ${label} lacks exactly one during-leg record`);
    }
    const during = records[recordIndex];
    requireExactKeys(
      during,
      ["phase", "label", "timestamp", "cpu", "cpu_delta", "contaminants", "passed"],
      `gate during-record ${recordIndex + 1}`,
    );
    requireIsoTimestamp(during.timestamp, `gate during-record ${recordIndex + 1}.timestamp`);
    if (during.cpu !== config.host_gate.benchmark_cpu) fail("gate during-record has the wrong CPU");
    const delta = validateCpuDelta(during.cpu_delta, `gate during-record ${recordIndex + 1}.cpu_delta`);
    if (!Array.isArray(during.contaminants)) fail("gate during-record contaminants is not an array");
    const shouldPass = timedStealGatePasses(config, delta) && during.contaminants.length === 0;
    if (during.passed !== shouldPass || !shouldPass) {
      fail(`gate label ${label} did not pass the 0.1%-steal/clean-process timed gate`);
    }
    recordIndex += 1;
  }
  if (recordIndex !== records.length) fail("gate.jsonl contains unexpected labels or extra records");
}

function parseCanonicalJsonl(file, location) {
  const text = strictUtf8(fs.readFileSync(file), location);
  if (!text.endsWith("\n") || text.includes("\r")) fail(`${location} is not canonical JSONL`);
  const lines = text.slice(0, -1).split("\n");
  if (lines.some((line) => line === "")) fail(`${location} contains an empty record`);
  return lines.map((line, index) => {
    let value;
    try {
      value = JSON.parse(line);
    } catch (error) {
      fail(`${location} record ${index + 1} is invalid JSON: ${error.message}`);
    }
    if (line !== JSON.stringify(value)) fail(`${location} record ${index + 1} is not canonical JSON`);
    return requireObject(value, `${location} record ${index + 1}`);
  });
}

function validateMatrixCpuDelta(record, location) {
  requireInteger(record.total_ticks, `${location}.total_ticks`, 1);
  requireInteger(record.idle_ticks, `${location}.idle_ticks`, 0);
  requireInteger(record.steal_ticks, `${location}.steal_ticks`, 0);
  requireNumber(record.idle_percent, `${location}.idle_percent`, 0);
  requireNumber(record.steal_percent, `${location}.steal_percent`, 0);
  if (record.idle_ticks > record.total_ticks || record.steal_ticks > record.total_ticks) {
    fail(`${location} has impossible CPU tick counts`);
  }
  const expectedIdlePercent = (record.idle_ticks / record.total_ticks) * 100;
  const expectedStealPercent = (record.steal_ticks / record.total_ticks) * 100;
  if (record.idle_percent !== expectedIdlePercent || record.steal_percent !== expectedStealPercent) {
    fail(`${location} percentages do not match CPU tick counts`);
  }
}

function validateMatrixGateLog(config, gateFile, status) {
  const records = parseCanonicalJsonl(gateFile, "matrix gate.jsonl");
  const runStarted = requireIsoTimestamp(status.started_at, "matrix gate status.started_at");
  const runFinished = requireIsoTimestamp(status.finished_at, "matrix gate status.finished_at");
  let previousTimestamp = runStarted;
  const requireRunTimestamp = (value, location) => {
    const timestamp = requireIsoTimestamp(value, location);
    if (timestamp < runStarted || timestamp > runFinished) {
      fail(`${location} falls outside the matrix run window`);
    }
    if (timestamp < previousTimestamp) fail(`${location} precedes the prior matrix gate timestamp`);
    previousTimestamp = timestamp;
    return timestamp;
  };
  const expectedLabels = [
    "native-all-engines",
    "native-lua54",
    "wasm-adapter-matrix",
    "wasm-dispatch-matrix",
  ];
  let recordIndex = 0;
  for (const label of expectedLabels) {
    let acceptedTimedAttempt = false;
    for (
      let attempt = 1;
      attempt <= config.host_gate.maximum_timed_attempts_per_group;
      attempt += 1
    ) {
      let acceptedQuietWindow = false;
      let quietAttempt = 0;
      while (
        recordIndex < records.length &&
        records[recordIndex].phase === "pre" &&
        records[recordIndex].label === label &&
        records[recordIndex].timed_attempt === attempt
      ) {
        quietAttempt += 1;
        const record = records[recordIndex];
        const location = `matrix gate pre-record ${recordIndex + 1}`;
        requireExactKeys(
          record,
          [
            "phase", "label", "timed_attempt", "quiet_attempt", "captured_at", "load_1m", "total_ticks",
            "idle_ticks", "steal_ticks", "idle_percent", "steal_percent", "accepted",
          ],
          location,
        );
        if (record.quiet_attempt !== quietAttempt) fail(`${location}.quiet_attempt is not sequential`);
        requireRunTimestamp(record.captured_at, `${location}.captured_at`);
        requireNumber(record.load_1m, `${location}.load_1m`, 0);
        validateMatrixCpuDelta(record, location);
        const shouldAccept =
          record.idle_percent >= config.host_gate.minimum_idle_percent &&
          record.steal_ticks <= config.host_gate.maximum_steal_ticks &&
          record.load_1m <= config.host_gate.maximum_load_1m;
        if (record.accepted !== shouldAccept) fail(`${location} has a false accepted flag`);
        acceptedQuietWindow = record.accepted;
        recordIndex += 1;
        if (acceptedQuietWindow) break;
      }
      if (!acceptedQuietWindow) {
        fail(`matrix gate ${label} attempt ${attempt} lacks a final accepted quiet window`);
      }
      if (
        recordIndex >= records.length ||
        records[recordIndex].phase !== "timed" ||
        records[recordIndex].label !== label ||
        records[recordIndex].timed_attempt !== attempt
      ) {
        fail(`matrix gate ${label} attempt ${attempt} lacks one matching timed record`);
      }
      const record = records[recordIndex];
      const location = `matrix gate timed-record ${recordIndex + 1}`;
      requireExactKeys(
        record,
        [
          "phase", "label", "timed_attempt", "started_at", "finished_at", "total_ticks", "idle_ticks",
          "steal_ticks", "idle_percent", "steal_percent", "accepted",
        ],
        location,
      );
      const started = requireRunTimestamp(record.started_at, `${location}.started_at`);
      const finished = requireRunTimestamp(record.finished_at, `${location}.finished_at`);
      if (finished < started) fail(`${location} finishes before it starts`);
      validateMatrixCpuDelta(record, location);
      const shouldAccept = timedStealGatePasses(config, record);
      if (record.accepted !== shouldAccept) fail(`${location} has a false accepted flag`);
      recordIndex += 1;
      if (record.accepted) {
        acceptedTimedAttempt = true;
        break;
      }
    }
    if (!acceptedTimedAttempt) {
      fail(
        `matrix gate ${label} exhausted ${config.host_gate.maximum_timed_attempts_per_group} ` +
        "attempts without passing the 0.1% / one-tick timed-steal gate",
      );
    }
  }
  if (recordIndex !== records.length) fail("matrix gate.jsonl contains unexpected labels or extra records");
  return records;
}

function validateMemoryGateLog(config, gateFile, declaredRecords, warmupAttempts, acceptedAttempts) {
  const records = parseCanonicalJsonl(gateFile, "memory gate.jsonl");
  if (records.length !== declaredRecords) fail("memory gate record count does not match the manifest");
  const rounds = [
    ...warmupAttempts.map((attempts, index) => ({ prefix: "warmup", round: index + 1, attempts })),
    ...acceptedAttempts.map((attempts, index) => ({ prefix: "round", round: index + 1, attempts })),
  ];
  let recordIndex = 0;
  for (const logical of rounds) {
    requireInteger(logical.attempts, `${logical.prefix} ${logical.round} attempts`, 1);
    if (logical.attempts > config.measurement.round_retry_limit + 1) {
      fail(`${logical.prefix} ${logical.round} exceeded the frozen retry limit`);
    }
    for (let attempt = 1; attempt <= logical.attempts; attempt += 1) {
      const label = `memory-${logical.prefix}-${String(logical.round).padStart(2, "0")}-attempt-${attempt}`;
      let passedPre = false;
      while (
        recordIndex < records.length &&
        records[recordIndex].label === label &&
        records[recordIndex].phase === "pre_leg_idle_window"
      ) {
        const record = records[recordIndex];
        requireExactKeys(
          record,
          ["phase", "label", "timestamp", "cpu", "load_1m", "cpu_delta", "contaminants", "passed"],
          `memory gate pre-record ${recordIndex + 1}`,
        );
        requireIsoTimestamp(record.timestamp, `memory gate pre-record ${recordIndex + 1}.timestamp`);
        if (record.cpu !== config.host_gate.benchmark_cpu) fail("memory gate pre-record has the wrong CPU");
        requireNumber(record.load_1m, `memory gate pre-record ${recordIndex + 1}.load_1m`, 0);
        const delta = validateCpuDelta(record.cpu_delta, `memory gate pre-record ${recordIndex + 1}.cpu_delta`);
        if (!Array.isArray(record.contaminants)) fail("memory gate pre-record contaminants is not an array");
        const shouldPass =
          record.load_1m <= config.host_gate.maximum_load_1m &&
          delta.idle_percent >= config.host_gate.minimum_idle_percent &&
          delta.steal_ticks <= config.host_gate.maximum_steal_ticks &&
          record.contaminants.length === 0;
        if (record.passed !== shouldPass) fail("memory gate pre-record has a false passed flag");
        passedPre = record.passed;
        recordIndex += 1;
        if (passedPre) break;
      }
      if (!passedPre) fail(`memory gate label ${label} lacks a passing pre-idle record`);
      const during = records[recordIndex];
      if (!during || during.label !== label || during.phase !== "during_leg") {
        fail(`memory gate label ${label} lacks exactly one whole-round record`);
      }
      requireExactKeys(
        during,
        ["phase", "label", "timestamp", "cpu", "cpu_delta", "contaminants", "passed"],
        `memory gate during-record ${recordIndex + 1}`,
      );
      requireIsoTimestamp(during.timestamp, `memory gate during-record ${recordIndex + 1}.timestamp`);
      if (during.cpu !== config.host_gate.benchmark_cpu) fail("memory gate during-record has the wrong CPU");
      const delta = validateCpuDelta(during.cpu_delta, `memory gate during-record ${recordIndex + 1}.cpu_delta`);
      if (!Array.isArray(during.contaminants)) fail("memory gate during-record contaminants is not an array");
      const shouldPass = timedStealGatePasses(config, delta) && during.contaminants.length === 0;
      if (during.passed !== shouldPass) fail("memory gate during-record has a false passed flag");
      const isFinalAttempt = attempt === logical.attempts;
      if (during.passed !== isFinalAttempt) {
        fail(`memory gate ${label} acceptance does not match the frozen first-clean-attempt rule`);
      }
      recordIndex += 1;
    }
  }
  if (recordIndex !== records.length) fail("memory gate.jsonl contains unexpected labels or extra records");
}

function validateMemorySampleSnapshot(snapshotValue, location) {
  const snapshot = requireObject(snapshotValue, location);
  requireExactKeys(snapshot, ["status", "smaps_rollup"], location);
  const status = requireObject(snapshot.status, `${location}.status`);
  requireExactKeys(status, ["vm_rss", "vm_hwm", "rss_anon", "rss_file", "rss_shmem"], `${location}.status`);
  for (const [field, value] of Object.entries(status)) requireInteger(value, `${location}.status.${field}`, 0);
  const smaps = requireObject(snapshot.smaps_rollup, `${location}.smaps_rollup`);
  requireExactKeys(
    smaps,
    ["rss", "pss", "private_clean", "private_dirty", "anonymous", "swap"],
    `${location}.smaps_rollup`,
  );
  for (const [field, value] of Object.entries(smaps)) requireInteger(value, `${location}.smaps_rollup.${field}`, 0);
  if (smaps.swap !== 0) fail(`${location}.smaps_rollup.swap must be zero`);
}

function validateMemorySampleRecord(value, expected, config, location) {
  requireExactKeys(
    value,
    [
      "schema_version",
      "schema",
      "sample_id",
      "protocol_sha256",
      "timestamp_unix",
      "build",
      "round",
      "scenario",
      "platform",
      "layout_bytes",
      "timing_ns",
      "memory_kib",
      "validation",
    ],
    location,
  );
  if (value.schema_version !== 1 || value.schema !== "rustia.bench.vm-string-sample.v1") {
    fail(`${location} has the wrong schema`);
  }
  if (
    value.sample_id !== expected.sampleId ||
    value.protocol_sha256 !== expected.protocolHash ||
    value.round !== expected.round
  ) {
    fail(`${location} has the wrong sample, protocol, or round identity`);
  }
  requireInteger(value.timestamp_unix, `${location}.timestamp_unix`, 1);
  const build = requireObject(value.build, `${location}.build`);
  requireExactKeys(build, ["role", "commit"], `${location}.build`);
  if (build.role !== expected.role || build.commit !== expected.commit) fail(`${location} has the wrong build identity`);

  const scenario = requireObject(value.scenario, `${location}.scenario`);
  requireExactKeys(
    scenario,
    [
      "id",
      "vm_count",
      "profile",
      "constant_slots",
      "instruction_slots",
      "executes_constants",
      "strings",
      "nil_constant_count",
    ],
    `${location}.scenario`,
  );
  const shapes = {
    empty: [false, "none", 0, 0, 1024],
    short_unloaded: [false, "short", 1024, 13, 0],
    short_loaded: [true, "short", 1024, 13, 0],
    long_unloaded: [false, "long", 128, 2048, 896],
    long_loaded: [true, "long", 128, 2048, 896],
  };
  const shape = shapes[expected.scenario];
  const strings = requireObject(scenario.strings, `${location}.scenario.strings`);
  requireExactKeys(strings, ["kind", "count", "bytes_each", "generator"], `${location}.scenario.strings`);
  if (
    scenario.id !== expected.scenario ||
    scenario.vm_count !== config.measurement.vm_count ||
    scenario.profile !== "reproducible-standard" ||
    scenario.constant_slots !== 1024 ||
    scenario.instruction_slots !== 1025 ||
    scenario.executes_constants !== shape[0] ||
    strings.kind !== shape[1] ||
    strings.count !== shape[2] ||
    strings.bytes_each !== shape[3] ||
    strings.generator !== "indexed-binary-v1" ||
    scenario.nil_constant_count !== shape[4]
  ) {
    fail(`${location} scenario shape is not frozen`);
  }
  const platform = requireObject(value.platform, `${location}.platform`);
  requireExactKeys(platform, ["os", "arch", "pid", "memory_probe_warmups"], `${location}.platform`);
  if (platform.os !== "linux" || platform.arch !== "x86_64" || platform.memory_probe_warmups !== 2) {
    fail(`${location} platform is not frozen to linux-x86_64`);
  }
  requireInteger(platform.pid, `${location}.platform.pid`, 1);
  for (const [section, fields] of [
    ["layout_bytes", ["value", "instruction", "constant", "vm"]],
    ["timing_ns", ["program_build", "vm_create", "program_run"]],
  ]) {
    const object = requireObject(value[section], `${location}.${section}`);
    requireExactKeys(object, fields, `${location}.${section}`);
    for (const [field, fieldValue] of Object.entries(object)) {
      requireInteger(fieldValue, `${location}.${section}.${field}`, 1);
    }
  }
  const memory = requireObject(value.memory_kib, `${location}.memory_kib`);
  requireExactKeys(memory, ["clean", "program_ready", "vms_ready", "after_run"], `${location}.memory_kib`);
  for (const snapshot of ["clean", "program_ready", "vms_ready", "after_run"]) {
    validateMemorySampleSnapshot(memory[snapshot], `${location}.memory_kib.${snapshot}`);
  }
  const validation = requireObject(value.validation, `${location}.validation`);
  requireExactKeys(
    validation,
    [
      "created_vms",
      "ran_vms",
      "expected_result_kind",
      "expected_result_bytes",
      "expected_result_fnv1a64",
      "all_results_matched",
    ],
    `${location}.validation`,
  );
  const loaded = shape[0];
  if (
    validation.created_vms !== config.measurement.vm_count ||
    validation.ran_vms !== config.measurement.vm_count ||
    validation.expected_result_kind !== (loaded ? "string" : "nil") ||
    validation.expected_result_bytes !== (loaded ? shape[3] : null) ||
    (loaded ? !/^[0-9a-f]{16}$/.test(validation.expected_result_fnv1a64) : validation.expected_result_fnv1a64 !== null) ||
    validation.all_results_matched !== true
  ) {
    fail(`${location} validation identity is wrong`);
  }
}

function validateMemorySampleFile(config, file, prefix, rounds, protocolHash) {
  const records = parseCanonicalJsonl(file, path.basename(file));
  const expectedCount = rounds * config.measurement.scenarios.length * 2;
  if (records.length !== expectedCount) {
    fail(`${path.basename(file)} has ${records.length} records, expected ${expectedCount}`);
  }
  let index = 0;
  const processIds = new Set();
  for (let round = 1; round <= rounds; round += 1) {
    for (const entry of memoryRoundOrder(round, config.measurement.scenarios)) {
      const commit = entry.role === "baseline" ? config.source.baseline_commit : config.source.candidate_commit;
      validateMemorySampleRecord(
        records[index],
        {
          sampleId: memorySampleId(prefix, round, entry.role, entry.scenario),
          protocolHash,
          round,
          role: entry.role,
          commit,
          scenario: entry.scenario,
        },
        config,
        `${path.basename(file)} record ${index + 1}`,
      );
      addFreshMemoryProcessId(processIds, records[index].platform.pid, path.basename(file));
      index += 1;
    }
  }
  return records;
}

function validateRawSidecar(file, config, location, expectedCommit, expectedIdentity = null) {
  const sidecar = requireObject(loadJsonRegular(file, location), location);
  if (sidecar.schema !== "rustia.bench.raw-samples.v1" || sidecar.schema_version !== 1) {
    fail(`${location} has the wrong raw-sample schema`);
  }
  if (sidecar.source !== "script-bench-harness") fail(`${location} has the wrong source`);
  const metadata = requireObject(sidecar.metadata, `${location}.metadata`);
  if (
    metadata.measurement_count !== 1 ||
    metadata.sample_count !== config.measurement.repetitions ||
    !Array.isArray(sidecar.measurements) ||
    sidecar.measurements.length !== 1
  ) {
    fail(`${location} has the wrong measurement/sample counts`);
  }
  const measurement = requireObject(sidecar.measurements[0], `${location}.measurements[0]`);
  const expected = {
    commit: expectedCommit,
    target: "linux-x86_64",
    engine: "rustia",
    workload: config.measurement.workload,
    n: config.measurement.fixed_n,
    n_selection: "fixed",
    reps: config.measurement.repetitions,
    checksum: config.measurement.expected_checksum,
  };
  for (const [field, wanted] of Object.entries(expected)) {
    if (measurement[field] !== wanted) fail(`${location}.measurement.${field} is not frozen`);
  }
  requireInteger(measurement.timestamp_unix, `${location}.measurement.timestamp_unix`, 1);
  for (const field of ["dialect", "unit"]) requireString(measurement[field], `${location}.measurement.${field}`);
  if (
    expectedIdentity &&
    (measurement.dialect !== expectedIdentity.dialect || measurement.unit !== expectedIdentity.unit)
  ) {
    fail(`${location} dialect/unit do not match the recomputed comparison identity`);
  }
  if (!Array.isArray(measurement.samples_ns_per_unit) || measurement.samples_ns_per_unit.length !== measurement.reps) {
    fail(`${location} has the wrong sample count`);
  }
  const ordered = measurement.samples_ns_per_unit.slice().sort((left, right) => left - right);
  if (ordered.some((sample) => typeof sample !== "number" || !Number.isFinite(sample) || sample <= 0)) {
    fail(`${location} contains invalid raw samples`);
  }
  const minimum = ordered[0];
  const harnessMedian = ordered[Math.floor(ordered.length / 2)];
  if (!numbersClose(measurement.ns_per_unit_min, minimum)) fail(`${location} minimum does not match its samples`);
  if (!numbersClose(measurement.ns_per_unit_median, harnessMedian)) fail(`${location} median does not match its samples`);
  if (!numbersClose(measurement.units_per_sec, 1e9 / minimum)) fail(`${location} throughput does not match its samples`);
  const q1 = quantileSorted(ordered, 0.25);
  const q3 = quantileSorted(ordered, 0.75);
  return {
    file: path.basename(file),
    sha256: sha256File(file),
    measurement,
    derived: {
      sample_count: ordered.length,
      min: minimum,
      q1,
      median: quantileSorted(ordered, 0.5),
      harness_median: measurement.ns_per_unit_median,
      q3,
      max: ordered[ordered.length - 1],
      iqr: q3 - q1,
    },
  };
}

function requireCompleteSummary(summaryValue, sidecar, config, location, variant = null) {
  const summary = requireObject(summaryValue, location);
  const measurement = sidecar.measurement;
  const expected = {
    file: sidecar.file,
    sha256: sidecar.sha256,
    timestamp_unix: measurement.timestamp_unix,
    commit: measurement.commit,
    target: measurement.target,
    engine: measurement.engine,
    dialect: measurement.dialect,
    workload: config.measurement.workload,
    unit: measurement.unit,
    n: config.measurement.fixed_n,
    reps: config.measurement.repetitions,
    checksum: config.measurement.expected_checksum,
    ns_per_unit_min: measurement.ns_per_unit_min,
    ns_per_unit_median: measurement.ns_per_unit_median,
    units_per_sec: measurement.units_per_sec,
    ...sidecar.derived,
  };
  if (variant !== null) expected.variant = variant;
  if (Object.keys(summary).length !== Object.keys(expected).length) fail(`${location} has unexpected fields`);
  for (const [field, wanted] of Object.entries(expected)) {
    const matches = typeof wanted === "number" ? numbersClose(summary[field], wanted) : summary[field] === wanted;
    if (!matches) fail(`${location}.${field} does not match its raw sidecar`);
  }
}

function normalizedAnalysis(value) {
  const copy = JSON.parse(JSON.stringify(value));
  for (const group of ["baseline", "candidate"]) {
    if (copy[group] && Array.isArray(copy[group].legs)) {
      for (const leg of copy[group].legs) leg.file = path.basename(leg.file);
    }
  }
  return copy;
}

async function recomputeAnalysis(config, evidenceDirectory) {
  const argumentsList = [path.join(REPO, "bench/analyze-fixed-samples.js")];
  for (const leg of config.measurement.run_order.filter((entry) => entry.variant === "baseline")) {
    argumentsList.push("--baseline", path.join(evidenceDirectory, leg.file));
  }
  for (const leg of config.measurement.run_order.filter((entry) => entry.variant === "candidate")) {
    argumentsList.push("--candidate", path.join(evidenceDirectory, leg.file));
  }
  argumentsList.push(
    "--bootstrap-reps",
    String(config.measurement.bootstrap_repetitions),
    "--seed",
    String(config.measurement.bootstrap_seed),
    "--material-threshold",
    String(config.measurement.material_regression_threshold_percent),
    "--expected-legs",
    String(config.measurement.expected_legs_per_group),
    "--require-full-commits",
  );
  const result = await run("node", argumentsList);
  try {
    return JSON.parse(result.stdout.toString("utf8"));
  } catch (error) {
    fail(`local fixed-sample analyzer returned invalid JSON: ${error.message}`);
  }
}

async function validateAcceptedEvidence(config, state, evidenceDirectory, localHistory, remoteHistory) {
  const manifestFile = path.join(evidenceDirectory, "manifest.json");
  const analysisFile = path.join(evidenceDirectory, "analysis.json");
  const protocolFile = path.join(evidenceDirectory, "protocol.json");
  const providerFile = path.join(evidenceDirectory, "provider.json");
  for (const [file, location] of [
    [manifestFile, "evidence manifest"],
    [analysisFile, "fixed-sample analysis"],
    [protocolFile, "evidence protocol"],
    [providerFile, "evidence provider metadata"],
  ]) {
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) fail(`${location} is not a regular file`);
  }

  const manifest = requireObject(loadJsonRegular(manifestFile, "evidence manifest"), "evidence manifest");
  requireExactKeys(
    manifest,
    [
      "schema_version",
      "schema",
      "created_at",
      "provider",
      "source",
      "build",
      "host",
      "protocol",
      "qualification",
      "comparison",
      "publication",
      "evidence_sha256",
    ],
    "evidence manifest",
  );
  if (manifest.schema_version !== 1 || manifest.schema !== "rustia.bench.cloud-evidence.v1") {
    fail("accepted Linode evidence has the wrong manifest schema");
  }
  requireIsoTimestamp(manifest.created_at, "evidence manifest.created_at");
  const actualFiles = evidenceFiles(evidenceDirectory).filter((file) => file !== "manifest.json");
  const hashes = requireObject(manifest.evidence_sha256, "evidence manifest.evidence_sha256");
  const listed = Object.keys(hashes).sort();
  if (canonicalJson(listed) !== canonicalJson(actualFiles)) {
    fail("accepted Linode evidence hash inventory does not exactly match its files");
  }
  for (const file of listed) {
    if (
      file.split("/").includes("..") ||
      !SHA256.test(hashes[file]) ||
      sha256File(path.join(evidenceDirectory, ...file.split("/"))) !== hashes[file]
    ) {
      fail(`accepted Linode evidence hash is wrong or unsafe for ${file}`);
    }
  }

  const protocol = loadJsonRegular(protocolFile, "evidence protocol");
  validateConfig(protocol);
  requireJsonEqual(protocol, config, "evidence protocol");
  if (sha256File(protocolFile) !== state.plan.source.config_sha256) {
    fail("evidence protocol bytes do not match the planned frozen config");
  }
  const provider = requireObject(loadJsonRegular(providerFile, "evidence provider metadata"), "evidence provider metadata");
  const localProvider = loadJsonRegular(path.join(state.__directory, "provider.json"), "local provider snapshot");
  requireJsonEqual(provider, localProvider, "evidence provider metadata");
  requireJsonEqual(provider.catalog_plan, state.plan, "evidence provider catalog plan");
  const providerIdentity = [
    [provider.provider, "linode", "provider"],
    [provider.linode_id, state.linode_id, "Linode ID"],
    [provider.linode_label, state.linode_label, "Linode label"],
    [provider.region, config.provider.region, "region"],
    [provider.size_slug, config.provider.size.id, "plan"],
    [provider.image, config.provider.image, "image"],
    [provider.vcpus, config.provider.size.vcpus, "vCPUs"],
    [provider.memory_mib, config.provider.size.memory_mib, "memory"],
    [provider.disk_gib, config.provider.size.disk_gib, "disk"],
    [provider.public_ipv4, state.public_ipv4, "public IPv4"],
    [provider.backups_enabled, false, "backup state"],
    [provider.interface_generation, "legacy_config", "interface generation"],
  ];
  for (const [actual, expected, field] of providerIdentity) {
    if (actual !== expected) fail(`evidence provider ${field} does not match the destroyed Linode`);
  }
  requireJsonEqual(manifest.provider, provider, "manifest provider identity");

  const source = requireObject(manifest.source, "evidence manifest.source");
  const expectedSource = {
    baseline_commit: config.source.baseline_commit,
    candidate_commit: config.source.candidate_commit,
    harness_commit: config.source.harness_commit,
    baseline_original_harness_sha256: config.source.expected_sha256.baseline_original_harness,
    baseline_overlay_harness_sha256: config.source.expected_sha256.overlay_harness,
    baseline_overlay_disclosure:
      "bench/src/main.rs from the harness commit was copied byte-for-byte over the baseline source before compilation",
    workload_sha256: config.source.expected_sha256.workload,
    cargo_lock_sha256: config.source.expected_sha256.cargo_lock,
    toolchain_sha256: config.source.expected_sha256.toolchain,
    analyzer_sha256: config.source.expected_sha256.analyzer,
    git_bundle_sha256: state.bundle_sha256,
    protocol_config_sha256: sha256File(protocolFile),
    provider_metadata_sha256: sha256File(providerFile),
  };
  requireJsonEqual(source, expectedSource, "manifest source identity");
  if (sha256File(path.join(REPO, "bench/analyze-fixed-samples.js")) !== source.analyzer_sha256) {
    fail("local analyzer does not match the frozen manifest identity");
  }

  const hostFile = path.join(evidenceDirectory, "host.json");
  const hostJson = requireObject(loadJsonRegular(hostFile, "host.json"), "host.json");
  if (hostJson.virtualization !== "kvm") fail("Linode host evidence is not a KVM guest");
  const host = requireObject(manifest.host, "manifest.host");
  if (host.file !== "host.json" || host.sha256 !== sha256File(hostFile)) {
    fail("manifest host identity does not match host.json");
  }
  for (const [field, wanted] of Object.entries({
    cpu_model: hostJson.cpu_model,
    microcode: hostJson.microcode,
    clocksource: hostJson.clocksource,
    scaling_governors: hostJson.scaling_governors,
    benchmark_cpu: config.host_gate.benchmark_cpu,
  })) {
    requireJsonEqual(host[field], wanted, `manifest.host.${field}`);
  }

  const protocolManifest = requireObject(manifest.protocol, "manifest.protocol");
  const expectedProtocol = {
    workload: config.measurement.workload,
    fixed_n: config.measurement.fixed_n,
    repetitions_per_leg: config.measurement.repetitions,
    order: config.measurement.run_order,
    common_arguments: [
      "--engines",
      "rustia",
      "--workloads",
      config.measurement.workload,
      "--fixed-n",
      String(config.measurement.fixed_n),
      "--reps",
      String(config.measurement.repetitions),
      "--no-history",
      "--no-results",
      "--no-latest-json",
    ],
    bootstrap_repetitions: config.measurement.bootstrap_repetitions,
    bootstrap_seed: config.measurement.bootstrap_seed,
    material_regression_threshold_percent: config.measurement.material_regression_threshold_percent,
    required_verdict: config.publication.required_verdict,
    host_gate: config.host_gate,
  };
  for (const [field, wanted] of Object.entries(expectedProtocol)) {
    requireJsonEqual(protocolManifest[field], wanted, `manifest.protocol.${field}`);
  }
  if (
    protocolManifest.exclusion_rule !==
    "any pre/during-leg host-gate violation discards the whole eight-leg block; no leg is selected by timing outcome"
  ) {
    fail("manifest protocol exclusion rule is not frozen");
  }
  const gateLog = requireObject(protocolManifest.gate_log, "manifest.protocol.gate_log");
  const gateFile = path.join(evidenceDirectory, "gate.jsonl");
  if (gateLog.file !== "gate.jsonl" || gateLog.sha256 !== sha256File(gateFile)) {
    fail("manifest gate log identity does not match gate.jsonl");
  }
  requireInteger(gateLog.records, "manifest.protocol.gate_log.records", 1);
  validateGateLog(config, gateFile, gateLog.records);

  const analysis = requireObject(loadJsonRegular(analysisFile, "fixed-sample analysis"), "fixed-sample analysis");
  const recomputed = await recomputeAnalysis(config, evidenceDirectory);
  requireJsonEqual(normalizedAnalysis(analysis), normalizedAnalysis(recomputed), "fixed-sample analysis");
  if (
    analysis.schema_version !== 1 ||
    analysis.schema !== "rustia.bench.fixed-sample-analysis.v1" ||
    analysis.comparison.verdict.status !== config.publication.required_verdict
  ) {
    fail("fixed-sample analysis did not independently exclude a material regression");
  }
  const analysisIdentity = requireObject(analysis.identity, "fixed-sample analysis.identity");
  const expectedAnalysisIdentity = {
    target: "linux-x86_64",
    engine: "rustia",
    workload: config.measurement.workload,
    fixed_n: config.measurement.fixed_n,
    reps_per_leg: config.measurement.repetitions,
    legs_per_group: config.measurement.expected_legs_per_group,
    checksum: config.measurement.expected_checksum,
  };
  for (const [field, wanted] of Object.entries(expectedAnalysisIdentity)) {
    if (analysisIdentity[field] !== wanted) fail(`fixed-sample analysis.identity.${field} is not frozen`);
  }
  for (const field of ["dialect", "unit"]) requireString(analysisIdentity[field], `analysis.identity.${field}`);
  if (
    analysis.baseline.commit !== config.source.baseline_commit ||
    analysis.candidate.commit !== config.source.candidate_commit ||
    analysis.method.bootstrap_iterations !== config.measurement.bootstrap_repetitions ||
    analysis.method.bootstrap_seed_uint32 !== config.measurement.bootstrap_seed ||
    analysis.method.expected_legs !== config.measurement.expected_legs_per_group ||
    analysis.method.require_full_commits !== true ||
    analysis.comparison.material_regression_threshold_percent !==
      config.measurement.material_regression_threshold_percent
  ) {
    fail("fixed-sample analysis source or method identity is not frozen");
  }

  const qualification = requireObject(manifest.qualification, "evidence manifest.qualification");
  if (
    qualification.schema !== "rustia.bench.cloud-qualification.v1" ||
    qualification.passed !== true ||
    qualification.maximum_median_spread_percent !== config.qualification.max_median_spread_percent ||
    qualification.note !== "qualification-only runs; never accepted as A/B evidence or public history" ||
    !Array.isArray(qualification.pilot_legs) ||
    qualification.pilot_legs.length !== config.qualification.candidate_pilot_legs
  ) {
    fail("accepted Linode evidence did not pass the frozen qualification gate");
  }
  const qualificationDirectory = path.join(evidenceDirectory, "qualification-sidecars");
  const pilotMedians = [];
  qualification.pilot_legs.forEach((summary, index) => {
    const sidecar = validateRawSidecar(
      path.join(qualificationDirectory, `candidate-pilot-${index + 1}.json`),
      config,
      `qualification sidecar ${index + 1}`,
      config.source.candidate_commit,
      analysisIdentity,
    );
    requireCompleteSummary(summary, sidecar, config, `manifest qualification.pilot_legs[${index}]`);
    pilotMedians.push(sidecar.measurement.ns_per_unit_median);
  });
  const spread = ((Math.max(...pilotMedians) - Math.min(...pilotMedians)) / Math.min(...pilotMedians)) * 100;
  if (!numbersClose(qualification.median_spread_percent, spread) || spread > config.qualification.max_median_spread_percent) {
    fail("qualification spread does not match its raw pilot sidecars or threshold");
  }
  requireJsonEqual(
    loadJsonRegular(path.join(evidenceDirectory, "qualification.json"), "qualification.json"),
    qualification,
    "qualification.json",
  );
  requireJsonEqual(
    loadJsonRegular(path.join(qualificationDirectory, "qualification.json"), "qualification sidecar summary"),
    qualification,
    "qualification-sidecars/qualification.json",
  );

  const comparison = requireObject(manifest.comparison, "evidence manifest.comparison");
  if (
    comparison.analysis_sha256 !== sha256File(analysisFile) ||
    comparison.verdict.status !== config.publication.required_verdict
  ) {
    fail("manifest comparison analysis identity or verdict is wrong");
  }
  requireJsonEqual(comparison.verdict, analysis.comparison.verdict, "manifest comparison verdict");
  for (const [field, wanted] of Object.entries({
    candidate_delta_percent: analysis.comparison.candidate_delta_percent,
    pooled_bootstrap_ci_95_percent: analysis.comparison.pooled_bootstrap_ci_95_percent,
    hierarchical_bootstrap_ci_95_percent: analysis.comparison.hierarchical_bootstrap_ci_95_percent,
    conservative_bootstrap_ci_95_percent: analysis.comparison.bootstrap_ci_95_percent,
  })) {
    requireJsonEqual(comparison[field], wanted, `manifest.comparison.${field}`);
  }
  if (!Array.isArray(comparison.legs) || comparison.legs.length !== config.measurement.run_order.length) {
    fail("manifest comparison has the wrong number of legs");
  }
  config.measurement.run_order.forEach((leg, index) => {
    const expectedCommit = leg.variant === "baseline" ? config.source.baseline_commit : config.source.candidate_commit;
    const sidecar = validateRawSidecar(
      path.join(evidenceDirectory, leg.file),
      config,
      `comparison sidecar ${leg.file}`,
      expectedCommit,
      analysisIdentity,
    );
    requireCompleteSummary(comparison.legs[index], sidecar, config, `manifest comparison.legs[${index}]`, leg.variant);
  });

  const suffix = strictUtf8(remoteHistory.subarray(localHistory.length), "remote history suffix");
  if (!suffix.endsWith("\n") || suffix.includes("\r")) fail("remote history suffix is not canonical JSONL");
  const historyRows = suffix.slice(0, -1).split("\n").map((line, index) => {
    let row;
    try {
      row = JSON.parse(line);
    } catch (error) {
      fail(`remote history row ${index + 1} is invalid JSON: ${error.message}`);
    }
    requireObject(row, `remote history row ${index + 1}`);
    requireExactKeys(
      row,
      [
        "timestamp_unix",
        "commit",
        "target",
        "engine",
        "dialect",
        "workload",
        "unit",
        "n",
        "reps",
        "ns_per_unit_min",
        "ns_per_unit_median",
        "units_per_sec",
        "checksum",
      ],
      `remote history row ${index + 1}`,
    );
    return row;
  });
  if (historyRows.length !== config.publication.final_measurements) fail("remote history has the wrong suffix length");
  const finalSidecars = historyRows.map((row, index) => {
    const finalFile = finalSidecarName(config, index + 1);
    const sidecar = validateRawSidecar(
      path.join(evidenceDirectory, finalFile),
      config,
      `final measurement sidecar ${index + 1}`,
      config.source.candidate_commit,
      analysisIdentity,
    );
    const measurement = sidecar.measurement;
    if (
      row.timestamp_unix !== measurement.timestamp_unix ||
      row.commit !== measurement.commit ||
      row.target !== measurement.target ||
      row.engine !== measurement.engine ||
      row.dialect !== measurement.dialect ||
      row.workload !== measurement.workload ||
      row.unit !== measurement.unit ||
      row.n !== measurement.n ||
      row.reps !== measurement.reps ||
      !matchesHistoryPrecision(row.ns_per_unit_min, measurement.ns_per_unit_min) ||
      !matchesHistoryPrecision(row.ns_per_unit_median, measurement.ns_per_unit_median) ||
      !matchesHistoryPrecision(row.units_per_sec, measurement.units_per_sec) ||
      row.checksum !== measurement.checksum
    ) {
      fail(`remote history row ${index + 1} does not match ${finalFile}`);
    }
    return sidecar;
  });

  const publication = requireObject(manifest.publication, "evidence manifest.publication");
  const beforeLines = strictUtf8(localHistory, "local history").trimEnd().split("\n").length;
  const expectedPublication = {
    performed: true,
    history_before_sha256: config.source.expected_sha256.history_before,
    history_after_sha256: sha256(remoteHistory),
    history_lines_before: beforeLines,
    history_lines_after: beforeLines + config.publication.final_measurements,
  };
  for (const [field, wanted] of Object.entries(expectedPublication)) {
    if (publication[field] !== wanted) fail(`accepted Linode evidence publication.${field} is wrong`);
  }
  requireJsonEqual(publication.appended_rows, historyRows, "manifest publication rows");
  if (!Array.isArray(publication.sidecars) || publication.sidecars.length !== finalSidecars.length) {
    fail("manifest publication has the wrong number of sidecars");
  }
  finalSidecars.forEach((sidecar, index) =>
    requireCompleteSummary(publication.sidecars[index], sidecar, config, `manifest publication.sidecars[${index}]`),
  );
}

async function recomputeMemoryAnalysis(config, evidenceDirectory) {
  const argumentsList = [
    path.join(REPO, "bench/analyze-vm-string-memory.js"),
    "--input",
    path.join(evidenceDirectory, "samples.jsonl"),
    "--baseline",
    config.source.baseline_commit,
    "--candidate",
    config.source.candidate_commit,
    "--rounds",
    String(config.measurement.accepted_rounds),
    "--bootstrap-reps",
    String(config.measurement.bootstrap_repetitions),
    "--seed",
    String(config.measurement.bootstrap_seed),
  ];
  const result = await run("node", argumentsList);
  try {
    return requireObject(JSON.parse(result.stdout.toString("utf8")), "recomputed vm string memory analysis");
  } catch (error) {
    fail(`local vm string memory analyzer returned invalid JSON: ${error.message}`);
  }
}

async function validateMemoryEvidence(config, state, evidenceDirectory) {
  const files = {
    manifest: path.join(evidenceDirectory, "manifest.json"),
    analysis: path.join(evidenceDirectory, "analysis.json"),
    protocol: path.join(evidenceDirectory, "protocol.json"),
    provider: path.join(evidenceDirectory, "provider.json"),
    host: path.join(evidenceDirectory, "host.json"),
    samples: path.join(evidenceDirectory, "samples.jsonl"),
    warmup: path.join(evidenceDirectory, "warmup-samples.jsonl"),
    gate: path.join(evidenceDirectory, "gate.jsonl"),
  };
  for (const [name, file] of Object.entries(files)) {
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
      fail(`vm string memory ${name} must be a single regular file`);
    }
  }
  requireJsonEqual(loadJsonRegular(files.protocol, "memory protocol"), config, "memory evidence protocol");
  const manifest = requireObject(loadJsonRegular(files.manifest, "memory evidence manifest"), "memory evidence manifest");
  requireExactKeys(
    manifest,
    [
      "schema_version",
      "schema",
      "created_at",
      "provider",
      "source",
      "build",
      "host",
      "protocol",
      "analysis",
      "publication",
      "evidence_sha256",
    ],
    "memory evidence manifest",
  );
  if (manifest.schema_version !== 1 || manifest.schema !== "rustia.bench.vm-string-memory-evidence.v1") {
    fail("vm string memory evidence has the wrong manifest schema");
  }
  requireIsoTimestamp(manifest.created_at, "memory evidence manifest.created_at");
  const completeInventory = evidenceFiles(evidenceDirectory);
  requireJsonEqual(
    completeInventory,
    VM_STRING_EVIDENCE_FILES,
    "vm string memory evidence file allowlist",
  );
  const actualFiles = completeInventory.filter((file) => file !== "manifest.json");
  const hashes = requireObject(manifest.evidence_sha256, "memory evidence manifest.evidence_sha256");
  if (
    canonicalJson(Object.keys(hashes).sort()) !== canonicalJson(VM_STRING_HASHED_EVIDENCE_FILES) ||
    canonicalJson(actualFiles) !== canonicalJson(VM_STRING_HASHED_EVIDENCE_FILES)
  ) {
    fail("vm string memory evidence hash inventory does not exactly match its files");
  }
  for (const file of actualFiles) {
    if (
      file.split("/").includes("..") ||
      !SHA256.test(hashes[file]) ||
      sha256File(path.join(evidenceDirectory, ...file.split("/"))) !== hashes[file]
    ) {
      fail(`vm string memory evidence hash is wrong or unsafe for ${file}`);
    }
  }

  const provider = requireObject(loadJsonRegular(files.provider, "memory provider metadata"), "memory provider metadata");
  const localProvider = loadJsonRegular(path.join(state.__directory, "provider.json"), "local memory provider snapshot");
  requireJsonEqual(provider, localProvider, "memory evidence provider metadata");
  requireJsonEqual(provider.catalog_plan, state.plan, "memory evidence provider catalog plan");
  const providerIdentity = [
    [provider.provider, "linode", "provider"],
    [provider.linode_id, state.linode_id, "Linode ID"],
    [provider.linode_label, state.linode_label, "Linode label"],
    [provider.region, config.provider.region, "region"],
    [provider.size_slug, config.provider.size.id, "plan"],
    [provider.image, config.provider.image, "image"],
    [provider.vcpus, config.provider.size.vcpus, "vCPUs"],
    [provider.memory_mib, config.provider.size.memory_mib, "memory"],
    [provider.disk_gib, config.provider.size.disk_gib, "disk"],
    [provider.public_ipv4, state.public_ipv4, "public IPv4"],
    [provider.backups_enabled, false, "backup state"],
    [provider.interface_generation, "legacy_config", "interface generation"],
    [provider.maintenance_policy, "linode/power_off_on", "maintenance policy"],
  ];
  for (const [actual, wanted, field] of providerIdentity) {
    if (actual !== wanted) fail(`memory evidence provider ${field} does not match the destroyed Linode`);
  }
  requireJsonEqual(manifest.provider, provider, "memory manifest provider identity");

  const source = requireObject(manifest.source, "memory manifest.source");
  const expectedSource = {
    baseline_commit: config.source.baseline_commit,
    candidate_commit: config.source.candidate_commit,
    harness_commit: config.source.harness_commit,
    baseline_original_bench_manifest_sha256: config.source.expected_sha256.baseline_original_bench_manifest,
    candidate_original_bench_manifest_sha256: config.source.expected_sha256.candidate_original_bench_manifest,
    overlay_bench_manifest_sha256: config.source.expected_sha256.overlay_bench_manifest,
    sampler_sha256: config.source.expected_sha256.sampler,
    analyzer_sha256: config.source.expected_sha256.analyzer,
    overlay_disclosure:
      "bench/Cargo.toml and bench/src/bin/vm-string-memory.rs from the harness commit were copied byte-for-byte over both source trees before compilation",
    cargo_lock_sha256: config.source.expected_sha256.cargo_lock,
    toolchain_sha256: config.source.expected_sha256.toolchain,
    git_bundle_sha256: state.bundle_sha256,
    protocol_config_sha256: sha256File(files.protocol),
    provider_metadata_sha256: sha256File(files.provider),
  };
  requireJsonEqual(source, expectedSource, "memory manifest source identity");
  if (sha256File(path.join(REPO, "bench/analyze-vm-string-memory.js")) !== source.analyzer_sha256) {
    fail("local vm string memory analyzer does not match the frozen source identity");
  }

  const hostJson = requireObject(loadJsonRegular(files.host, "memory host.json"), "memory host.json");
  if (hostJson.virtualization !== "kvm") fail("memory host evidence is not a KVM guest");
  const host = requireObject(manifest.host, "memory manifest.host");
  const expectedHost = {
    file: "host.json",
    sha256: sha256File(files.host),
    cpu_model: hostJson.cpu_model,
    microcode: hostJson.microcode,
    clocksource: hostJson.clocksource,
    scaling_governors: hostJson.scaling_governors,
    benchmark_cpu: config.host_gate.benchmark_cpu,
  };
  requireJsonEqual(host, expectedHost, "memory manifest host identity");

  const build = requireObject(manifest.build, "memory manifest.build");
  requireExactKeys(
    build,
    [
      "rustc_vv",
      "cargo_version",
      "baseline_binary_sha256",
      "baseline_binary_bytes",
      "candidate_binary_sha256",
      "candidate_binary_bytes",
      "baseline_command",
      "candidate_command",
      "binary",
      "cargo_incremental",
      "rustc_wrapper",
      "locked",
      "offline_build_after_fetch",
      "source_paths_retained_through_all_measurements",
    ],
    "memory manifest.build",
  );
  for (const field of ["baseline_binary_sha256", "candidate_binary_sha256"]) {
    if (!SHA256.test(build[field])) fail(`memory manifest.build.${field} is not a SHA-256`);
  }
  for (const field of ["baseline_binary_bytes", "candidate_binary_bytes"]) {
    requireInteger(build[field], `memory manifest.build.${field}`, 1);
  }
  for (const field of ["rustc_vv", "cargo_version"]) requireString(build[field], `memory manifest.build.${field}`);
  if (build.rustc_vv !== hostJson.rustc_vv || build.cargo_version !== hostJson.cargo_version) {
    fail("memory manifest build tool versions do not match host.json");
  }
  if (
    build.binary !== "vm-string-memory" ||
    build.cargo_incremental !== "0" ||
    build.rustc_wrapper !== "unset" ||
    build.locked !== true ||
    build.offline_build_after_fetch !== true
  ) {
    fail("memory manifest build mode is not frozen");
  }
  const expectedBuildSuffix = [
    "+1.95.0",
    "build",
    "--locked",
    "--offline",
    "--manifest-path",
  ];
  for (const [role, sourcePath] of [["baseline", "/opt/rustia/src-a/Cargo.toml"], ["candidate", "/opt/rustia/src-b/Cargo.toml"]]) {
    const command = build[`${role}_command`];
    const expected = [
      "/root/.cargo/bin/cargo",
      ...expectedBuildSuffix,
      sourcePath,
      "-p",
      "script-bench-harness",
      "--bin",
      "vm-string-memory",
      "--release",
    ];
    requireJsonEqual(command, expected, `memory manifest ${role} build command`);
  }
  requireJsonEqual(
    build.source_paths_retained_through_all_measurements,
    ["/opt/rustia/src-a", "/opt/rustia/src-b", "/opt/rustia/src-harness"],
    "memory manifest retained source paths",
  );

  const protocol = requireObject(manifest.protocol, "memory manifest.protocol");
  const warmupAttempts = protocol.warmup_attempts;
  const acceptedAttempts = protocol.accepted_attempts;
  const expectedAnalyzerCommand = [
    "node",
    "bench/analyze-vm-string-memory.js",
    "--input",
    "samples.jsonl",
    "--baseline",
    config.source.baseline_commit,
    "--candidate",
    config.source.candidate_commit,
    "--rounds",
    String(config.measurement.accepted_rounds),
    "--bootstrap-reps",
    String(config.measurement.bootstrap_repetitions),
    "--seed",
    String(config.measurement.bootstrap_seed),
  ];
  const expectedProtocol = {
    kind: VM_STRING_MEMORY_PROTOCOL,
    vm_count: config.measurement.vm_count,
    scenarios: config.measurement.scenarios,
    warmup_rounds: config.measurement.warmup_rounds,
    accepted_rounds: config.measurement.accepted_rounds,
    round_retry_limit: config.measurement.round_retry_limit,
    warmup_sample_count: config.measurement.warmup_rounds * config.measurement.scenarios.length * 2,
    accepted_sample_count: config.measurement.accepted_rounds * config.measurement.scenarios.length * 2,
    warmup_attempts: warmupAttempts,
    accepted_attempts: acceptedAttempts,
    warmup_order: memoryRoundOrder(1, config.measurement.scenarios),
    accepted_orders: Array.from(
      { length: config.measurement.accepted_rounds },
      (_, index) => memoryRoundOrder(index + 1, config.measurement.scenarios),
    ),
    order_rule:
      "ABBA/BAAB build-block alternation in consecutive opposite-role pairs; each pair shares one rotated first-block scenario order and the second block reverses it; every sample is a fresh process",
    bootstrap_repetitions: config.measurement.bootstrap_repetitions,
    bootstrap_seed: config.measurement.bootstrap_seed,
    analyzer_command: expectedAnalyzerCommand,
    host_gate: config.host_gate,
    contaminating_process_comm_regex:
      "^(apt|apt-get|as|cargo|cc|cc1|cc1plus|clang|clang\\+\\+|curl|dpkg|g\\+\\+|gcc|git|ld|ld\\.lld|make|ninja|node|rustc|rustup|sccache|script-bench|unattended-upgr|vm-string-memor|vm-string-memory)$",
    exclusion_rule:
      "a host-gate violation discards all ten samples in the round; retries are capped before execution and never selected by measurement outcome",
    gate_log: protocol.gate_log,
  };
  requireJsonEqual(protocol, expectedProtocol, "memory manifest protocol");
  if (!Array.isArray(warmupAttempts) || warmupAttempts.length !== config.measurement.warmup_rounds) {
    fail("memory manifest has the wrong warmup attempt count");
  }
  if (!Array.isArray(acceptedAttempts) || acceptedAttempts.length !== config.measurement.accepted_rounds) {
    fail("memory manifest has the wrong accepted attempt count");
  }
  const gateLog = requireObject(protocol.gate_log, "memory manifest.protocol.gate_log");
  requireExactKeys(gateLog, ["file", "sha256", "records"], "memory manifest.protocol.gate_log");
  if (gateLog.file !== "gate.jsonl" || gateLog.sha256 !== sha256File(files.gate)) {
    fail("memory manifest gate log identity is wrong");
  }
  requireInteger(gateLog.records, "memory manifest gate records", 1);
  validateMemoryGateLog(config, files.gate, gateLog.records, warmupAttempts, acceptedAttempts);

  const protocolHash = sha256File(files.protocol);
  validateMemorySampleFile(config, files.warmup, "warmup", config.measurement.warmup_rounds, protocolHash);
  validateMemorySampleFile(config, files.samples, "round", config.measurement.accepted_rounds, protocolHash);
  const analysis = requireObject(loadJsonRegular(files.analysis, "vm string memory analysis"), "vm string memory analysis");
  const recomputed = await recomputeMemoryAnalysis(config, evidenceDirectory);
  requireJsonEqual(analysis, recomputed, "vm string memory analysis");
  requireExactKeys(
    analysis,
    ["schema_version", "schema", "identity", "method", "baseline", "candidate", "comparison"],
    "vm string memory analysis",
  );
  if (analysis.schema_version !== 1 || analysis.schema !== "rustia.bench.vm-string-memory-analysis.v1") {
    fail("vm string memory analysis has the wrong schema");
  }
  const identity = requireObject(analysis.identity, "vm string memory analysis.identity");
  const expectedIdentity = {
    baseline_commit: config.source.baseline_commit,
    candidate_commit: config.source.candidate_commit,
    accepted_rounds: config.measurement.accepted_rounds,
    sample_count: config.measurement.accepted_rounds * config.measurement.scenarios.length * 2,
    protocol_sha256: protocolHash,
  };
  for (const [field, wanted] of Object.entries(expectedIdentity)) {
    if (identity[field] !== wanted) fail(`vm string memory analysis.identity.${field} is not frozen`);
  }
  if (
    analysis.method.bootstrap_repetitions !== config.measurement.bootstrap_repetitions ||
    analysis.method.bootstrap_seed_uint32 !== config.measurement.bootstrap_seed ||
    analysis.baseline.commit !== config.source.baseline_commit ||
    analysis.candidate.commit !== config.source.candidate_commit
  ) {
    fail("vm string memory analysis method or commit identity is not frozen");
  }
  requireJsonEqual(
    manifest.analysis,
    { file: "analysis.json", sha256: sha256File(files.analysis), schema: analysis.schema, identity: analysis.identity },
    "memory manifest analysis identity",
  );
  requireJsonEqual(
    manifest.publication,
    {
      mode: "evidence_only",
      throughput_history_modified: false,
      imported_path: `bench/evidence/vm-string-memory/${config.source.candidate_commit}`,
    },
    "memory manifest publication",
  );
}

async function promoteMemoryEvidence(
  config,
  state,
  downloadDirectory,
  verifyCheckout = verifyPromotionCheckout,
) {
  if (!state.linode_destroyed_at) fail("refusing memory evidence import before confirmed Linode deletion");
  throwIfInterrupted();
  const status = loadJsonRegular(path.join(downloadDirectory, "status.json"), "remote memory status");
  state.remote_status = status.status;
  state.remote_status_detail = status.detail || null;
  saveState(state);
  if (status.status !== "evidence_complete") {
    process.stdout.write(
      `Linode memory run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
        `(${status.status}${status.detail ? `: ${status.detail}` : ""}).\n`,
    );
    return false;
  }
  validateMemoryEvidenceStatus(status);
  await verifyCheckout(config, state);
  const layout = memoryPromotionLayout(config, downloadDirectory);
  const evidenceSource = layout.evidence_source;
  const evidenceDestination = layout.evidence_destination;
  for (const required of ["manifest.json", "analysis.json", "samples.jsonl", "warmup-samples.jsonl"]) {
    const file = path.join(evidenceSource, required);
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
      fail(`remote memory export is missing a single regular ${required}`);
    }
  }
  if (fs.existsSync(path.join(downloadDirectory, "publication"))) {
    fail("evidence-only memory export unexpectedly contains a publication tree");
  }
  if (fs.existsSync(evidenceDestination)) {
    fail(`refusing to overwrite existing memory evidence directory ${evidenceDestination}`);
  }
  const historyFile = path.join(REPO, "bench/history/runs.jsonl");
  const historyBefore = sha256File(historyFile);
  await validateMemoryEvidence(config, state, evidenceSource);
  if (sha256File(historyFile) !== historyBefore) fail("memory evidence validation changed throughput history");

  const stageNonce = crypto.randomBytes(6).toString("hex");
  const evidenceStage = `${evidenceDestination}.incoming-${state.run_id}-${stageNonce}`;
  let evidenceCommitted = false;
  try {
    fs.mkdirSync(path.dirname(evidenceDestination), { recursive: true });
    if (fs.existsSync(evidenceStage)) fail("randomized memory evidence stage already exists");
    fs.cpSync(evidenceSource, evidenceStage, { recursive: true, errorOnExist: true, force: false });
    const sourceFiles = evidenceFiles(evidenceSource);
    const stagedFiles = evidenceFiles(evidenceStage);
    requireJsonEqual(stagedFiles, sourceFiles, "staged memory evidence inventory");
    for (const file of sourceFiles) {
      if (
        sha256File(path.join(evidenceStage, ...file.split("/"))) !==
        sha256File(path.join(evidenceSource, ...file.split("/")))
      ) {
        fail(`staged memory evidence bytes differ for ${file}`);
      }
    }
    if (sha256File(historyFile) !== historyBefore) fail("throughput history changed during memory evidence staging");
    throwIfInterrupted();
    state.promotion_started_at = new Date().toISOString();
    saveState(state);
    fs.renameSync(evidenceStage, evidenceDestination);
    evidenceCommitted = true;
    if (sha256File(historyFile) !== historyBefore) fail("memory evidence import changed throughput history");
  } catch (error) {
    fs.rmSync(evidenceStage, { recursive: true, force: true });
    if (evidenceCommitted) fs.rmSync(evidenceDestination, { recursive: true, force: true });
    state.promotion_rolled_back_at = new Date().toISOString();
    state.promotion_rollback_reason = error.message;
    saveState(state);
    throw error;
  }
  state.promoted_at = new Date().toISOString();
  saveState(state);
  process.stdout.write(
    `Memory evidence imported to ${path.relative(REPO, evidenceDestination)}; throughput history was untouched.\n`,
  );
  return true;
}

function validateMatrixStatus(status) {
  requireObject(status, "matrix status");
  requireExactKeys(
    status,
    ["schema", "status", "detail", "started_at", "finished_at"],
    "matrix status",
  );
  if (status.schema !== "rustia.bench.cloud-run-status.v1" || status.status !== "matrix_complete") {
    fail("matrix status has the wrong schema or terminal state");
  }
  if (status.detail !== "fresh Linux native and wasm benchmark matrix completed") {
    fail("matrix status has an unexpected detail");
  }
  const started = requireIsoTimestamp(status.started_at, "matrix status.started_at");
  const finished = requireIsoTimestamp(status.finished_at, "matrix status.finished_at");
  if (finished < started) fail("matrix status finished before it started");
}

function loadStrictJsonLines(file, location) {
  const bytes = fs.readFileSync(file);
  const text = strictUtf8(bytes, location);
  if (text.length === 0 || !text.endsWith("\n")) fail(`${location} must be non-empty and newline terminated`);
  const lines = text.slice(0, -1).split("\n");
  if (lines.some((line) => line.trim() === "")) fail(`${location} contains a blank row`);
  return lines.map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      fail(`${location} row ${index + 1} is not JSON: ${error.message}`);
    }
  });
}

function requirePositiveFinite(value, location) {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    fail(`${location} must be positive and finite`);
  }
  return value;
}

function requireFreshMatrixTimestamp(value, status, location) {
  requireInteger(value, location, 1);
  const started = Date.parse(status.started_at) / 1000;
  const finished = Date.parse(status.finished_at) / 1000;
  if (value < Math.floor(started) || value > Math.ceil(finished)) {
    fail(`${location} falls outside the matrix run window`);
  }
}

function validateMatrixNativeRows(config, file, status) {
  const rows = loadStrictJsonLines(file, "matrix native history");
  if (rows.length !== config.publication.native_rows) {
    fail(`matrix native history has ${rows.length} rows, expected ${config.publication.native_rows}`);
  }
  const dialects = {
    rustia: "luau",
    boa: "javascript",
    koto: "koto",
    "mlua-luau": "luau",
    rhai: "rhai",
    roto: "roto",
    rquickjs: "javascript",
    wasmi: "wasm",
    wasmtime: "wasm",
    "mlua-lua54": "lua54",
  };
  const expectedKeys = [
    "timestamp_unix", "commit", "target", "engine", "dialect", "workload", "unit", "n", "reps",
    "ns_per_unit_min", "ns_per_unit_median", "units_per_sec", "checksum",
  ];
  const slots = new Set();
  for (const [index, row] of rows.entries()) {
    requireObject(row, `matrix native row ${index + 1}`);
    requireExactKeys(row, expectedKeys, `matrix native row ${index + 1}`);
    if (row.commit !== config.source.benchmark_commit) fail(`matrix native row ${index + 1} has the wrong commit`);
    if (row.target !== MATRIX_NATIVE_TARGET) fail(`matrix native row ${index + 1} is not ${MATRIX_NATIVE_TARGET}`);
    if (!MATRIX_NATIVE_ENGINES.includes(row.engine) || row.dialect !== dialects[row.engine]) {
      fail(`matrix native row ${index + 1} has an invalid engine/dialect identity`);
    }
    if (!MATRIX_WORKLOADS.includes(row.workload)) fail(`matrix native row ${index + 1} has an invalid workload`);
    if (row.reps !== config.measurement.native_repetitions) fail(`matrix native row ${index + 1} has the wrong repetitions`);
    requireFreshMatrixTimestamp(row.timestamp_unix, status, `matrix native row ${index + 1}.timestamp_unix`);
    requireInteger(row.n, `matrix native row ${index + 1}.n`, 1);
    for (const field of ["ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
      requirePositiveFinite(row[field], `matrix native row ${index + 1}.${field}`);
    }
    if (row.ns_per_unit_median < row.ns_per_unit_min) fail(`matrix native row ${index + 1} median is below min`);
    requireString(row.unit, `matrix native row ${index + 1}.unit`);
    requireString(row.checksum, `matrix native row ${index + 1}.checksum`);
    const slot = `${row.engine}\0${row.workload}`;
    if (slots.has(slot)) fail(`matrix native history duplicates ${row.engine}/${row.workload}`);
    slots.add(slot);
  }
  for (const engine of MATRIX_NATIVE_ENGINES) {
    for (const workload of MATRIX_WORKLOADS) {
      if (!slots.has(`${engine}\0${workload}`)) fail(`matrix native history is missing ${engine}/${workload}`);
    }
  }
  return rows;
}

function requireLinuxWasmTarget(target, location, dispatch = false) {
  requireString(target, location);
  const prefix = dispatch ? "wasm32-dispatch-" : "wasm32-";
  if (!target.startsWith(prefix) || !/-linux-(?:x64|x86_64)$/.test(target)) {
    fail(`${location} must identify a Linux x86-64 wasm host`);
  }
}

function validateMatrixWasm(config, file, status) {
  const value = requireObject(loadJsonRegular(file, "matrix wasm results"), "matrix wasm results");
  if (
    value.schema !== "rustia.bench.wasm-gate-current-results.v1" ||
    value.schema_version !== 1 ||
    value.commit !== config.source.benchmark_commit
  ) {
    fail("matrix wasm results have the wrong schema or commit");
  }
  if (!Array.isArray(value.skipped) || value.skipped.length !== 0) fail("matrix wasm results skipped a runtime");
  if (!Array.isArray(value.runs) || value.runs.length !== MATRIX_WASM_RUNTIMES.length * MATRIX_WASM_ADAPTERS.length) {
    fail("matrix wasm results have the wrong runtime/adapter run count");
  }
  const slots = new Set();
  let rowCount = 0;
  for (const [runIndex, run] of value.runs.entries()) {
    requireObject(run, `matrix wasm run ${runIndex + 1}`);
    if (run.schema !== "rustia.bench.wasm-runtime-results.v1" || run.commit !== config.source.benchmark_commit) {
      fail(`matrix wasm run ${runIndex + 1} has the wrong schema or commit`);
    }
    requireFreshMatrixTimestamp(run.timestamp_unix, status, `matrix wasm run ${runIndex + 1}.timestamp_unix`);
    const runtime = requireObject(run.runtime, `matrix wasm run ${runIndex + 1}.runtime`);
    const adapter = requireObject(run.adapter, `matrix wasm run ${runIndex + 1}.adapter`);
    if (!MATRIX_WASM_RUNTIMES.includes(runtime.name) || !MATRIX_WASM_ADAPTERS.includes(adapter.id)) {
      fail(`matrix wasm run ${runIndex + 1} has an unexpected runtime or adapter`);
    }
    if (runtime.family !== MATRIX_RUNTIME_FAMILIES[runtime.name]) {
      fail(`matrix wasm run ${runIndex + 1} has the wrong runtime family`);
    }
    requireLinuxWasmTarget(run.target, `matrix wasm run ${runIndex + 1}.target`);
    const familySlug = runtime.family.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    if (!run.target.startsWith(`wasm32-${familySlug}-${runtime.name}-`)) {
      fail(`matrix wasm run ${runIndex + 1} target disagrees with its runtime identity`);
    }
    const slot = `${runtime.name}\0${adapter.id}`;
    if (slots.has(slot)) fail(`matrix wasm results duplicate ${runtime.name}/${adapter.id}`);
    slots.add(slot);
    if (!Array.isArray(run.rows) || run.rows.length !== MATRIX_WORKLOADS.length) {
      fail(`matrix wasm run ${runtime.name}/${adapter.id} has the wrong workload count`);
    }
    const workloads = new Set();
    for (const [rowIndex, row] of run.rows.entries()) {
      requireObject(row, `matrix wasm ${runtime.name}/${adapter.id} row ${rowIndex + 1}`);
      const adapterDialects = { rustia: "luau", koto: "koto", rhai: "rhai" };
      if (
        row.engine !== adapter.id ||
        row.dialect !== adapterDialects[adapter.id] ||
        !MATRIX_WORKLOADS.includes(row.workload)
      ) {
        fail(`matrix wasm ${runtime.name}/${adapter.id} row ${rowIndex + 1} has the wrong identity`);
      }
      if (row.reps !== config.measurement.wasm_repetitions) {
        fail(`matrix wasm ${runtime.name}/${adapter.id} row ${rowIndex + 1} has the wrong repetitions`);
      }
      if (workloads.has(row.workload)) fail(`matrix wasm ${runtime.name}/${adapter.id} duplicates ${row.workload}`);
      workloads.add(row.workload);
      requireInteger(row.n, `matrix wasm row ${rowIndex + 1}.n`, 1);
      for (const field of ["ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
        requirePositiveFinite(row[field], `matrix wasm row ${rowIndex + 1}.${field}`);
      }
      if (row.ns_per_unit_median < row.ns_per_unit_min) fail("matrix wasm median is below min");
      rowCount += 1;
    }
  }
  if (rowCount !== config.publication.wasm_rows) fail(`matrix wasm result has ${rowCount} rows`);
  for (const runtime of MATRIX_WASM_RUNTIMES) {
    for (const adapter of MATRIX_WASM_ADAPTERS) {
      if (!slots.has(`${runtime}\0${adapter}`)) fail(`matrix wasm results are missing ${runtime}/${adapter}`);
    }
  }
  return value;
}

function validateMatrixDispatch(config, file, status) {
  const value = requireObject(loadJsonRegular(file, "matrix wasm dispatch results"), "matrix wasm dispatch results");
  if (
    value.schema !== "rustia.bench.wasm-dispatch-current-results.v1" ||
    value.schema_version !== 1 ||
    value.commit !== config.source.benchmark_commit
  ) {
    fail("matrix dispatch results have the wrong schema or commit");
  }
  if (!Array.isArray(value.skipped) || value.skipped.length !== 0) fail("matrix dispatch results skipped a runtime");
  requireJsonEqual(value.required_families, ["V8"], "matrix dispatch required families");
  const requirements = requireObject(value.requirements, "matrix dispatch requirements");
  if (requirements["call-ref-vs-br-table"] !== true) {
    fail("matrix dispatch Node/V8 requirement is not measured");
  }
  if (!Array.isArray(value.runs) || value.runs.length !== config.publication.dispatch_runtimes) {
    fail("matrix dispatch results have the wrong runtime count");
  }
  const runtimes = new Set();
  for (const [index, run] of value.runs.entries()) {
    requireObject(run, `matrix dispatch run ${index + 1}`);
    if (run.schema !== "rustia.bench.wasm-dispatch-results.v1" || run.commit !== config.source.benchmark_commit) {
      fail(`matrix dispatch run ${index + 1} has the wrong schema or commit`);
    }
    requireFreshMatrixTimestamp(run.timestamp_unix, status, `matrix dispatch run ${index + 1}.timestamp_unix`);
    const runtime = requireObject(run.runtime, `matrix dispatch run ${index + 1}.runtime`);
    if (!MATRIX_WASM_RUNTIMES.includes(runtime.name) || runtimes.has(runtime.name)) {
      fail(`matrix dispatch run ${index + 1} has an unexpected or duplicate runtime`);
    }
    if (runtime.family !== MATRIX_RUNTIME_FAMILIES[runtime.name]) {
      fail(`matrix dispatch run ${index + 1} has the wrong runtime family`);
    }
    runtimes.add(runtime.name);
    requireLinuxWasmTarget(run.target, `matrix dispatch run ${index + 1}.target`, true);
    const familySlug = runtime.family.toLowerCase().replace(/[^a-z0-9]+/g, "-");
    if (!run.target.startsWith(`wasm32-dispatch-${familySlug}-${runtime.name}-`)) {
      fail(`matrix dispatch run ${index + 1} target disagrees with its runtime identity`);
    }
    const comparison = requireObject(run.comparison, `matrix dispatch run ${index + 1}.comparison`);
    if (!["measured", "unsupported"].includes(comparison.status)) {
      fail(`matrix dispatch run ${index + 1} has an invalid comparison status`);
    }
    if (!Array.isArray(run.rows)) fail(`matrix dispatch run ${index + 1}.rows must be an array`);
    if (comparison.status === "measured" && run.rows.length !== 2) {
      fail(`matrix dispatch run ${index + 1} must contain two measured styles`);
    }
    for (const row of run.rows) {
      if (row.reps !== config.measurement.dispatch_repetitions) fail("matrix dispatch row has the wrong repetitions");
      requirePositiveFinite(row.ns_per_dispatch_min, "matrix dispatch row.ns_per_dispatch_min");
      requirePositiveFinite(row.ns_per_dispatch_median, "matrix dispatch row.ns_per_dispatch_median");
    }
  }
  for (const runtime of MATRIX_WASM_RUNTIMES) {
    if (!runtimes.has(runtime)) fail(`matrix dispatch results are missing ${runtime}`);
  }
  return value;
}

function validateMatrixEvidence(config, state, evidenceDirectory, publications, status) {
  const expectedFiles = [
    "gate.jsonl",
    "host.json",
    "manifest.json",
    "native-all-samples.json",
    "native-lua54-samples.json",
    "protocol.json",
    "provider.json",
    "runtime-versions.json",
  ];
  requireJsonEqual(evidenceFiles(evidenceDirectory), expectedFiles, "matrix evidence file allowlist");
  const manifest = requireObject(
    loadJsonRegular(path.join(evidenceDirectory, "manifest.json"), "matrix evidence manifest"),
    "matrix evidence manifest",
  );
  if (
    manifest.schema !== "rustia.bench.cross-engine-matrix-evidence.v1" ||
    manifest.benchmark_commit !== config.source.benchmark_commit
  ) {
    fail("matrix evidence manifest has the wrong schema or commit");
  }
  requireIsoTimestamp(manifest.created_at, "matrix evidence manifest.created_at");
  requireJsonEqual(manifest.source_hashes, config.source.expected_sha256, "matrix manifest source hashes");
  const providerFile = path.join(evidenceDirectory, "provider.json");
  const localProvider = loadJsonRegular(path.join(state.__directory, "provider.json"), "local matrix provider snapshot");
  requireJsonEqual(loadJsonRegular(providerFile, "matrix provider evidence"), localProvider, "matrix provider evidence");
  if (manifest.protocol_sha256 !== sha256File(path.join(evidenceDirectory, "protocol.json"))) {
    fail("matrix manifest protocol hash is wrong");
  }
  if (manifest.protocol_sha256 !== state.plan.source.config_sha256) fail("matrix protocol is not the planned config");
  if (manifest.provider_sha256 !== sha256File(providerFile)) fail("matrix manifest provider hash is wrong");
  const host = requireObject(loadJsonRegular(path.join(evidenceDirectory, "host.json"), "matrix host evidence"), "matrix host evidence");
  if (host.platform !== "linux" || host.arch !== "x86_64" || host.virtualization !== "kvm") {
    fail("matrix host evidence is not a Linux x86-64 KVM guest");
  }
  if (manifest.host_sha256 !== sha256File(path.join(evidenceDirectory, "host.json"))) {
    fail("matrix manifest host hash is wrong");
  }
  validateMatrixGateLog(config, path.join(evidenceDirectory, "gate.jsonl"), status);
  const publicationHashes = {
    history: sha256File(publications.history),
    wasm: sha256File(publications.wasm),
    dispatch: sha256File(publications.dispatch),
  };
  requireJsonEqual(manifest.publication_sha256, publicationHashes, "matrix publication hashes");
  const hashes = requireObject(manifest.evidence_sha256, "matrix evidence manifest.evidence_sha256");
  const hashedFiles = expectedFiles.filter((file) => file !== "manifest.json");
  requireJsonEqual(Object.keys(hashes).sort(), hashedFiles, "matrix evidence hash keys");
  for (const file of hashedFiles) {
    if (!SHA256.test(hashes[file]) || hashes[file] !== sha256File(path.join(evidenceDirectory, file))) {
      fail(`matrix evidence hash is wrong for ${file}`);
    }
  }
  const shape = {
    native_rows: config.publication.native_rows,
    wasm_rows: config.publication.wasm_rows,
    dispatch_runtimes: config.publication.dispatch_runtimes,
  };
  requireJsonEqual(manifest.result_shape, shape, "matrix manifest result shape");
}

async function promoteMatrixResult(config, state, downloadDirectory, verifyCheckout) {
  if (!state.linode_destroyed_at) fail("refusing matrix publication before confirmed Linode deletion");
  const status = loadJsonRegular(path.join(downloadDirectory, "status.json"), "remote matrix status");
  state.remote_status = status.status;
  state.remote_status_detail = status.detail || null;
  saveState(state);
  if (status.status !== "matrix_complete") {
    process.stdout.write(
      `Linode matrix run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
        `(${status.status}${status.detail ? `: ${status.detail}` : ""}).\n`,
    );
    return false;
  }
  validateMatrixStatus(status);
  await verifyCheckout(config, state);
  const evidenceSource = path.join(downloadDirectory, "evidence", "matrix", config.source.benchmark_commit);
  const evidenceDestination = path.join(
    REPO,
    "bench/evidence/matrix",
    config.source.benchmark_commit,
    state.run_id,
  );
  const publications = {
    history: path.join(downloadDirectory, "publication/bench/history/runs.jsonl"),
    wasm: path.join(downloadDirectory, "publication/bench/wasm/current-results.json"),
    dispatch: path.join(downloadDirectory, "publication/bench/wasm/dispatch-current-results.json"),
  };
  for (const file of [path.join(evidenceSource, "manifest.json"), ...Object.values(publications)]) {
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
      fail(`remote matrix export is missing a single regular file at ${file}`);
    }
  }
  if (fs.existsSync(evidenceDestination)) fail(`refusing to overwrite matrix evidence ${evidenceDestination}`);
  const historyDestination = path.join(REPO, "bench/history/runs.jsonl");
  if (sha256File(historyDestination) !== config.source.expected_sha256.history_before) {
    fail("local benchmark history changed while the Linode matrix was in flight");
  }
  validateMatrixNativeRows(config, publications.history, status);
  validateMatrixWasm(config, publications.wasm, status);
  validateMatrixDispatch(config, publications.dispatch, status);
  validateMatrixEvidence(config, state, evidenceSource, publications, status);

  const destinations = {
    history: historyDestination,
    wasm: path.join(REPO, "bench/wasm/current-results.json"),
    dispatch: path.join(REPO, "bench/wasm/dispatch-current-results.json"),
  };
  const generated = [
    path.join(REPO, "bench/RESULTS.md"),
    path.join(REPO, "bench/current-results.json"),
    path.join(REPO, "bench/current-results.html"),
    path.join(REPO, "bench/CLAIMS.md"),
  ];
  const snapshots = [...Object.values(destinations), ...generated].map((file) =>
    fs.existsSync(file)
      ? { file, existed: true, bytes: fs.readFileSync(file), mode: fs.statSync(file).mode & 0o777 }
      : { file, existed: false },
  );
  const nonce = crypto.randomBytes(6).toString("hex");
  const evidenceStage = `${evidenceDestination}.incoming-${nonce}`;
  const stages = Object.fromEntries(
    Object.entries(destinations).map(([name, destination]) => [name, `${destination}.incoming-${state.run_id}-${nonce}`]),
  );
  let evidenceCommitted = false;
  let publicationsCommitted = false;
  try {
    fs.mkdirSync(path.dirname(evidenceDestination), { recursive: true });
    fs.cpSync(evidenceSource, evidenceStage, { recursive: true, errorOnExist: true, force: false });
    requireJsonEqual(evidenceFiles(evidenceStage), evidenceFiles(evidenceSource), "staged matrix evidence inventory");
    for (const file of evidenceFiles(evidenceSource)) {
      if (sha256File(path.join(evidenceStage, file)) !== sha256File(path.join(evidenceSource, file))) {
        fail(`staged matrix evidence bytes differ for ${file}`);
      }
    }
    for (const [name, source] of Object.entries(publications)) {
      fs.copyFileSync(source, stages[name], fs.constants.COPYFILE_EXCL);
      if (sha256File(stages[name]) !== sha256File(source)) fail(`staged matrix ${name} bytes changed`);
    }
    throwIfInterrupted();
    state.promotion_started_at = new Date().toISOString();
    saveState(state);
    fs.renameSync(evidenceStage, evidenceDestination);
    evidenceCommitted = true;
    for (const [name, destination] of Object.entries(destinations)) {
      fs.renameSync(stages[name], destination);
      publicationsCommitted = true;
    }
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "gen-current-results"], {
      capture: false,
      announce: true,
    });
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "check-benchmark-results"], {
      capture: false,
      announce: true,
    });
  } catch (error) {
    fs.rmSync(evidenceStage, { recursive: true, force: true });
    for (const stage of Object.values(stages)) fs.rmSync(stage, { force: true });
    if (evidenceCommitted) fs.rmSync(evidenceDestination, { recursive: true, force: true });
    if (publicationsCommitted) {
      for (const snapshot of snapshots) {
        if (snapshot.existed) writeBytesAtomic(snapshot.file, snapshot.bytes, snapshot.mode);
        else fs.rmSync(snapshot.file, { force: true });
      }
    }
    state.promotion_rolled_back_at = new Date().toISOString();
    state.promotion_rollback_reason = error.message;
    saveState(state);
    throw error;
  }
  state.promoted_at = new Date().toISOString();
  saveState(state);
  process.stdout.write(
    `Fresh Linode matrix replaced active history and wasm results; evidence imported to ` +
      `${path.relative(REPO, evidenceDestination)}.\n`,
  );
  return true;
}

async function promoteAcceptedResult(
  config,
  state,
  downloadDirectory,
  verifyCheckout = verifyPromotionCheckout,
) {
  if (isCrossEngineMatrix(config)) {
    return promoteMatrixResult(config, state, downloadDirectory, verifyCheckout);
  }
  if (isVmStringMemory(config)) {
    return promoteMemoryEvidence(config, state, downloadDirectory, verifyCheckout);
  }
  if (!state.linode_destroyed_at) {
    fail("refusing publication before the Linode API has confirmed instance deletion");
  }
  throwIfInterrupted();
  const statusFile = path.join(downloadDirectory, "status.json");
  const status = loadJsonRegular(statusFile, "remote status");
  state.remote_status = status.status;
  state.remote_status_detail = status.detail || null;
  saveState(state);
  if (status.status !== "accepted_and_published") {
    process.stdout.write(
      `Linode run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
        `(${status.status}${status.detail ? `: ${status.detail}` : ""}).\n`,
    );
    return false;
  }
  validateAcceptedStatus(status, config.publication.required_verdict);
  await verifyCheckout(config, state);

  const candidate = config.source.candidate_commit;
  const evidenceSource = path.join(downloadDirectory, "evidence", candidate);
  const evidenceDestination = path.join(REPO, "bench/history/evidence", candidate);
  const historySource = path.join(downloadDirectory, "publication", "bench/history/runs.jsonl");
  const historyDestination = path.join(REPO, "bench/history/runs.jsonl");
  for (const required of [
    path.join(evidenceSource, "manifest.json"),
    path.join(evidenceSource, "analysis.json"),
    historySource,
  ]) {
    const stat = fs.lstatSync(required);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
      fail(`remote accepted export is missing a single regular file at ${required}`);
    }
  }
  if (fs.existsSync(evidenceDestination)) {
    fail(`refusing to overwrite existing evidence directory ${evidenceDestination}`);
  }
  const localHistory = fs.readFileSync(historyDestination);
  if (sha256(localHistory) !== config.source.expected_sha256.history_before) {
    fail("local benchmark history changed while the Linode run was in flight");
  }
  const remoteHistory = fs.readFileSync(historySource);
  if (
    remoteHistory.length <= localHistory.length ||
    !remoteHistory.subarray(0, localHistory.length).equals(localHistory)
  ) {
    fail("remote history is not an append-only extension of the exact local history");
  }
  const oldLines = localHistory.toString("utf8").trimEnd().split("\n").length;
  const newLines = remoteHistory.toString("utf8").trimEnd().split("\n").length;
  if (newLines - oldLines !== config.publication.final_measurements) {
    fail(`remote history appended ${newLines - oldLines} rows, expected ${config.publication.final_measurements}`);
  }
  await validateAcceptedEvidence(config, state, evidenceSource, localHistory, remoteHistory);

  const stageNonce = crypto.randomBytes(6).toString("hex");
  const evidenceStage = `${evidenceDestination}.incoming-${state.run_id}-${stageNonce}`;
  const historyStage = `${historyDestination}.incoming-${state.run_id}-${stageNonce}`;
  const generatedFiles = [
    path.join(REPO, "bench/RESULTS.md"),
    path.join(REPO, "bench/current-results.json"),
    path.join(REPO, "bench/current-results.html"),
    path.join(REPO, "bench/CLAIMS.md"),
  ];
  const snapshots = [historyDestination, ...generatedFiles].map((file) => {
    if (!fs.existsSync(file)) return { file, existed: false };
    return { file, existed: true, bytes: fs.readFileSync(file), mode: fs.statSync(file).mode & 0o777 };
  });
  let evidenceCommitted = false;
  let historyCommitted = false;
  try {
    if (fs.existsSync(evidenceStage) || fs.existsSync(historyStage)) fail("randomized promotion stage already exists");
    fs.cpSync(evidenceSource, evidenceStage, { recursive: true, errorOnExist: true, force: false });
    fs.copyFileSync(historySource, historyStage, fs.constants.COPYFILE_EXCL);
    const sourceFiles = evidenceFiles(evidenceSource);
    const stagedFiles = evidenceFiles(evidenceStage);
    requireJsonEqual(stagedFiles, sourceFiles, "staged evidence inventory");
    for (const file of sourceFiles) {
      if (
        sha256File(path.join(evidenceStage, ...file.split("/"))) !==
        sha256File(path.join(evidenceSource, ...file.split("/")))
      ) {
        fail(`staged evidence bytes differ for ${file}`);
      }
    }
    if (sha256File(historyStage) !== sha256(remoteHistory)) fail("staged history bytes differ from the export");
    throwIfInterrupted();
    state.promotion_started_at = new Date().toISOString();
    saveState(state);
    fs.renameSync(evidenceStage, evidenceDestination);
    evidenceCommitted = true;
    fs.renameSync(historyStage, historyDestination);
    historyCommitted = true;
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "gen-current-results"], {
      capture: false,
      announce: true,
    });
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "check-benchmark-results"], {
      capture: false,
      announce: true,
    });
  } catch (error) {
    fs.rmSync(evidenceStage, { recursive: true, force: true });
    fs.rmSync(historyStage, { force: true });
    if (evidenceCommitted) fs.rmSync(evidenceDestination, { recursive: true, force: true });
    if (historyCommitted) {
      for (const snapshot of snapshots) {
        if (snapshot.existed) writeBytesAtomic(snapshot.file, snapshot.bytes, snapshot.mode);
        else fs.rmSync(snapshot.file, { force: true });
      }
    }
    state.promotion_rolled_back_at = new Date().toISOString();
    state.promotion_rollback_reason = error.message;
    saveState(state);
    throw error;
  }
  state.promoted_at = new Date().toISOString();
  saveState(state);
  process.stdout.write(
    `Accepted evidence promoted to ${path.relative(REPO, evidenceDestination)} and ` +
      `${config.publication.final_measurements} actual history rows appended.\n`,
  );
  return true;
}

async function benchLocked(config, configFile = DEFAULT_CONFIG) {
  const { token, plan, provider, source } = await buildPlan(config, { print: true, configFile });
  const selectedRemoteRunner = remoteRunnerForConfig(config);
  fs.mkdirSync(RUNS_ROOT, { recursive: true, mode: 0o700 });
  const runId = `${safeTimestamp()}-${crypto.randomBytes(4).toString("hex")}`;
  const directory = path.join(RUNS_ROOT, runId);
  fs.mkdirSync(directory, { recursive: false, mode: 0o700 });
  const shortCandidate = benchmarkCommit(config).slice(0, 8);
  const linodeLabel = `rustia-bench-${shortCandidate}-${crypto.randomBytes(4).toString("hex")}`;
  const state = {
    __directory: directory,
    schema: "rustia.linode-benchmark-state.v1",
    run_id: runId,
    created_at: new Date().toISOString(),
    protocol_mode: config.publication.mode,
    linode_label: linodeLabel,
    linode_id: null,
    linode_destroyed_at: null,
    private_key_path: path.join(directory, "id_ed25519"),
    public_key_path: path.join(directory, "id_ed25519.pub"),
    known_hosts_path: path.join(directory, "known_hosts"),
    bundle_path: path.join(directory, "rustia.bundle"),
    source_config_path: configFile,
    config_path: path.join(directory, "protocol.json"),
    runner_path: path.join(directory, path.basename(selectedRemoteRunner)),
    plan,
  };
  saveState(state);

  let primaryError = null;
  let cleanupError = null;
  try {
    fs.copyFileSync(configFile, state.config_path);
    fs.copyFileSync(selectedRemoteRunner, state.runner_path);
    if (sha256File(state.config_path) !== plan.source.config_sha256) {
      fail("protocol config changed after planning and before snapshotting");
    }
    if (sha256File(state.runner_path) !== plan.source.remote_runner_sha256) {
      fail("remote runner changed after planning and before snapshotting");
    }
    await run(
      "ssh-keygen",
      ["-q", "-t", "ed25519", "-N", "", "-C", `${linodeLabel}-key`, "-f", state.private_key_path],
      { capture: false, announce: true },
    );
    fs.chmodSync(state.private_key_path, 0o600);
    await run("git", ["bundle", "create", state.bundle_path, "HEAD"], { announce: true });
    await run("git", ["bundle", "verify", state.bundle_path], { capture: false, announce: true });
    state.bundle_sha256 = sha256File(state.bundle_path);
    saveState(state);

    // Repeat every cost/scope-sensitive check immediately before the first billable request.
    await preflightAccount(token);
    await preflightAccountRegion(token, config);
    const refreshedProvider = await resolveProviderPlan(token, config);
    const refreshedPlan = sanitizedProviderPlan(config, refreshedProvider, source);
    verifyProviderPlanUnchanged(plan, refreshedPlan);
    await verifyDeleteAuthority(token);

    const publicKey = fs.readFileSync(state.public_key_path, "utf8").trim();
    const createBody = createLinodeBody(config, linodeLabel, publicKey);
    startBillingWindow(state, config);
    const createStarted = Date.now();
    state.linode_create_attempted_at = new Date(createStarted).toISOString();
    state.linode_create_request_deadline_at = new Date(createStarted + API_TIMEOUT_MS).toISOString();
    saveState(state);

    let createResponse = null;
    let createError = null;
    try {
      createResponse = await apiRequest(token, "POST", "/v4/linode/instances", { body: createBody });
    } catch (error) {
      createError = error;
    }
    state.linode_create_finished_at = new Date().toISOString();
    saveState(state);
    if (createResponse) {
      const created = requireApiObject(createResponse, "Linode create response");
      if (Number.isSafeInteger(created.id) && created.id > 0) {
        state.linode_id = created.id;
        saveState(state);
      }
      if (!state.linode_id || created.label !== state.linode_label) {
        const reconciled = await reconcileLinodeAfterCreateAttempt(state, token);
        if (!reconciled) fail("Linode create returned no uniquely reconcilable owned instance");
      }
    } else {
      const reconciled = await reconcileLinodeAfterCreateAttempt(state, token);
      if (!reconciled) fail(`Linode creation failed: ${createError ? createError.message : "unknown API error"}`);
    }

    const linode = await waitForLinode(state, token);
    verifyLinodeMatchesPlan(linode, plan);
    await waitForSsh(state);
    await run(
      "ssh",
      [...sshOptions(state), `root@${state.public_ipv4}`, "cloud-init", "status", "--wait"],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "cloud-init"),
      },
    );
    await run(
      "ssh",
      [
        ...sshOptions(state),
        `root@${state.public_ipv4}`,
        "env",
        "DEBIAN_FRONTEND=noninteractive",
        "apt-get",
        "update",
        "-qq",
      ],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "apt metadata refresh"),
      },
    );
    await run(
      "ssh",
      [
        ...sshOptions(state),
        `root@${state.public_ipv4}`,
        "env",
        "DEBIAN_FRONTEND=noninteractive",
        "apt-get",
        "install",
        "-y",
        "--no-install-recommends",
        "build-essential",
        "ca-certificates",
        "curl",
        "git",
        "nodejs",
        "unzip",
        "xz-utils",
        "bzip2",
      ],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 15 * 60 * 1000, "prerequisite install"),
      },
    );
    await run(
      "ssh",
      [...sshOptions(state), `root@${state.public_ipv4}`, "mkdir", "-p", "/opt/rustia/incoming"],
      { announce: true, timeoutMs: remainingWorkTimeout(state, 60 * 1000, "remote directory setup") },
    );

    const providerMetadata = path.join(directory, "provider.json");
    writeJsonAtomic(providerMetadata, safeLinodeMetadata(linode, plan, state));
    await run(
      "scp",
      [
        ...sshOptions(state),
        state.bundle_path,
        state.config_path,
        state.runner_path,
        providerMetadata,
        `root@${state.public_ipv4}:/opt/rustia/incoming/`,
      ],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "source transfer"),
      },
    );

    const remoteResult = await run(
      "ssh",
      [
        ...sshOptions(state),
        `root@${state.public_ipv4}`,
        "node",
        `/opt/rustia/incoming/${path.basename(selectedRemoteRunner)}`,
        "/opt/rustia/incoming/protocol.json",
        "/opt/rustia/incoming/provider.json",
        "/opt/rustia/incoming/rustia.bundle",
      ],
      {
        capture: false,
        allowFailure: true,
        announce: true,
        timeoutMs: remainingWorkTimeout(
          state,
          config.provider.max_runtime_minutes * 60 * 1000,
          "remote benchmark",
        ),
      },
    );
    state.remote_exit_status = remoteResult.status;
    state.remote_timed_out = remoteResult.timedOut;
    saveState(state);

    const archive = path.join(directory, "export.tar.gz");
    const fetchResult = await run(
      "scp",
      [...sshOptions(state), `root@${state.public_ipv4}:/opt/rustia/export.tar.gz`, archive],
      {
        capture: false,
        allowFailure: true,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 5 * 60 * 1000, "result retrieval"),
      },
    );
    if (fetchResult.status !== 0 || !fs.existsSync(archive)) {
      fail("remote benchmark did not produce a retrievable export archive");
    }
    state.export_sha256 = sha256File(archive);
    state.export_path = archive;
    saveState(state);
  } catch (error) {
    primaryError = error;
  } finally {
    cleanupInProgress = true;
    try {
      await cleanupOwned(state, token);
    } catch (error) {
      cleanupError = error;
    } finally {
      cleanupInProgress = false;
    }
  }

  if (cleanupError) {
    const prefix = primaryError ? `${primaryError.message}; additionally, ` : "";
    fail(`${prefix}${cleanupError.message}`);
  }
  if (primaryError) throw primaryError;
  if (!state.linode_destroyed_at) fail("Linode deletion was not confirmed");
  throwIfInterrupted();
  const downloadDirectory = path.join(directory, "download");
  await extractArchive(state.export_path, downloadDirectory);
  let promoted;
  try {
    promoted = await promoteAcceptedResult(config, state, downloadDirectory);
  } catch (error) {
    if (!state.promoted_at) {
      state.promotion_rolled_back_at = new Date().toISOString();
      state.promotion_rollback_reason = error.message;
      saveState(state);
    }
    throw error;
  }
  if (!promoted && state.remote_exit_status !== 0) {
    fail(`remote benchmark ended with status ${state.remote_exit_status}; diagnostics were retained`);
  }
}

async function bench(config, configFile = DEFAULT_CONFIG) {
  const lockToken = acquireCoordinatorLock();
  try {
    const incomplete = incompleteStates();
    if (incomplete.length > 0) {
      fail(
        "incomplete Linode benchmark cleanup exists; run status and clean up: " +
          incomplete.map((state) => state.run_id).join(", "),
      );
    }
    throwIfInterrupted();
    return await benchLocked(config, configFile);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function authenticate(config) {
  const lockToken = acquireCoordinatorLock();
  try {
    const token = await readHiddenToken();
    await preflightAccount(token);
    await preflightAccountRegion(token, config);
    await resolveProviderPlan(token, config);
    await verifyDeleteAuthority(token);
    writeBytesAtomic(TOKEN_FILE, Buffer.from(`${token}\n`), 0o600);
    process.stdout.write(`Linode token validated and stored with mode 0600 at ${TOKEN_FILE}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function status() {
  const states = listStates();
  if (states.length === 0) {
    process.stdout.write("No locally owned Linode benchmark runs.\n");
    return;
  }
  const token = readStoredToken({ allowMissing: true });
  const rows = [];
  for (const state of states) {
    let remote = null;
    if (!state.linode_destroyed_at) {
      if (!token) {
        remote = { error: "stored token unavailable; run auth or supply cleanup from an authenticated host" };
      } else {
        try {
          if (state.linode_id) {
            const linode = await getOwnedLinode(state, token, { allowMissing: true });
            remote = linode
              ? { id: linode.id, label: linode.label, status: linode.status, public_ipv4: publicIpv4(linode) }
              : { status: "not returned by get; run destroy to reconcile safely" };
          } else {
            const matches = matchingLinodesByLabel(await listLinodes(token), state.linode_label);
            remote =
              matches.length === 1
                ? {
                    id: matches[0].id,
                    label: matches[0].label,
                    status: matches[0].status,
                    public_ipv4: publicIpv4(matches[0]),
                    note: "found by the journaled unique label; run destroy to adopt and clean it safely",
                  }
                : matches.length === 0
                  ? { status: "unique label absent from a complete list; run destroy to confirm cleanup" }
                  : {
                      error: `duplicate owned label returned ${matches.length} Linodes; manual review required`,
                    };
          }
        } catch (error) {
          remote = { error: error.message };
        }
      }
    }
    const started = Date.parse(state.billing_window_started_at || "");
    const ended = Date.parse(state.linode_destroyed_at || "");
    const seconds = Number.isFinite(started)
      ? Math.max(0, ((Number.isFinite(ended) ? ended : Date.now()) - started) / 1000)
      : 0;
    const hourly = state.plan && state.plan.size ? state.plan.size.price_hourly_usd : null;
    rows.push({
      run_id: state.run_id,
      created_at: state.created_at,
      linode_id: state.linode_id,
      linode_label: state.linode_label,
      linode_create_attempted_at: state.linode_create_attempted_at || null,
      linode_absence_confirmed_at: state.linode_absence_confirmed_at || null,
      destroyed_at: state.linode_destroyed_at,
      cleanup_complete: Boolean(state.linode_destroyed_at),
      billing_deadline_at: state.billing_deadline_at || null,
      billable_seconds_upper_bound: Number(seconds.toFixed(3)),
      rounded_compute_charge_upper_bound_usd:
        typeof hourly === "number" ? roundedHourlyCharge(hourly, seconds) : null,
      remote_status: state.remote_status || null,
      promoted_at: state.promoted_at || null,
      remote,
    });
  }
  process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
}

async function destroy(runId) {
  const lockToken = acquireCoordinatorLock();
  try {
    const token = readStoredToken();
    const state = stateForRunId(runId);
    cleanupInProgress = true;
    try {
      await cleanupOwned(state, token);
    } finally {
      cleanupInProgress = false;
    }
    process.stdout.write(`Confirmed cleanup for owned Linode run ${state.run_id}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function promote(runId) {
  const lockToken = acquireCoordinatorLock();
  let recoveryDirectory = null;
  try {
    const state = completedStateForRunId(runId);
    const expectedDirectory = path.join(RUNS_ROOT, runId);
    if (
      state.schema !== "rustia.linode-benchmark-state.v1" ||
      path.resolve(state.__directory) !== expectedDirectory
    ) {
      fail("Linode run state identity or directory is invalid");
    }
    if (!state.linode_destroyed_at || !Number.isFinite(Date.parse(state.linode_destroyed_at))) {
      fail("offline promotion requires API-confirmed Linode deletion");
    }
    if (state.remote_exit_status !== 0 || state.remote_timed_out !== false) {
      fail("offline promotion requires a successful, non-timeout remote benchmark");
    }
    const config = loadConfig(state.config_path);
    if (state.remote_status !== expectedTerminalStatus(config)) {
      fail(`offline promotion requires a recorded ${expectedTerminalStatus(config)} result`);
    }
    const recordedRollback =
      Number.isFinite(Date.parse(state.promotion_rolled_back_at || "")) &&
      typeof state.promotion_rollback_reason === "string";
    const preJournalMemoryFailure =
      isVmStringMemory(config) &&
      state.remote_status === "evidence_complete" &&
      !state.promotion_started_at &&
      !fs.existsSync(path.join(REPO, memoryEvidenceRelativePath(config.source.candidate_commit)));
    if (!recordedRollback && !preJournalMemoryFailure) {
      fail("offline promotion requires a completed result whose first local import failed or was interrupted");
    }
    if (state.promoted_at) fail("Linode result is already promoted");

    const archive = path.join(expectedDirectory, "export.tar.gz");
    if (
      path.resolve(state.export_path || "") !== archive ||
      !SHA256.test(state.export_sha256 || "") ||
      !fs.lstatSync(archive).isFile() ||
      sha256File(archive) !== state.export_sha256
    ) {
      fail("Linode export archive is missing, redirected, or differs from the journaled download");
    }
    await verifyRecoveryCheckout(config, state);
    if (!isVmStringMemory(config)) {
      await run("env", ["-u", "RUSTC_WRAPPER", "just", "check-benchmark-results"], {
        capture: false,
        announce: true,
      });
    }

    recoveryDirectory = path.join(
      expectedDirectory,
      `recovery-download-${safeTimestamp()}-${crypto.randomBytes(4).toString("hex")}`,
    );
    state.recovery_promotion_started_at = new Date().toISOString();
    state.recovery_promotion_commit = outputText(await run("git", ["rev-parse", "HEAD"]));
    saveState(state);
    await extractArchive(archive, recoveryDirectory);
    if (sha256File(archive) !== state.export_sha256) {
      fail("Linode export archive changed during offline promotion");
    }
    const promoted = await promoteAcceptedResult(
      config,
      state,
      recoveryDirectory,
      verifyRecoveryCheckout,
    );
    if (!promoted) fail(`journaled Linode export is no longer ${expectedTerminalStatus(config)}`);
    state.recovery_promoted_at = new Date().toISOString();
    saveState(state);
    process.stdout.write(`Offline promotion completed for Linode run ${runId}.\n`);
  } catch (error) {
    if (runId && RUN_ID.test(runId)) {
      const matches = listStates().filter((state) => state.run_id === runId);
      if (matches.length === 1 && matches[0].recovery_promotion_started_at && !matches[0].promoted_at) {
        matches[0].recovery_promotion_failed_at = new Date().toISOString();
        matches[0].recovery_promotion_failure = error.message;
        saveState(matches[0]);
      }
    }
    throw error;
  } finally {
    if (recoveryDirectory) fs.rmSync(recoveryDirectory, { recursive: true, force: true });
    releaseCoordinatorLock(lockToken);
  }
}

async function selfTest(config) {
  assert.equal(config.publication.mode, "acceptance");
  assert.equal(config.provider.region, "ap-southeast");
  assert.equal(config.provider.image, "linode/ubuntu24.04");
  assert.equal(config.provider.size.id, "g6-dedicated-2");
  assert.equal(config.provider.size.class, "dedicated");
  assert.deepEqual(
    config.measurement.run_order.map((entry) => entry.variant),
    ["baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate"],
  );

  const responses = {
    type: {
      data: {
        id: "g6-dedicated-2",
        label: "Dedicated 4GB",
        class: "dedicated",
        vcpus: 2,
        memory: 4096,
        disk: 81920,
        price: { hourly: 0.054, monthly: 36 },
        region_prices: [],
      },
    },
    image: {
      data: {
        id: "linode/ubuntu24.04",
        label: "Ubuntu 24.04 LTS",
        is_public: true,
        status: "available",
      },
    },
    region: { data: { id: "ap-southeast", label: "Sydney, AU", country: "au", status: "ok" } },
    availability: { data: [{ region: "ap-southeast", plan: "g6-dedicated-2", available: true }] },
  };
  const selected = validateProviderCatalog(config, responses);
  assert.equal(selected.type.id, "g6-dedicated-2");
  assert.equal(selected.price.hourly, 0.054);
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      type: { data: { ...responses.type.data, vcpus: 4 } },
    }),
  );
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      availability: { data: [{ region: "ap-southeast", plan: "g6-dedicated-2", available: false }] },
    }),
  );
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      type: { data: { ...responses.type.data, price: { hourly: 0.055, monthly: 37 } } },
    }),
  );

  const readOnlyCalls = [];
  const catalogRequest = async (token, method, apiPath, options) => {
    readOnlyCalls.push([token, method, apiPath, options]);
    assert.equal(token, null);
    assert.equal(options.unauthenticated, true);
    if (apiPath.includes("/linode/types/")) return responses.type;
    if (apiPath.includes("/images/")) return responses.image;
    if (apiPath.endsWith("/availability")) return responses.availability;
    if (apiPath.includes("/regions/")) return responses.region;
    throw new Error(`unexpected fixture request ${method} ${apiPath}`);
  };
  const resolved = await resolveProviderPlan("fixture-token", config, catalogRequest);
  assert.equal(resolved.region.id, "ap-southeast");
  assert.equal(readOnlyCalls.length, 4);
  assert(readOnlyCalls.every(([, method]) => method === "GET"));

  await preflightAccount("fixture-token", async (_token, method, apiPath) => {
    assert.equal(method, "GET");
    assert.equal(apiPath, "/v4/account/settings");
    return { data: { backups_enabled: false, interfaces_for_new_linodes: "legacy_config_only" } };
  });
  await assert.rejects(() =>
    preflightAccount("fixture-token", async () => ({
      data: { backups_enabled: true, interfaces_for_new_linodes: "legacy_config_only" },
    })),
  );
  await assert.rejects(() =>
    preflightAccount("fixture-token", async () => ({
      data: { backups_enabled: false, interfaces_for_new_linodes: "linode_only" },
    })),
  );
  await preflightAccountRegion("fixture-token", config, async (_token, method, apiPath) => {
    assert.equal(method, "GET");
    assert.equal(apiPath, "/v4/account/availability/ap-southeast");
    return {
      data: {
        available: ["Linodes", "NodeBalancers", "Block Storage", "Kubernetes"],
        unavailable: [],
      },
    };
  });
  await assert.rejects(() =>
    preflightAccountRegion("fixture-token", config, async () => ({
      data: { available: ["NodeBalancers"], unavailable: ["Linodes"] },
    })),
  );
  await assert.rejects(() =>
    preflightAccountRegion("fixture-token", config, async () => ({
      data: { available: null, unavailable: [] },
    })),
  );

  const authorityCalls = [];
  await verifyDeleteAuthority(
    "fixture-token",
    async (_token, method, apiPath, options = {}) => {
      authorityCalls.push([method, apiPath, options]);
      if (method === "GET") return { data: { data: [], page: 1, pages: 1, results: 0 } };
      assert.equal(method, "DELETE");
      assert.deepEqual(options.acceptedStatuses, [404]);
      return { status: 404, data: { errors: [{ reason: "Not found" }] } };
    },
    { announce: false },
  );
  assert.deepEqual(authorityCalls.map(([method]) => method), ["GET", "DELETE"]);

  assert.equal(roundedHourlyCharge(0.054, 0), 0);
  assert.equal(roundedHourlyCharge(0.054, 1), 0.054);
  assert.equal(roundedHourlyCharge(0.054, 3599.999), 0.054);
  assert.equal(roundedHourlyCharge(0.054, 3600), 0.054);
  assert.equal(roundedHourlyCharge(0.054, 3600.001), 0.108);
  assert.equal(roundedHourlyCharge(0.054, config.provider.max_runtime_minutes * 60), 0.054);

  const publicKey = `ssh-ed25519 ${Buffer.alloc(32, 7).toString("base64")} rustia-test`;
  const body = createLinodeBody(config, "rustia-bench-test", publicKey);
  assert.deepEqual(body.authorized_keys, [publicKey]);
  assert.equal(body.backups_enabled, false);
  assert.equal(body.booted, true);
  assert.equal(body.interface_generation, "legacy_config");
  assert.equal(body.firewall_id, -1);
  assert.equal(body.private_ip, false);
  assert.equal(body.maintenance_policy, "linode/power_off_on");
  assert.equal(Object.prototype.hasOwnProperty.call(body, "root_pass"), false);
  assert.equal(Object.prototype.hasOwnProperty.call(body, "token"), false);
  assert.throws(() => createLinodeBody(config, "rustia-bench-test", "not-a-key"));

  assert.equal(isPublicIpv4("203.0.113.9"), true);
  for (const address of ["999.1.2.3", "10.0.0.1", "127.0.0.1", "169.254.1.1", "172.16.0.1", "192.168.1.1"]) {
    assert.equal(isPublicIpv4(address), false);
  }
  assert.equal(publicIpv4({ ipv4: ["10.0.0.1", "203.0.113.9"] }), "203.0.113.9");
  assert.equal(matchingLinodesByLabel([{ label: "a" }, { label: "b" }, { label: "a" }], "a").length, 2);

  const plannedShape = {
    region: { id: "ap-southeast" },
    image: { id: "linode/ubuntu24.04" },
    size: { id: "g6-dedicated-2", vcpus: 2, memory_mib: 4096, disk_gib: 80 },
  };
  const actualShape = {
    type: "g6-dedicated-2",
    region: "ap-southeast",
    image: "linode/ubuntu24.04",
    specs: { vcpus: 2, memory: 4096, disk: 81920 },
    backups: { enabled: false },
    interface_generation: "legacy_config",
    maintenance_policy: "linode/power_off_on",
  };
  verifyLinodeMatchesPlan(actualShape, plannedShape);
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, specs: { ...actualShape.specs, disk: 81921 } }, plannedShape));
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, backups: { enabled: true } }, plannedShape));
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, interface_generation: "linode" }, plannedShape));
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, maintenance_policy: null }, plannedShape));
  assert.throws(() =>
    validateConfig({
      ...config,
      source: { ...config.source, candidate_commit: config.source.baseline_commit },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      publication: { ...config.publication, mode: "diagnostic" },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      host_gate: { ...config.host_gate, maximum_timed_steal_percent: 0.2 },
    }),
  );
  assert.equal(
    validateConfig({
      ...config,
      measurement: { protocol: FIXED_THROUGHPUT_PROTOCOL, ...config.measurement },
    }).measurement.protocol,
    FIXED_THROUGHPUT_PROTOCOL,
  );

  const memoryHash = "a".repeat(64);
  const memoryConfig = {
    schema: "rustia.linode-fixed-benchmark.v2",
    provider: config.provider,
    source: {
      baseline_commit: config.source.baseline_commit,
      candidate_commit: config.source.candidate_commit,
      harness_commit: config.source.harness_commit,
      expected_sha256: {
        baseline_original_bench_manifest: memoryHash,
        candidate_original_bench_manifest: memoryHash,
        overlay_bench_manifest: memoryHash,
        sampler: memoryHash,
        analyzer: memoryHash,
        cargo_lock: memoryHash,
        toolchain: memoryHash,
      },
    },
    measurement: {
      protocol: VM_STRING_MEMORY_PROTOCOL,
      vm_count: 512,
      accepted_rounds: 16,
      warmup_rounds: 1,
      round_retry_limit: 3,
      scenarios: [...VM_STRING_SCENARIOS],
      bootstrap_repetitions: 100000,
      bootstrap_seed: 1381323604,
    },
    host_gate: config.host_gate,
    publication: { mode: "evidence_only" },
  };
  assert.equal(validateConfig(memoryConfig), memoryConfig);
  for (const mutation of [
    { measurement: { ...memoryConfig.measurement, protocol: "unknown" } },
    { measurement: { ...memoryConfig.measurement, vm_count: 511 } },
    { measurement: { ...memoryConfig.measurement, accepted_rounds: 15 } },
    { measurement: { ...memoryConfig.measurement, warmup_rounds: 2 } },
    { measurement: { ...memoryConfig.measurement, round_retry_limit: 4 } },
    { measurement: { ...memoryConfig.measurement, scenarios: [...VM_STRING_SCENARIOS].reverse() } },
    { publication: { mode: "acceptance" } },
  ]) {
    assert.throws(() => validateConfig({ ...memoryConfig, ...mutation }));
  }
  assert.throws(() => validateConfig({ ...memoryConfig, qualification: {} }));
  const matrixArtifact = {
    version: "fixture",
    url: "https://example.invalid/runtime.zip",
    sha256: memoryHash,
    archive: "zip",
    binary_relative_path: "bin/runtime",
  };
  const matrixConfig = {
    schema: "rustia.linode-fixed-benchmark.v2",
    provider: config.provider,
    source: {
      benchmark_commit: config.source.candidate_commit,
      expected_sha256: {
        bench_manifest: memoryHash,
        native_harness: memoryHash,
        cargo_lock: memoryHash,
        toolchain: memoryHash,
        wasm_gate: memoryHash,
        wasm_dispatch_gate: memoryHash,
        wasm_adapter_manifest: memoryHash,
        wasm_measure_module: memoryHash,
        wasm_dispatch_measure: memoryHash,
        history_before: memoryHash,
      },
    },
    measurement: {
      protocol: CROSS_ENGINE_MATRIX_PROTOCOL,
      native_target: MATRIX_NATIVE_TARGET,
      native_engines: [...MATRIX_NATIVE_ENGINES],
      workloads: [...MATRIX_WORKLOADS],
      native_target_ms: 120,
      native_repetitions: 5,
      wasm_runtimes: [...MATRIX_WASM_RUNTIMES],
      wasm_adapters: [...MATRIX_WASM_ADAPTERS],
      wasm_target_ms: 80,
      wasm_repetitions: 5,
      dispatch_target_ms: 80,
      dispatch_repetitions: 5,
      runtime_artifacts: Object.fromEntries(
        MATRIX_WASM_RUNTIMES.map((runtime) => [runtime, { ...matrixArtifact }]),
      ),
    },
    host_gate: {
      ...config.host_gate,
      maximum_timed_steal_ticks: 1,
      maximum_timed_attempts_per_group: 3,
      wait_timeout_seconds: 300,
    },
    publication: { mode: "matrix", native_rows: 50, wasm_rows: 15, dispatch_runtimes: 1 },
  };
  assert.equal(validateConfig(matrixConfig), matrixConfig);
  assert.equal(expectedTerminalStatus(matrixConfig), "matrix_complete");
  assert.equal(benchmarkCommit(matrixConfig), matrixConfig.source.benchmark_commit);
  assert.equal(remoteRunnerForConfig(matrixConfig), MATRIX_REMOTE_RUNNER);
  const matrixStatus = {
    schema: "rustia.bench.cloud-run-status.v1",
    status: "matrix_complete",
    detail: "fresh Linux native and wasm benchmark matrix completed",
    started_at: "2026-07-20T00:00:00.000Z",
    finished_at: "2026-07-20T00:01:00.000Z",
  };
  validateMatrixStatus(matrixStatus);
  let matrixGateTimestamp = Date.parse(matrixStatus.started_at);
  const nextMatrixGateTimestamp = () => new Date(matrixGateTimestamp += 1000).toISOString();
  const matrixGateRecords = [];
  for (const label of [
    "native-all-engines",
    "native-lua54",
    "wasm-adapter-matrix",
    "wasm-dispatch-matrix",
  ]) {
    const attempts = label === "native-lua54" ? 2 : 1;
    for (let timedAttempt = 1; timedAttempt <= attempts; timedAttempt += 1) {
      matrixGateRecords.push({
        phase: "pre",
        label,
        timed_attempt: timedAttempt,
        quiet_attempt: 1,
        captured_at: nextMatrixGateTimestamp(),
        load_1m: 0.01,
        total_ticks: 1000,
        idle_ticks: 1000,
        steal_ticks: 0,
        idle_percent: 100,
        steal_percent: 0,
        accepted: true,
      });
      const rejected = label === "native-lua54" && timedAttempt === 1;
      matrixGateRecords.push({
        phase: "timed",
        label,
        timed_attempt: timedAttempt,
        started_at: nextMatrixGateTimestamp(),
        finished_at: nextMatrixGateTimestamp(),
        total_ticks: 1000,
        idle_ticks: rejected ? 998 : 999,
        steal_ticks: rejected ? 2 : 1,
        idle_percent: rejected ? 99.8 : 99.9,
        steal_percent: rejected ? 0.2 : 0.1,
        accepted: !rejected,
      });
    }
  }
  requireLinuxWasmTarget("wasm32-v8-node-linux-x64", "matrix target fixture");
  requireLinuxWasmTarget("wasm32-dispatch-v8-node-linux-x64", "matrix dispatch target fixture", true);
  assert.throws(() => requireLinuxWasmTarget("wasm32-v8-node-darwin-arm64", "matrix target fixture"));
  requireFreshMatrixTimestamp(1784505630, matrixStatus, "matrix timestamp fixture");
  assert.throws(() => requireFreshMatrixTimestamp(1784505691, matrixStatus, "matrix timestamp fixture"));
  assert.throws(() => validateConfig({
    ...matrixConfig,
    measurement: { ...matrixConfig.measurement, native_engines: [...MATRIX_NATIVE_ENGINES].reverse() },
  }));
  assert.throws(() => validateConfig({
    ...matrixConfig,
    publication: { ...matrixConfig.publication, wasm_rows: 14 },
  }));
  for (const maximumTimedAttempts of [0, MAX_TIMED_ATTEMPTS_PER_GROUP + 1]) {
    assert.throws(() => validateConfig({
      ...matrixConfig,
      host_gate: {
        ...matrixConfig.host_gate,
        maximum_timed_attempts_per_group: maximumTimedAttempts,
      },
    }));
  }
  for (const maximumTimedStealTicks of [0, 2]) {
    assert.throws(() => validateConfig({
      ...matrixConfig,
      host_gate: {
        ...matrixConfig.host_gate,
        maximum_timed_steal_ticks: maximumTimedStealTicks,
      },
    }));
  }
  for (const waitTimeoutSeconds of [299, 301]) {
    assert.throws(() => validateConfig({
      ...matrixConfig,
      host_gate: {
        ...matrixConfig.host_gate,
        wait_timeout_seconds: waitTimeoutSeconds,
      },
    }));
  }
  assert.equal(timedStealGatePasses(matrixConfig, { total_ticks: 946, steal_ticks: 1 }), true);
  assert.equal(timedStealGatePasses(matrixConfig, { total_ticks: 946, steal_ticks: 2 }), false);
  assert.equal(timedStealGatePasses(matrixConfig, { total_ticks: 10_000, steal_ticks: 2 }), true);
  const firstRoles = [];
  for (let round = 1; round <= 16; round += 1) {
    const order = memoryRoundOrder(round);
    assert.equal(order.length, 10);
    firstRoles.push(order[0].role);
    assert(order.slice(0, 5).every((entry) => entry.role === order[0].role));
    assert(order.slice(5).every((entry) => entry.role !== order[0].role));
    assert.deepEqual(
      order.slice(5).map((entry) => entry.scenario),
      order.slice(0, 5).map((entry) => entry.scenario).reverse(),
    );
    for (const role of ["baseline", "candidate"]) {
      assert.deepEqual(
        order.filter((entry) => entry.role === role).map((entry) => entry.scenario).sort(),
        [...VM_STRING_SCENARIOS].sort(),
      );
    }
  }
  assert.deepEqual(firstRoles, [
    "baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate",
    "baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate",
  ]);
  assert.equal(firstRoles.filter((role) => role === "baseline").length, 8);
  assert.equal(firstRoles.filter((role) => role === "candidate").length, 8);
  const memoryPositions = Object.fromEntries(
    VM_STRING_SCENARIOS.map((scenario) => [scenario, { baseline: [], candidate: [] }]),
  );
  for (let round = 1; round <= 16; round += 1) {
    memoryRoundOrder(round).forEach((entry, index) => {
      memoryPositions[entry.scenario][entry.role].push(index + 1);
    });
    if (round % 2 === 1) {
      const first = memoryRoundOrder(round);
      const second = memoryRoundOrder(round + 1);
      assert.notEqual(first[0].role, second[0].role);
      assert.deepEqual(first.slice(0, 5).map((entry) => entry.scenario), second.slice(0, 5).map((entry) => entry.scenario));
    }
  }
  for (const positions of Object.values(memoryPositions)) {
    assert.deepEqual([...positions.baseline].sort((left, right) => left - right), [...positions.candidate].sort((left, right) => left - right));
    assert.equal(positions.baseline.reduce((sum, value) => sum + value, 0), 88);
    assert.equal(positions.candidate.reduce((sum, value) => sum + value, 0), 88);
  }
  assert.deepEqual([...VM_STRING_EVIDENCE_FILES].sort(), VM_STRING_EVIDENCE_FILES);
  assert.deepEqual(
    VM_STRING_HASHED_EVIDENCE_FILES,
    VM_STRING_EVIDENCE_FILES.filter((file) => file !== "manifest.json"),
  );
  const memoryProcessIds = new Set();
  addFreshMemoryProcessId(memoryProcessIds, 101, "fixture");
  assert.throws(() => addFreshMemoryProcessId(memoryProcessIds, 101, "fixture"));
  assert.equal(memorySampleId("round", 16, "candidate", "long_loaded"), "round-16-candidate-long-loaded");
  assert.equal(expectedTerminalStatus(memoryConfig), "evidence_complete");
  assert.equal(expectedTerminalStatus(config), "accepted_and_published");
  assert.equal(
    memoryEvidenceRelativePath(memoryConfig.source.candidate_commit),
    path.join("bench/evidence/vm-string-memory", memoryConfig.source.candidate_commit),
  );
  const memoryLayout = memoryPromotionLayout(memoryConfig, "/tmp/fixture-download");
  assert.equal(
    memoryLayout.evidence_source,
    path.join("/tmp/fixture-download/evidence/vm-string-memory", memoryConfig.source.candidate_commit),
  );
  assert.equal(
    memoryLayout.evidence_destination,
    path.join(REPO, "bench/evidence/vm-string-memory", memoryConfig.source.candidate_commit),
  );
  assert.equal(memoryLayout.publication_source, null);
  assert.equal(memoryLayout.history_destination, null);
  assert.throws(() => memoryPromotionLayout(config, "/tmp/fixture-download"));
  assert.throws(() => memoryEvidenceRelativePath("not-a-commit"));
  const evidenceStatus = {
    schema: "rustia.bench.cloud-run-status.v1",
    status: "evidence_complete",
    detail: "vm string memory evidence completed; public throughput history was not changed",
    started_at: "2026-07-20T00:00:00.000Z",
    finished_at: "2026-07-20T00:01:00.000Z",
  };
  validateMemoryEvidenceStatus(evidenceStatus);
  assert.throws(() => validateMemoryEvidenceStatus({ ...evidenceStatus, status: "accepted_and_published" }));
  assert.throws(() => validateMemoryEvidenceStatus({ ...evidenceStatus, verdict: "anything" }));

  assert.equal(finalSidecarName(config, 1), `final-${config.measurement.workload}-1.json`);
  assert.equal(
    finalSidecarName({ measurement: { workload: "oop_metamethods" } }, 3),
    "final-oop_metamethods-3.json",
  );
  const escapedWorkloadFile = finalSidecarName(
    { measurement: { workload: "../../Odd workload/\u2728" } },
    2,
  );
  assert.match(escapedWorkloadFile, /^final-odd-workload-[0-9a-f]{12}-2\.json$/);
  assert.equal(path.basename(escapedWorkloadFile), escapedWorkloadFile);
  assert.throws(() => finalSidecarName(config, 0));

  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-linode-self-test-"));
  try {
    const file = path.join(temporary, "secret");
    writeBytesAtomic(file, Buffer.from("fixture\n"), 0o600);
    assert.equal(fs.statSync(file).mode & 0o777, 0o600);
    assert.equal(fs.readFileSync(file, "utf8"), "fixture\n");

    const fixtureRepo = path.join(temporary, "repo");
    const outside = path.join(temporary, "outside");
    fs.mkdirSync(path.join(fixtureRepo, "bench", "cloud"), { recursive: true });
    fs.mkdirSync(outside);
    const defaultFixture = path.join(fixtureRepo, "bench", "cloud", "default.json");
    const alternateFixture = path.join(fixtureRepo, "bench", "cloud", "alternate.json");
    const outsideFixture = path.join(outside, "outside.json");
    fs.writeFileSync(defaultFixture, "{}\n");
    fs.writeFileSync(alternateFixture, "{}\n");
    fs.writeFileSync(outsideFixture, "{}\n");
    const malformedMemorySamples = path.join(temporary, "malformed-memory-samples.jsonl");
    const malformedMemoryGate = path.join(temporary, "malformed-memory-gate.jsonl");
    const matrixGate = path.join(temporary, "matrix-gate.jsonl");
    fs.writeFileSync(malformedMemorySamples, "{}\n");
    fs.writeFileSync(malformedMemoryGate, "{}\n");
    assert.throws(() =>
      validateMemorySampleFile(
        memoryConfig,
        malformedMemorySamples,
        "round",
        memoryConfig.measurement.accepted_rounds,
        memoryHash,
      ),
    );
    assert.throws(() =>
      validateMemoryGateLog(memoryConfig, malformedMemoryGate, 1, [1], Array(16).fill(1)),
    );
    const writeMatrixGate = (records) => {
      fs.writeFileSync(matrixGate, `${records.map((record) => JSON.stringify(record)).join("\n")}\n`);
    };
    writeMatrixGate(matrixGateRecords);
    validateMatrixGateLog(matrixConfig, matrixGate, matrixStatus);
    const falseAcceptance = structuredClone(matrixGateRecords);
    falseAcceptance.find(
      (record) => record.phase === "timed" && record.label === "native-lua54" && record.timed_attempt === 1,
    ).accepted = true;
    writeMatrixGate(falseAcceptance);
    assert.throws(() => validateMatrixGateLog(matrixConfig, matrixGate, matrixStatus));
    const missingAcceptedRetry = matrixGateRecords.filter(
      (record) => !(record.label === "native-lua54" && record.timed_attempt === 2),
    );
    writeMatrixGate(missingAcceptedRetry);
    assert.throws(() => validateMatrixGateLog(matrixConfig, matrixGate, matrixStatus));
    const outOfWindow = structuredClone(matrixGateRecords);
    outOfWindow[0].captured_at = "2026-07-19T23:59:59.000Z";
    writeMatrixGate(outOfWindow);
    assert.throws(() => validateMatrixGateLog(matrixConfig, matrixGate, matrixStatus));
    const outOfOrder = structuredClone(matrixGateRecords);
    outOfOrder[1].started_at = outOfOrder[0].captured_at;
    outOfOrder[0].captured_at = outOfOrder[1].finished_at;
    writeMatrixGate(outOfOrder);
    assert.throws(() => validateMatrixGateLog(matrixConfig, matrixGate, matrixStatus));
    const resolverOptions = { repo: fixtureRepo, defaultConfig: defaultFixture };
    assert.equal(resolveConfigFile(undefined, resolverOptions), defaultFixture);
    assert.equal(resolveConfigFile("bench/cloud/alternate.json", resolverOptions), alternateFixture);
    assert.throws(() => resolveConfigFile("", resolverOptions));
    assert.throws(() => resolveConfigFile("../outside/outside.json", resolverOptions));
    assert.throws(() => resolveConfigFile("bench/../bench/cloud/alternate.json", resolverOptions));
    assert.throws(() => resolveConfigFile(outsideFixture, resolverOptions));
    assert.throws(() => resolveConfigFile("bench", resolverOptions));
    fs.symlinkSync(alternateFixture, path.join(fixtureRepo, "linked-config.json"));
    assert.throws(() => resolveConfigFile("linked-config.json", resolverOptions));
    fs.symlinkSync(outside, path.join(fixtureRepo, "linked-directory"));
    assert.throws(() => resolveConfigFile("linked-directory/outside.json", resolverOptions));
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }

  const runner = await run("node", [REMOTE_RUNNER, "self-test"]);
  assert.match(outputText(runner), /self-test passed/);
  const matrixRunner = await run("node", [MATRIX_REMOTE_RUNNER, "self-test"]);
  assert.match(outputText(matrixRunner), /self-test passed/);
  process.stdout.write("linode benchmark coordinator offline self-test passed\n");
}

function requireCommandArguments(minimum, maximum = minimum) {
  const count = process.argv.length - 2;
  if (count < minimum || count > maximum) fail("unexpected command arguments; tokens are never accepted in argv");
}

function usage() {
  process.stdout.write(`Usage:
  node tools/run-linode-benchmark.js auth
  node tools/run-linode-benchmark.js plan [CONFIG.json]
  node tools/run-linode-benchmark.js bench [CONFIG.json]
  node tools/run-linode-benchmark.js status
  node tools/run-linode-benchmark.js destroy [RUN-ID]
  node tools/run-linode-benchmark.js promote RUN-ID
  node tools/run-linode-benchmark.js unlock
  node tools/run-linode-benchmark.js self-test

CONFIG.json must be a regular, non-symlink repository-relative file. The default is
${path.relative(REPO, DEFAULT_CONFIG)}.
`);
}

async function main() {
  const command = process.argv[2];
  switch (command) {
    case "auth":
      requireCommandArguments(1);
      await authenticate(loadConfig());
      break;
    case "plan":
      requireCommandArguments(1, 2);
      {
        const configFile = resolveConfigFile(process.argv[3]);
        await buildPlan(loadConfig(configFile), { configFile });
      }
      break;
    case "bench":
      requireCommandArguments(1, 2);
      {
        const configFile = resolveConfigFile(process.argv[3]);
        await bench(loadConfig(configFile), configFile);
      }
      break;
    case "status":
      requireCommandArguments(1);
      await status();
      break;
    case "destroy":
      requireCommandArguments(1, 2);
      await destroy(process.argv[3] || "");
      break;
    case "promote":
      requireCommandArguments(2);
      await promote(process.argv[3]);
      break;
    case "unlock":
      requireCommandArguments(1);
      unlockStaleCoordinator();
      break;
    case "self-test":
      requireCommandArguments(1);
      await selfTest(loadConfig());
      break;
    case "--help":
    case "-h":
    case "help":
      requireCommandArguments(1);
      usage();
      break;
    default:
      usage();
      process.exitCode = 2;
  }
}

main().catch((error) => {
  process.stderr.write(`run-linode-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
