#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawn, spawnSync } = require("node:child_process");

const ROOT = "/opt/rustia";
const REPOSITORY = path.join(ROOT, "repo.git");
const SOURCE_A = path.join(ROOT, "src-a");
const SOURCE_B = path.join(ROOT, "src-b");
const TARGET_A = path.join(ROOT, "target-a");
const TARGET_B = path.join(ROOT, "target-b");
const PILOT = path.join(ROOT, "qualification");
const EXPORT = path.join(ROOT, "export");
const EXPORT_ARCHIVE = path.join(ROOT, "export.tar.gz");
const RUN_LOG = path.join(ROOT, "run.log");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const CONTAMINATING_PROCESS = /^(apt|apt-get|as|cargo|cc|cc1|cc1plus|clang|clang\+\+|curl|dpkg|g\+\+|gcc|git|ld|ld\.lld|make|ninja|node|rustc|rustup|sccache|script-bench|unattended-upgr)$/;
const PACKAGE_ACTIVITY_UNITS = Object.freeze([
  "apt-daily.service",
  "apt-daily-upgrade.service",
  "apt-daily.timer",
  "apt-daily-upgrade.timer",
  "unattended-upgrades.service",
  "packagekit.service",
]);

let logReady = false;

class ControlledStop extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}

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

function validateConfig(config) {
  requireObject(config, "config");
  if (
    config.schema !== "rustia.digitalocean-fixed-benchmark.v2" &&
    config.schema !== "rustia.linode-fixed-benchmark.v1" &&
    config.schema !== "rustia.linode-fixed-benchmark.v2" &&
    config.schema !== "rustia.vultr-fixed-benchmark.v1" &&
    config.schema !== "rustia.scaleway-fixed-benchmark.v1"
  ) {
    fail("unsupported cloud benchmark config schema");
  }
  for (const field of ["provider", "source", "measurement", "qualification", "host_gate", "publication"]) {
    requireObject(config[field], `config.${field}`);
  }
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    if (!FULL_COMMIT.test(config.source[field])) fail(`config.source.${field} must be a full commit`);
  }
  for (const [field, value] of Object.entries(config.source.expected_sha256)) {
    if (!SHA256.test(value)) fail(`config.source.expected_sha256.${field} must be a SHA-256`);
  }
  if (!Array.isArray(config.measurement.run_order)) fail("measurement.run_order must be an array");
  const expectedLegs = requireInteger(
    config.measurement.expected_legs_per_group,
    "measurement.expected_legs_per_group",
    1,
  );
  if (config.measurement.run_order.length !== expectedLegs * 2) {
    fail("measurement.run_order has the wrong length");
  }
  const counts = { baseline: 0, candidate: 0 };
  const files = new Set();
  for (const leg of config.measurement.run_order) {
    if (!(leg.variant in counts)) fail("run-order variant must be baseline or candidate");
    if (typeof leg.file !== "string" || path.basename(leg.file) !== leg.file || !leg.file.endsWith(".json")) {
      fail("run-order files must be JSON basenames");
    }
    if (files.has(leg.file)) fail(`duplicate run-order file ${leg.file}`);
    files.add(leg.file);
    counts[leg.variant] += 1;
  }
  if (counts.baseline !== expectedLegs || counts.candidate !== expectedLegs) {
    fail("run-order group counts do not match expected legs");
  }
  requireInteger(config.measurement.fixed_n, "measurement.fixed_n", 1);
  requireInteger(config.measurement.repetitions, "measurement.repetitions", 1);
  requireInteger(config.qualification.candidate_pilot_legs, "qualification.candidate_pilot_legs", 2);
  requireInteger(config.host_gate.benchmark_cpu, "host_gate.benchmark_cpu", 0);
  requireInteger(config.host_gate.maximum_steal_ticks, "host_gate.maximum_steal_ticks", 0);
  if (config.schema === "rustia.linode-fixed-benchmark.v2") {
    requireNumber(
      config.host_gate.maximum_timed_steal_percent,
      "host_gate.maximum_timed_steal_percent",
      0,
    );
    if (
      config.host_gate.maximum_steal_ticks !== 0 ||
      config.host_gate.maximum_timed_steal_percent !== 0.1
    ) {
      fail("Linode v2 steal budget must remain zero while idle and 0.1% during a timed leg");
    }
  }
  if (!["acceptance", "diagnostic"].includes(config.publication.mode)) {
    fail("publication.mode must be acceptance or diagnostic");
  }
  requireInteger(config.publication.final_measurements, "publication.final_measurements", 3);
  return config;
}

