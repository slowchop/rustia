#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const ROOT = "/opt/rustia-history-backfill";
const REPOSITORY = path.join(ROOT, "repo.git");
const HARNESS = path.join(ROOT, "harness");
const SOURCES = path.join(ROOT, "sources");
const TARGET = path.join(ROOT, "target");
const BINARIES = path.join(ROOT, "binaries");
const ATTEMPTS = path.join(ROOT, "attempts");
const EXPORT = path.join(ROOT, "export");
const EXPORT_ARCHIVE = "/opt/rustia/export.tar.gz";
const RUN_LOG = path.join(ROOT, "remote-run.log");
const PROTOCOL = "rustia_history_backfill_v1";
const WORKLOADS = Object.freeze([
  "dense_tables",
  "sparse_tables",
  "closures_upvalues",
  "generic_iteration",
  "stdlib_math_table",
]);
const WORKLOAD_FILES = Object.freeze({
  dense_tables: "bench/workloads/luau/01_dense_tables.luau",
  sparse_tables: "bench/workloads/luau/02_sparse_tables.luau",
  closures_upvalues: "bench/workloads/luau/04_closures_upvalues.luau",
  generic_iteration: "bench/workloads/luau/09_generic_iteration.luau",
  stdlib_math_table: "bench/workloads/luau/10_stdlib_math_table.luau",
});
const OVERLAY_FILES = Object.freeze({
  bench_manifest: "bench/Cargo.toml",
  native_harness: "bench/src/main.rs",
  cargo_lock: "Cargo.lock",
  toolchain: "rust-toolchain.toml",
  ...Object.fromEntries(
    WORKLOADS.map((workload) => [`workload_${workload}`, WORKLOAD_FILES[workload]]),
  ),
});
const CORE_TREES = Object.freeze([
  "crates/rustia-compiler",
  "crates/rustia-lexer",
  "crates/rustia-parser",
  "crates/rustia-vm",
]);
const PACKAGE_ACTIVITY_UNITS = Object.freeze([
  "apt-daily.service",
  "apt-daily-upgrade.service",
  "apt-daily.timer",
  "apt-daily-upgrade.timer",
  "unattended-upgrades.service",
]);
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const MAX_TARGET_COMMITS = 64;
const MAX_TIMED_ATTEMPTS = 3;

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

