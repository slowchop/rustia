#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const {
  cgroupBlockerMessage,
  delegatedCgroupRoot,
} = require("../bench/cgroup-delegation.js");

// The run's working tree. Was fixed at /opt/rustia, which only a root process on
// a throwaway instance can write; the owned host runs this as an ordinary user.
const ROOT = process.env.RUSTIA_MEMORY_ROOT ?? path.join(os.homedir(), ".rustia-memory");
const DEDICATED_SCHEMA = "rustia.dedicated-memory-benchmark.v1";
const REPOSITORY = path.join(ROOT, "repo.git");
const SOURCE = path.join(ROOT, "src-memory");
const TARGET_ALL = path.join(ROOT, "target-memory-all");
const TARGET_LUA54 = path.join(ROOT, "target-memory-lua54");
const EXPORT = path.join(ROOT, "export");
const EXPORT_ARCHIVE = path.join(ROOT, "export.tar.gz");
const RUN_LOG = path.join(ROOT, "run.log");
const EVIDENCE_ROOT = path.join(ROOT, "memory-evidence");
const PROTOCOL = "cross_engine_memory_v1";
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;

let logReady = false;

function fail(message) {
  throw new Error(message);
}

function log(message) {
  const line = `[${new Date().toISOString()}] ${message}`;
  process.stdout.write(`${line}\n`);
  if (logReady) fs.appendFileSync(RUN_LOG, `${line}\n`);
}

function object(value, label) {
  if (value == null || typeof value !== "object" || Array.isArray(value)) fail(`${label} must be an object`);
  return value;
}

function integer(value, label, minimum = 0) {
  if (!Number.isSafeInteger(value) || value < minimum) fail(`${label} must be an integer >= ${minimum}`);
  return value;
}

function exactKeys(value, expected, label) {
  const actual = Object.keys(object(value, label)).sort();
  if (JSON.stringify(actual) !== JSON.stringify([...expected].sort())) fail(`${label} has unexpected keys`);
}

function loadJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot load ${label}: ${error.message}`);
  }
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function run(command, args, options = {}) {
  log(`+ ${[command, ...args].join(" ")}`);
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? ROOT,
    env: options.env ?? process.env,
    encoding: options.capture === false ? undefined : "utf8",
    stdio: options.capture === false ? "inherit" : ["ignore", "pipe", "pipe"],
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.error) fail(`${command} failed to start: ${result.error.message}`);
  if (result.status !== 0 && !options.allowFailure) {
    fail(`${command} failed with status ${result.status}: ${String(result.stderr ?? "").trim()}`);
  }
  return result;
}

function runText(command, args, options = {}) {
  return String(run(command, args, options).stdout ?? "").trim();
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

// "4", "4,5" and "0-1,4" all name CPU sets in the kernel's own notation.
function parseCpuList(spec) {
  const text = String(spec).trim();
  if (!text) fail("CPU list is empty");
  const cpus = [];
  for (const entry of text.split(",")) {
    const range = entry.match(/^(\d+)-(\d+)$/);
    if (range) {
      const start = Number(range[1]);
      const end = Number(range[2]);
      if (end < start) fail(`invalid CPU range ${entry}`);
      for (let cpu = start; cpu <= end; cpu += 1) cpus.push(cpu);
      continue;
    }
    if (!/^\d+$/.test(entry)) fail(`invalid CPU list entry ${entry}`);
    cpus.push(Number(entry));
  }
  return cpus;
}

function validateConfig(config) {
  exactKeys(config, ["schema", "provider", "source", "measurement", "host_gate", "publication"], "config");
  // Two lifecycles during the move to the owned host. Only the host description
  // differs; every measurement rule below is shared.
  const dedicated = config.schema === DEDICATED_SCHEMA;
  if (!dedicated && config.schema !== "rustia.linode-fixed-benchmark.v2") {
    fail(`memory runner requires ${DEDICATED_SCHEMA} or Linode v2`);
  }
  if (dedicated) {
    const host = object(object(config.provider, "provider").host, "provider.host");
    exactKeys(
      host,
      ["name", "cpu_model", "logical_cpus", "physical_cores", "pinned_cpus", "max_runtime_minutes"],
      "provider.host",
    );
    // Two pinned cores, not one: the supervisor samples while the benchmark
    // runs, and sharing a core would make the supervisor part of what it
    // measures.
    const pinned = parseCpuList(host.pinned_cpus);
    for (const field of ["benchmark_cpu", "supervisor_cpu"]) {
      const cpu = object(config.host_gate, "host_gate")[field];
      if (!pinned.includes(cpu)) {
        fail(`host_gate.${field} ${cpu} is not one of provider.host.pinned_cpus (${pinned.join(",")})`);
      }
    }
  }
  const source = object(config.source, "source");
  exactKeys(source, ["benchmark_commit", "expected_sha256"], "source");
  if (!FULL_COMMIT.test(source.benchmark_commit)) fail("source.benchmark_commit must be a full commit");
  const hashes = object(source.expected_sha256, "source.expected_sha256");
  exactKeys(hashes, [
    "bench_manifest",
    "native_harness",
    "cargo_lock",
    "toolchain",
    "memory_supervisor",
    "memory_analyzer",
    "memory_cohort",
    "memory_publisher",
    "memory_history_before",
    "memory_current_before",
  ], "source.expected_sha256");
  for (const [name, hash] of Object.entries(hashes)) {
    if (!SHA256.test(hash)) fail(`source.expected_sha256.${name} must be a SHA-256`);
  }
  const measurement = object(config.measurement, "measurement");
  exactKeys(measurement, [
    "protocol",
    "cohort_id",
    "samples",
    "bootstrap_repetitions",
    "bootstrap_seed",
    "growth_thresholds",
    "sample_interval_ms",
    "sample_timeout_ms",
    "groups",
  ], "measurement");
  if (measurement.protocol !== PROTOCOL) fail(`measurement.protocol must be ${PROTOCOL}`);
  integer(measurement.samples, "measurement.samples", 9);
  integer(measurement.bootstrap_repetitions, "measurement.bootstrap_repetitions", 1000);
  integer(measurement.bootstrap_seed, "measurement.bootstrap_seed", 0);
  integer(measurement.sample_interval_ms, "measurement.sample_interval_ms", 1);
  integer(measurement.sample_timeout_ms, "measurement.sample_timeout_ms", 1000);
  if (!Array.isArray(measurement.groups) || measurement.groups.length === 0) {
    fail("measurement.groups must be nonempty");
  }
  const gate = object(config.host_gate, "host_gate");
  exactKeys(gate, [
    "benchmark_cpu",
    "supervisor_cpu",
    "idle_window_seconds",
    "minimum_idle_percent",
    "maximum_load_1m",
    "maximum_idle_steal_ticks",
    "maximum_run_steal_percent",
    "wait_timeout_seconds",
  ], "host_gate");
  integer(gate.benchmark_cpu, "host_gate.benchmark_cpu");
  integer(gate.supervisor_cpu, "host_gate.supervisor_cpu");
  if (gate.benchmark_cpu === gate.supervisor_cpu) fail("benchmark and supervisor CPUs must differ");
  integer(gate.idle_window_seconds, "host_gate.idle_window_seconds", 1);
  integer(gate.maximum_idle_steal_ticks, "host_gate.maximum_idle_steal_ticks");
  integer(gate.wait_timeout_seconds, "host_gate.wait_timeout_seconds", 1);
  const publication = object(config.publication, "publication");
  exactKeys(publication, ["mode", "rows"], "publication");
  if (!["memory", "rustia_ci_memory"].includes(publication.mode)) {
    fail("publication.mode must be memory or rustia_ci_memory");
  }
  integer(publication.rows, "publication.rows", 1);
  return config;
}

function verifySource(config, bundle) {
  run("git", ["clone", "--bare", bundle, REPOSITORY], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "bundle", "verify", bundle], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", SOURCE, config.source.benchmark_commit], {
    capture: false,
  });
  assert.equal(runText("git", ["-C", SOURCE, "rev-parse", "HEAD"]), config.source.benchmark_commit);
  const files = {
    bench_manifest: "bench/Cargo.toml",
    native_harness: "bench/src/main.rs",
    cargo_lock: "Cargo.lock",
    toolchain: "rust-toolchain.toml",
    memory_supervisor: "bench/run-memory-supervisor.js",
    memory_analyzer: "bench/analyze-memory-samples.js",
    memory_cohort: "bench/run-memory-cohort.js",
    memory_publisher: "bench/publish-memory-analysis.js",
    memory_history_before: "bench/memory/history/runs.jsonl",
    memory_current_before: "bench/memory/current-results.json",
  };
  for (const [field, relative] of Object.entries(files)) {
    const actual = sha256File(path.join(SOURCE, relative));
    if (actual !== config.source.expected_sha256[field]) {
      fail(`${relative} SHA-256 is ${actual}, expected ${config.source.expected_sha256[field]}`);
    }
  }
}

function installRust() {
  if (spawnSync("rustup", ["--version"], { stdio: "ignore" }).status !== 0) {
    const installer = path.join(ROOT, "rustup-init.sh");
    run("curl", [
      "--proto", "=https", "--tlsv1.2", "--fail", "--silent", "--show-error",
      "https://sh.rustup.rs", "-o", installer,
    ]);
    run("sh", [installer, "-y", "--profile", "minimal", "--default-toolchain", "none"], { capture: false });
  }
  run("rustup", ["toolchain", "install", "1.95.0", "--profile", "minimal"], {
    capture: false,
  });
}

function build(config) {
  const cargo = "cargo";
  const rootManifest = path.join(SOURCE, "Cargo.toml");
  const benchManifest = path.join(SOURCE, "bench/Cargo.toml");
  const environment = (target) => ({
    ...process.env,
    CARGO_INCREMENTAL: "0",
    CARGO_TARGET_DIR: target,
    RUSTC_WRAPPER: "",
  });
  run(cargo, ["+1.95.0", "fetch", "--locked", "--manifest-path", rootManifest], {
    env: environment(TARGET_ALL),
    capture: false,
  });
  if (config.publication.mode === "rustia_ci_memory") {
    run(cargo, [
      "+1.95.0", "build", "--locked", "--offline", "--release", "--manifest-path", benchManifest,
      "--bin", "script-bench", "--no-default-features", "--features", "rustia-engine",
    ], { env: environment(TARGET_ALL), capture: false });
    const all = path.join(TARGET_ALL, "release/script-bench");
    fs.accessSync(all, fs.constants.X_OK);
    const binarySha256 = sha256File(all);
    return {
      all,
      // The Rustia-only matrix never selects the lua54 binary. Supplying the
      // validated Rustia executable keeps the child interface uniform without
      // compiling any competitor runtime.
      lua54: all,
      all_sha256: binarySha256,
      lua54_sha256: binarySha256,
    };
  }
  run(cargo, [
    "+1.95.0", "build", "--locked", "--offline", "--release", "--manifest-path", benchManifest,
    "--bin", "script-bench", "--no-default-features", "--features", "all-engines",
  ], { env: environment(TARGET_ALL), capture: false });
  run(cargo, [
    "+1.95.0", "build", "--locked", "--offline", "--release", "--manifest-path", benchManifest,
    "--bin", "script-bench", "--no-default-features", "--features", "mlua-lua54",
  ], { env: environment(TARGET_LUA54), capture: false });
  const all = path.join(TARGET_ALL, "release/script-bench");
  const lua54 = path.join(TARGET_LUA54, "release/script-bench");
  fs.accessSync(all, fs.constants.X_OK);
  fs.accessSync(lua54, fs.constants.X_OK);
  return { all, lua54, all_sha256: sha256File(all), lua54_sha256: sha256File(lua54) };
}

function validateRustiaWasmQuota(config) {
  if (config.publication.mode !== "rustia_ci_memory") return;
  const cargo = "cargo";
  run("rustup", [
    "target", "add", "--toolchain", "1.95.0", "wasm32-unknown-unknown",
  ], { capture: false });
  run(cargo, [
    "+1.95.0", "build", "--locked", "--release",
    "--manifest-path", path.join(SOURCE, "bench/wasm/adapter-runner/Cargo.toml"),
    "--target", "wasm32-unknown-unknown",
  ], { capture: false, env: { ...process.env, RUSTC_WRAPPER: "" } });
  run("node", [
    path.join(SOURCE, "bench/wasm/test-rustia-memory-limit.mjs"),
    path.join(
      SOURCE,
      "bench/wasm/adapter-runner/target/wasm32-unknown-unknown/release/script_bench_wasm_adapter.wasm",
    ),
  ], { cwd: SOURCE, capture: false });
}

function readCpu(cpu) {
  const line = fs.readFileSync("/proc/stat", "utf8").split("\n").find((entry) => entry.startsWith(`cpu${cpu} `));
  if (!line) fail(`cannot read CPU ${cpu}`);
  const fields = line.trim().split(/\s+/).slice(1).map(Number);
  if (fields.length < 8 || fields.some((value) => !Number.isFinite(value))) fail("malformed /proc/stat");
  return fields;
}

function cpuDelta(before, after) {
  const values = before.map((value, index) => after[index] - value);
  if (values.some((value) => value < 0)) fail("CPU counters moved backwards");
  const total = values.reduce((sum, value) => sum + value, 0);
  const idle = values[3] + values[4];
  return {
    total_ticks: total,
    idle_ticks: idle,
    steal_ticks: values[7],
    idle_percent: total === 0 ? 0 : idle * 100 / total,
    steal_percent: total === 0 ? 0 : values[7] * 100 / total,
  };
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitForQuiet(config) {
  const gate = config.host_gate;
  const deadline = Date.now() + gate.wait_timeout_seconds * 1000;
  while (Date.now() < deadline) {
    const before = readCpu(gate.benchmark_cpu);
    await sleep(gate.idle_window_seconds * 1000);
    const delta = cpuDelta(before, readCpu(gate.benchmark_cpu));
    const load = Number(fs.readFileSync("/proc/loadavg", "utf8").trim().split(/\s+/)[0]);
    if (
      delta.idle_percent >= gate.minimum_idle_percent &&
      delta.steal_ticks <= gate.maximum_idle_steal_ticks &&
      load <= gate.maximum_load_1m
    ) {
      return { captured_at: new Date().toISOString(), load_1m: load, ...delta };
    }
    await sleep(5000);
  }
  fail("memory cohort did not obtain a quiet pre-run host window");
}

function memoryProtocol(config) {
  const measurement = config.measurement;
  return {
    schema: "rustia.bench.memory-protocol.v1",
    cohort_id: measurement.cohort_id,
    commit: config.source.benchmark_commit,
    samples: measurement.samples,
    bootstrap_repetitions: measurement.bootstrap_repetitions,
    bootstrap_seed: measurement.bootstrap_seed,
    growth_thresholds: measurement.growth_thresholds,
    host: {
      benchmark_cpu: config.host_gate.benchmark_cpu,
      supervisor_cpu: config.host_gate.supervisor_cpu,
      sample_interval_ms: measurement.sample_interval_ms,
      timeout_ms: measurement.sample_timeout_ms,
    },
    groups: measurement.groups,
  };
}

function exportRun(status, config, evidenceDirectory) {
  fs.rmSync(EXPORT, { recursive: true, force: true });
  fs.mkdirSync(EXPORT, { recursive: true });
  writeJson(path.join(EXPORT, "status.json"), status);
  if (fs.existsSync(RUN_LOG)) fs.copyFileSync(RUN_LOG, path.join(EXPORT, "run.log"));
  if (evidenceDirectory && fs.existsSync(evidenceDirectory)) {
    const destination = path.join(
      EXPORT,
      "evidence/memory",
      config.source.benchmark_commit,
      config.measurement.cohort_id,
    );
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.cpSync(evidenceDirectory, destination, { recursive: true });
  }
  fs.rmSync(EXPORT_ARCHIVE, { force: true });
  run("tar", ["--format=ustar", "-czf", EXPORT_ARCHIVE, "-C", EXPORT, "."], { capture: false });
}

// Evidence filenames. The rented lifecycle named two of them after the provider
// it ran on; on an owned host that is a provenance artifact naming a machine it
// never touched, which is exactly the drift this project's evidence exists to
// prevent. The rented lane keeps its names so its committed evidence stays
// readable by the validator that has always read it.
function evidenceNames(config) {
  const dedicated = config.schema === DEDICATED_SCHEMA;
  return {
    gate: dedicated ? "run-gate.json" : "linode-run-gate.json",
    build: dedicated ? "build.json" : "linode-build.json",
  };
}

const LOCK_FILE = path.join(ROOT, "lock");

// Every directory keyed to one run's state. The rented model destroyed the
// machine after each run, so nothing was ever cleared and a second run would
// find a bare clone and a worktree already in place. The two cargo target
// directories stay: they are the warm caches that make a repeat run minutes
// rather than an hour, and every build records its own hashes.
const WIPE_ON_START = Object.freeze([REPOSITORY, SOURCE, EXPORT, EVIDENCE_ROOT]);

function acquireLock() {
  fs.mkdirSync(ROOT, { recursive: true });
  for (;;) {
    try {
      const descriptor = fs.openSync(LOCK_FILE, "wx");
      try {
        fs.writeSync(descriptor, JSON.stringify({ pid: process.pid, started_at: new Date().toISOString() }));
      } finally {
        fs.closeSync(descriptor);
      }
      return;
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
    }
    const holder = loadJson(LOCK_FILE, "lock file");
    if (!Number.isSafeInteger(holder.pid)) fail(`${LOCK_FILE} does not name a pid`);
    let holderAlive = true;
    try {
      process.kill(holder.pid, 0);
    } catch (error) {
      if (error.code === "ESRCH") holderAlive = false;
      else if (error.code !== "EPERM") throw error;
    }
    if (holderAlive) {
      fail(
        `memory runner is already running as pid ${holder.pid} (started ${holder.started_at}); ` +
          `refusing to share ${ROOT} with a second, concurrent run`,
      );
    }
    log(`lock held by dead pid ${holder.pid} (started ${holder.started_at}); taking it over`);
    fs.rmSync(LOCK_FILE, { force: true });
  }
}

function releaseLock() {
  fs.rmSync(LOCK_FILE, { force: true });
}

function resetWorkingState() {
  // WIPE_ON_START is a fixed list of names joined onto ROOT, so a ROOT of "/" or
  // the bare home directory would recursively delete real directories that share
  // those names. ROOT is operator-supplied through an environment variable.
  const resolved = path.resolve(ROOT);
  if (resolved === path.parse(resolved).root || resolved === path.resolve(os.homedir())) {
    fail(
      `RUSTIA_MEMORY_ROOT resolves to ${resolved}; it must be a directory dedicated to this ` +
        `runner, because starting a run erases the working state beneath it`,
    );
  }
  for (const directory of WIPE_ON_START) fs.rmSync(directory, { recursive: true, force: true });
  fs.rmSync(EXPORT_ARCHIVE, { force: true });
}

// The cohort measures inside a dedicated cgroup, so a host that cannot give it
// one cannot run this protocol at all. Checked here, before the clone and the
// build, because discovering it afterwards costs five minutes to say something
// that was knowable in milliseconds -- which is how the first two owned-host
// attempts were spent.
function assertCgroupCapability() {
  const delegated = delegatedCgroupRoot();
  if (delegated.root !== null || delegated.reason === "root" || delegated.reason === "unsupported") return;
  fail(cgroupBlockerMessage(delegated));
}

async function execute(configFile, providerFile, bundle) {
  if (process.platform !== "linux") fail("memory runner requires Linux");
  assertCgroupCapability();
  fs.mkdirSync(ROOT, { recursive: true });
  acquireLock();
  resetWorkingState();
  fs.writeFileSync(RUN_LOG, "", { mode: 0o600 });
  logReady = true;
  const config = validateConfig(loadJson(configFile, "protocol config"));
  const provider = object(loadJson(providerFile, "provider metadata"), "provider metadata");
  const evidenceDirectory = EVIDENCE_ROOT;
  let status = {
    schema: "rustia.bench.cloud-run-status.v1",
    status: "failed",
    detail: "memory runner did not reach its terminal state",
    started_at: new Date().toISOString(),
  };
  try {
    // The owned lifecycle already checked both cores against the kernel's
    // isolated set in validateConfig, which is the stronger statement; the vCPU
    // count only exists on a rented instance.
    if (
      config.schema !== DEDICATED_SCHEMA &&
      (config.host_gate.benchmark_cpu >= provider.vcpus || config.host_gate.supervisor_cpu >= provider.vcpus)
    ) {
      fail("configured memory CPU is absent from the Linode");
    }
    verifySource(config, bundle);
    installRust();
    const binaries = build(config);
    validateRustiaWasmQuota(config);
    // The rented model stopped and permanently masked the host's package timers,
    // which needs root and outlives the run. On an owned machine that is an
    // unrequested change to somebody's box; quiet is asserted by the idle gate.
    fs.rmSync(evidenceDirectory, { recursive: true, force: true });
    const protocolFile = path.join(ROOT, "memory-protocol.json");
    writeJson(protocolFile, memoryProtocol(config));
    const quiet = await waitForQuiet(config);
    const benchmarkBefore = readCpu(config.host_gate.benchmark_cpu);
    const supervisorBefore = readCpu(config.host_gate.supervisor_cpu);
    run("node", [
      path.join(SOURCE, "bench/run-memory-cohort.js"),
      "run",
      "--config", protocolFile,
      "--binary-all", binaries.all,
      "--binary-lua54", binaries.lua54,
      "--output-dir", evidenceDirectory,
    ], {
      cwd: SOURCE,
      capture: false,
      env: { ...process.env, PAGESIZE: runText("getconf", ["PAGESIZE"]) },
    });
    const runGate = {
      benchmark_cpu: cpuDelta(benchmarkBefore, readCpu(config.host_gate.benchmark_cpu)),
      supervisor_cpu: cpuDelta(supervisorBefore, readCpu(config.host_gate.supervisor_cpu)),
    };
    writeJson(path.join(evidenceDirectory, evidenceNames(config).gate), {
      schema: config.schema === DEDICATED_SCHEMA
        ? "rustia.bench.memory-host-gate.v1"
        : "rustia.bench.memory-linode-gate.v1",
      pre_run: quiet,
      run: runGate,
    });
    if (runGate.benchmark_cpu.steal_percent > config.host_gate.maximum_run_steal_percent) {
      fail(`benchmark CPU steal ${runGate.benchmark_cpu.steal_percent}% exceeded the frozen ceiling`);
    }
    const analysis = loadJson(path.join(evidenceDirectory, "analysis.json"), "memory analysis");
    if (!Array.isArray(analysis.rows) || analysis.rows.length !== config.publication.rows) {
      fail(`memory analysis row count is not ${config.publication.rows}`);
    }
    writeJson(path.join(evidenceDirectory, evidenceNames(config).build), {
      all_binary_sha256: binaries.all_sha256,
      lua54_binary_sha256: binaries.lua54_sha256,
      rustc_vv: runText("rustc", ["+1.95.0", "-vV"]),
      cargo_incremental: "0",
      locked: true,
      offline_build_after_fetch: true,
    });
    status = {
      ...status,
      status: "memory_complete",
      detail: "fresh-process cross-engine Linux memory cohort completed",
      finished_at: new Date().toISOString(),
    };
  } catch (error) {
    status = { ...status, status: "failed", detail: error.message, finished_at: new Date().toISOString() };
    log(`failed: ${error.message}`);
  } finally {
    exportRun(status, config, evidenceDirectory);
    releaseLock();
  }
  return status.status === "memory_complete" ? 0 : 2;
}

function selfTest() {
  const before = [0, 0, 0, 10, 0, 0, 0, 0];
  const after = [1, 0, 1, 108, 0, 0, 0, 0];
  assert.deepEqual(cpuDelta(before, after), {
    total_ticks: 100,
    idle_ticks: 98,
    steal_ticks: 0,
    idle_percent: 98,
    steal_percent: 0,
  });
  const config = {
    source: { benchmark_commit: "a".repeat(40) },
    measurement: {
      cohort_id: "fixture",
      samples: 9,
      bootstrap_repetitions: 1000,
      bootstrap_seed: 7,
      growth_thresholds: { absolute_kib_per_epoch: 4, proportional_percent_per_epoch: 0.05 },
      sample_interval_ms: 10,
      sample_timeout_ms: 600000,
      groups: [],
    },
    host_gate: { benchmark_cpu: 3, supervisor_cpu: 2 },
  };
  assert.equal(memoryProtocol(config).host.benchmark_cpu, 3);
  // The load-bearing link: what the freezer writes is what this runner accepts,
  // for both lifecycles, asserted against the freezer's own functions rather
  // than a fixture that resembles their output.
  const freezer = require("./create-rustia-ci-memory-config.js");
  const head = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: path.join(__dirname, ".."),
    encoding: "utf8",
  }).stdout.trim();
  const rented = freezer.createConfig(head);
  assert.equal(validateConfig(rented).schema, "rustia.linode-fixed-benchmark.v2");
  const dedicated = freezer.toDedicated(rented);
  assert.equal(validateConfig(dedicated).schema, DEDICATED_SCHEMA);
  assert.equal(dedicated.provider.host.name, "ci-bench");
  // Two distinct pinned cores, both in the kernel's isolated set.
  assert.notEqual(dedicated.host_gate.benchmark_cpu, dedicated.host_gate.supervisor_cpu);
  const pinnedCpus = parseCpuList(dedicated.provider.host.pinned_cpus);
  assert(pinnedCpus.includes(dedicated.host_gate.benchmark_cpu));
  assert(pinnedCpus.includes(dedicated.host_gate.supervisor_cpu));
  // A core outside the pinned set must be refused.
  assert.throws(
    () =>
      validateConfig({
        ...dedicated,
        host_gate: { ...dedicated.host_gate, supervisor_cpu: 0 },
      }),
    /supervisor_cpu 0 is not one of provider\.host\.pinned_cpus/,
  );

  // The warm cargo target directories must survive a repeat run, or every run
  // is a cold build.
  assert.deepEqual(
    WIPE_ON_START.filter((directory) => directory === TARGET_ALL || directory === TARGET_LUA54),
    [],
    "the warm cargo target directories must survive a repeat run",
  );

  // Evidence names follow the lifecycle, so an owned-host artifact does not
  // claim a provider it never ran on, and the rented one keeps the names its
  // committed evidence already uses.
  assert.deepEqual(evidenceNames({ schema: DEDICATED_SCHEMA }), {
    gate: "run-gate.json", build: "build.json",
  });
  assert.deepEqual(evidenceNames({ schema: "rustia.linode-fixed-benchmark.v2" }), {
    gate: "linode-run-gate.json", build: "linode-build.json",
  });

  assert.deepEqual(parseCpuList("4,5"), [4, 5]);
  assert.deepEqual(parseCpuList("0-1,4"), [0, 1, 4]);
  assert.throws(() => parseCpuList(""), /empty/);
  assert.throws(() => parseCpuList("5-4"), /invalid CPU range/);

  process.stdout.write("linux memory benchmark runner self-test passed\n");
}

async function main() {
  if (process.argv[2] === "self-test" && process.argv.length === 3) return selfTest();
  if (process.argv.length !== 5) {
    process.stderr.write("usage: run-linux-memory-benchmark.js CONFIG PROVIDER BUNDLE\n");
    process.exitCode = 2;
    return;
  }
  process.exitCode = await execute(process.argv[2], process.argv[3], process.argv[4]);
}

main().catch((error) => {
  process.stderr.write(`run-linux-memory-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
