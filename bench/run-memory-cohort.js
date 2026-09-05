#!/usr/bin/env node
"use strict";

const assert = require("assert");
const crypto = require("crypto");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { spawnSync } = require("child_process");
const { cgroupBlockerMessage, delegatedCgroupRoot } = require("./cgroup-delegation.js");

const PROTOCOL_SCHEMA = "rustia.bench.memory-protocol.v1";
const MANIFEST_SCHEMA = "rustia.bench.memory-cohort-evidence.v1";
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_ID = /^[A-Za-z0-9._-]{1,128}$/;
const ROOT = path.resolve(__dirname, "..");
const SUPERVISOR = path.join(__dirname, "run-memory-supervisor.js");
const ANALYZER = path.join(__dirname, "analyze-memory-samples.js");

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { command: "self-test" };
  const [command, ...rest] = argv;
  if (command !== "run") fail("expected command run or self-test");
  const values = new Map();
  for (let index = 0; index < rest.length; index += 2) {
    const flag = rest[index];
    const value = rest[index + 1];
    if (!flag?.startsWith("--") || value == null) fail(`${flag ?? "argument"} requires a value`);
    if (values.has(flag)) fail(`${flag} was supplied more than once`);
    values.set(flag, value);
  }
  const take = (name) => {
    const value = values.get(name);
    values.delete(name);
    if (value == null) fail(`missing ${name}`);
    return value;
  };
  const result = {
    command,
    config: path.resolve(take("--config")),
    binaryAll: path.resolve(take("--binary-all")),
    binaryLua54: path.resolve(take("--binary-lua54")),
    outputDirectory: path.resolve(take("--output-dir")),
  };
  if (values.size !== 0) fail(`unknown argument(s): ${[...values.keys()].join(", ")}`);
  return result;
}

function loadJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot parse ${label}: ${error.message}`);
  }
}

function exactKeys(value, keys, label) {
  if (value == null || typeof value !== "object" || Array.isArray(value)) fail(`${label} must be an object`);
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    fail(`${label} keys must be exactly ${expected.join(", ")}`);
  }
}

function positiveInteger(value, label) {
  if (!Number.isSafeInteger(value) || value <= 0) fail(`${label} must be a positive integer`);
  return value;
}

function validateProtocol(value) {
  exactKeys(
    value,
    [
      "schema",
      "cohort_id",
      "commit",
      "samples",
      "bootstrap_repetitions",
      "bootstrap_seed",
      "growth_thresholds",
      "host",
      "groups",
    ],
    "protocol",
  );
  if (value.schema !== PROTOCOL_SCHEMA) fail("protocol schema is wrong");
  if (!SAFE_ID.test(value.cohort_id)) fail("protocol.cohort_id is not a safe identifier");
  if (!FULL_COMMIT.test(value.commit)) fail("protocol.commit must be a lowercase full commit");
  positiveInteger(value.samples, "protocol.samples");
  if (value.samples < 9) fail("publication protocol requires at least nine fresh-process samples");
  positiveInteger(value.bootstrap_repetitions, "protocol.bootstrap_repetitions");
  if (!Number.isSafeInteger(value.bootstrap_seed) || value.bootstrap_seed < 0 || value.bootstrap_seed > 0xffffffff) {
    fail("protocol.bootstrap_seed must fit in u32");
  }
  exactKeys(value.growth_thresholds, ["absolute_kib_per_epoch", "proportional_percent_per_epoch"], "growth_thresholds");
  for (const [key, threshold] of Object.entries(value.growth_thresholds)) {
    if (!Number.isFinite(threshold) || threshold <= 0) fail(`growth_thresholds.${key} must be positive`);
  }
  exactKeys(
    value.host,
    ["benchmark_cpu", "supervisor_cpu", "sample_interval_ms", "timeout_ms"],
    "protocol.host",
  );
  for (const field of ["benchmark_cpu", "supervisor_cpu"]) {
    if (!Number.isSafeInteger(value.host[field]) || value.host[field] < 0) fail(`protocol.host.${field}`);
  }
  if (value.host.benchmark_cpu === value.host.supervisor_cpu) fail("benchmark and supervisor CPUs must differ");
  positiveInteger(value.host.sample_interval_ms, "protocol.host.sample_interval_ms");
  positiveInteger(value.host.timeout_ms, "protocol.host.timeout_ms");
  if (!Array.isArray(value.groups) || value.groups.length === 0) fail("protocol.groups must be nonempty");
  const identities = new Set();
  for (const [groupIndex, group] of value.groups.entries()) {
    exactKeys(group, ["binary", "engines", "workloads"], `groups[${groupIndex}]`);
    if (!["all", "lua54"].includes(group.binary)) fail(`groups[${groupIndex}].binary is invalid`);
    if (!Array.isArray(group.engines) || group.engines.length === 0) fail(`groups[${groupIndex}].engines`);
    if (!Array.isArray(group.workloads) || group.workloads.length === 0) fail(`groups[${groupIndex}].workloads`);
    for (const engine of group.engines) {
      if (!SAFE_ID.test(engine)) fail(`groups[${groupIndex}] has an unsafe engine`);
      for (const [workloadIndex, workload] of group.workloads.entries()) {
        exactKeys(workload, ["id", "n", "epochs"], `groups[${groupIndex}].workloads[${workloadIndex}]`);
        if (!SAFE_ID.test(workload.id)) fail(`groups[${groupIndex}] has an unsafe workload`);
        positiveInteger(workload.n, `${workload.id}.n`);
        positiveInteger(workload.epochs, `${workload.id}.epochs`);
        if (workload.epochs < 8) fail(`${workload.id}.epochs must be at least eight`);
        const identity = `${engine}\0${workload.id}`;
        if (identities.has(identity)) fail(`duplicate protocol case ${engine}/${workload.id}`);
        identities.add(identity);
      }
    }
  }
  return value;
}

function sha256File(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd ?? ROOT,
    encoding: "utf8",
    stdio: options.capture === false ? "inherit" : ["ignore", "pipe", "pipe"],
    env: { ...process.env, RUSTC_WRAPPER: "" },
  });
  if (result.error) fail(`${command} failed to start: ${result.error.message}`);
  if (result.status !== 0) {
    fail(
      `${command} ${args.join(" ")} failed with status ${result.status}: ` +
        `${String(result.stderr ?? "").trim()}`,
    );
  }
  return result;
}

function cases(protocol, binaries) {
  const result = [];
  for (const group of protocol.groups) {
    for (const engine of group.engines) {
      for (const workload of group.workloads) {
        result.push({
          engine,
          workload: workload.id,
          n: workload.n,
          epochs: workload.epochs,
          binary: binaries[group.binary],
          binaryRole: group.binary,
        });
      }
    }
  }
  return result;
}

function roundOrder(allCases, round) {
  const offset = (round - 1) % allCases.length;
  const rotated = allCases.slice(offset).concat(allCases.slice(0, offset));
  return round % 2 === 0 ? rotated.reverse() : rotated;
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx" });
}

function collectHost() {
  return {
    captured_at: new Date().toISOString(),
    platform: process.platform,
    arch: process.arch,
    release: os.release(),
    hostname: os.hostname(),
    uname: run("uname", ["-a"]).stdout.trim(),
    cgroup_filesystems: fs.readFileSync("/proc/filesystems", "utf8"),
    cpuinfo_sha256: sha256File("/proc/cpuinfo"),
  };
}

function runCohort(config) {
  if (process.platform !== "linux") fail("memory cohort requires Linux");
  // Root, or a delegated subtree. What the protocol needs is a cgroup it can
  // create children in with the memory controller enabled -- being uid 0 is one
  // way to have that, not the requirement itself.
  const delegated = delegatedCgroupRoot();
  const cgroupRoot = delegated.root;
  if (typeof process.getuid === "function" && process.getuid() !== 0 && cgroupRoot === null) {
    fail(cgroupBlockerMessage(process.getuid(), delegated.reason));
  }
  const protocol = validateProtocol(loadJson(config.config, "memory protocol"));
  for (const binary of [config.binaryAll, config.binaryLua54]) {
    if (!fs.existsSync(binary) || !fs.statSync(binary).isFile()) fail(`missing benchmark binary ${binary}`);
  }
  if (fs.existsSync(config.outputDirectory)) fail(`refusing to overwrite ${config.outputDirectory}`);
  fs.mkdirSync(config.outputDirectory, { recursive: true });
  const rawDirectory = path.join(config.outputDirectory, "raw");
  fs.mkdirSync(rawDirectory);
  const protocolDestination = path.join(config.outputDirectory, "protocol.json");
  fs.copyFileSync(config.config, protocolDestination, fs.constants.COPYFILE_EXCL);
  writeJson(path.join(config.outputDirectory, "host.json"), collectHost());

  const binaries = { all: config.binaryAll, lua54: config.binaryLua54 };
  const allCases = cases(protocol, binaries);
  const rawFiles = [];
  const runLog = [];
  for (let round = 1; round <= protocol.samples; round += 1) {
    for (const testCase of roundOrder(allCases, round)) {
      const sampleId = [
        `round-${String(round).padStart(2, "0")}`,
        testCase.engine,
        testCase.workload,
      ].join("-");
      const output = path.join(rawDirectory, `${sampleId}.json`);
      const startedAt = new Date().toISOString();
      run(
        "taskset",
        [
          "-c",
          String(protocol.host.supervisor_cpu),
          process.execPath,
          SUPERVISOR,
          "sample",
          "--binary",
          testCase.binary,
          "--engine",
          testCase.engine,
          "--workload",
          testCase.workload,
          "--n",
          String(testCase.n),
          "--epochs",
          String(testCase.epochs),
          "--output",
          output,
          "--commit",
          protocol.commit,
          "--sample-id",
          sampleId,
          "--benchmark-cpu",
          String(protocol.host.benchmark_cpu),
          "--supervisor-cpu",
          String(protocol.host.supervisor_cpu),
          "--sample-interval-ms",
          String(protocol.host.sample_interval_ms),
          "--timeout-ms",
          String(protocol.host.timeout_ms),
          ...(cgroupRoot === null ? ["--allow-root-cgroup"] : ["--cgroup-root", cgroupRoot]),
        ],
        { capture: false },
      );
      rawFiles.push(output);
      runLog.push({
        sample_id: sampleId,
        round,
        engine: testCase.engine,
        workload: testCase.workload,
        binary_role: testCase.binaryRole,
        started_at: startedAt,
        finished_at: new Date().toISOString(),
        sha256: sha256File(output),
      });
    }
  }
  writeJson(path.join(config.outputDirectory, "run-order.json"), runLog);
  const analysisPath = path.join(config.outputDirectory, "analysis.json");
  const analyzerArgs = [
    ANALYZER,
    ...rawFiles.flatMap((file) => ["--input", file]),
    "--output",
    analysisPath,
    "--minimum-samples",
    String(protocol.samples),
    "--bootstrap-repetitions",
    String(protocol.bootstrap_repetitions),
    "--bootstrap-seed",
    String(protocol.bootstrap_seed),
    "--absolute-growth-kib-per-epoch",
    String(protocol.growth_thresholds.absolute_kib_per_epoch),
    "--proportional-growth-percent-per-epoch",
    String(protocol.growth_thresholds.proportional_percent_per_epoch),
  ];
  run(process.execPath, analyzerArgs, { capture: false });

  const evidenceHashes = {};
  for (const entry of fs.readdirSync(config.outputDirectory, { withFileTypes: true })) {
    if (entry.isFile() && entry.name !== "manifest.json") {
      evidenceHashes[entry.name] = sha256File(path.join(config.outputDirectory, entry.name));
    }
  }
  evidenceHashes.raw = Object.fromEntries(
    fs.readdirSync(rawDirectory).sort().map((file) => [file, sha256File(path.join(rawDirectory, file))]),
  );
  const manifest = {
    schema_version: 1,
    schema: MANIFEST_SCHEMA,
    cohort_id: protocol.cohort_id,
    created_at: new Date().toISOString(),
    commit: protocol.commit,
    protocol_sha256: sha256File(protocolDestination),
    supervisor_sha256: sha256File(SUPERVISOR),
    analyzer_sha256: sha256File(ANALYZER),
    binaries: {
      all_sha256: sha256File(config.binaryAll),
      lua54_sha256: sha256File(config.binaryLua54),
    },
    case_count: allCases.length,
    samples_per_case: protocol.samples,
    raw_sample_count: rawFiles.length,
    evidence_sha256: evidenceHashes,
  };
  writeJson(path.join(config.outputDirectory, "manifest.json"), manifest);
  process.stdout.write(
    `memory cohort complete: ${allCases.length} cases × ${protocol.samples} samples = ${rawFiles.length}\n`,
  );
}

function selfTest() {
  const commit = "a".repeat(40);
  const protocol = validateProtocol({
    schema: PROTOCOL_SCHEMA,
    cohort_id: "fixture",
    commit,
    samples: 9,
    bootstrap_repetitions: 100,
    bootstrap_seed: 7,
    growth_thresholds: {
      absolute_kib_per_epoch: 4,
      proportional_percent_per_epoch: 0.05,
    },
    host: {
      benchmark_cpu: 3,
      supervisor_cpu: 2,
      sample_interval_ms: 10,
      timeout_ms: 600000,
    },
    groups: [{
      binary: "all",
      engines: ["rustia", "koto"],
      workloads: [{ id: "memory_churn", n: 1, epochs: 40 }],
    }],
  });
  const fixtures = cases(protocol, { all: "/all", lua54: "/lua54" });
  assert.equal(fixtures.length, 2);
  assert.deepEqual(roundOrder(fixtures, 1), fixtures);
  assert.deepEqual(roundOrder(fixtures, 2), fixtures);
  assert.throws(() => validateProtocol({ ...protocol, samples: 8 }));

  // Each cause has to send the reader at the host change that fixes it. The
  // read-only case is the one that cost a run: the subtree was present and
  // correctly owned, and the message said delegation was missing.
  assert.match(cgroupBlockerMessage(1001, "absent"), /does not exist/);
  assert.match(cgroupBlockerMessage(1001, "absent"), /enable-linger/);
  assert.match(cgroupBlockerMessage(1001, "read-only"), /ProtectControlGroups=yes/);
  assert.match(cgroupBlockerMessage(1001, "read-only"), /ReadWritePaths=/);
  assert.doesNotMatch(cgroupBlockerMessage(1001, "read-only"), /enable-linger/);
  assert.match(cgroupBlockerMessage(1001, "unwritable"), /owned by that user/);
  for (const reason of ["absent", "read-only", "unwritable"]) {
    assert.match(cgroupBlockerMessage(1001, reason), /user-1001\.slice\/user@1001\.service/);
  }
  process.stdout.write("memory cohort self-test passed\n");
}

function main() {
  const config = parseArgs(process.argv.slice(2));
  if (config.command === "self-test") return selfTest();
  runCohort(config);
}

try {
  main();
} catch (error) {
  process.stderr.write(`run-memory-cohort: ${error.message}\n`);
  process.exitCode = 1;
}