function requireFrozenArray(value, frozen, location) {
  if (!Array.isArray(value) || JSON.stringify(value) !== JSON.stringify(frozen)) {
    fail(`${location} must be exactly ${frozen.join(", ")}`);
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

function validateConfig(value) {
  const root = requireObject(value, "config");
  requireExactKeys(
    root,
    ["schema", "provider", "source", "measurement", "host_gate", "publication"],
    "config",
  );
  if (root.schema !== "rustia.linode-fixed-benchmark.v2") {
    fail("unsupported history backfill config schema");
  }

  const provider = requireObject(root.provider, "config.provider");
  requireExactKeys(
    provider,
    ["region", "image", "size", "max_runtime_minutes"],
    "config.provider",
  );
  requireString(provider.region, "config.provider.region");
  requireString(provider.image, "config.provider.image");
  const maxRuntime = requireInteger(
    provider.max_runtime_minutes,
    "config.provider.max_runtime_minutes",
    1,
  );
  if (maxRuntime >= 60) fail("history backfill must remain inside one Linode billing hour");
  const size = requireObject(provider.size, "config.provider.size");
  requireExactKeys(
    size,
    ["id", "description", "class", "vcpus", "memory_mib", "disk_gib", "max_hourly_price_usd"],
    "config.provider.size",
  );
  requireString(size.id, "config.provider.size.id");
  requireString(size.description, "config.provider.size.description");
  if (requireString(size.class, "config.provider.size.class") !== "dedicated") {
    fail("history backfill requires a dedicated Linode");
  }
  requireInteger(size.vcpus, "config.provider.size.vcpus", 2);
  requireInteger(size.memory_mib, "config.provider.size.memory_mib", 1);
  requireInteger(size.disk_gib, "config.provider.size.disk_gib", 1);
  requireNumber(size.max_hourly_price_usd, "config.provider.size.max_hourly_price_usd", 0.001);

  const source = requireObject(root.source, "config.source");
  requireExactKeys(
    source,
    ["harness_commit", "target_commits", "expected_sha256"],
    "config.source",
  );
  if (!FULL_COMMIT.test(requireString(source.harness_commit, "config.source.harness_commit"))) {
    fail("config.source.harness_commit must be a full lowercase commit");
  }
  if (
    !Array.isArray(source.target_commits) ||
    source.target_commits.length === 0 ||
    source.target_commits.length > MAX_TARGET_COMMITS
  ) {
    fail(`config.source.target_commits must contain 1..${MAX_TARGET_COMMITS} commits`);
  }
  const seen = new Set();
  for (const [index, commit] of source.target_commits.entries()) {
    if (!FULL_COMMIT.test(commit)) {
      fail(`config.source.target_commits[${index}] must be a full lowercase commit`);
    }
    if (seen.has(commit)) fail(`config.source.target_commits duplicates ${commit}`);
    seen.add(commit);
  }
  const expectedHashes = requireObject(source.expected_sha256, "config.source.expected_sha256");
  requireExactKeys(expectedHashes, Object.keys(OVERLAY_FILES), "config.source.expected_sha256");
  for (const [field, hash] of Object.entries(expectedHashes)) {
    if (!SHA256.test(hash)) fail(`config.source.expected_sha256.${field} must be a lowercase SHA-256`);
  }

  const measurement = requireObject(root.measurement, "config.measurement");
  requireExactKeys(
    measurement,
    ["protocol", "native_target", "workloads", "fixed_n", "repetitions", "expected_checksums"],
    "config.measurement",
  );
  if (measurement.protocol !== PROTOCOL) fail(`config.measurement.protocol must be ${PROTOCOL}`);
  if (measurement.native_target !== "linux-x86_64") {
    fail("config.measurement.native_target must be linux-x86_64");
  }
  requireFrozenArray(measurement.workloads, WORKLOADS, "config.measurement.workloads");
  if (requireInteger(measurement.repetitions, "config.measurement.repetitions", 1) !== 5) {
    fail("config.measurement.repetitions must remain frozen at 5");
  }
  const fixedN = requireObject(measurement.fixed_n, "config.measurement.fixed_n");
  const checksums = requireObject(
    measurement.expected_checksums,
    "config.measurement.expected_checksums",
  );
  requireExactKeys(fixedN, WORKLOADS, "config.measurement.fixed_n");
  requireExactKeys(checksums, WORKLOADS, "config.measurement.expected_checksums");
  for (const workload of WORKLOADS) {
    requireInteger(fixedN[workload], `config.measurement.fixed_n.${workload}`, 1);
    requireString(checksums[workload], `config.measurement.expected_checksums.${workload}`);
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
      "maximum_timed_steal_ticks",
      "maximum_timed_attempts_per_group",
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
  if (requireInteger(gate.maximum_steal_ticks, "config.host_gate.maximum_steal_ticks", 0) !== 0) {
    fail("history backfill idle steal budget must remain zero");
  }
  if (
    requireNumber(
      gate.maximum_timed_steal_percent,
      "config.host_gate.maximum_timed_steal_percent",
      0,
    ) !== 0.1
  ) {
    fail("history backfill timed steal budget must remain 0.1%");
  }
  if (
    requireInteger(
      gate.maximum_timed_steal_ticks,
      "config.host_gate.maximum_timed_steal_ticks",
      0,
    ) !== 1
  ) {
    fail("history backfill timed steal tick budget must remain one");
  }
  const maximumAttempts = requireInteger(
    gate.maximum_timed_attempts_per_group,
    "config.host_gate.maximum_timed_attempts_per_group",
    1,
  );
  if (maximumAttempts > MAX_TIMED_ATTEMPTS) {
    fail(`history backfill allows at most ${MAX_TIMED_ATTEMPTS} timed attempts`);
  }
  if (requireInteger(gate.wait_timeout_seconds, "config.host_gate.wait_timeout_seconds", 1) !== 300) {
    fail("history backfill quiet-host timeout must remain 300 seconds");
  }

  const publication = requireObject(root.publication, "config.publication");
  requireExactKeys(publication, ["mode", "rows"], "config.publication");
  if (publication.mode !== "history_backfill") {
    fail("config.publication.mode must be history_backfill");
  }
  const wantedRows = source.target_commits.length * WORKLOADS.length;
  if (requireInteger(publication.rows, "config.publication.rows", 1) !== wantedRows) {
    fail(`config.publication.rows must be ${wantedRows}`);
  }
  return root;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env || process.env,
    encoding: "utf8",
    stdio: options.capture === false ? "inherit" : ["ignore", "pipe", "pipe"],
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !options.allowFailure) {
    const stderr = String(result.stderr || "").trim();
    fail(`${command} ${args.join(" ")} failed with status ${result.status}${stderr ? `: ${stderr}` : ""}`);
  }
  return {
    status: result.status,
    stdout: String(result.stdout || ""),
    stderr: String(result.stderr || ""),
  };
}

function runText(command, args, options = {}) {
  return run(command, args, options).stdout.trim();
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

function appendJsonLine(file, value) {
  fs.appendFileSync(file, `${JSON.stringify(value)}\n`);
}

function verifySource(config, bundle) {
  run("git", ["clone", "--bare", bundle, REPOSITORY], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "bundle", "verify", bundle], { capture: false });
  run(
    "git",
    ["--git-dir", REPOSITORY, "worktree", "add", "--detach", HARNESS, config.source.harness_commit],
    { capture: false },
  );
  assert.equal(runText("git", ["-C", HARNESS, "rev-parse", "HEAD"]), config.source.harness_commit);
  for (const commit of config.source.target_commits) {
    run("git", ["--git-dir", REPOSITORY, "cat-file", "-e", `${commit}^{commit}`]);
    const ancestor = run(
      "git",
      ["--git-dir", REPOSITORY, "merge-base", "--is-ancestor", commit, config.source.harness_commit],
      { allowFailure: true },
    );
    if (ancestor.status !== 0) fail(`target ${commit} is not an ancestor of the harness commit`);
  }
  for (const [field, relative] of Object.entries(OVERLAY_FILES)) {
    const actual = sha256File(path.join(HARNESS, relative));
    if (actual !== config.source.expected_sha256[field]) {
      fail(`${relative} SHA-256 is ${actual}, expected ${config.source.expected_sha256[field]}`);
    }
  }
}

function installRust() {
  if (!fs.existsSync("/root/.cargo/bin/rustup")) {
    const installer = path.join(ROOT, "rustup-init.sh");
    run("curl", [
      "--proto",
      "=https",
      "--tlsv1.2",
      "--fail",
      "--silent",
      "--show-error",
      "https://sh.rustup.rs",
      "-o",
      installer,
    ]);
    run("sh", [installer, "-y", "--profile", "minimal", "--default-toolchain", "none"], {
      capture: false,
    });
  }
  run(
    "/root/.cargo/bin/rustup",
    ["toolchain", "install", "1.95.0", "--profile", "minimal"],
    { capture: false },
  );
}

function overlayHarness(target) {
  fs.copyFileSync(path.join(HARNESS, "bench/Cargo.toml"), path.join(target, "bench/Cargo.toml"));
  fs.rmSync(path.join(target, "bench/src"), { recursive: true, force: true });
  fs.cpSync(path.join(HARNESS, "bench/src"), path.join(target, "bench/src"), { recursive: true });
  fs.rmSync(path.join(target, "bench/workloads"), { recursive: true, force: true });
  fs.cpSync(path.join(HARNESS, "bench/workloads"), path.join(target, "bench/workloads"), {
    recursive: true,
  });
  fs.copyFileSync(path.join(HARNESS, "Cargo.lock"), path.join(target, "Cargo.lock"));
  fs.copyFileSync(
    path.join(HARNESS, "rust-toolchain.toml"),
    path.join(target, "rust-toolchain.toml"),
  );
}

function buildEnvironment() {
  return {
    ...process.env,
    CARGO_INCREMENTAL: "0",
    CARGO_TARGET_DIR: TARGET,
    RUSTC_WRAPPER: "",
  };
}

function targetMetadata(commit) {
  const committedAt = Number(
    runText("git", ["--git-dir", REPOSITORY, "show", "-s", "--format=%ct", commit]),
  );
  if (!Number.isSafeInteger(committedAt) || committedAt <= 0) {
    fail(`target ${commit} has an invalid commit timestamp`);
  }
  const subject = runText("git", ["--git-dir", REPOSITORY, "show", "-s", "--format=%s", commit]);
  const trees = Object.fromEntries(
    CORE_TREES.map((relative) => [
      relative,
      runText("git", ["--git-dir", REPOSITORY, "rev-parse", `${commit}:${relative}`]),
    ]),
  );
  return { commit, committed_at_unix: committedAt, subject, core_trees: trees };
}

function buildTargets(config) {
  const cargo = "/root/.cargo/bin/cargo";
  fs.mkdirSync(SOURCES, { recursive: true });
  fs.mkdirSync(BINARIES, { recursive: true });
  const builds = [];
  for (const [index, commit] of config.source.target_commits.entries()) {
    const source = path.join(SOURCES, commit);
    log(`building ${index + 1}/${config.source.target_commits.length} ${commit.slice(0, 8)}`);
    run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", source, commit], {
      capture: false,
    });
    overlayHarness(source);
    const cargoArgs = [
        "+1.95.0",
        "build",
        ...(index === 0 ? [] : ["--offline"]),
        "--manifest-path",
        path.join(source, "bench/Cargo.toml"),
        "--bin",
        "script-bench",
        "--release",
        "--no-default-features",
        "--features",
        "rustia-engine",
      ];
    run(cargo, cargoArgs, { env: buildEnvironment(), capture: false });
    const builtBinary = path.join(TARGET, "release/script-bench");
    fs.accessSync(builtBinary, fs.constants.X_OK);
    const binary = path.join(BINARIES, commit);
    fs.copyFileSync(builtBinary, binary);
    fs.chmodSync(binary, 0o700);
    builds.push({
      ...targetMetadata(commit),
      binary,
      binary_sha256: sha256File(binary),
      resolved_lock_sha256: sha256File(path.join(source, "Cargo.lock")),
    });
  }
  return builds;
}

