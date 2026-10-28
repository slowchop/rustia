#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { checkDiskSpace } = require("./benchmark-disk-space.js");

// The run's whole working tree. Was hardcoded at /opt/rustia, which only a root
// process on a throwaway instance can write. The owned host runs this as an
// ordinary user, so the location is a parameter with a home-directory default.
const ROOT = process.env.RUSTIA_MATRIX_ROOT ?? path.join(os.homedir(), ".rustia-matrix");
const REPOSITORY = path.join(ROOT, "repo.git");
const SOURCE = path.join(ROOT, "src-matrix");
const TARGET_ALL = path.join(ROOT, "target-matrix-all");
const TARGET_LUA54 = path.join(ROOT, "target-matrix-lua54");
const EXPORT = path.join(ROOT, "export");
const EXPORT_ARCHIVE = path.join(ROOT, "export.tar.gz");
const RUN_LOG = path.join(ROOT, "run.log");
const HISTORY = path.join(ROOT, "matrix-history.jsonl");
const WASM_RESULTS = path.join(ROOT, "wasm-current-results.json");
const DISPATCH_RESULTS = path.join(ROOT, "wasm-dispatch-current-results.json");
const EVIDENCE_ROOT = path.join(ROOT, "matrix-evidence");
const PROTOCOL = "cross_engine_matrix_v4";
const DEDICATED_SCHEMA = "rustia.dedicated-cross-engine-matrix.v1";
const MAX_TIMED_ATTEMPTS_PER_GROUP = 3;
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const NATIVE_TARGET = "linux-x86_64";
const NATIVE_ENGINES = Object.freeze([
  "rustia", "boa", "koto", "luaur", "mlua-luau", "mlua-luau-jit", "rhai", "roto", "ruau", "rquickjs", "wasmi", "wasmtime", "mlua-lua54",
]);
const ALL_ENGINES = Object.freeze(NATIVE_ENGINES.filter((engine) => engine !== "mlua-lua54"));
const WORKLOADS = Object.freeze([
  "dense_tables", "sparse_tables", "closures_upvalues", "generic_iteration", "stdlib_math_table",
]);
const WASM_RUNTIMES = Object.freeze(["node"]);
const WASM_ADAPTERS = Object.freeze(["rustia", "koto", "rhai"]);
let logReady = false;

function fail(message) {
  throw new Error(message);
}

function log(message) {
  const line = `[${new Date().toISOString()}] ${message}`;
  process.stdout.write(`${line}\n`);
  if (logReady) fs.appendFileSync(RUN_LOG, `${line}\n`);
}

function requireObject(value, location) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail(`${location} must be an object`);
  return value;
}

function requireString(value, location) {
  if (typeof value !== "string" || value.length === 0) fail(`${location} must be a non-empty string`);
  return value;
}

function requireInteger(value, location, minimum = 0) {
  if (!Number.isSafeInteger(value) || value < minimum) fail(`${location} must be an integer >= ${minimum}`);
  return value;
}

function requireNumber(value, location, minimum = 0) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < minimum) {
    fail(`${location} must be a finite number >= ${minimum}`);
  }
  return value;
}

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function requireExactKeys(value, expected, location) {
  if (canonicalJson(Object.keys(value).sort()) !== canonicalJson(expected.slice().sort())) {
    fail(`${location} has unexpected keys`);
  }
}

function requireFrozenArray(value, expected, location) {
  if (canonicalJson(value) !== canonicalJson(expected)) fail(`${location} does not match the frozen matrix`);
}