function loadJson(file, description) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot load ${description} ${file}: ${error.message}`);
  }
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  return sha256(fs.readFileSync(file));
}

function commandText(command, args) {
  return [command, ...args]
    .map((value) => (/^[A-Za-z0-9_./:=+,-]+$/.test(value) ? value : JSON.stringify(value)))
    .join(" ");
}

function run(command, args, options = {}) {
  const {
    cwd = ROOT,
    env = process.env,
    capture = true,
    allowFailure = false,
    announce = true,
    input = undefined,
  } = options;
  if (announce) log(`+ ${commandText(command, args)}`);
  const result = spawnSync(command, args, {
    cwd,
    env,
    encoding: capture ? "utf8" : undefined,
    input,
    maxBuffer: 64 * 1024 * 1024,
    stdio: capture ? [input === undefined ? "ignore" : "pipe", "pipe", "pipe"] : "inherit",
  });
  if (result.error) fail(`cannot run ${command}: ${result.error.message}`);
  const status = result.status === null ? 1 : result.status;
  if (status !== 0 && !allowFailure) {
    const detail = capture && result.stderr ? String(result.stderr).trim() : "";
    fail(`${commandText(command, args)} failed with status ${status}${detail ? `: ${detail}` : ""}`);
  }
  return {
    status,
    stdout: capture ? String(result.stdout || "") : "",
    stderr: capture ? String(result.stderr || "") : "",
  };
}

function runText(command, args, options = {}) {
  return run(command, args, options).stdout.trim();
}

function expandCpuList(text, description) {
  const cpus = new Set();
  for (const token of String(text).trim().split(",")) {
    const match = token.match(/^(\d+)(?:-(\d+))?$/);
    if (!match) fail(`${description} has invalid CPU-list syntax`);
    const start = Number(match[1]);
    const end = match[2] === undefined ? start : Number(match[2]);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || end < start || end - start > 4096) {
      fail(`${description} has an invalid CPU range`);
    }
    for (let cpu = start; cpu <= end; cpu += 1) cpus.add(cpu);
  }
  return [...cpus].sort((left, right) => left - right);
}

function verifyBareMetalHost(config, provider) {
  requireObject(provider, "provider metadata");
  let profile;
  if (config.schema === "rustia.vultr-fixed-benchmark.v1") {
    profile = {
      label: "Vultr",
      expectedCpuModel: config.provider.size.cpu_model,
      expectedLogicalCpus: config.provider.size.vcpus,
      expectedPhysicalCpus: config.provider.size.physical_cpus,
      expectedPhysicalCores: config.provider.size.cpu_cores,
      metadataChecks: [
        ["provider", provider.provider, "vultr"],
        ["resource type", provider.resource_type, "bare_metal"],
        ["plan", provider.size_slug, config.provider.size.id],
        ["region", provider.region, config.provider.region],
        ["OS ID", provider.os_id, config.provider.os.id],
        ["logical CPU count", provider.vcpus, config.provider.size.vcpus],
      ],
    };
  } else if (config.schema === "rustia.scaleway-fixed-benchmark.v1") {
    profile = {
      label: "Scaleway",
      expectedCpuModel: config.provider.size.host_cpu_model,
      expectedLogicalCpus: config.provider.size.cpu_threads,
      expectedPhysicalCpus: config.provider.size.physical_cpus,
      expectedPhysicalCores: config.provider.size.cpu_cores,
      metadataChecks: [
        ["provider", provider.provider, "scaleway"],
        ["resource type", provider.resource_type, "elastic_metal"],
        ["offer", provider.offer_name, config.provider.size.name],
        ["logical CPU count", provider.vcpus, config.provider.size.cpu_threads],
      ],
    };
    if (!config.provider.zones.includes(provider.zone)) {
      fail(
        `Scaleway Elastic Metal provider zone was ${JSON.stringify(provider.zone)}, ` +
          `expected one of ${config.provider.zones.join(", ")}`,
      );
    }
    if (provider.os_name !== config.provider.os.name || provider.os_version !== config.provider.os.version) {
      fail(
        `Scaleway Elastic Metal OS was ${JSON.stringify(`${provider.os_name} ${provider.os_version}`)}, ` +
          `expected ${config.provider.os.name} ${config.provider.os.version}`,
      );
    }
  } else {
    return null;
  }
  for (const [field, actual, expected] of profile.metadataChecks) {
    if (String(actual) !== String(expected)) {
      fail(
        `${profile.label} bare-metal provider ${field} was ${JSON.stringify(actual)}, ` +
          `expected ${JSON.stringify(expected)}`,
      );
    }
  }

  const detected = run("systemd-detect-virt", [], { allowFailure: true });
  if (detected.status !== 1 || detected.stdout.trim() !== "none") {
    fail(
      `${profile.label} resource failed bare-metal virtualization attestation: systemd-detect-virt ` +
        `reported ${JSON.stringify(detected.stdout.trim())} with status ${detected.status}`,
    );
  }
  const cpuInfo = fs.readFileSync("/proc/cpuinfo", "utf8");
  if (/^flags\s*:.*\bhypervisor\b/m.test(cpuInfo)) {
    fail(`${profile.label} resource failed bare-metal attestation: /proc/cpuinfo exposes the hypervisor flag`);
  }
  const cpuModels = [...cpuInfo.matchAll(/^model name\s*:\s*(.+)$/gm)].map((match) => match[1].trim());
  if (
    cpuModels.length !== profile.expectedLogicalCpus ||
    new Set(cpuModels).size !== 1 ||
    !cpuModels[0].includes(profile.expectedCpuModel)
  ) {
    fail(
      `${profile.label} bare-metal CPU model rows did not exactly identify one ${profile.expectedCpuModel} ` +
        `across ${profile.expectedLogicalCpus} logical CPUs`,
    );
  }
  const hypervisorType = "/sys/hypervisor/type";
  if (fs.existsSync(hypervisorType) && fs.readFileSync(hypervisorType, "utf8").trim() !== "") {
    fail(`${profile.label} resource failed bare-metal attestation: /sys/hypervisor/type is populated`);
  }

  const online = expandCpuList(fs.readFileSync("/sys/devices/system/cpu/online", "utf8"), "online CPU list");
  const expectedOnline = Array.from({ length: profile.expectedLogicalCpus }, (_, cpu) => cpu);
  if (JSON.stringify(online) !== JSON.stringify(expectedOnline)) {
    fail(`${profile.label} bare metal exposed CPUs ${online.join(",")}, expected ${expectedOnline.join(",")}`);
  }
  const sockets = new Set();
  const cores = new Set();
  const topology = {};
  const expectedSiblings = profile.expectedLogicalCpus / profile.expectedPhysicalCores;
  if (!Number.isSafeInteger(expectedSiblings) || expectedSiblings < 1) {
    fail(`configured ${profile.label} CPU thread/core ratio is invalid`);
  }
  for (const cpu of online) {
    const root = `/sys/devices/system/cpu/cpu${cpu}/topology`;
    const socket = fs.readFileSync(`${root}/physical_package_id`, "utf8").trim();
    const core = fs.readFileSync(`${root}/core_id`, "utf8").trim();
    const siblings = expandCpuList(fs.readFileSync(`${root}/thread_siblings_list`, "utf8"), `CPU ${cpu} siblings`);
    if (
      siblings.length !== expectedSiblings ||
      !siblings.includes(cpu) ||
      siblings.some((sibling) => !online.includes(sibling))
    ) {
      fail(`CPU ${cpu} exposes a malformed SMT sibling set ${siblings.join(",")}`);
    }
    sockets.add(socket);
    cores.add(`${socket}:${core}`);
    topology[`cpu${cpu}`] = { socket, core, siblings };
  }
  if (sockets.size !== profile.expectedPhysicalCpus || cores.size !== profile.expectedPhysicalCores) {
    fail(
      `${profile.label} bare-metal topology exposed ${sockets.size} socket(s) and ${cores.size} core(s), expected ` +
        `${profile.expectedPhysicalCpus} and ${profile.expectedPhysicalCores}`,
    );
  }
  for (const [name, entry] of Object.entries(topology)) {
    for (const sibling of entry.siblings) {
      const peer = topology[`cpu${sibling}`];
      if (
        !peer ||
        peer.socket !== entry.socket ||
        peer.core !== entry.core ||
        JSON.stringify(peer.siblings) !== JSON.stringify(entry.siblings)
      ) {
        fail(`${name} has an asymmetric or cross-core SMT sibling set`);
      }
    }
  }
  const benchmarkCpu = config.host_gate.benchmark_cpu;
  if (topology.cpu0.siblings.includes(benchmarkCpu)) {
    fail(`controller CPU 0 and benchmark CPU ${benchmarkCpu} are SMT siblings`);
  }
  return {
    systemd_detect_virt: detected.stdout.trim(),
    systemd_detect_virt_status: detected.status,
    hypervisor_cpu_flag: false,
    sys_hypervisor_type: "absent_or_empty",
    logical_cpus: online.length,
    physical_sockets: sockets.size,
    physical_cores: cores.size,
    cpu_model: cpuModels[0],
    controller_cpu: 0,
    benchmark_cpu: benchmarkCpu,
    controller_and_benchmark_are_smt_siblings: false,
    topology,
  };
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function readCpuSampleFromText(text, cpu) {
  const prefix = `cpu${cpu} `;
  const line = text.split("\n").find((entry) => entry.startsWith(prefix));
  if (!line) fail(`/proc/stat does not expose benchmark CPU ${cpu}`);
  const fields = line.trim().split(/\s+/).slice(1).map(Number);
  if (fields.length < 8 || fields.some((value) => !Number.isFinite(value))) {
    fail(`invalid /proc/stat row for CPU ${cpu}`);
  }
  return {
    fields,
    idle: fields[3] + (fields[4] || 0),
    steal: fields[7] || 0,
    total: fields.reduce((sum, value) => sum + value, 0),
  };
}

function readCpuSample(cpu) {
  return readCpuSampleFromText(fs.readFileSync("/proc/stat", "utf8"), cpu);
}

function cpuDelta(before, after) {
  const total = after.total - before.total;
  const idle = after.idle - before.idle;
  const stealTicks = after.steal - before.steal;
  if (total <= 0 || idle < 0 || stealTicks < 0) fail("non-monotonic CPU counters");
  return {
    total_ticks: total,
    idle_ticks: idle,
    idle_percent: Number(((idle / total) * 100).toFixed(6)),
    steal_ticks: stealTicks,
  };
}

function timedStealGatePasses(config, delta) {
  if (config.schema === "rustia.linode-fixed-benchmark.v2") {
    return (
      delta.steal_ticks * 100 <=
      config.host_gate.maximum_timed_steal_percent * delta.total_ticks
    );
  }
  return delta.steal_ticks <= config.host_gate.maximum_steal_ticks;
}

function loadOneMinute() {
  const value = Number(fs.readFileSync("/proc/loadavg", "utf8").trim().split(/\s+/)[0]);
  if (!Number.isFinite(value)) fail("invalid /proc/loadavg");
  return value;
}

function cpuAllowedListFromStatus(text) {
  const match = text.match(/^Cpus_allowed_list:\s*(\S+)\s*$/m);
  if (!match) fail("missing Cpus_allowed_list in process status");
  return match[1];
}

function processThreadIds(pid) {
  const taskDirectory = `/proc/${pid}/task`;
  return fs
    .readdirSync(taskDirectory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && /^[0-9]+$/.test(entry.name))
    .map((entry) => Number(entry.name))
    .sort((left, right) => left - right);
}

function sameThreadIds(left, right) {
  return left.length === right.length && left.every((tid, index) => tid === right[index]);
}

function stableProcessThreadAffinities(pid, maximumAttempts = 5) {
  const taskDirectory = `/proc/${pid}/task`;
  for (let attempt = 1; attempt <= maximumAttempts; attempt += 1) {
    const before = processThreadIds(pid);
    if (before.length === 0) continue;
    const records = [];
    let vanished = false;
    for (const tid of before) {
      try {
        const status = fs.readFileSync(path.join(taskDirectory, String(tid), "status"), "utf8");
        records.push({ tid, cpus_allowed_list: cpuAllowedListFromStatus(status) });
      } catch (error) {
        if (error && error.code === "ENOENT") {
          vanished = true;
          break;
        }
        throw error;
      }
    }
    const after = processThreadIds(pid);
    if (!vanished && sameThreadIds(before, after)) return records;
  }
  return null;
}

function mismatchedThreadAffinities(records, cpu) {
  return records.filter((record) => record.cpus_allowed_list !== String(cpu));
}

function pinControllerThreads(cpu) {
  let lastPin = null;
  let lastAffinities = null;
  for (let attempt = 1; attempt <= 3; attempt += 1) {
    lastPin = run("taskset", ["-apc", String(cpu), String(process.pid)], {
      allowFailure: true,
    });
    lastAffinities = stableProcessThreadAffinities(process.pid);
    if (lastAffinities === null) continue;
    const mismatched = mismatchedThreadAffinities(lastAffinities, cpu);
    if (mismatched.length > 0) continue;
    const transient = lastPin.status === 0 ? "" : ` after taskset status ${lastPin.status}`;
    log(
      `verified stable set of ${lastAffinities.length} controller threads on CPU ${cpu}${transient}`,
    );
    return;
  }
  const affinityDetail =
    lastAffinities === null
      ? "no stable nonempty thread snapshot"
      : mismatchedThreadAffinities(lastAffinities, cpu)
          .map((record) => `${record.tid}:${record.cpus_allowed_list}`)
          .join(", ");
  const tasksetDetail = lastPin && lastPin.stderr.trim() ? `: ${lastPin.stderr.trim()}` : "";
  fail(
    `controller affinity verification failed after taskset status ${lastPin?.status}: ` +
      `${affinityDetail}${tasksetDetail}`,
  );
}

function contaminatingProcesses(excludedPids = new Set()) {
  const records = [];
  for (const entry of fs.readdirSync("/proc", { withFileTypes: true })) {
    if (!entry.isDirectory() || !/^[0-9]+$/.test(entry.name)) continue;
    const pid = Number(entry.name);
    if (pid === process.pid || excludedPids.has(pid)) continue;
    try {
      const comm = fs.readFileSync(`/proc/${pid}/comm`, "utf8").trim();
      if (!CONTAMINATING_PROCESS.test(comm)) continue;
      const command = fs
        .readFileSync(`/proc/${pid}/cmdline`, "utf8")
        .replace(/\0/g, " ")
        .trim();
      records.push({ pid, comm, command });
    } catch {
      // The process exited while /proc was being scanned.
    }
  }
  return records;
}

function appendGateRecord(file, records, record) {
  records.push(record);
  fs.appendFileSync(file, `${JSON.stringify(record)}\n`);
}

async function idleSample(config, label) {
  const cpu = config.host_gate.benchmark_cpu;
  const before = readCpuSample(cpu);
  const beforeProcesses = contaminatingProcesses();
  await sleep(config.host_gate.idle_window_seconds * 1000);
  const after = readCpuSample(cpu);
  const afterProcesses = contaminatingProcesses();
  return {
    phase: "pre_leg_idle_window",
    label,
    timestamp: new Date().toISOString(),
    cpu,
    load_1m: loadOneMinute(),
    cpu_delta: cpuDelta(before, after),
    contaminants: [...beforeProcesses, ...afterProcesses],
  };
}

function gatePasses(config, sample) {
  return (
    sample.load_1m <= config.host_gate.maximum_load_1m &&
    sample.cpu_delta.idle_percent >= config.host_gate.minimum_idle_percent &&
    sample.cpu_delta.steal_ticks <= config.host_gate.maximum_steal_ticks &&
    sample.contaminants.length === 0
  );
}

async function waitForIdle(config, label, gateFile, gateRecords) {
  log(`${label}: fixed ${config.host_gate.cooldown_seconds}s cooldown`);
  await sleep(config.host_gate.cooldown_seconds * 1000);
  const deadline = Date.now() + config.host_gate.wait_timeout_seconds * 1000;
  while (Date.now() < deadline) {
    const sample = await idleSample(config, label);
    sample.passed = gatePasses(config, sample);
    appendGateRecord(gateFile, gateRecords, sample);
    if (sample.passed) {
      log(
        `${label}: idle gate passed (load1=${sample.load_1m}, ` +
          `cpu${sample.cpu} idle=${sample.cpu_delta.idle_percent}%, ` +
          `steal=${sample.cpu_delta.steal_ticks})`,
      );
      return sample;
    }
    log(
      `${label}: idle gate not ready (load1=${sample.load_1m}, ` +
        `idle=${sample.cpu_delta.idle_percent}%, steal=${sample.cpu_delta.steal_ticks}, ` +
        `contaminants=${sample.contaminants.length}); retrying`,
    );
    await sleep(5000);
  }
  throw new ControlledStop("host_qualification_failed", `${label}: idle gate timed out`);
}

async function runMonitoredBenchmark({
  config,
  label,
  binary,
  args,
  cwd,
  gateFile,
  gateRecords,
  acceptedBlock,
}) {
  await waitForIdle(config, label, gateFile, gateRecords);
  const cpu = config.host_gate.benchmark_cpu;
  const before = readCpuSample(cpu);
  const observed = [];
  log(`+ taskset -c ${cpu} ${commandText(binary, args)}`);
  const result = await new Promise((resolve, reject) => {
    const child = spawn("taskset", ["-c", String(cpu), binary, ...args], {
      cwd,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
    child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
    const timer = setInterval(() => {
      const contaminants = contaminatingProcesses(new Set([child.pid]));
      if (contaminants.length > 0) observed.push(...contaminants);
    }, 250);
    child.on("error", (error) => {
      clearInterval(timer);
      reject(error);
    });
    child.on("close", (status, signal) => {
      clearInterval(timer);
      resolve({
        status: status === null ? 1 : status,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    });
  });
  if (result.stdout) process.stdout.write(result.stdout);
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.status !== 0) {
    fail(`${label}: benchmark command failed with status ${result.status}`);
  }
  const after = readCpuSample(cpu);
  const postProcesses = contaminatingProcesses();
  const during = {
    phase: "during_leg",
    label,
    timestamp: new Date().toISOString(),
    cpu,
    cpu_delta: cpuDelta(before, after),
    contaminants: [...observed, ...postProcesses],
    passed: false,
  };
  during.passed =
    timedStealGatePasses(config, during.cpu_delta) &&
    during.contaminants.length === 0;
  appendGateRecord(gateFile, gateRecords, during);
  if (!during.passed) {
    const status = acceptedBlock ? "accepted_block_invalid" : "host_qualification_failed";
    throw new ControlledStop(
      status,
      `${label}: contamination or steal violated the predeclared gate; the whole block is discarded`,
    );
  }
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

function matchesHistoryPrecision(serialized, raw) {
  return (
    typeof serialized === "number" &&
    Number.isFinite(serialized) &&
    typeof raw === "number" &&
    Number.isFinite(raw) &&
    Math.abs(serialized - raw) <= 0.500001e-6
  );
}

function sidecarSummary(file, expected) {
  const value = loadJson(file, "raw sample sidecar");
  if (value.schema !== "rustia.bench.raw-samples.v1" || value.schema_version !== 1) {
    fail(`${file} has the wrong raw-sample schema`);
  }
  if (!Array.isArray(value.measurements) || value.measurements.length !== 1) {
    fail(`${file} must contain exactly one measurement`);
  }
  const measurement = value.measurements[0];
  for (const [field, wanted] of Object.entries(expected)) {
    if (measurement[field] !== wanted) {
      fail(`${file} measurement.${field} is ${JSON.stringify(measurement[field])}, expected ${JSON.stringify(wanted)}`);
    }
  }
  if (!Array.isArray(measurement.samples_ns_per_unit) || measurement.samples_ns_per_unit.length !== measurement.reps) {
    fail(`${file} has an invalid sample vector`);
  }
  const samples = measurement.samples_ns_per_unit.slice().sort((left, right) => left - right);
  if (samples.some((sample) => typeof sample !== "number" || !Number.isFinite(sample) || sample <= 0)) {
    fail(`${file} contains invalid samples`);
  }
  return {
    file: path.basename(file),
    sha256: sha256File(file),
    timestamp_unix: measurement.timestamp_unix,
    commit: measurement.commit,
    target: measurement.target,
    engine: measurement.engine,
    dialect: measurement.dialect,
    workload: measurement.workload,
    unit: measurement.unit,
    n: measurement.n,
    reps: measurement.reps,
    checksum: measurement.checksum,
    ns_per_unit_min: measurement.ns_per_unit_min,
    ns_per_unit_median: measurement.ns_per_unit_median,
    units_per_sec: measurement.units_per_sec,
    sample_count: samples.length,
    min: samples[0],
    q1: quantileSorted(samples, 0.25),
    median: quantileSorted(samples, 0.5),
    harness_median: measurement.ns_per_unit_median,
    q3: quantileSorted(samples, 0.75),
    max: samples[samples.length - 1],
    iqr: quantileSorted(samples, 0.75) - quantileSorted(samples, 0.25),
  };
}

function commonArgs(config) {
  return [
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
  ];
}

function benchmarkEnvironment(targetDirectory) {
  const env = { ...process.env, CARGO_INCREMENTAL: "0", CARGO_TARGET_DIR: targetDirectory };
  delete env.RUSTC_WRAPPER;
  return env;
}

function installAndBuild(config, bundle) {
  log("disabling periodic package activity before the quiet measurement phase");
  for (const unit of PACKAGE_ACTIVITY_UNITS) {
    run("systemctl", ["stop", unit], { allowFailure: true });
    run("systemctl", ["mask", unit], { allowFailure: true });
  }

  if (!fs.existsSync("/root/.cargo/bin/rustup")) {
    const installer = path.join(ROOT, "rustup-init.sh");
    run("curl", ["--proto", "=https", "--tlsv1.2", "-sSf", "https://sh.rustup.rs", "-o", installer]);
    run("sh", [installer, "-y", "--profile", "minimal", "--default-toolchain", "none"], {
      capture: false,
    });
  }
  const rustup = "/root/.cargo/bin/rustup";
  const cargo = "/root/.cargo/bin/cargo";
  run(rustup, ["toolchain", "install", "1.95.0", "--profile", "minimal"], { capture: false });

  run("git", ["clone", "--bare", bundle, REPOSITORY], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "bundle", "verify", bundle], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", SOURCE_A, config.source.baseline_commit], {
    capture: false,
  });
  run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", SOURCE_B, config.source.candidate_commit], {
    capture: false,
  });
  assert.equal(runText("git", ["-C", SOURCE_A, "rev-parse", "HEAD"]), config.source.baseline_commit);
  assert.equal(runText("git", ["-C", SOURCE_B, "rev-parse", "HEAD"]), config.source.candidate_commit);

  const originalHarnessHash = sha256File(path.join(SOURCE_A, "bench/src/main.rs"));
  fs.copyFileSync(path.join(SOURCE_B, "bench/src/main.rs"), path.join(SOURCE_A, "bench/src/main.rs"));
  const overlayHarnessHash = sha256File(path.join(SOURCE_A, "bench/src/main.rs"));
  const expected = config.source.expected_sha256;
  assert.equal(originalHarnessHash, expected.baseline_original_harness);
  assert.equal(overlayHarnessHash, expected.overlay_harness);
  assert.equal(sha256File(path.join(SOURCE_B, "bench/src/main.rs")), expected.overlay_harness);
  for (const source of [SOURCE_A, SOURCE_B]) {
    assert.equal(sha256File(path.join(source, config.measurement.workload_path)), expected.workload);
    assert.equal(sha256File(path.join(source, "Cargo.lock")), expected.cargo_lock);
    assert.equal(sha256File(path.join(source, "rust-toolchain.toml")), expected.toolchain);
  }
  assert.equal(sha256File(path.join(SOURCE_B, "bench/analyze-fixed-samples.js")), expected.analyzer);
  assert.equal(sha256File(path.join(SOURCE_B, "bench/history/runs.jsonl")), expected.history_before);

  run(cargo, ["+1.95.0", "fetch", "--locked", "--manifest-path", path.join(SOURCE_B, "Cargo.toml")], {
    env: benchmarkEnvironment(TARGET_B),
    capture: false,
  });
  const buildArgs = (source) => [
    "+1.95.0",
    "build",
    "--locked",
    "--offline",
    "--manifest-path",
    path.join(source, "Cargo.toml"),
    "-p",
    "script-bench-harness",
    "--bin",
    "script-bench",
    "--release",
  ];
  run(cargo, buildArgs(SOURCE_A), { env: benchmarkEnvironment(TARGET_A), capture: false });
  run(cargo, buildArgs(SOURCE_B), { env: benchmarkEnvironment(TARGET_B), capture: false });

  const baselineBinary = path.join(TARGET_A, "release/script-bench");
  const candidateBinary = path.join(TARGET_B, "release/script-bench");
  for (const binary of [baselineBinary, candidateBinary]) {
    fs.accessSync(binary, fs.constants.X_OK);
  }
  return {
    cargo,
    baselineBinary,
    candidateBinary,
    originalHarnessHash,
    overlayHarnessHash,
    bundleHash: sha256File(bundle),
    baselineBinaryHash: sha256File(baselineBinary),
    candidateBinaryHash: sha256File(candidateBinary),
    baselineBinaryBytes: fs.statSync(baselineBinary).size,
    candidateBinaryBytes: fs.statSync(candidateBinary).size,
    buildCommands: {
      baseline: [cargo, ...buildArgs(SOURCE_A)],
      candidate: [cargo, ...buildArgs(SOURCE_B)],
    },
  };
}

function collectHost(config, build) {
  const readIfPresent = (file) => (fs.existsSync(file) ? fs.readFileSync(file, "utf8").trim() : "not_exposed");
  const governorRoot = "/sys/devices/system/cpu";
  const governors = {};
  const logicalCpus = config.schema === "rustia.scaleway-fixed-benchmark.v1"
    ? config.provider.size.cpu_threads
    : config.provider.size.vcpus;
  for (let cpu = 0; cpu < logicalCpus; cpu += 1) {
    governors[`cpu${cpu}`] = readIfPresent(`${governorRoot}/cpu${cpu}/cpufreq/scaling_governor`);
  }
  const cpuInfo = fs.readFileSync("/proc/cpuinfo", "utf8");
  const model = cpuInfo.match(/^model name\s*:\s*(.+)$/m);
  const microcode = cpuInfo.match(/^microcode\s*:\s*(.+)$/m);
  return {
    captured_at: new Date().toISOString(),
    uname: runText("uname", ["-a"]),
    os_release: fs.readFileSync("/etc/os-release", "utf8"),
    lscpu: runText("lscpu", ["--json"]),
    virtualization: runText("systemd-detect-virt", [], { allowFailure: true }),
    cpu_model: model ? model[1].trim() : "unknown",
    microcode: microcode ? microcode[1].trim() : "not_exposed",
    cpuinfo_sha256: sha256(Buffer.from(cpuInfo)),
    clocksource: readIfPresent("/sys/devices/system/clocksource/clocksource0/current_clocksource"),
    available_clocksources: readIfPresent("/sys/devices/system/clocksource/clocksource0/available_clocksource"),
    scaling_governors: governors,
    node_version: process.version,
    rustc_vv: runText("/root/.cargo/bin/rustc", ["+1.95.0", "-vV"]),
    cargo_version: runText(build.cargo, ["+1.95.0", "--version"]),
    taskset_version: runText("taskset", ["--version"]).split("\n")[0],
    systemd_timers: runText("systemctl", ["list-timers", "--all", "--no-pager"], { allowFailure: true }),
  };
}

async function runPilot(config, build, gateFile, gateRecords) {
  fs.mkdirSync(PILOT, { recursive: true });
  const summaries = [];
  for (let index = 1; index <= config.qualification.candidate_pilot_legs; index += 1) {
    const file = path.join(PILOT, `candidate-pilot-${index}.json`);
    await runMonitoredBenchmark({
      config,
      label: `qualification-${index}`,
      binary: build.candidateBinary,
      args: [
        ...commonArgs(config),
        "--commit",
        config.source.candidate_commit,
        "--samples-json",
        file,
      ],
      cwd: SOURCE_B,
      gateFile,
      gateRecords,
      acceptedBlock: false,
    });
    summaries.push(
      sidecarSummary(file, {
        commit: config.source.candidate_commit,
        target: "linux-x86_64",
        engine: "rustia",
        workload: config.measurement.workload,
        n: config.measurement.fixed_n,
        n_selection: "fixed",
        reps: config.measurement.repetitions,
        checksum: config.measurement.expected_checksum,
      }),
    );
  }
  const medians = summaries.map((summary) => summary.harness_median);
  const spread = ((Math.max(...medians) - Math.min(...medians)) / Math.min(...medians)) * 100;
  const result = {
    schema: "rustia.bench.cloud-qualification.v1",
    pilot_legs: summaries,
    median_spread_percent: spread,
    maximum_median_spread_percent: config.qualification.max_median_spread_percent,
    passed: spread <= config.qualification.max_median_spread_percent,
    note: "qualification-only runs; never accepted as A/B evidence or public history",
  };
  writeJson(path.join(PILOT, "qualification.json"), result);
  if (!result.passed) {
    throw new ControlledStop(
      "host_qualification_failed",
      `candidate pilot median spread ${spread.toFixed(3)}% exceeded ` +
        `${config.qualification.max_median_spread_percent}%`,
    );
  }
  return result;
}

async function runComparison(config, build, evidenceDirectory, gateFile, gateRecords) {
  const summaries = [];
  for (const [index, leg] of config.measurement.run_order.entries()) {
    const baseline = leg.variant === "baseline";
    const binary = baseline ? build.baselineBinary : build.candidateBinary;
    const commit = baseline ? config.source.baseline_commit : config.source.candidate_commit;
    const relativeFile = path.join("bench/history/evidence", config.source.candidate_commit, leg.file);
    await runMonitoredBenchmark({
      config,
      label: `accepted-${index + 1}-${leg.file}`,
      binary,
      args: [...commonArgs(config), "--commit", commit, "--samples-json", relativeFile],
      cwd: SOURCE_B,
      gateFile,
      gateRecords,
      acceptedBlock: true,
    });
    summaries.push({
      variant: leg.variant,
      ...sidecarSummary(path.join(SOURCE_B, relativeFile), {
        commit,
        target: "linux-x86_64",
        engine: "rustia",
        workload: config.measurement.workload,
        n: config.measurement.fixed_n,
        n_selection: "fixed",
        reps: config.measurement.repetitions,
        checksum: config.measurement.expected_checksum,
      }),
    });
  }

  const baselineFiles = config.measurement.run_order
    .filter((leg) => leg.variant === "baseline")
    .flatMap((leg) => ["--baseline", path.join("bench/history/evidence", config.source.candidate_commit, leg.file)]);
  const candidateFiles = config.measurement.run_order
    .filter((leg) => leg.variant === "candidate")
    .flatMap((leg) => ["--candidate", path.join("bench/history/evidence", config.source.candidate_commit, leg.file)]);
  const analyzerArgs = [
    "bench/analyze-fixed-samples.js",
    ...baselineFiles,
    ...candidateFiles,
    "--bootstrap-reps",
    String(config.measurement.bootstrap_repetitions),
    "--seed",
    String(config.measurement.bootstrap_seed),
    "--material-threshold",
    String(config.measurement.material_regression_threshold_percent),
    "--expected-legs",
    String(config.measurement.expected_legs_per_group),
    "--require-full-commits",
  ];
  const analysisResult = run("node", analyzerArgs, { cwd: SOURCE_B });
  const analysisFile = path.join(evidenceDirectory, "analysis.json");
  fs.writeFileSync(analysisFile, analysisResult.stdout);
  const analysis = loadJson(analysisFile, "fixed-sample analysis");
  if (analysis.identity.target !== "linux-x86_64") fail("analysis target is not linux-x86_64");
  return { summaries, analysis, analyzerCommand: ["node", ...analyzerArgs] };
}

async function publishFinal(config, build, evidenceDirectory, gateFile, gateRecords) {
  const history = path.join(SOURCE_B, "bench/history/runs.jsonl");
  const beforeBytes = fs.readFileSync(history);
  const beforeHash = sha256(beforeBytes);
  assert.equal(beforeHash, config.source.expected_sha256.history_before);
  const beforeLines = beforeBytes.toString("utf8").trimEnd().split("\n").length;
  const summaries = [];
  const commands = [];
  const candidateHash = sha256File(build.candidateBinary);
  assert.equal(candidateHash, build.candidateBinaryHash);

  for (let index = 1; index <= config.publication.final_measurements; index += 1) {
    const relativeFile = path.join(
      "bench/history/evidence",
      config.source.candidate_commit,
      `final-dense-${index}.json`,
    );
    const args = [
      "--engines",
      "rustia",
      "--workloads",
      config.measurement.workload,
      "--fixed-n",
      String(config.measurement.fixed_n),
      "--reps",
      String(config.measurement.repetitions),
      "--history",
      "bench/history/runs.jsonl",
      "--no-results",
      "--no-latest-json",
      "--commit",
      config.source.candidate_commit,
      "--samples-json",
      relativeFile,
    ];
    await runMonitoredBenchmark({
      config,
      label: `publication-${index}`,
      binary: build.candidateBinary,
      args,
      cwd: SOURCE_B,
      gateFile,
      gateRecords,
      acceptedBlock: true,
    });
    commands.push(["taskset", "-c", String(config.host_gate.benchmark_cpu), build.candidateBinary, ...args]);
    summaries.push(
      sidecarSummary(path.join(SOURCE_B, relativeFile), {
        commit: config.source.candidate_commit,
        target: "linux-x86_64",
        engine: "rustia",
        workload: config.measurement.workload,
        n: config.measurement.fixed_n,
        n_selection: "fixed",
        reps: config.measurement.repetitions,
        checksum: config.measurement.expected_checksum,
      }),
    );
    assert.equal(sha256File(build.candidateBinary), build.candidateBinaryHash);
  }

  run(build.candidateBinary, ["--check-history", "--history", "bench/history/runs.jsonl"], {
    cwd: SOURCE_B,
    capture: false,
  });
  const afterBytes = fs.readFileSync(history);
  if (!afterBytes.subarray(0, beforeBytes.length).equals(beforeBytes)) {
    fail("history publication changed existing bytes instead of appending");
  }
  const rows = afterBytes.toString("utf8").trimEnd().split("\n").map((line) => JSON.parse(line));
  if (rows.length - beforeLines !== config.publication.final_measurements) {
    fail("history publication appended the wrong number of rows");
  }
  const appendedRows = rows.slice(-config.publication.final_measurements);
  for (let index = 0; index < appendedRows.length; index += 1) {
    const row = appendedRows[index];
    const summary = summaries[index];
    if (
      row.timestamp_unix !== summary.timestamp_unix ||
      row.commit !== config.source.candidate_commit ||
      row.target !== "linux-x86_64" ||
      row.engine !== summary.engine ||
      row.dialect !== summary.dialect ||
      row.workload !== config.measurement.workload ||
      row.unit !== summary.unit ||
      row.n !== summary.n ||
      row.reps !== summary.reps ||
      !matchesHistoryPrecision(row.ns_per_unit_min, summary.ns_per_unit_min) ||
      !matchesHistoryPrecision(row.ns_per_unit_median, summary.harness_median) ||
      !matchesHistoryPrecision(row.units_per_sec, summary.units_per_sec) ||
      row.checksum !== config.measurement.expected_checksum
    ) {
      fail(`published history row ${index + 1} does not match its raw sidecar`);
    }
  }
  return {
    history_before_sha256: beforeHash,
    history_after_sha256: sha256(afterBytes),
    history_lines_before: beforeLines,
    history_lines_after: rows.length,
    sidecars: summaries,
    appended_rows: appendedRows,
    commands,
  };
}

function evidenceFileHashes(evidenceDirectory) {
  const output = {};
  const visit = (directory, prefix = "") => {
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      const absolute = path.join(directory, entry.name);
      if (entry.isDirectory()) {
        visit(absolute, relative);
      } else if (entry.isFile() && relative !== "manifest.json") {
        output[relative] = sha256File(absolute);
      }
    }
  };
  visit(evidenceDirectory);
  return output;
}

function writeManifest({
  config,
  configFile,
  provider,
  providerFile,
  bundle,
  build,
  host,
  hostFile,
  gateFile,
  gateRecords,
  qualification,
  comparison,
  publication,
  evidenceDirectory,
}) {
  fs.copyFileSync(RUN_LOG, path.join(evidenceDirectory, "remote-run.log"));
  const manifest = {
    schema_version: 1,
    schema: "rustia.bench.cloud-evidence.v1",
    created_at: new Date().toISOString(),
    provider,
    source: {
      baseline_commit: config.source.baseline_commit,
      candidate_commit: config.source.candidate_commit,
      harness_commit: config.source.harness_commit,
      baseline_original_harness_sha256: build.originalHarnessHash,
      baseline_overlay_harness_sha256: build.overlayHarnessHash,
      baseline_overlay_disclosure:
        "bench/src/main.rs from the harness commit was copied byte-for-byte over the baseline source before compilation",
      workload_sha256: config.source.expected_sha256.workload,
      cargo_lock_sha256: config.source.expected_sha256.cargo_lock,
      toolchain_sha256: config.source.expected_sha256.toolchain,
      analyzer_sha256: config.source.expected_sha256.analyzer,
      git_bundle_sha256: build.bundleHash,
      protocol_config_sha256: sha256File(configFile),
      provider_metadata_sha256: sha256File(providerFile),
    },
    build: {
      rustc_vv: host.rustc_vv,
      cargo_version: host.cargo_version,
      baseline_binary_sha256: build.baselineBinaryHash,
      baseline_binary_bytes: build.baselineBinaryBytes,
      candidate_binary_sha256: build.candidateBinaryHash,
      candidate_binary_bytes: build.candidateBinaryBytes,
      baseline_command: build.buildCommands.baseline,
      candidate_command: build.buildCommands.candidate,
      cargo_incremental: "0",
      rustc_wrapper: "unset",
      locked: true,
      offline_build_after_fetch: true,
      source_paths_retained_through_all_measurements: [SOURCE_A, SOURCE_B],
    },
    host: {
      file: "host.json",
      sha256: sha256File(hostFile),
      cpu_model: host.cpu_model,
      microcode: host.microcode,
      clocksource: host.clocksource,
      scaling_governors: host.scaling_governors,
      benchmark_cpu: config.host_gate.benchmark_cpu,
    },
    protocol: {
      workload: config.measurement.workload,
      fixed_n: config.measurement.fixed_n,
      repetitions_per_leg: config.measurement.repetitions,
      order: config.measurement.run_order,
      common_arguments: commonArgs(config),
      bootstrap_repetitions: config.measurement.bootstrap_repetitions,
      bootstrap_seed: config.measurement.bootstrap_seed,
      material_regression_threshold_percent: config.measurement.material_regression_threshold_percent,
      required_verdict: config.publication.required_verdict,
      host_gate: config.host_gate,
      contaminating_process_comm_regex: CONTAMINATING_PROCESS.source,
      exclusion_rule:
        "any pre/during-leg host-gate violation discards the whole eight-leg block; no leg is selected by timing outcome",
      gate_log: { file: "gate.jsonl", sha256: sha256File(gateFile), records: gateRecords.length },
    },
    qualification,
    comparison: {
      legs: comparison.summaries,
      analyzer_command: comparison.analyzerCommand,
      analysis_sha256: sha256File(path.join(evidenceDirectory, "analysis.json")),
      candidate_delta_percent: comparison.analysis.comparison.candidate_delta_percent,
      pooled_bootstrap_ci_95_percent: comparison.analysis.comparison.pooled_bootstrap_ci_95_percent,
      hierarchical_bootstrap_ci_95_percent:
        comparison.analysis.comparison.hierarchical_bootstrap_ci_95_percent,
      conservative_bootstrap_ci_95_percent: comparison.analysis.comparison.bootstrap_ci_95_percent,
      verdict: comparison.analysis.comparison.verdict,
    },
    publication,
    evidence_sha256: evidenceFileHashes(evidenceDirectory),
  };
  writeJson(path.join(evidenceDirectory, "manifest.json"), manifest);
  return manifest;
}

function exportRun(status, config, evidenceDirectory) {
  fs.rmSync(EXPORT, { recursive: true, force: true });
  fs.mkdirSync(EXPORT, { recursive: true });
  writeJson(path.join(EXPORT, "status.json"), status);
  if (fs.existsSync(RUN_LOG)) fs.copyFileSync(RUN_LOG, path.join(EXPORT, "run.log"));
  if (evidenceDirectory && fs.existsSync(evidenceDirectory)) {
    const destination = path.join(EXPORT, "evidence", config.source.candidate_commit);
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.cpSync(evidenceDirectory, destination, { recursive: true });
  }
  if (status.status === "accepted_and_published") {
    const history = path.join(SOURCE_B, "bench/history/runs.jsonl");
    const destination = path.join(EXPORT, "publication/bench/history/runs.jsonl");
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.copyFileSync(history, destination);
  }
  fs.rmSync(EXPORT_ARCHIVE, { force: true });
  run("tar", ["--format=ustar", "-czf", EXPORT_ARCHIVE, "-C", EXPORT, "."], { capture: false });
}

async function execute(configFile, providerFile, bundle) {
  if (process.platform !== "linux" || process.getuid() !== 0) {
    fail("remote benchmark runner must execute as root on Linux");
  }
  fs.mkdirSync(ROOT, { recursive: true });
  fs.writeFileSync(RUN_LOG, "", { mode: 0o600 });
  logReady = true;
  const config = validateConfig(loadJson(configFile, "protocol config"));
  const provider = loadJson(providerFile, "provider metadata");
  let evidenceDirectory = null;
  let status = {
    schema: "rustia.bench.cloud-run-status.v1",
    status: "failed",
    detail: "runner did not reach a terminal protocol state",
    started_at: new Date().toISOString(),
  };
  const executionStarted = Date.now();
  const phaseTimings = {};

  try {
    const bareMetalAttestation = verifyBareMetalHost(config, provider);
    if (bareMetalAttestation) log(`${provider.provider} physical-host topology attestation passed`);
    log("preparing exact source trees and frozen release binaries");
    const buildStarted = Date.now();
    const build = installAndBuild(config, bundle);
    phaseTimings.build_seconds = Number(((Date.now() - buildStarted) / 1000).toFixed(3));
    evidenceDirectory = path.join(SOURCE_B, "bench/history/evidence", config.source.candidate_commit);
    fs.mkdirSync(evidenceDirectory, { recursive: false });
    const gateFile = path.join(evidenceDirectory, "gate.jsonl");
    fs.writeFileSync(gateFile, "");
    const gateRecords = [];
    const host = collectHost(config, build);
    if (bareMetalAttestation) host.bare_metal_attestation = bareMetalAttestation;
    if (config.host_gate.benchmark_cpu >= provider.vcpus) {
      fail(`benchmark CPU ${config.host_gate.benchmark_cpu} is not present on a ${provider.vcpus}-logical-CPU host`);
    }
    writeJson(path.join(evidenceDirectory, "host.json"), host);
    writeJson(path.join(evidenceDirectory, "provider.json"), provider);
    fs.copyFileSync(configFile, path.join(evidenceDirectory, "protocol.json"));

    // Keep the coordinator and monitoring work off the benchmark CPU.
    const controllerCpu = config.host_gate.benchmark_cpu === 0 ? 1 : 0;
    pinControllerThreads(controllerCpu);

    log("running predeclared candidate-only host stability qualification");
    const qualificationStarted = Date.now();
    let qualification;
    try {
      qualification = await runPilot(config, build, gateFile, gateRecords);
    } finally {
      phaseTimings.qualification_seconds = Number(
        ((Date.now() - qualificationStarted) / 1000).toFixed(3),
      );
    }
    writeJson(path.join(evidenceDirectory, "qualification.json"), qualification);
    fs.cpSync(PILOT, path.join(evidenceDirectory, "qualification-sidecars"), { recursive: true });

    if (config.publication.mode === "diagnostic") {
      const diagnostic = {
        schema: "rustia.bench.cloud-diagnostic.v1",
        mode: "qualification_only",
        created_at: new Date().toISOString(),
        provider,
        source: {
          baseline_commit: config.source.baseline_commit,
          candidate_commit: config.source.candidate_commit,
          harness_commit: config.source.harness_commit,
          protocol_config_sha256: sha256File(configFile),
          git_bundle_sha256: build.bundleHash,
          baseline_binary_sha256: build.baselineBinaryHash,
          candidate_binary_sha256: build.candidateBinaryHash,
        },
        host: {
          cpu_model: host.cpu_model,
          microcode: host.microcode,
          benchmark_cpu: config.host_gate.benchmark_cpu,
        },
        timings: phaseTimings,
        qualification,
        gate: {
          records: gateRecords.length,
          sha256: sha256File(gateFile),
        },
        publication: {
          performed: false,
          reason: "diagnostic mode stops after host qualification",
        },
      };
      writeJson(path.join(evidenceDirectory, "diagnostic.json"), diagnostic);
      status = {
        ...status,
        status: "diagnostic_complete",
        detail: "host qualification completed; comparison and publication were not run",
        finished_at: new Date().toISOString(),
        timings: phaseTimings,
        diagnostic,
      };
    } else {
      log("running frozen eight-leg A/B comparison");
      const comparison = await runComparison(config, build, evidenceDirectory, gateFile, gateRecords);
      const verdict = comparison.analysis.comparison.verdict.status;
      if (verdict !== config.publication.required_verdict) {
        writeManifest({
          config,
          configFile,
          provider,
          providerFile,
          bundle,
          build,
          host,
          hostFile: path.join(evidenceDirectory, "host.json"),
          gateFile,
          gateRecords,
          qualification,
          comparison,
          publication: {
            performed: false,
            reason: `comparison verdict was ${verdict}`,
            required_verdict: config.publication.required_verdict,
          },
          evidenceDirectory,
        });
        throw new ControlledStop(
          "comparison_not_accepted",
          `comparison verdict ${verdict}; public history was not changed`,
        );
      }

      log("comparison accepted; appending three actual final-binary history measurements");
      const publication = await publishFinal(config, build, evidenceDirectory, gateFile, gateRecords);
      writeManifest({
        config,
        configFile,
        provider,
        providerFile,
        bundle,
        build,
        host,
        hostFile: path.join(evidenceDirectory, "host.json"),
        gateFile,
        gateRecords,
        qualification,
        comparison,
        publication: { performed: true, ...publication },
        evidenceDirectory,
      });
      status = {
        ...status,
        status: "accepted_and_published",
        detail: "material regression excluded and three actual final-binary history rows appended",
        finished_at: new Date().toISOString(),
        verdict,
      };
    }
  } catch (error) {
    status = {
      ...status,
      status: error instanceof ControlledStop ? error.status : "failed",
      detail: error.message,
      finished_at: new Date().toISOString(),
      timings: phaseTimings,
      elapsed_seconds: Number(((Date.now() - executionStarted) / 1000).toFixed(3)),
    };
    log(`${status.status}: ${status.detail}`);
  } finally {
    if (evidenceDirectory && fs.existsSync(PILOT)) {
      const destination = path.join(evidenceDirectory, "qualification-sidecars");
      if (!fs.existsSync(destination)) fs.cpSync(PILOT, destination, { recursive: true });
    }
    exportRun(status, config, evidenceDirectory);
  }
  return ["accepted_and_published", "diagnostic_complete"].includes(status.status) ? 0 : 2;
}

function selfTest() {
  assert.deepEqual(expandCpuList("0-3,6,8-9", "fixture"), [0, 1, 2, 3, 6, 8, 9]);
  assert.throws(() => expandCpuList("0-", "fixture"));
  assert.throws(() => expandCpuList("3-1", "fixture"));
  const before = readCpuSampleFromText("cpu  1 2 3 4 5 6 7 8\ncpu1 10 0 5 100 2 1 1 0 0 0\n", 1);
  const after = readCpuSampleFromText("cpu  1 2 3 4 5 6 7 8\ncpu1 11 0 6 198 2 1 1 0 0 0\n", 1);
  const delta = cpuDelta(before, after);
  assert.equal(delta.total_ticks, 100);
  assert.equal(delta.idle_ticks, 98);
  assert.equal(delta.idle_percent, 98);
  assert.equal(delta.steal_ticks, 0);
  const boundedStealConfig = {
    schema: "rustia.linode-fixed-benchmark.v2",
    host_gate: {
      maximum_steal_ticks: 0,
      maximum_timed_steal_percent: 0.1,
    },
  };
  assert.equal(
    timedStealGatePasses(boundedStealConfig, { total_ticks: 1000, steal_ticks: 1 }),
    true,
  );
  assert.equal(
    timedStealGatePasses(boundedStealConfig, { total_ticks: 999, steal_ticks: 1 }),
    false,
  );
  assert.equal(
    timedStealGatePasses(boundedStealConfig, { total_ticks: 2000, steal_ticks: 2 }),
    true,
  );
  assert.equal(
    timedStealGatePasses(boundedStealConfig, { total_ticks: 2000, steal_ticks: 3 }),
    false,
  );
  assert.equal(
    timedStealGatePasses(
      {
        schema: "rustia.linode-fixed-benchmark.v1",
        host_gate: { maximum_steal_ticks: 0, maximum_timed_steal_percent: 0.1 },
      },
      { total_ticks: 1000, steal_ticks: 1 },
    ),
    false,
  );
  assert.equal(quantileSorted([1, 2, 3, 4], 0.25), 1.75);
  assert.equal(matchesHistoryPrecision(1.234568, 1.23456789), true);
  assert.equal(matchesHistoryPrecision(1.23457, 1.23456789), false);
  assert.throws(() => readCpuSampleFromText("cpu0 1 2 3\n", 1));
  assert.equal(cpuAllowedListFromStatus("Name:\tnode\nCpus_allowed_list:\t0\n"), "0");
  assert.equal(cpuAllowedListFromStatus("Cpus_allowed_list:\t2-3\n"), "2-3");
  assert.throws(() => cpuAllowedListFromStatus("Name:\tnode\n"));
  assert.equal(sameThreadIds([1, 2, 8], [1, 2, 8]), true);
  assert.equal(sameThreadIds([1, 2], [1, 3]), false);
  assert.deepEqual(
    mismatchedThreadAffinities(
      [
        { tid: 1, cpus_allowed_list: "0" },
        { tid: 2, cpus_allowed_list: "0-1" },
      ],
      0,
    ),
    [{ tid: 2, cpus_allowed_list: "0-1" }],
  );
  assert.equal(new Set(PACKAGE_ACTIVITY_UNITS).size, PACKAGE_ACTIVITY_UNITS.length);
  for (const unit of ["unattended-upgrades.service", "packagekit.service"]) {
    assert.equal(PACKAGE_ACTIVITY_UNITS.includes(unit), true);
  }
  process.stdout.write("linux fixed benchmark runner self-test passed\n");
}

async function main() {
  if (process.argv[2] === "self-test") {
    selfTest();
    return;
  }
  if (process.argv.length !== 5) {
    process.stderr.write(
      "Usage: node tools/run-linux-fixed-benchmark.js CONFIG.json PROVIDER.json REPO.bundle\n",
    );
    process.exitCode = 2;
    return;
  }
  process.exitCode = await execute(process.argv[2], process.argv[3], process.argv[4]);
}

main().catch((error) => {
  process.stderr.write(`run-linux-fixed-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