function readCpu(cpu) {
  const line = fs
    .readFileSync("/proc/stat", "utf8")
    .split("\n")
    .find((entry) => entry.startsWith(`cpu${cpu} `));
  if (!line) fail(`cannot read benchmark CPU ${cpu} from /proc/stat`);
  const fields = line.trim().split(/\s+/).slice(1).map(Number);
  if (fields.length < 8 || fields.some((value) => !Number.isFinite(value))) {
    fail("malformed /proc/stat CPU row");
  }
  return { fields };
}

function cpuDelta(before, after) {
  const deltas = after.fields.map((value, index) => value - before.fields[index]);
  if (deltas.some((value) => !Number.isFinite(value) || value < 0)) {
    fail("invalid /proc/stat CPU delta");
  }
  const total = deltas.reduce((sum, value) => sum + value, 0);
  if (total <= 0) fail("empty /proc/stat CPU window");
  const idle = deltas[3] + (deltas[4] || 0);
  const steal = deltas[7] || 0;
  return {
    total_ticks: total,
    idle_ticks: idle,
    steal_ticks: steal,
    idle_percent: (idle / total) * 100,
    steal_percent: (steal / total) * 100,
  };
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitForQuiet(config, gateFile, timedAttempt, deadline) {
  const gate = config.host_gate;
  let quietAttempt = 0;
  while (Date.now() < deadline) {
    quietAttempt += 1;
    await sleep(gate.cooldown_seconds * 1000);
    const before = readCpu(gate.benchmark_cpu);
    await sleep(gate.idle_window_seconds * 1000);
    const delta = cpuDelta(before, readCpu(gate.benchmark_cpu));
    const load1m = Number(fs.readFileSync("/proc/loadavg", "utf8").trim().split(/\s+/)[0]);
    const accepted =
      delta.idle_percent >= gate.minimum_idle_percent &&
      delta.steal_ticks <= gate.maximum_steal_ticks &&
      load1m <= gate.maximum_load_1m;
    appendJsonLine(gateFile, {
      phase: "pre",
      label: "rustia-history-backfill",
      timed_attempt: timedAttempt,
      quiet_attempt: quietAttempt,
      captured_at: new Date().toISOString(),
      load_1m: load1m,
      ...delta,
      accepted,
    });
    if (accepted) return;
  }
  fail(`history backfill timed attempt ${timedAttempt} could not obtain a quiet host window`);
}

function timedGatePasses(gate, delta) {
  return (
    delta.steal_percent <= gate.maximum_timed_steal_percent ||
    delta.steal_ticks <= gate.maximum_timed_steal_ticks
  );
}

function measurementArgs(config, build, historyFile, samplesFile) {
  return [
    "-c",
    String(config.host_gate.benchmark_cpu),
    build.binary,
    "--engines",
    "rustia",
    "--workloads",
    WORKLOADS.join(","),
    "--reps",
    String(config.measurement.repetitions),
    "--history",
    historyFile,
    "--samples-json",
    samplesFile,
    "--commit",
    build.commit,
    "--no-results",
    "--no-latest-json",
  ];
}

function measureOne(config, build, attemptDirectory) {
  const historyFile = path.join(attemptDirectory, `${build.commit}.jsonl`);
  const samplesFile = path.join(attemptDirectory, `${build.commit}-samples.json`);
  for (const workload of WORKLOADS) {
    const workloadHistory = path.join(attemptDirectory, `${build.commit}-${workload}.jsonl`);
    const workloadSamples = path.join(attemptDirectory, `${build.commit}-${workload}-samples.json`);
    const args = measurementArgs(config, build, workloadHistory, workloadSamples);
    args.push(
      "--workloads",
      workload,
      "--fixed-n",
      String(config.measurement.fixed_n[workload]),
    );
    run("taskset", args, {
      cwd: path.join(SOURCES, build.commit),
      capture: false,
    });
    fs.appendFileSync(historyFile, fs.readFileSync(workloadHistory));
    const sample = loadJson(workloadSamples, `${build.commit}/${workload} samples`);
    const combined = fs.existsSync(samplesFile)
      ? loadJson(samplesFile, `${build.commit} combined samples`)
      : { schema: "rustia.bench.history-backfill-samples.v1", commit: build.commit, measurements: [] };
    const measurements = Array.isArray(sample.measurements) ? sample.measurements : [];
    combined.measurements.push(...measurements);
    writeJson(samplesFile, combined);
  }
  return { historyFile, samplesFile };
}

function parseHistory(file) {
  const text = fs.readFileSync(file, "utf8").trim();
  if (!text) return [];
  return text.split("\n").map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      fail(`${file} line ${index + 1} is invalid JSON: ${error.message}`);
    }
  });
}