function validateConfig(value) {
  const config = requireObject(value, "config");
  requireExactKeys(config, ["schema", "provider", "source", "measurement", "host_gate", "publication"], "config");
  // Two lifecycles during the move to the owned host: the rented instance this
  // was written for, and the owned machine that replaces it. They differ only in
  // how the host is described and how strict the steal ceilings can be, so the
  // measurement code below is shared rather than forked.
  const dedicated = config.schema === DEDICATED_SCHEMA;
  if (!dedicated && config.schema !== "rustia.linode-fixed-benchmark.v2") {
    fail(`matrix requires ${DEDICATED_SCHEMA} or the Linode v2 lifecycle`);
  }
  const provider = requireObject(config.provider, "provider");
  if (dedicated) {
    requireExactKeys(provider, ["host"], "provider");
    const host = requireObject(provider.host, "provider.host");
    requireExactKeys(
      host,
      ["name", "cpu_model", "logical_cpus", "physical_cores", "pinned_cpus", "max_runtime_minutes"],
      "provider.host",
    );
    requireString(host.name, "provider.host.name");
    requireString(host.cpu_model, "provider.host.cpu_model");
    requireInteger(host.logical_cpus, "provider.host.logical_cpus", 2);
    requireInteger(host.physical_cores, "provider.host.physical_cores", 2);
    requireString(host.pinned_cpus, "provider.host.pinned_cpus");
    requireInteger(host.max_runtime_minutes, "provider.host.max_runtime_minutes", 1);
  } else {
    requireExactKeys(provider, ["region", "image", "size", "max_runtime_minutes"], "provider");
    const size = requireObject(provider.size, "provider.size");
    requireInteger(size.vcpus, "provider.size.vcpus", 2);
    requireInteger(size.memory_mib, "provider.size.memory_mib", 1);
    // Both feed hostLabel below. Unvalidated they would silently produce
    // "linode-undefined-undefined", which is a provenance label that looks like
    // one and identifies nothing.
    requireString(provider.region, "provider.region");
    requireString(size.id, "provider.size.id");
  }
  const source = requireObject(config.source, "source");
  requireExactKeys(source, ["benchmark_commit", "expected_sha256"], "source");
  if (!FULL_COMMIT.test(source.benchmark_commit)) fail("source.benchmark_commit must be a full commit");
  const expectedHashes = requireObject(source.expected_sha256, "source.expected_sha256");
  requireExactKeys(expectedHashes, [
    "bench_manifest", "native_harness", "cargo_lock", "toolchain", "wasm_gate", "wasm_dispatch_gate",
    "wasm_adapter_manifest", "wasm_measure_module", "wasm_dispatch_measure",
    "history_before",
  ], "source.expected_sha256");
  for (const [field, hash] of Object.entries(expectedHashes)) {
    if (!SHA256.test(hash)) fail(`source.expected_sha256.${field} must be a SHA-256`);
  }
  const measurement = requireObject(config.measurement, "measurement");
  requireExactKeys(measurement, [
    "protocol", "native_target", "native_engines", "workloads", "native_target_ms", "native_repetitions",
    "wasm_runtimes", "wasm_adapters", "wasm_target_ms", "wasm_repetitions", "dispatch_target_ms",
    "dispatch_repetitions", "runtime_artifacts",
  ], "measurement");
  if (measurement.protocol !== PROTOCOL || measurement.native_target !== NATIVE_TARGET) {
    fail("measurement protocol or native target is not frozen");
  }
  requireFrozenArray(measurement.native_engines, NATIVE_ENGINES, "measurement.native_engines");
  requireFrozenArray(measurement.workloads, WORKLOADS, "measurement.workloads");
  requireFrozenArray(measurement.wasm_runtimes, WASM_RUNTIMES, "measurement.wasm_runtimes");
  requireFrozenArray(measurement.wasm_adapters, WASM_ADAPTERS, "measurement.wasm_adapters");
  for (const field of ["native_target_ms", "wasm_target_ms", "dispatch_target_ms"]) {
    requireInteger(measurement[field], `measurement.${field}`, 1);
  }
  for (const field of ["native_repetitions", "wasm_repetitions", "dispatch_repetitions"]) {
    if (requireInteger(measurement[field], `measurement.${field}`, 1) !== 5) fail(`measurement.${field} must be 5`);
  }
  const artifacts = requireObject(measurement.runtime_artifacts, "measurement.runtime_artifacts");
  requireExactKeys(artifacts, WASM_RUNTIMES, "measurement.runtime_artifacts");
  for (const runtime of WASM_RUNTIMES) {
    const artifact = requireObject(artifacts[runtime], `runtime artifact ${runtime}`);
    requireExactKeys(artifact, ["version", "url", "sha256", "archive", "binary_relative_path"], `runtime artifact ${runtime}`);
    requireString(artifact.version, `runtime artifact ${runtime}.version`);
    const url = new URL(requireString(artifact.url, `runtime artifact ${runtime}.url`));
    if (url.protocol !== "https:") fail(`runtime artifact ${runtime} must use HTTPS`);
    if (!SHA256.test(artifact.sha256)) fail(`runtime artifact ${runtime}.sha256 must be a SHA-256`);
    if (!["tar", "zip"].includes(artifact.archive)) fail(`runtime artifact ${runtime}.archive is unsupported`);
    const binary = requireString(artifact.binary_relative_path, `runtime artifact ${runtime}.binary_relative_path`);
    if (path.isAbsolute(binary) || binary.split(/[\\/]+/).includes("..")) fail(`runtime artifact ${runtime} binary is unsafe`);
  }
  const gate = requireObject(config.host_gate, "host_gate");
  requireExactKeys(gate, [
    "benchmark_cpu", "cooldown_seconds", "idle_window_seconds", "minimum_idle_percent", "maximum_load_1m",
    "maximum_steal_ticks", "maximum_timed_steal_percent", "maximum_timed_steal_ticks",
    "maximum_timed_attempts_per_group", "wait_timeout_seconds",
  ], "host_gate");
  requireInteger(gate.benchmark_cpu, "host_gate.benchmark_cpu", 0);
  requireInteger(gate.cooldown_seconds, "host_gate.cooldown_seconds", 1);
  requireInteger(gate.idle_window_seconds, "host_gate.idle_window_seconds", 1);
  requireNumber(gate.minimum_idle_percent, "host_gate.minimum_idle_percent", 0);
  requireNumber(gate.maximum_load_1m, "host_gate.maximum_load_1m", 0);
  if (requireInteger(gate.maximum_steal_ticks, "host_gate.maximum_steal_ticks", 0) !== 0) {
    fail("host_gate.maximum_steal_ticks must be zero");
  }
  // On a rented instance steal is a real quantity and the ceilings are
  // tolerances. On bare metal it is structurally zero, so a non-zero reading is
  // a fault rather than something to tolerate; the owned lifecycle therefore
  // requires exactly zero, which is a strengthening rather than a relaxation.
  const timedStealPercent = dedicated ? 0 : 0.1;
  const timedStealTicks = dedicated ? 0 : 1;
  if (
    requireNumber(gate.maximum_timed_steal_percent, "host_gate.maximum_timed_steal_percent", 0) !==
    timedStealPercent
  ) {
    fail(`host_gate.maximum_timed_steal_percent must be ${timedStealPercent}`);
  }
  if (
    requireInteger(gate.maximum_timed_steal_ticks, "host_gate.maximum_timed_steal_ticks", 0) !==
    timedStealTicks
  ) {
    fail(`host_gate.maximum_timed_steal_ticks must be ${timedStealTicks}`);
  }
  const maximumTimedAttempts = requireInteger(
    gate.maximum_timed_attempts_per_group,
    "host_gate.maximum_timed_attempts_per_group",
    1,
  );
  if (maximumTimedAttempts > MAX_TIMED_ATTEMPTS_PER_GROUP) {
    fail(`host_gate.maximum_timed_attempts_per_group must be <= ${MAX_TIMED_ATTEMPTS_PER_GROUP}`);
  }
  if (requireInteger(gate.wait_timeout_seconds, "host_gate.wait_timeout_seconds", 1) !== 300) {
    fail("host_gate.wait_timeout_seconds must be 300");
  }
  const publication = requireObject(config.publication, "publication");
  requireExactKeys(publication, ["mode", "native_rows", "wasm_rows", "dispatch_runtimes"], "publication");
  if (
    publication.mode !== "matrix" || publication.native_rows !== 65 ||
    publication.wasm_rows !== 15 || publication.dispatch_runtimes !== 1
  ) {
    fail("publication matrix dimensions are not frozen at 65/15/1");
  }
  return config;
}

// max_runtime_minutes was a billing ceiling on a rented instance -- the
// coordinator destroyed the machine when it expired, so nothing here enforced
// it. On an owned host nothing destroys anything, so the bound has to mean
// something else or mean nothing: it is a hang detector, checked at the phase
// boundaries, so a wedged build fails in seconds rather than after another hour.
function checkRuntimeBudget(config, startedAtMs, phase) {
  const limit = config.schema === DEDICATED_SCHEMA
    ? config.provider.host.max_runtime_minutes
    : config.provider.max_runtime_minutes;
  const elapsedMinutes = (Date.now() - startedAtMs) / 60_000;
  if (elapsedMinutes > limit) {
    fail(
      `matrix run exceeded provider max_runtime_minutes (${limit}) after ` +
        `${elapsedMinutes.toFixed(1)} minutes, ${phase}`,
    );
  }
}

function loadJson(file, location) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot load ${location}: ${error.message}`);
  }
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  return sha256(fs.readFileSync(file));
}

function commandText(command, args) {
  return [command, ...args].map((value) => (/^[A-Za-z0-9_./:=+,@%-]+$/.test(value) ? value : JSON.stringify(value))).join(" ");
}

function run(command, args, options = {}) {
  const { cwd = ROOT, env = process.env, capture = true, allowFailure = false } = options;
  log(`+ ${commandText(command, args)}`);
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: capture ? "utf8" : undefined,
    maxBuffer: 128 * 1024 * 1024,
    stdio: capture ? ["ignore", "pipe", "pipe"] : "inherit",
  });
  if (result.error) fail(`cannot run ${command}: ${result.error.message}`);
  const status = result.status === null ? 1 : result.status;
  if (status !== 0 && !allowFailure) {
    const detail = capture ? String(result.stderr || "").trim() : "";
    fail(`${commandText(command, args)} failed with status ${status}${detail ? `: ${detail}` : ""}`);
  }
  return { status, stdout: capture ? String(result.stdout || "") : "", stderr: capture ? String(result.stderr || "") : "" };
}

function runText(command, args, options = {}) {
  return run(command, args, options).stdout.trim();
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

function benchmarkEnvironment(targetDir) {
  return {
    ...process.env,
    CARGO_INCREMENTAL: "0",
    CARGO_TARGET_DIR: targetDir,
    RUSTC_WRAPPER: "",
  };
}

function verifySource(config, bundle) {
  run("git", ["clone", "--bare", bundle, REPOSITORY], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "bundle", "verify", bundle], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", SOURCE, config.source.benchmark_commit], { capture: false });
  assert.equal(runText("git", ["-C", SOURCE, "rev-parse", "HEAD"]), config.source.benchmark_commit);
  run("git", ["-C", SOURCE, "config", "core.abbrev", "40"]);
  const files = {
    bench_manifest: "bench/Cargo.toml",
    native_harness: "bench/src/main.rs",
    cargo_lock: "Cargo.lock",
    toolchain: "rust-toolchain.toml",
    wasm_gate: "tools/run-wasm-gate.js",
    wasm_dispatch_gate: "tools/run-wasm-dispatch-gate.js",
    wasm_adapter_manifest: "bench/wasm/adapter-runner/Cargo.toml",
    wasm_measure_module: "bench/wasm/measure-rustia-wasm.mjs",
    wasm_dispatch_measure: "bench/wasm/measure-dispatch-wasm.js",
    history_before: "bench/history/runs.jsonl",
  };
  for (const [field, relative] of Object.entries(files)) {
    const actual = sha256File(path.join(SOURCE, relative));
    if (actual !== config.source.expected_sha256[field]) {
      fail(`${relative} SHA-256 is ${actual}, expected ${config.source.expected_sha256[field]}`);
    }
  }
}

// Resolves rustup through PATH rather than at /root/.cargo/bin, and only
// installs one when the machine has none. An owned host arrives with a toolchain
// already provisioned; a throwaway instance does not.
function installRust() {
  if (spawnSync("rustup", ["--version"], { stdio: "ignore" }).status !== 0) {
    const installer = path.join(ROOT, "rustup-init.sh");
    run("curl", ["--proto", "=https", "--tlsv1.2", "--fail", "--silent", "--show-error", "https://sh.rustup.rs", "-o", installer]);
    run("sh", [installer, "-y", "--profile", "minimal", "--default-toolchain", "none"], { capture: false });
    process.env.PATH = `${path.join(os.homedir(), ".cargo/bin")}:${process.env.PATH ?? ""}`;
  }
  run("rustup", ["toolchain", "install", "1.95.0", "--profile", "minimal"], { capture: false });
  run("rustup", ["target", "add", "--toolchain", "1.95.0", "wasm32-unknown-unknown"], { capture: false });
}

// The pinned core must be one the kernel actually holds off the scheduler. On a
// rented instance the only available check was "the vCPU exists"; on the owned
// host the config names the isolated set, so the stronger statement is
// available and is the one that matters -- a pinned core the scheduler still
// uses is the failure this arrangement exists to prevent, and it does not show
// up in the timings.
function validateBenchmarkCpu(config, provider) {
  if (config.schema !== DEDICATED_SCHEMA) {
    if (config.host_gate.benchmark_cpu >= provider.vcpus) {
      fail("configured benchmark CPU is absent from the Linode");
    }
    return;
  }
  const pinned = parseCpuList(config.provider.host.pinned_cpus);
  if (!pinned.includes(config.host_gate.benchmark_cpu)) {
    fail(
      `host_gate.benchmark_cpu ${config.host_gate.benchmark_cpu} is not one of ` +
        `provider.host.pinned_cpus (${pinned.join(",")})`,
    );
  }
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

function safeArtifactPath(root, relative, runtime) {
  const resolved = path.resolve(root, relative);
  const prefix = `${path.resolve(root)}${path.sep}`;
  if (!resolved.startsWith(prefix)) fail(`${runtime} binary path escapes its extraction root`);
  return resolved;
}

function installRuntime(runtime, artifact) {
  const directory = path.join(ROOT, "runtimes", runtime);
  const archive = path.join(ROOT, "runtimes", `${runtime}.archive`);
  fs.mkdirSync(directory, { recursive: true });
  run("curl", ["--proto", "=https", "--tlsv1.2", "--fail", "--location", "--silent", "--show-error", artifact.url, "-o", archive]);
  const actual = sha256File(archive);
  if (actual !== artifact.sha256) fail(`${runtime} archive SHA-256 is ${actual}, expected ${artifact.sha256}`);
  if (artifact.archive === "zip") run("unzip", ["-q", archive, "-d", directory]);
  else run("tar", ["-xf", archive, "-C", directory]);
  const binary = safeArtifactPath(directory, artifact.binary_relative_path, runtime);
  const stat = fs.lstatSync(binary);
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`${runtime} extracted binary is not a regular file`);
  fs.chmodSync(binary, stat.mode | 0o500);
  // Into a directory this run owns, not /usr/local/bin, which needs root. The
  // rented model had root and the whole machine; an owned host has neither
  // going spare. PATH is prepended rather than appended so the pinned runtime
  // wins over any the machine already has -- which is the entire point of
  // pinning it by URL and hash.
  const command = runtime === "spidermonkey" ? "js" : runtime;
  const binDirectory = path.join(ROOT, "bin");
  fs.mkdirSync(binDirectory, { recursive: true });
  const link = path.join(binDirectory, command);
  fs.rmSync(link, { force: true });
  fs.symlinkSync(binary, link);
  if (!`${process.env.PATH ?? ""}`.split(path.delimiter).includes(binDirectory)) {
    process.env.PATH = `${binDirectory}${path.delimiter}${process.env.PATH ?? ""}`;
  }
  return { configured_version: artifact.version, archive_sha256: actual, command: link };
}

function installRuntimes(config) {
  const installed = {};
  for (const runtime of WASM_RUNTIMES) installed[runtime] = installRuntime(runtime, config.measurement.runtime_artifacts[runtime]);
  installed.node.reported_version = runText("node", ["--version"]);
  return installed;
}

function build(config) {
  // Through PATH, like rustup above: /root/.cargo/bin belongs to the rented
  // model, where the runner was the only user on the machine.
  const cargo = "cargo";
  const manifest = path.join(SOURCE, "Cargo.toml");
  const benchManifest = path.join(SOURCE, "bench/Cargo.toml");
  const wasmManifest = path.join(SOURCE, "bench/wasm/adapter-runner/Cargo.toml");
  const wasmEnvironment = { ...process.env, CARGO_INCREMENTAL: "0", RUSTC_WRAPPER: "" };
  delete wasmEnvironment.CARGO_TARGET_DIR;
  run(cargo, ["+1.95.0", "fetch", "--locked", "--manifest-path", manifest], { env: benchmarkEnvironment(TARGET_ALL), capture: false });
  run(cargo, ["+1.95.0", "fetch", "--locked", "--manifest-path", wasmManifest], { capture: false });
  run(cargo, [
    "+1.95.0", "build", "--locked", "--offline", "--manifest-path", benchManifest,
    "--bin", "script-bench", "--release", "--no-default-features", "--features", "all-engines",
  ], { env: benchmarkEnvironment(TARGET_ALL), capture: false });
  run(cargo, [
    "+1.95.0", "build", "--locked", "--offline", "--manifest-path", benchManifest,
    "--bin", "script-bench", "--release", "--no-default-features", "--features", "mlua-lua54",
  ], { env: benchmarkEnvironment(TARGET_LUA54), capture: false });
  run(cargo, [
    "+1.95.0", "build", "--locked", "--offline", "--manifest-path", wasmManifest,
    "--target", "wasm32-unknown-unknown", "--release",
  ], { cwd: SOURCE, env: wasmEnvironment, capture: false });
  const allBinary = path.join(TARGET_ALL, "release/script-bench");
  const lua54Binary = path.join(TARGET_LUA54, "release/script-bench");
  fs.accessSync(allBinary, fs.constants.X_OK);
  fs.accessSync(lua54Binary, fs.constants.X_OK);
  return {
    cargo,
    allBinary,
    lua54Binary,
    all_binary_sha256: sha256File(allBinary),
    lua54_binary_sha256: sha256File(lua54Binary),
    wasm_sha256: sha256File(path.join(SOURCE, "bench/wasm/adapter-runner/target/wasm32-unknown-unknown/release/script_bench_wasm_adapter.wasm")),
  };
}

function readCpu(cpu) {
  const line = fs.readFileSync("/proc/stat", "utf8").split("\n").find((entry) => entry.startsWith(`cpu${cpu} `));
  if (!line) fail(`cannot read benchmark CPU ${cpu} from /proc/stat`);
  const fields = line.trim().split(/\s+/).slice(1).map(Number);
  if (fields.length < 8 || fields.some((value) => !Number.isFinite(value))) fail("malformed /proc/stat CPU row");
  return { fields, steal: fields[7] };
}

function cpuDelta(before, after) {
  const deltas = before.fields.map((value, index) => after.fields[index] - value);
  if (deltas.some((value) => value < 0)) fail("CPU counters moved backwards");
  const total = deltas.reduce((sum, value) => sum + value, 0);
  const idle = deltas[3] + deltas[4];
  const steal = deltas[7];
  return {
    total_ticks: total,
    idle_ticks: idle,
    steal_ticks: steal,
    idle_percent: total === 0 ? 0 : (idle / total) * 100,
    steal_percent: total === 0 ? 0 : (steal / total) * 100,
  };
}

function timedStealGatePasses(gate, delta) {
  return (
    delta.steal_ticks <= gate.maximum_timed_steal_ticks ||
    delta.steal_ticks * 100 <= gate.maximum_timed_steal_percent * delta.total_ticks
  );
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function appendGateRecord(file, record) {
  const descriptor = fs.openSync(file, "a");
  try {
    fs.appendFileSync(descriptor, `${JSON.stringify(record)}\n`);
    fs.fsyncSync(descriptor);
  } finally {
    fs.closeSync(descriptor);
  }
}

function snapshotFiles(files) {
  const seen = new Set();
  return files.map((file) => {
    if (seen.has(file)) fail(`duplicate rollback file ${file}`);
    seen.add(file);
    return fs.existsSync(file)
      ? { file, existed: true, bytes: fs.readFileSync(file) }
      : { file, existed: false, bytes: null };
  });
}

function restoreFileSnapshots(snapshots) {
  for (const snapshot of snapshots) {
    if (snapshot.existed) {
      fs.writeFileSync(snapshot.file, snapshot.bytes);
    } else {
      fs.rmSync(snapshot.file, { force: true });
    }
  }
}

async function waitForQuiet(config, label, gateFile, timedAttempt, deadline) {
  const gate = config.host_gate;
  let quietAttempt = 0;
  while (Date.now() < deadline) {
    quietAttempt += 1;
    const before = readCpu(gate.benchmark_cpu);
    await sleep(gate.idle_window_seconds * 1000);
    const after = readCpu(gate.benchmark_cpu);
    const delta = cpuDelta(before, after);
    const load1m = Number(fs.readFileSync("/proc/loadavg", "utf8").trim().split(/\s+/)[0]);
    const accepted =
      delta.idle_percent >= gate.minimum_idle_percent &&
      delta.steal_ticks <= gate.maximum_steal_ticks &&
      load1m <= gate.maximum_load_1m;
    appendGateRecord(gateFile, {
      phase: "pre", label, timed_attempt: timedAttempt, quiet_attempt: quietAttempt,
      captured_at: new Date().toISOString(), load_1m: load1m, ...delta, accepted,
    });
    if (accepted) return;
    await sleep(gate.cooldown_seconds * 1000);
  }
  fail(`${label} timed attempt ${timedAttempt} did not obtain a quiet host window before timeout`);
}

async function timedGroup(config, label, command, args, gateFile, options = {}, hooks = {}) {
  const rollbackFiles = options.rollbackFiles ?? [];
  const runOptions = { ...options };
  delete runOptions.rollbackFiles;
  const waitForQuietImpl = hooks.waitForQuiet ?? waitForQuiet;
  const readCpuImpl = hooks.readCpu ?? readCpu;
  const runImpl = hooks.run ?? run;
  const sleepImpl = hooks.sleep ?? sleep;
  const now = hooks.now ?? (() => new Date().toISOString());
  const logImpl = hooks.log ?? log;
  const appendGate = hooks.appendGate ?? appendGateRecord;
  const maximumAttempts = config.host_gate.maximum_timed_attempts_per_group;
  const quietDeadline = Date.now() + config.host_gate.wait_timeout_seconds * 1000;

  for (let attempt = 1; attempt <= maximumAttempts; attempt += 1) {
    await waitForQuietImpl(config, label, gateFile, attempt, quietDeadline);
    const snapshots = snapshotFiles(rollbackFiles);
    logImpl(`${label} timed attempt ${attempt}/${maximumAttempts} starting`);
    const before = readCpuImpl(config.host_gate.benchmark_cpu);
    const started = now();
    try {
      runImpl(
        "taskset",
        ["-c", String(config.host_gate.benchmark_cpu), command, ...args],
        { ...runOptions, capture: false },
      );
    } catch (error) {
      restoreFileSnapshots(snapshots);
      logImpl(`${label} timed attempt ${attempt}/${maximumAttempts} failed; restored attempt outputs`);
      throw error;
    }
    const finished = now();
    let delta;
    try {
      delta = cpuDelta(before, readCpuImpl(config.host_gate.benchmark_cpu));
    } catch (error) {
      restoreFileSnapshots(snapshots);
      logImpl(`${label} timed attempt ${attempt}/${maximumAttempts} could not be gated; restored attempt outputs`);
      throw error;
    }
    const accepted = timedStealGatePasses(config.host_gate, delta);
    try {
      appendGate(gateFile, {
        phase: "timed", label, timed_attempt: attempt,
        started_at: started, finished_at: finished, ...delta, accepted,
      });
    } catch (error) {
      restoreFileSnapshots(snapshots);
      try {
        logImpl(`${label} timed attempt ${attempt}/${maximumAttempts} gate write failed; restored attempt outputs`);
      } catch (_) {
        // Preserve the durable gate-write failure after best-effort logging.
      }
      throw error;
    }
    if (accepted) {
      logImpl(`${label} timed attempt ${attempt}/${maximumAttempts} accepted`);
      await sleepImpl(config.host_gate.cooldown_seconds * 1000);
      return;
    }

    restoreFileSnapshots(snapshots);
    logImpl(
      `${label} timed attempt ${attempt}/${maximumAttempts} rejected at ` +
      `${delta.steal_percent.toFixed(6)}% / ${delta.steal_ticks} steal tick(s); restored attempt outputs`,
    );
    if (attempt < maximumAttempts) {
      await sleepImpl(config.host_gate.cooldown_seconds * 1000);
    }
  }

  fail(
    `${label} exhausted ${maximumAttempts} timed attempt(s) without meeting the ` +
    `${config.host_gate.maximum_timed_steal_percent}% / ` +
    `${config.host_gate.maximum_timed_steal_ticks}-tick steal gate`,
  );
}

// The machine a row was measured on, as a label the row carries into published
// data. The owned host has a name; a rented instance does not -- it is created
// and destroyed per run -- so the honest label there is the class of machine the
// frozen config bought, which is what "same environment" actually means for a
// rental. Either way the two lifecycles produce different strings, which is the
// whole point: a table can no longer hold rows from both without saying so.
//
// Deliberately duplicated in each on-host runner rather than shared. Only the
// single runner file is copied to the benchmark host (see shipAndRun in
// run-dedicated-benchmark.js), so a require() of a sibling module would resolve
// on a developer machine and fail on the host.
// A label, never an address: no dots and no colons, so it can be neither a
// fully qualified name nor an IPv4 or IPv6 address, whatever an operator types.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

function hostLabel(config) {
  const label = config.schema === DEDICATED_SCHEMA
    ? config.provider.host.name
    : `linode-${config.provider.region}-${config.provider.size.id}`;
  if (!HOST_LABEL.test(label)) {
    fail(`host label ${JSON.stringify(label)} is not a label: it may contain only [A-Za-z0-9_-], never an address`);
  }
  return label;
}

function nativeArgs(config, engines, samplesFile) {
  return [
    "--engines", engines.join(","),
    "--workloads", WORKLOADS.join(","),
    "--target-ms", String(config.measurement.native_target_ms),
    "--reps", String(config.measurement.native_repetitions),
    "--history", HISTORY,
    "--samples-json", samplesFile,
    "--commit", config.source.benchmark_commit,
    "--host", hostLabel(config),
    "--no-results",
    "--no-latest-json",
  ];
}

async function measure(config, buildInfo, evidenceDirectory) {
  // The benchmark harness appends accepted measurements. Seed its remote ledger
  // from the checked-out, integrity-verified publication instead of clearing it.
  fs.copyFileSync(path.join(SOURCE, "bench", "history", "runs.jsonl"), HISTORY);
  fs.rmSync(WASM_RESULTS, { force: true });
  fs.rmSync(DISPATCH_RESULTS, { force: true });
  const gateFile = path.join(evidenceDirectory, "gate.jsonl");
  fs.writeFileSync(gateFile, "");
  const allSamples = path.join(evidenceDirectory, "native-all-samples.json");
  const luaSamples = path.join(evidenceDirectory, "native-lua54-samples.json");
  await timedGroup(
    config,
    "native-all-engines",
    buildInfo.allBinary,
    nativeArgs(config, ALL_ENGINES, allSamples),
    gateFile,
    { cwd: SOURCE, rollbackFiles: [HISTORY, allSamples] },
  );
  await timedGroup(
    config,
    "native-lua54",
    buildInfo.lua54Binary,
    nativeArgs(config, ["mlua-lua54"], luaSamples),
    gateFile,
    { cwd: SOURCE, rollbackFiles: [HISTORY, luaSamples] },
  );
  await timedGroup(config, "wasm-adapter-matrix", "node", [
    "tools/run-wasm-gate.js", "--no-build", "--runtime", WASM_RUNTIMES.join(","),
    "--adapters", WASM_ADAPTERS.join(","), "--target-ms", String(config.measurement.wasm_target_ms),
    "--reps", String(config.measurement.wasm_repetitions), "--output", WASM_RESULTS,
  ], gateFile, { cwd: SOURCE, rollbackFiles: [WASM_RESULTS] });
  await timedGroup(config, "wasm-dispatch-matrix", "node", [
    "tools/run-wasm-dispatch-gate.js", "--runtime", WASM_RUNTIMES.join(","),
    "--target-ms", String(config.measurement.dispatch_target_ms),
    "--reps", String(config.measurement.dispatch_repetitions), "--output", DISPATCH_RESULTS,
  ], gateFile, { cwd: SOURCE, rollbackFiles: [DISPATCH_RESULTS] });
  return { gateFile, allSamples, luaSamples };
}

function collectHost(config, buildInfo) {
  const cpuInfo = fs.readFileSync("/proc/cpuinfo", "utf8");
  const model = cpuInfo.match(/^model name\s*:\s*(.+)$/m);
  const dedicated = config.schema === DEDICATED_SCHEMA;
  return {
    schema: "rustia.bench.matrix-host.v1",
    captured_at: new Date().toISOString(),
    platform: process.platform,
    arch: runText("uname", ["-m"]),
    // `uname -a` includes the node name. On a throwaway instance that is a
    // random string that stops existing with the machine; on an owned host it
    // is the real name of a real machine, and this file is committed to a
    // public repository. The kernel, release and architecture are what the
    // evidence is actually for, so the owned lane records those and leaves the
    // node name out rather than publishing it and hoping nobody minds.
    uname: dedicated ? runText("uname", ["-srvm"]) : runText("uname", ["-a"]),
    os_release: fs.readFileSync("/etc/os-release", "utf8"),
    virtualization: runText("systemd-detect-virt", [], { allowFailure: true }),
    cpu_model: model ? model[1].trim() : "unknown",
    cpuinfo_sha256: sha256(Buffer.from(cpuInfo)),
    benchmark_cpu: config.host_gate.benchmark_cpu,
    // Resolved through PATH; /root/.cargo/bin only exists on the rented model.
    rustc_vv: runText("rustc", ["+1.95.0", "-vV"]),
    cargo_version: runText(buildInfo.cargo, ["+1.95.0", "--version"]),
    taskset_version: runText("taskset", ["--version"]).split("\n")[0],
  };
}

function evidenceHashes(directory) {
  const hashes = {};
  for (const file of fs.readdirSync(directory).sort()) {
    if (file !== "manifest.json") hashes[file] = sha256File(path.join(directory, file));
  }
  return hashes;
}

function writeManifest(config, configFile, providerFile, evidenceDirectory, buildInfo) {
  const manifest = {
    schema: "rustia.bench.cross-engine-matrix-evidence.v1",
    created_at: new Date().toISOString(),
    benchmark_commit: config.source.benchmark_commit,
    source_hashes: config.source.expected_sha256,
    protocol_sha256: sha256File(configFile),
    provider_sha256: sha256File(providerFile),
    host_sha256: sha256File(path.join(evidenceDirectory, "host.json")),
    publication_sha256: {
      history: sha256File(HISTORY),
      wasm: sha256File(WASM_RESULTS),
      dispatch: sha256File(DISPATCH_RESULTS),
    },
    result_shape: {
      native_rows: config.publication.native_rows,
      wasm_rows: config.publication.wasm_rows,
      dispatch_runtimes: config.publication.dispatch_runtimes,
    },
    build: {
      all_engines_binary_sha256: buildInfo.all_binary_sha256,
      lua54_binary_sha256: buildInfo.lua54_binary_sha256,
      wasm_artifact_sha256: buildInfo.wasm_sha256,
      cargo_incremental: "0",
      rustc_wrapper: "unset",
      locked: true,
      offline_build_after_fetch: true,
    },
    evidence_sha256: evidenceHashes(evidenceDirectory),
  };
  writeJson(path.join(evidenceDirectory, "manifest.json"), manifest);
}

function exportRun(status, config, evidenceDirectory) {
  fs.rmSync(EXPORT, { recursive: true, force: true });
  fs.mkdirSync(EXPORT, { recursive: true });
  writeJson(path.join(EXPORT, "status.json"), status);
  if (fs.existsSync(RUN_LOG)) fs.copyFileSync(RUN_LOG, path.join(EXPORT, "run.log"));
  if (evidenceDirectory && fs.existsSync(evidenceDirectory)) {
    const destination = path.join(EXPORT, "evidence/matrix", config.source.benchmark_commit);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.cpSync(evidenceDirectory, destination, { recursive: true });
  }
  if (status.status === "matrix_complete") {
    const publications = [
      [HISTORY, "publication/bench/history/runs.jsonl"],
      [WASM_RESULTS, "publication/bench/wasm/current-results.json"],
      [DISPATCH_RESULTS, "publication/bench/wasm/dispatch-current-results.json"],
    ];
    for (const [source, relative] of publications) {
      const destination = path.join(EXPORT, relative);
      fs.mkdirSync(path.dirname(destination), { recursive: true });
      fs.copyFileSync(source, destination);
    }
  }
  fs.rmSync(EXPORT_ARCHIVE, { force: true });
  run("tar", ["--format=ustar", "-czf", EXPORT_ARCHIVE, "-C", EXPORT, "."], { capture: false });
}

const LOCK_FILE = path.join(ROOT, "lock");

// Every directory keyed to one run's state. The rented-instance model destroyed
// the machine after each run, so nothing here was ever cleared; on an owned host
// a second run would find a bare clone and a worktree already in place and fail.
// The two cargo target directories are deliberately absent: they are the warm
// caches that make a repeat run minutes rather than an hour, and every build
// records its own hashes, so a poisoned cache would be visible rather than
// silent.
const WIPE_ON_START = Object.freeze([
  REPOSITORY,
  SOURCE,
  EXPORT,
  EVIDENCE_ROOT,
  // The extracted runtimes and the links into them. Re-downloading costs a
  // couple of seconds against a hash that is checked anyway, and a half-extracted
  // archive left by a failed run is not something to inherit.
  path.join(ROOT, "runtimes"),
  path.join(ROOT, "bin"),
]);

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
        `matrix runner is already running as pid ${holder.pid} (started ${holder.started_at}); ` +
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
      `RUSTIA_MATRIX_ROOT resolves to ${resolved}; it must be a directory dedicated to this ` +
        `runner, because starting a run erases the working state beneath it`,
    );
  }
  for (const directory of WIPE_ON_START) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
  fs.rmSync(EXPORT_ARCHIVE, { force: true });
}

async function execute(configFile, providerFile, bundle) {
  if (process.platform !== "linux") fail("matrix runner must execute on Linux");
  fs.mkdirSync(ROOT, { recursive: true });
  acquireLock();
  resetWorkingState();
  fs.writeFileSync(RUN_LOG, "", { mode: 0o600 });
  logReady = true;
  const config = validateConfig(loadJson(configFile, "protocol config"));
  const provider = requireObject(loadJson(providerFile, "provider metadata"), "provider metadata");
  const evidenceDirectory = EVIDENCE_ROOT;
  fs.rmSync(evidenceDirectory, { recursive: true, force: true });
  fs.mkdirSync(evidenceDirectory, { recursive: true });
  let status = {
    schema: "rustia.bench.cloud-run-status.v1",
    status: "failed",
    detail: "matrix runner did not reach its terminal state",
    started_at: new Date().toISOString(),
  };
  try {
    validateBenchmarkCpu(config, provider);
    const startedAtMs = Date.now();
    log("verifying frozen source and preparing pinned toolchains");
    verifySource(config, bundle);
    installRust();
    const runtimeVersions = installRuntimes(config);
    writeJson(path.join(evidenceDirectory, "runtime-versions.json"), runtimeVersions);
    checkRuntimeBudget(config, startedAtMs, "before building the adapters");
    log("building all native adapters and the wasm adapter offline");
    const buildInfo = build(config);
    checkDiskSpace(ROOT, path.join(evidenceDirectory, "disk-space.json"));
    checkRuntimeBudget(config, startedAtMs, "before measuring");
    // The rented-instance model stopped and permanently masked the package
    // timers, which needs root and outlives the run. On an owned machine that is
    // an unrequested change to somebody's box; quiet is a property of the host
    // and is asserted by the pre-window idle gate below, not imposed here.
    fs.copyFileSync(configFile, path.join(evidenceDirectory, "protocol.json"));
    fs.copyFileSync(providerFile, path.join(evidenceDirectory, "provider.json"));
    writeJson(path.join(evidenceDirectory, "host.json"), collectHost(config, buildInfo));
    log("running fresh Linux native and wasm matrices");
    await measure(config, buildInfo, evidenceDirectory);
    writeManifest(config, configFile, providerFile, evidenceDirectory, buildInfo);
    status = {
      ...status,
      status: "matrix_complete",
      detail: "fresh Linux native and wasm benchmark matrix completed",
      finished_at: new Date().toISOString(),
    };
  } catch (error) {
    status = { ...status, status: "failed", detail: error.message, finished_at: new Date().toISOString() };
    log(`failed: ${error.message}`);
  } finally {
    exportRun(status, config, evidenceDirectory);
    releaseLock();
  }
  return status.status === "matrix_complete" ? 0 : 2;
}

// Turns the committed rented-instance config into the owned-host one, which is
// exactly the transformation the freezer performs. Deriving the fixture from a
// real config rather than writing one by hand means the test cannot pass against
// a shape the repository does not actually contain.
function dedicatedFixture(linodeConfig) {
  return {
    ...linodeConfig,
    schema: DEDICATED_SCHEMA,
    provider: {
      host: {
        name: "ci-bench",
        cpu_model: "AMD Ryzen 5 3600X",
        logical_cpus: 6,
        physical_cores: 6,
        pinned_cpus: "4",
        max_runtime_minutes: 180,
      },
    },
    host_gate: {
      ...linodeConfig.host_gate,
      benchmark_cpu: 4,
      maximum_timed_steal_percent: 0,
      maximum_timed_steal_ticks: 0,
    },
  };
}

async function selfTest() {
  assert.deepEqual(ALL_ENGINES, NATIVE_ENGINES.slice(0, -1));
  assert.equal(NATIVE_ENGINES.length * WORKLOADS.length, 65);
  assert.equal(WASM_RUNTIMES.length * WASM_ADAPTERS.length * WORKLOADS.length, 15);
  assert.deepEqual(cpuDelta(
    { fields: [10, 0, 5, 100, 2, 1, 1, 0] },
    { fields: [11, 0, 6, 198, 2, 1, 1, 0] },
  ), { total_ticks: 100, idle_ticks: 98, steal_ticks: 0, idle_percent: 98, steal_percent: 0 });
  const timedGate = { maximum_timed_steal_percent: 0.1, maximum_timed_steal_ticks: 1 };
  assert.equal(timedStealGatePasses(timedGate, { total_ticks: 946, steal_ticks: 1 }), true);
  assert.equal(timedStealGatePasses(timedGate, { total_ticks: 946, steal_ticks: 2 }), false);
  assert.equal(timedStealGatePasses(timedGate, { total_ticks: 10_000, steal_ticks: 2 }), true);
  assert.throws(() => safeArtifactPath("/tmp/a", "../b", "fixture"));

  assert.deepEqual(parseCpuList("4"), [4]);
  assert.deepEqual(parseCpuList("4,5"), [4, 5]);
  assert.deepEqual(parseCpuList("0-1,4"), [0, 1, 4]);
  assert.throws(() => parseCpuList(""), /empty/);
  assert.throws(() => parseCpuList("5-4"), /invalid CPU range/);
  assert.throws(() => parseCpuList("x"), /invalid CPU list entry/);

  // Both lifecycles, against a config the repository actually holds. The rented
  // one must keep validating unchanged -- adding the owned lifecycle is not
  // allowed to quietly alter the one still in service.
  const linodeConfig = loadJson(
    path.join(__dirname, "../bench/cloud/linode-cross-engine-matrix-8f0e7f474f2b.json"),
    "committed matrix config",
  );
  assert.equal(validateConfig(linodeConfig).schema, "rustia.linode-fixed-benchmark.v2");
  const dedicated = dedicatedFixture(linodeConfig);
  assert.equal(validateConfig(dedicated).schema, DEDICATED_SCHEMA);

  // The load-bearing link: what the freezer writes is what this runner accepts.
  // Asserted against the freezer's own functions rather than a fixture that
  // merely resembles its output, so the two cannot drift apart silently.
  const freezer = require("./create-linode-matrix-config.js");
  // HEAD, because createConfig re-pins every source hash from Git and a
  // synthetic commit has no tree to read.
  const frozenCommit = spawnSync("git", ["rev-parse", "HEAD"], {
    cwd: path.join(__dirname, ".."),
    encoding: "utf8",
  }).stdout.trim();
  const frozenLinode = freezer.createConfig(frozenCommit);
  assert.equal(validateConfig(frozenLinode).schema, "rustia.linode-fixed-benchmark.v2");
  const frozenDedicated = freezer.toDedicated(frozenLinode);
  assert.equal(validateConfig(frozenDedicated).schema, DEDICATED_SCHEMA);
  assert.equal(frozenDedicated.provider.host.name, "ci-bench");
  validateBenchmarkCpu(frozenDedicated, {});

  // Every measured row carries the machine it was measured on, and the two
  // lifecycles label differently -- a rented instance has no name, only a class.
  // Rows from the two can therefore never merge into one table unnoticed, which
  // is the failure this label exists to make impossible.
  assert.equal(hostLabel(frozenDedicated), "ci-bench");
  assert.equal(hostLabel(linodeConfig), "linode-ap-southeast-g6-dedicated-4");
  assert.notEqual(hostLabel(linodeConfig), hostLabel(frozenDedicated));
  // A label is never an address: a name with a dot or a colon is refused
  // before it can be stamped on a row.
  for (const name of ["bench.example.com", "10.0.0.5", "fe80::1"]) {
    assert.throws(
      () => hostLabel({ ...frozenDedicated, provider: { ...frozenDedicated.provider, host: { ...frozenDedicated.provider.host, name } } }),
      /not a label/,
    );
  }
  for (const config of [frozenDedicated, linodeConfig]) {
    const args = nativeArgs(config, ALL_ENGINES, "samples.json");
    assert.equal(args[args.indexOf("--host") + 1], hostLabel(config));
  }
  // A rented config that cannot name its class is refused before it can label
  // a row "linode-undefined-undefined".
  assert.throws(
    () => validateConfig({ ...linodeConfig, provider: { ...linodeConfig.provider, region: undefined } }),
    /provider\.region/,
  );
  assert.throws(
    () => validateConfig({
      ...linodeConfig,
      provider: { ...linodeConfig.provider, size: { ...linodeConfig.provider.size, id: undefined } },
    }),
    /provider\.size\.id/,
  );

  // Steal is structurally zero on bare metal, so the owned lifecycle requires
  // exactly zero and must refuse the rented one's tolerances.
  assert.throws(
    () => validateConfig({ ...dedicated, host_gate: { ...dedicated.host_gate, maximum_timed_steal_percent: 0.1 } }),
    /maximum_timed_steal_percent must be 0/,
  );
  assert.throws(
    () => validateConfig({ ...dedicated, host_gate: { ...dedicated.host_gate, maximum_timed_steal_ticks: 1 } }),
    /maximum_timed_steal_ticks must be 0/,
  );
  // A rented config may not smuggle in an owned host block, or the reverse.
  assert.throws(() => validateConfig({ ...dedicated, schema: "rustia.linode-fixed-benchmark.v2" }), /provider/);
  assert.throws(() => validateConfig({ ...linodeConfig, schema: DEDICATED_SCHEMA }), /provider/);
  assert.throws(() => validateConfig({ ...dedicated, schema: "rustia.something-else.v1" }), /matrix requires/);

  // The pinned core must be one the kernel holds off the scheduler.
  validateBenchmarkCpu(dedicated, {});
  assert.throws(
    () => validateBenchmarkCpu({ ...dedicated, host_gate: { ...dedicated.host_gate, benchmark_cpu: 2 } }, {}),
    /is not one of provider\.host\.pinned_cpus/,
  );
  // The rented check is unchanged and still consults the instance's vCPU count.
  assert.throws(
    () => validateBenchmarkCpu(linodeConfig, { vcpus: 1 }),
    /absent from the Linode/,
  );

  // The budget is a hang detector on an owned host: nothing destroys the
  // machine when it expires, so the run has to end itself.
  assert.throws(
    () => checkRuntimeBudget(dedicated, Date.now() - 200 * 60_000, "in the self-test"),
    /exceeded provider max_runtime_minutes \(180\)/,
  );
  checkRuntimeBudget(dedicated, Date.now(), "in the self-test");

  // Starting a run erases the working state beneath ROOT, so ROOT must never be
  // a directory somebody else owns.
  assert.deepEqual(
    WIPE_ON_START.filter((directory) => directory === TARGET_ALL || directory === TARGET_LUA54),
    [],
    "the warm cargo target directories must survive a repeat run",
  );

  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-linux-matrix-self-test-"));
  try {
    const history = path.join(temporary, "history.jsonl");
    const samples = path.join(temporary, "samples.json");
    const gate = path.join(temporary, "gate.jsonl");
    const retryConfig = {
      host_gate: {
        benchmark_cpu: 3,
        cooldown_seconds: 1,
        maximum_timed_steal_percent: 0.1,
        maximum_timed_steal_ticks: 1,
        maximum_timed_attempts_per_group: 3,
        wait_timeout_seconds: 300,
      },
    };
    const cpu = (idle, steal) => ({ fields: [0, 0, 0, idle, 0, 0, 0, steal], steal });
    const timestamps = () => {
      let value = Date.parse("2026-07-20T00:00:00.000Z");
      return () => new Date(value += 1000).toISOString();
    };

    fs.writeFileSync(history, "prior-accepted\n");
    fs.writeFileSync(gate, "");
    const cpuReads = [cpu(0, 0), cpu(998, 2), cpu(998, 2), cpu(1997, 3)];
    const quietAttempts = [];
    const quietDeadlines = [];
    const sleeps = [];
    let runs = 0;
    await timedGroup(
      retryConfig,
      "retry-fixture",
      "fixture-command",
      [],
      gate,
      { rollbackFiles: [history, samples] },
      {
        waitForQuiet: async (_config, _label, _gate, attempt, deadline) => {
          quietAttempts.push(attempt);
          quietDeadlines.push(deadline);
        },
        readCpu: () => cpuReads.shift(),
        run: () => {
          runs += 1;
          fs.appendFileSync(history, `attempt-${runs}\n`);
          fs.appendFileSync(samples, `sample-${runs}\n`);
        },
        sleep: async (milliseconds) => sleeps.push(milliseconds),
        now: timestamps(),
        log: () => {},
      },
    );
    assert.equal(runs, 2);
    assert.deepEqual(quietAttempts, [1, 2]);
    assert.equal(quietDeadlines[0], quietDeadlines[1]);
    assert.deepEqual(sleeps, [1000, 1000]);
    assert.equal(fs.readFileSync(history, "utf8"), "prior-accepted\nattempt-2\n");
    assert.equal(fs.readFileSync(samples, "utf8"), "sample-2\n");
    const acceptedRecords = fs.readFileSync(gate, "utf8").trim().split("\n").map(JSON.parse);
    assert.deepEqual(
      acceptedRecords.map((record) => [record.timed_attempt, record.accepted]),
      [[1, false], [2, true]],
    );
    assert.equal(acceptedRecords[0].steal_percent, 0.2);
    assert.equal(acceptedRecords[1].steal_percent, 0.1);

    fs.writeFileSync(history, "prior-accepted\n");
    fs.rmSync(samples, { force: true });
    fs.writeFileSync(gate, "");
    const rejectedCpuReads = [cpu(0, 0), cpu(998, 2), cpu(998, 2), cpu(1996, 4)];
    const rejectedQuietAttempts = [];
    let rejectedRuns = 0;
    await assert.rejects(
      timedGroup(
        { host_gate: { ...retryConfig.host_gate, maximum_timed_attempts_per_group: 2 } },
        "exhaustion-fixture",
        "fixture-command",
        [],
        gate,
        { rollbackFiles: [history, samples] },
        {
          waitForQuiet: async (_config, _label, _gate, attempt) => rejectedQuietAttempts.push(attempt),
          readCpu: () => rejectedCpuReads.shift(),
          run: () => {
            rejectedRuns += 1;
            fs.appendFileSync(history, `rejected-${rejectedRuns}\n`);
            fs.appendFileSync(samples, `rejected-sample-${rejectedRuns}\n`);
          },
          sleep: async () => {},
          now: timestamps(),
          log: () => {},
        },
      ),
      /exhausted 2 timed attempt\(s\).*0\.1% \/ 1-tick steal gate/,
    );
    assert.equal(rejectedRuns, 2);
    assert.deepEqual(rejectedQuietAttempts, [1, 2]);
    assert.equal(fs.readFileSync(history, "utf8"), "prior-accepted\n");
    assert.equal(fs.existsSync(samples), false);
    const rejectedRecords = fs.readFileSync(gate, "utf8").trim().split("\n").map(JSON.parse);
    assert.deepEqual(
      rejectedRecords.map((record) => [record.timed_attempt, record.accepted]),
      [[1, false], [2, false]],
    );

    for (const failureKind of ["command", "gate"]) {
      fs.writeFileSync(history, "prior-accepted\n");
      fs.rmSync(samples, { force: true });
      fs.writeFileSync(gate, "");
      const failureCpuReads = [cpu(0, 0), cpu(999, 1)];
      let failureRuns = 0;
      let failureQuietWindows = 0;
      await assert.rejects(
        timedGroup(
          retryConfig,
          `${failureKind}-failure-fixture`,
          "fixture-command",
          [],
          gate,
          { rollbackFiles: [history, samples] },
          {
            waitForQuiet: async () => {
              failureQuietWindows += 1;
            },
            readCpu: () => failureCpuReads.shift(),
            run: () => {
              failureRuns += 1;
              fs.appendFileSync(history, `${failureKind}-attempt\n`);
              fs.appendFileSync(samples, `${failureKind}-sample\n`);
              if (failureKind === "command") throw new Error("fixture command failure");
            },
            appendGate: failureKind === "gate"
              ? () => { throw new Error("fixture gate failure"); }
              : undefined,
            sleep: async () => {},
            now: timestamps(),
            log: () => {},
          },
        ),
        new RegExp(`fixture ${failureKind} failure`),
      );
      assert.equal(failureRuns, 1);
      assert.equal(failureQuietWindows, 1);
      assert.equal(fs.readFileSync(history, "utf8"), "prior-accepted\n");
      assert.equal(fs.existsSync(samples), false);
      assert.equal(fs.readFileSync(gate, "utf8"), "");
    }
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
  process.stdout.write("linux matrix benchmark runner self-test passed\n");
}

async function main() {
  if (process.argv[2] === "self-test" && process.argv.length === 3) {
    await selfTest();
    return;
  }
  if (process.argv.length !== 5) {
    process.stderr.write("usage: run-linux-matrix-benchmark.js CONFIG PROVIDER BUNDLE\n");
    process.exitCode = 2;
    return;
  }
  process.exitCode = await execute(process.argv[2], process.argv[3], process.argv[4]);
}

main().catch((error) => {
  process.stderr.write(`run-linux-matrix-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