function validateRows(config, builds, rows) {
  if (rows.length !== config.publication.rows) {
    fail(`history backfill produced ${rows.length} rows, expected ${config.publication.rows}`);
  }
  const positions = new Map(builds.map((build, index) => [build.commit, index]));
  const slots = new Set();
  for (const [index, row] of rows.entries()) {
    requireObject(row, `history row ${index + 1}`);
    if (!positions.has(row.commit)) fail(`history row ${index + 1} has an unexpected commit`);
    if (row.engine !== "rustia" || row.dialect !== "luau" || row.target !== "linux-x86_64") {
      fail(`history row ${index + 1} has the wrong engine identity`);
    }
    if (!WORKLOADS.includes(row.workload)) fail(`history row ${index + 1} has an unknown workload`);
    if (row.n !== config.measurement.fixed_n[row.workload]) {
      fail(`history row ${index + 1} has the wrong fixed N`);
    }
    if (row.reps !== config.measurement.repetitions) {
      fail(`history row ${index + 1} has the wrong repetition count`);
    }
    if (row.checksum !== config.measurement.expected_checksums[row.workload]) {
      fail(
        `history row ${index + 1} checksum ${row.checksum} does not match ` +
          config.measurement.expected_checksums[row.workload],
      );
    }
    for (const field of ["ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
      requireNumber(row[field], `history row ${index + 1}.${field}`, Number.MIN_VALUE);
    }
    const slot = `${row.commit}\0${row.workload}`;
    if (slots.has(slot)) fail(`history backfill duplicates ${row.commit}/${row.workload}`);
    slots.add(slot);
  }
  for (const build of builds) {
    for (const workload of WORKLOADS) {
      if (!slots.has(`${build.commit}\0${workload}`)) {
        fail(`history backfill is missing ${build.commit}/${workload}`);
      }
    }
  }
}

async function measure(config, builds, evidenceDirectory) {
  const gateFile = path.join(evidenceDirectory, "gate.jsonl");
  fs.writeFileSync(gateFile, "");
  const quietDeadline = Date.now() + config.host_gate.wait_timeout_seconds * 1000;
  for (let attempt = 1; attempt <= config.host_gate.maximum_timed_attempts_per_group; attempt += 1) {
    await waitForQuiet(config, gateFile, attempt, quietDeadline);
    const attemptDirectory = path.join(ATTEMPTS, String(attempt));
    fs.rmSync(attemptDirectory, { recursive: true, force: true });
    fs.mkdirSync(attemptDirectory, { recursive: true });
    log(`history backfill timed attempt ${attempt}/${config.host_gate.maximum_timed_attempts_per_group} starting`);
    const before = readCpu(config.host_gate.benchmark_cpu);
    const startedAt = new Date().toISOString();
    const outputs = [];
    for (const [index, build] of builds.entries()) {
      log(`measuring ${index + 1}/${builds.length} ${build.commit.slice(0, 8)}`);
      outputs.push(measureOne(config, build, attemptDirectory));
    }
    const finishedAt = new Date().toISOString();
    const delta = cpuDelta(before, readCpu(config.host_gate.benchmark_cpu));
    const accepted = timedGatePasses(config.host_gate, delta);
    appendJsonLine(gateFile, {
      phase: "timed",
      label: "rustia-history-backfill",
      timed_attempt: attempt,
      started_at: startedAt,
      finished_at: finishedAt,
      ...delta,
      accepted,
    });
    if (!accepted) {
      log(
        `history backfill timed attempt ${attempt} rejected at ` +
          `${delta.steal_percent.toFixed(6)}% / ${delta.steal_ticks} steal tick(s)`,
      );
      continue;
    }
    const rows = outputs.flatMap((output) => parseHistory(output.historyFile));
    validateRows(config, builds, rows);
    log(`history backfill timed attempt ${attempt} accepted`);
    return { gateFile, attemptDirectory, rows, startedAt, finishedAt };
  }
  fail("history backfill exhausted its timed attempts without meeting the steal gate");
}

function collectHost(config, builds) {
  const cpuInfo = fs.readFileSync("/proc/cpuinfo", "utf8");
  const model = cpuInfo.match(/^model name\s*:\s*(.+)$/m);
  return {
    schema: "rustia.bench.history-backfill-host.v1",
    platform: process.platform,
    architecture: process.arch,
    release: os.release(),
    cpu_model: model ? model[1].trim() : "unknown",
    logical_cpus: os.cpus().length,
    benchmark_cpu: config.host_gate.benchmark_cpu,
    rustc_version: runText("/root/.cargo/bin/rustc", ["+1.95.0", "--version", "--verbose"]),
    cargo_version: runText("/root/.cargo/bin/cargo", ["+1.95.0", "--version", "--verbose"]),
    target_count: builds.length,
  };
}

function evidenceInventory(directory) {
  const files = [];
  function visit(current, prefix) {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) visit(path.join(current, entry.name), relative);
      else if (entry.isFile()) files.push(relative);
      else fail(`evidence contains unsupported entry ${relative}`);
    }
  }
  visit(directory, "");
  return files.sort();
}

function publish(config, configFile, providerFile, builds, measurement, evidenceDirectory) {
  const measuredAt = Math.floor(Date.parse(measurement.finishedAt) / 1000);
  const publication = {
    schema: "rustia.bench.history-backfill.v1",
    harness_commit: config.source.harness_commit,
    measured_at_unix: measuredAt,
    native_target: config.measurement.native_target,
    workloads: WORKLOADS,
    fixed_n: config.measurement.fixed_n,
    repetitions: config.measurement.repetitions,
    targets: builds.map(({ binary, ...build }) => build),
    rows: measurement.rows,
  };
  const publicationFile = path.join(
    EXPORT,
    "publication/bench/history/backfill.json",
  );
  writeJson(publicationFile, publication);
  fs.copyFileSync(configFile, path.join(evidenceDirectory, "protocol.json"));
  fs.copyFileSync(providerFile, path.join(evidenceDirectory, "provider.json"));
  fs.copyFileSync(RUN_LOG, path.join(evidenceDirectory, "remote-run.log"));
  writeJson(path.join(evidenceDirectory, "host.json"), collectHost(config, builds));
  writeJson(
    path.join(evidenceDirectory, "builds.json"),
    builds.map(({ binary, ...build }) => build),
  );
  const rawDirectory = path.join(evidenceDirectory, "raw");
  fs.mkdirSync(rawDirectory, { recursive: true });
  for (const build of builds) {
    for (const suffix of [".jsonl", "-samples.json"]) {
      const source = path.join(measurement.attemptDirectory, `${build.commit}${suffix}`);
      fs.copyFileSync(source, path.join(rawDirectory, `${build.commit}${suffix}`));
    }
  }
  writeJson(path.join(evidenceDirectory, "manifest.json"), {
    schema: "rustia.bench.history-backfill-evidence.v1",
    harness_commit: config.source.harness_commit,
    target_commits: config.source.target_commits,
    created_at: new Date().toISOString(),
    protocol_sha256: sha256File(configFile),
    provider_sha256: sha256File(providerFile),
    publication_sha256: sha256File(publicationFile),
    evidence_files: evidenceInventory(evidenceDirectory).filter((file) => file !== "manifest.json"),
  });
}

function exportRun(status, evidenceDirectory) {
  fs.rmSync(EXPORT, { recursive: true, force: true });
  fs.mkdirSync(EXPORT, { recursive: true });
  writeJson(path.join(EXPORT, "status.json"), status);
  if (fs.existsSync(evidenceDirectory)) {
    const destination = path.join(EXPORT, "evidence/history-backfill");
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.cpSync(evidenceDirectory, destination, { recursive: true });
  }
  if (status.status === "backfill_complete") {
    // publish() populated this before exportRun reset EXPORT; recover the staged copy.
    const staged = path.join(ROOT, "backfill-publication.json");
    fs.mkdirSync(path.join(EXPORT, "publication/bench/history"), { recursive: true });
    fs.copyFileSync(staged, path.join(EXPORT, "publication/bench/history/backfill.json"));
  }
  run("tar", ["--format=ustar", "-czf", EXPORT_ARCHIVE, "-C", EXPORT, "."], {
    capture: false,
  });
}

async function execute(configFile, providerFile, bundle) {
  if (process.platform !== "linux" || process.getuid() !== 0) {
    fail("history backfill runner must execute as root on Linux");
  }
  fs.mkdirSync(ROOT, { recursive: true });
  fs.writeFileSync(RUN_LOG, "", { mode: 0o600 });
  logReady = true;
  const config = validateConfig(loadJson(configFile, "protocol config"));
  const provider = requireObject(loadJson(providerFile, "provider metadata"), "provider metadata");
  const evidenceDirectory = path.join(ROOT, "evidence");
  fs.rmSync(evidenceDirectory, { recursive: true, force: true });
  fs.mkdirSync(evidenceDirectory, { recursive: true });
  let status = {
    schema: "rustia.bench.cloud-run-status.v1",
    status: "failed",
    detail: "history backfill runner did not reach its terminal state",
    started_at: new Date().toISOString(),
  };
  try {
    if (config.host_gate.benchmark_cpu >= provider.vcpus) {
      fail("configured benchmark CPU is absent from the Linode");
    }
    log("verifying fixed harness, historical targets, and source hashes");
    verifySource(config, bundle);
    installRust();
    log(`building ${config.source.target_commits.length} historical Rustia revisions`);
    const builds = buildTargets(config);
    for (const unit of PACKAGE_ACTIVITY_UNITS) {
      run("systemctl", ["stop", unit], { allowFailure: true });
      run("systemctl", ["mask", unit], { allowFailure: true });
    }
    log("measuring fixed current workloads across the historical binaries");
    const measurement = await measure(config, builds, evidenceDirectory);
    publish(config, configFile, providerFile, builds, measurement, evidenceDirectory);
    fs.copyFileSync(
      path.join(EXPORT, "publication/bench/history/backfill.json"),
      path.join(ROOT, "backfill-publication.json"),
    );
    status = {
      ...status,
      status: "backfill_complete",
      detail: "fixed-harness Rustia history backfill completed",
      finished_at: new Date().toISOString(),
    };
  } catch (error) {
    status = {
      ...status,
      status: "failed",
      detail: error.message,
      finished_at: new Date().toISOString(),
    };
    log(`failed: ${error.message}`);
  } finally {
    exportRun(status, evidenceDirectory);
  }
  return status.status === "backfill_complete" ? 0 : 2;
}

async function selfTest() {
  const hashes = Object.fromEntries(
    Object.keys(OVERLAY_FILES).map((field) => [field, "a".repeat(64)]),
  );
  const config = {
    schema: "rustia.linode-fixed-benchmark.v2",
    provider: {
      region: "au-mel",
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
      harness_commit: "1".repeat(40),
      target_commits: ["2".repeat(40), "3".repeat(40)],
      expected_sha256: hashes,
    },
    measurement: {
      protocol: PROTOCOL,
      native_target: "linux-x86_64",
      workloads: WORKLOADS,
      fixed_n: Object.fromEntries(WORKLOADS.map((workload) => [workload, 10])),
      repetitions: 5,
      expected_checksums: Object.fromEntries(
        WORKLOADS.map((workload) => [workload, `number:${workload.length}`]),
      ),
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
    publication: { mode: "history_backfill", rows: 10 },
  };
  assert.equal(validateConfig(config), config);
  assert.throws(() =>
    validateConfig({
      ...config,
      source: { ...config.source, target_commits: [config.source.target_commits[0], config.source.target_commits[0]] },
    }),
  );
  assert.throws(() =>
    validateConfig({ ...config, publication: { mode: "history_backfill", rows: 9 } }),
  );
  assert.deepEqual(
    cpuDelta(
      { fields: [10, 0, 5, 100, 2, 1, 1, 0] },
      { fields: [11, 0, 6, 198, 2, 1, 1, 0] },
    ),
    { total_ticks: 100, idle_ticks: 98, steal_ticks: 0, idle_percent: 98, steal_percent: 0 },
  );
  const gate = { maximum_timed_steal_percent: 0.1, maximum_timed_steal_ticks: 1 };
  assert.equal(timedGatePasses(gate, { total_ticks: 946, steal_ticks: 1, steal_percent: 0.1057 }), true);
  assert.equal(timedGatePasses(gate, { total_ticks: 946, steal_ticks: 2, steal_percent: 0.2114 }), false);
  process.stdout.write("history backfill runner self-test passed\n");
}

async function main() {
  if (process.argv[2] === "self-test") {
    if (process.argv.length !== 3) fail("self-test takes no additional arguments");
    await selfTest();
    return;
  }
  if (process.argv.length !== 5) {
    process.stderr.write(
      "usage: run-linux-history-backfill.js CONFIG PROVIDER BUNDLE\n",
    );
    process.exitCode = 2;
    return;
  }
  process.exitCode = await execute(process.argv[2], process.argv[3], process.argv[4]);
}

main().catch((error) => {
  process.stderr.write(`run-linux-history-backfill: ${error.message}\n`);
  process.exitCode = 1;
});
