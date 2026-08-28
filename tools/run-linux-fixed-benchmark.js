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
const SOURCE_HARNESS = path.join(ROOT, "src-harness");
const TARGET_A = path.join(ROOT, "target-a");
const TARGET_B = path.join(ROOT, "target-b");
const PILOT = path.join(ROOT, "qualification");
const EXPORT = path.join(ROOT, "export");
const EXPORT_ARCHIVE = path.join(ROOT, "export.tar.gz");
const RUN_LOG = path.join(ROOT, "run.log");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/;
const SAFE_WORKLOAD_FILE_COMPONENT = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const CONTAMINATING_PROCESS = /^(apt|apt-get|as|cargo|cc|cc1|cc1plus|clang|clang\+\+|curl|dpkg|g\+\+|gcc|git|ld|ld\.lld|make|ninja|node|rustc|rustup|sccache|script-bench|unattended-upgr)$/;
const VM_STRING_CONTAMINATING_PROCESS = /^(apt|apt-get|as|cargo|cc|cc1|cc1plus|clang|clang\+\+|curl|dpkg|g\+\+|gcc|git|ld|ld\.lld|make|ninja|node|rustc|rustup|sccache|script-bench|unattended-upgr|vm-string-memor|vm-string-memory)$/;
const FIXED_THROUGHPUT_PROTOCOL = "fixed_throughput_v1";
const VM_STRING_MEMORY_PROTOCOL = "vm_string_memory_v1";
const ALLOWED_PROTOCOLS = new Set([FIXED_THROUGHPUT_PROTOCOL, VM_STRING_MEMORY_PROTOCOL]);
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

function usesPairedDigitalOceanProtocol(config) {
  return [
    "rustia.digitalocean-fixed-benchmark.v3",
    "rustia.digitalocean-fixed-benchmark.v4",
    "rustia.digitalocean-fixed-benchmark.v5",
    "rustia.digitalocean-fixed-benchmark.v6",
    "rustia.dedicated-fixed-benchmark.v1",
  ].includes(config.schema);
}

function isDedicatedHost(config) {
  return config.schema === "rustia.dedicated-fixed-benchmark.v1";
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
    fail(`measurement.protocol must be one of ${[...ALLOWED_PROTOCOLS].join(", ")}`);
  }
  return kind;
}

function isVmStringMemory(config) {
  return protocolKind(config) === VM_STRING_MEMORY_PROTOCOL;
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

function validateLinodeHostGate(config) {
  requireInteger(config.host_gate.benchmark_cpu, "host_gate.benchmark_cpu", 0);
  requireInteger(config.host_gate.maximum_steal_ticks, "host_gate.maximum_steal_ticks", 0);
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

// An owned host has no provider catalog to validate against, so its provider
// section names what the operator guarantees instead: the machine, the cores
// held off the scheduler, and the core a timed leg must run on.
function validateDedicatedProvider(config) {
  requireExactKeys(config.provider, ["host"], "config.provider");
  const host = requireObject(config.provider.host, "config.provider.host");
  requireExactKeys(
    host,
    ["name", "cpu_model", "logical_cpus", "physical_cores", "pinned_cpus", "max_runtime_minutes"],
    "config.provider.host",
  );
  requireString(host.name, "config.provider.host.name");
  requireString(host.cpu_model, "config.provider.host.cpu_model");
  requireString(host.pinned_cpus, "config.provider.host.pinned_cpus");
  requireInteger(host.logical_cpus, "config.provider.host.logical_cpus", 1);
  requireInteger(host.physical_cores, "config.provider.host.physical_cores", 1);
  requireInteger(host.max_runtime_minutes, "config.provider.host.max_runtime_minutes", 1);
  // SMT is required off, so logical CPUs and physical cores must agree. A
  // config that disagrees with itself would otherwise pass here and fail only
  // on the host, after a build.
  if (host.logical_cpus !== host.physical_cores) {
    fail(
      `config.provider.host declares ${host.logical_cpus} logical CPUs across ${host.physical_cores} ` +
        `physical cores; the dedicated protocol requires SMT off, so these must be equal`,
    );
  }
}

function validateConfig(config) {
  requireObject(config, "config");
  const kind = protocolKind(config);
  if (
    config.schema !== "rustia.digitalocean-fixed-benchmark.v2" &&
    config.schema !== "rustia.digitalocean-fixed-benchmark.v3" &&
    config.schema !== "rustia.digitalocean-fixed-benchmark.v4" &&
    config.schema !== "rustia.digitalocean-fixed-benchmark.v5" &&
    config.schema !== "rustia.digitalocean-fixed-benchmark.v6" &&
    config.schema !== "rustia.linode-fixed-benchmark.v1" &&
    config.schema !== "rustia.linode-fixed-benchmark.v2" &&
    config.schema !== "rustia.vultr-fixed-benchmark.v1" &&
    config.schema !== "rustia.scaleway-fixed-benchmark.v1" &&
    config.schema !== "rustia.dedicated-fixed-benchmark.v1"
  ) {
    fail("unsupported cloud benchmark config schema");
  }
  if (isDedicatedHost(config)) validateDedicatedProvider(config);
  const requiredSections = kind === VM_STRING_MEMORY_PROTOCOL
    ? ["provider", "source", "measurement", "host_gate", "publication"]
    : ["provider", "source", "measurement", "qualification", "host_gate", "publication"];
  for (const field of requiredSections) {
    requireObject(config[field], `config.${field}`);
  }
  if (kind === VM_STRING_MEMORY_PROTOCOL) {
    requireExactKeys(
      config,
      ["schema", "provider", "source", "measurement", "host_gate", "publication"],
      "config",
    );
    requireExactKeys(
      config.source,
      ["baseline_commit", "candidate_commit", "harness_commit", "expected_sha256"],
      "source",
    );
    requireExactKeys(
      config.source.expected_sha256,
      [
        "baseline_original_bench_manifest",
        "candidate_original_bench_manifest",
        "overlay_bench_manifest",
        "sampler",
        "analyzer",
        "cargo_lock",
        "toolchain",
      ],
      "source.expected_sha256",
    );
  }
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    if (!FULL_COMMIT.test(config.source[field])) fail(`config.source.${field} must be a full commit`);
  }
  if (config.source.baseline_commit === config.source.candidate_commit) {
    fail("baseline and candidate commits must differ");
  }
  for (const [field, value] of Object.entries(config.source.expected_sha256)) {
    if (!SHA256.test(value)) fail(`config.source.expected_sha256.${field} must be a SHA-256`);
  }
  if (kind === VM_STRING_MEMORY_PROTOCOL) {
    if (config.schema !== "rustia.linode-fixed-benchmark.v2") {
      fail("vm_string_memory_v1 is supported only for the Linode v2 lifecycle");
    }
    requireExactKeys(
      config.measurement,
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
      "measurement",
    );
    const frozen = {
      vm_count: 512,
      accepted_rounds: 16,
      warmup_rounds: 1,
      round_retry_limit: 3,
      bootstrap_repetitions: 100000,
      bootstrap_seed: 1381323604,
    };
    for (const [field, expected] of Object.entries(frozen)) {
      if (requireInteger(config.measurement[field], `measurement.${field}`, field === "bootstrap_seed" ? 0 : 1) !== expected) {
        fail(`measurement.${field} must remain frozen at ${expected}`);
      }
    }
    if (canonicalJson(config.measurement.scenarios) !== canonicalJson(VM_STRING_SCENARIOS)) {
      fail(`measurement.scenarios must be exactly ${VM_STRING_SCENARIOS.join(", ")}`);
    }
    requireExactKeys(config.publication, ["mode"], "publication");
    if (config.publication.mode !== "evidence_only") {
      fail("vm_string_memory_v1 publication.mode must be evidence_only");
    }
    requireExactKeys(
      config.host_gate,
      [
        "benchmark_cpu",
        "cooldown_seconds",
        "idle_window_seconds",
        "minimum_idle_percent",
        "maximum_load_1m",
        "maximum_steal_ticks",
        "maximum_timed_steal_percent",
        "wait_timeout_seconds",
      ],
      "host_gate",
    );
    requireInteger(config.host_gate.cooldown_seconds, "host_gate.cooldown_seconds", 1);
    requireInteger(config.host_gate.idle_window_seconds, "host_gate.idle_window_seconds", 1);
    requireNumber(config.host_gate.minimum_idle_percent, "host_gate.minimum_idle_percent", 0);
    if (config.host_gate.minimum_idle_percent > 100) fail("host_gate.minimum_idle_percent must be <= 100");
    requireNumber(config.host_gate.maximum_load_1m, "host_gate.maximum_load_1m", 0);
    requireInteger(config.host_gate.wait_timeout_seconds, "host_gate.wait_timeout_seconds", 1);
    validateLinodeHostGate(config);
    return config;
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
  if (usesPairedDigitalOceanProtocol(config)) {
    requireExactKeys(
      config.qualification,
      ["paired_pilot_rounds", "max_ratio_spread_percent"],
      "qualification",
    );
    requireInteger(config.qualification.paired_pilot_rounds, "qualification.paired_pilot_rounds", 2);
    requireNumber(config.qualification.max_ratio_spread_percent, "qualification.max_ratio_spread_percent", 0);
    if (
      config.schema === "rustia.digitalocean-fixed-benchmark.v6" &&
      config.qualification.paired_pilot_rounds !== 4
    ) {
      fail("DigitalOcean v6 requires exactly four paired pilot rounds");
    }
    for (let index = 0; index < config.measurement.run_order.length; index += 2) {
      if (config.measurement.run_order[index].variant === config.measurement.run_order[index + 1].variant) {
        fail("DigitalOcean v3 run-order pairs must contain one baseline and one candidate leg");
      }
    }
  } else {
    requireInteger(config.qualification.candidate_pilot_legs, "qualification.candidate_pilot_legs", 2);
  }
  requireInteger(config.host_gate.benchmark_cpu, "host_gate.benchmark_cpu", 0);
  requireInteger(config.host_gate.maximum_steal_ticks, "host_gate.maximum_steal_ticks", 0);
  if (["rustia.digitalocean-fixed-benchmark.v4", "rustia.digitalocean-fixed-benchmark.v5", "rustia.digitalocean-fixed-benchmark.v6"].includes(config.schema)) {
    requireExactKeys(
      config.host_gate,
      [
        "benchmark_cpu",
        "cooldown_seconds",
        "idle_window_seconds",
        "minimum_idle_percent",
        "maximum_load_1m",
        "maximum_steal_ticks",
        "maximum_timed_steal_percent",
        "wait_timeout_seconds",
      ],
      "host_gate",
    );
    requireNumber(config.host_gate.maximum_timed_steal_percent, "host_gate.maximum_timed_steal_percent", 0);
    const expectedTimedStealPercent =
      ["rustia.digitalocean-fixed-benchmark.v5", "rustia.digitalocean-fixed-benchmark.v6"].includes(config.schema) ? 0.5 : 0.1;
    if (
      config.host_gate.maximum_steal_ticks !== 0 ||
      config.host_gate.maximum_timed_steal_percent !== expectedTimedStealPercent
    ) {
      fail(
        `DigitalOcean ${config.schema.split(".").at(-1)} requires zero idle steal and a ${expectedTimedStealPercent}% timed steal ceiling`,
      );
    }
  }
  if (config.schema === "rustia.linode-fixed-benchmark.v2") {
    validateLinodeHostGate(config);
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

function workloadFileComponent(workload) {
  if (typeof workload !== "string" || workload.length === 0) {
    fail("benchmark workload must be a non-empty string");
  }
  if (SAFE_WORKLOAD_FILE_COMPONENT.test(workload)) return workload;
  const slug = workload
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[-._]+|[-._]+$/g, "")
    .slice(0, 48)
    .replace(/[-._]+$/g, "");
  return `${slug || "workload"}-${sha256(Buffer.from(workload, "utf8")).slice(0, 12)}`;
}

function finalSidecarName(config, index) {
  requireInteger(index, "final sidecar index", 1);
  const file = `final-${workloadFileComponent(config.measurement.workload)}-${index}.json`;
  if (!SAFE_FILE.test(file)) fail("generated final sidecar name is not a safe JSON basename");
  return file;
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

// An owned, permanently-provisioned host is attested differently from a rented
// one: there is no provider metadata to compare against, and the properties
// that matter are the ones that make it quiet. Cores held off the scheduler
// with `isolcpus`, SMT disabled so a sibling thread cannot contend, and -- the
// one that bites -- the measuring process actually being allowed to run on the
// isolated core it names.
//
// That last check exists because the failure is silent rather than loud. An SSH
// session on this host inherits an affinity mask that excludes the isolated
// CPUs, so `nproc` reports the schedulable count and anything sizing itself
// from it runs on the busy cores while reporting entirely believable numbers.
// A benchmark that measures the wrong CPUs and says so is a nuisance; one that
// measures the wrong CPUs quietly is a corrupted result.
function verifyDedicatedIsolation(config) {
  const host = config.provider.host;
  const pinned = expandCpuList(String(host.pinned_cpus), "config.provider.host.pinned_cpus");
  if (pinned.length === 0) fail("config.provider.host.pinned_cpus must name at least one CPU");

  const isolatedRaw = fs.readFileSync("/sys/devices/system/cpu/isolated", "utf8").trim();
  const isolated = isolatedRaw === "" ? [] : expandCpuList(isolatedRaw, "kernel isolated CPU list");
  const notIsolated = pinned.filter((cpu) => !isolated.includes(cpu));
  if (notIsolated.length > 0) {
    fail(
      `pinned CPU(s) ${notIsolated.join(",")} are not held off the scheduler; ` +
        `/sys/devices/system/cpu/isolated reports ${JSON.stringify(isolatedRaw)}. ` +
        `Boot with isolcpus covering the pinned set before measuring.`,
    );
  }

  const smtPath = "/sys/devices/system/cpu/smt/control";
  if (fs.existsSync(smtPath)) {
    const smt = fs.readFileSync(smtPath, "utf8").trim();
    if (smt !== "off" && smt !== "notsupported" && smt !== "notimplemented") {
      fail(`simultaneous multithreading is ${JSON.stringify(smt)}; a sibling thread can contend with a timed leg`);
    }
  }

  // The silent one. Ask the kernel which CPUs this process may actually use
  // rather than inferring it from a core count.
  const affinity = run("taskset", ["-pc", String(process.pid)], { allowFailure: true });
  if (affinity.status !== 0) {
    fail("could not read this process's CPU affinity mask; taskset is required to attest the pinned core");
  }
  const mask = affinity.stdout.trim().split(":").pop().trim();
  const allowed = expandCpuList(mask, "process affinity mask");
  const unusable = pinned.filter((cpu) => !allowed.includes(cpu));
  if (unusable.length > 0) {
    fail(
      `this process may not run on pinned CPU(s) ${unusable.join(",")}: its affinity mask is ${mask}. ` +
        `Sessions on an isolcpus host inherit a mask excluding the isolated CPUs, so a run started this way ` +
        `would measure the schedulable cores and report plausible numbers for the wrong hardware. ` +
        `Re-invoke under taskset -c ${pinned.join(",")}.`,
    );
  }

  log(
    `dedicated host attested: pinned CPU(s) ${pinned.join(",")} isolated, SMT off, affinity mask ${mask}`,
  );
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
  if (
    config.schema === "rustia.linode-fixed-benchmark.v2" ||
    config.schema === "rustia.digitalocean-fixed-benchmark.v4" ||
    config.schema === "rustia.digitalocean-fixed-benchmark.v5" ||
    config.schema === "rustia.digitalocean-fixed-benchmark.v6"
  ) {
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

function contaminatingProcessPattern(config) {
  return isVmStringMemory(config) ? VM_STRING_CONTAMINATING_PROCESS : CONTAMINATING_PROCESS;
}

function contaminatingProcesses(excludedPids = new Set(), pattern = CONTAMINATING_PROCESS) {
  const records = [];
  for (const entry of fs.readdirSync("/proc", { withFileTypes: true })) {
    if (!entry.isDirectory() || !/^[0-9]+$/.test(entry.name)) continue;
    const pid = Number(entry.name);
    if (pid === process.pid || excludedPids.has(pid)) continue;
    try {
      const comm = fs.readFileSync(`/proc/${pid}/comm`, "utf8").trim();
      if (!pattern.test(comm)) continue;
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
  const pattern = contaminatingProcessPattern(config);
  const beforeProcesses = contaminatingProcesses(new Set(), pattern);
  await sleep(config.host_gate.idle_window_seconds * 1000);
  const after = readCpuSample(cpu);
  const afterProcesses = contaminatingProcesses(new Set(), pattern);
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
  skipPreLegGate = false,
}) {
  if (skipPreLegGate) {
    log(`${label}: continuing the admitted pair without a second cooldown`);
  } else {
    await waitForIdle(config, label, gateFile, gateRecords);
  }
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
      const contaminants = contaminatingProcesses(
        new Set([child.pid]),
        contaminatingProcessPattern(config),
      );
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
  const postProcesses = contaminatingProcesses(new Set(), contaminatingProcessPattern(config));
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

function installAndBuildVmStringMemory(config, bundle) {
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
  for (const [directory, commit] of [
    [SOURCE_A, config.source.baseline_commit],
    [SOURCE_B, config.source.candidate_commit],
    [SOURCE_HARNESS, config.source.harness_commit],
  ]) {
    run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", directory, commit], {
      capture: false,
    });
    assert.equal(runText("git", ["-C", directory, "rev-parse", "HEAD"]), commit);
  }

  const expected = config.source.expected_sha256;
  const baselineOriginalManifestHash = sha256File(path.join(SOURCE_A, "bench/Cargo.toml"));
  const candidateOriginalManifestHash = sha256File(path.join(SOURCE_B, "bench/Cargo.toml"));
  const overlayManifest = path.join(SOURCE_HARNESS, "bench/Cargo.toml");
  const sampler = path.join(SOURCE_HARNESS, "bench/src/bin/vm-string-memory.rs");
  const analyzer = path.join(SOURCE_HARNESS, "bench/analyze-vm-string-memory.js");
  assert.equal(baselineOriginalManifestHash, expected.baseline_original_bench_manifest);
  assert.equal(candidateOriginalManifestHash, expected.candidate_original_bench_manifest);
  assert.equal(sha256File(overlayManifest), expected.overlay_bench_manifest);
  assert.equal(sha256File(sampler), expected.sampler);
  assert.equal(sha256File(analyzer), expected.analyzer);

  for (const source of [SOURCE_A, SOURCE_B]) {
    fs.mkdirSync(path.join(source, "bench/src/bin"), { recursive: true });
    fs.copyFileSync(overlayManifest, path.join(source, "bench/Cargo.toml"));
    fs.copyFileSync(sampler, path.join(source, "bench/src/bin/vm-string-memory.rs"));
    assert.equal(sha256File(path.join(source, "bench/Cargo.toml")), expected.overlay_bench_manifest);
    assert.equal(sha256File(path.join(source, "bench/src/bin/vm-string-memory.rs")), expected.sampler);
  }
  for (const source of [SOURCE_A, SOURCE_B, SOURCE_HARNESS]) {
    assert.equal(sha256File(path.join(source, "Cargo.lock")), expected.cargo_lock);
    assert.equal(sha256File(path.join(source, "rust-toolchain.toml")), expected.toolchain);
  }

  run(cargo, ["+1.95.0", "fetch", "--locked", "--manifest-path", path.join(SOURCE_HARNESS, "Cargo.toml")], {
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
    "vm-string-memory",
    "--release",
  ];
  run(cargo, buildArgs(SOURCE_A), { env: benchmarkEnvironment(TARGET_A), capture: false });
  run(cargo, buildArgs(SOURCE_B), { env: benchmarkEnvironment(TARGET_B), capture: false });

  const baselineBinary = path.join(TARGET_A, "release/vm-string-memory");
  const candidateBinary = path.join(TARGET_B, "release/vm-string-memory");
  for (const binary of [baselineBinary, candidateBinary]) fs.accessSync(binary, fs.constants.X_OK);
  return {
    cargo,
    baselineBinary,
    candidateBinary,
    baselineOriginalManifestHash,
    candidateOriginalManifestHash,
    overlayManifestHash: expected.overlay_bench_manifest,
    samplerHash: expected.sampler,
    analyzerFile: analyzer,
    analyzerHash: expected.analyzer,
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

function installAndBuild(config, bundle) {
  if (isVmStringMemory(config)) return installAndBuildVmStringMemory(config, bundle);
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

function rotatedMemoryScenarios(round, scenarios = VM_STRING_SCENARIOS) {
  requireInteger(round, "memory round", 1);
  // Pair each scenario rotation with opposite first-build roles so every
  // scenario has exactly balanced baseline/candidate process positions.
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

function parseCanonicalJsonLine(stdout, location) {
  if (typeof stdout !== "string" || !stdout.endsWith("\n") || stdout.includes("\r")) {
    fail(`${location} did not emit one canonical JSON line`);
  }
  const line = stdout.slice(0, -1);
  if (!line || line.includes("\n")) fail(`${location} did not emit exactly one JSON record`);
  let value;
  try {
    value = JSON.parse(line);
  } catch (error) {
    fail(`${location} emitted invalid JSON: ${error.message}`);
  }
  if (line !== JSON.stringify(value)) fail(`${location} JSON is not canonical`);
  return { value: requireObject(value, location), line };
}

function validateMemorySnapshot(snapshotValue, location) {
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

function validateMemorySample(value, expected, config) {
  const location = `memory sample ${expected.sampleId}`;
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
  if (value.sample_id !== expected.sampleId || value.protocol_sha256 !== expected.protocolHash) {
    fail(`${location} has the wrong sample or protocol identity`);
  }
  requireInteger(value.timestamp_unix, `${location}.timestamp_unix`, 1);
  requireExactKeys(value.build, ["role", "commit"], `${location}.build`);
  if (value.build.role !== expected.role || value.build.commit !== expected.commit || value.round !== expected.round) {
    fail(`${location} has the wrong build or round identity`);
  }
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
  const scenarioShapes = {
    empty: [false, "none", 0, 0, 1024],
    short_unloaded: [false, "short", 1024, 13, 0],
    short_loaded: [true, "short", 1024, 13, 0],
    long_unloaded: [false, "long", 128, 2048, 896],
    long_loaded: [true, "long", 128, 2048, 896],
  };
  const shape = scenarioShapes[expected.scenario];
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
    fail(`${location} platform identity is not linux-x86_64`);
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
    validateMemorySnapshot(memory[snapshot], `${location}.memory_kib.${snapshot}`);
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

async function spawnMemorySample(binary, args, cwd, activePids) {
  return await new Promise((resolve, reject) => {
    const child = spawn("taskset", ["-c", args.cpu, binary, ...args.sample], {
      cwd,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    activePids.add(child.pid);
    const stdout = [];
    const stderr = [];
    child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
    child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
    child.on("error", (error) => {
      activePids.delete(child.pid);
      reject(error);
    });
    child.on("close", (status, signal) => {
      activePids.delete(child.pid);
      resolve({
        status: status === null ? 1 : status,
        signal,
        stdout: Buffer.concat(stdout).toString("utf8"),
        stderr: Buffer.concat(stderr).toString("utf8"),
      });
    });
  });
}

async function runMemoryRoundAttempt({
  config,
  build,
  protocolHash,
  prefix,
  round,
  scheduleRound,
  attempt,
  gateFile,
  gateRecords,
}) {
  const label = `memory-${prefix}-${String(round).padStart(2, "0")}-attempt-${attempt}`;
  assert.equal(sha256File(build.baselineBinary), build.baselineBinaryHash);
  assert.equal(sha256File(build.candidateBinary), build.candidateBinaryHash);
  await waitForIdle(config, label, gateFile, gateRecords);
  const cpu = config.host_gate.benchmark_cpu;
  const before = readCpuSample(cpu);
  const observed = [];
  const activePids = new Set();
  const timer = setInterval(() => {
    const contaminants = contaminatingProcesses(activePids, VM_STRING_CONTAMINATING_PROCESS);
    if (contaminants.length > 0) observed.push(...contaminants);
  }, 250);
  const records = [];
  const processIds = new Set();
  try {
    for (const entry of memoryRoundOrder(scheduleRound, config.measurement.scenarios)) {
      const baseline = entry.role === "baseline";
      const binary = baseline ? build.baselineBinary : build.candidateBinary;
      const commit = baseline ? config.source.baseline_commit : config.source.candidate_commit;
      const cwd = baseline ? SOURCE_A : SOURCE_B;
      const sampleId = memorySampleId(prefix, round, entry.role, entry.scenario);
      const sampleArgs = [
        "sample",
        "--scenario",
        entry.scenario,
        "--vm-count",
        String(config.measurement.vm_count),
        "--round",
        String(round),
        "--sample-id",
        sampleId,
        "--build-role",
        entry.role,
        "--commit",
        commit,
        "--protocol-sha256",
        protocolHash,
      ];
      log(`+ taskset -c ${cpu} ${commandText(binary, sampleArgs)}`);
      const result = await spawnMemorySample(
        binary,
        { cpu: String(cpu), sample: sampleArgs },
        cwd,
        activePids,
      );
      if (result.stderr) process.stderr.write(result.stderr);
      if (result.status !== 0) {
        fail(`${label} ${sampleId}: sampler failed with status ${result.status}`);
      }
      const parsed = parseCanonicalJsonLine(result.stdout, sampleId);
      validateMemorySample(
        parsed.value,
        { sampleId, protocolHash, role: entry.role, commit, round, scenario: entry.scenario },
        config,
      );
      addFreshMemoryProcessId(processIds, parsed.value.platform.pid, label);
      records.push(parsed.line);
    }
  } finally {
    clearInterval(timer);
  }
  const after = readCpuSample(cpu);
  const during = {
    phase: "during_leg",
    label,
    timestamp: new Date().toISOString(),
    cpu,
    cpu_delta: cpuDelta(before, after),
    contaminants: [...observed, ...contaminatingProcesses(new Set(), VM_STRING_CONTAMINATING_PROCESS)],
    passed: false,
  };
  during.passed = timedStealGatePasses(config, during.cpu_delta) && during.contaminants.length === 0;
  appendGateRecord(gateFile, gateRecords, during);
  assert.equal(sha256File(build.baselineBinary), build.baselineBinaryHash);
  assert.equal(sha256File(build.candidateBinary), build.candidateBinaryHash);
  assert.equal(processIds.size, config.measurement.scenarios.length * 2);
  return { passed: during.passed, records, label };
}

async function collectMemoryRound(options) {
  const maximumAttempts = options.config.measurement.round_retry_limit + 1;
  for (let attempt = 1; attempt <= maximumAttempts; attempt += 1) {
    const result = await runMemoryRoundAttempt({ ...options, attempt });
    if (result.passed) return { ...result, attempt };
    log(`${result.label}: whole round rejected by the frozen host gate; discarding all ten samples`);
  }
  throw new ControlledStop(
    "memory_round_retry_exhausted",
    `${options.prefix} ${options.round}: host gate rejected ${maximumAttempts} complete attempts`,
  );
}

function writeCanonicalJsonl(file, lines) {
  if (!Array.isArray(lines) || lines.length === 0 || lines.some((line) => typeof line !== "string" || line.includes("\n"))) {
    fail(`refusing invalid canonical JSONL payload for ${file}`);
  }
  fs.writeFileSync(file, `${lines.join("\n")}\n`);
}

async function runVmStringMemoryProtocol(config, build, evidenceDirectory, gateFile, gateRecords, configFile) {
  const protocolHash = sha256File(configFile);
  const warmupLines = [];
  const warmupAttempts = [];
  const acceptedLines = [];
  const acceptedAttempts = [];
  for (let round = 1; round <= config.measurement.warmup_rounds; round += 1) {
    const result = await collectMemoryRound({
      config,
      build,
      protocolHash,
      prefix: "warmup",
      round,
      scheduleRound: round,
      gateFile,
      gateRecords,
    });
    warmupLines.push(...result.records);
    warmupAttempts.push(result.attempt);
  }
  writeCanonicalJsonl(path.join(evidenceDirectory, "warmup-samples.jsonl"), warmupLines);
  for (let round = 1; round <= config.measurement.accepted_rounds; round += 1) {
    const result = await collectMemoryRound({
      config,
      build,
      protocolHash,
      prefix: "round",
      round,
      scheduleRound: round,
      gateFile,
      gateRecords,
    });
    acceptedLines.push(...result.records);
    acceptedAttempts.push(result.attempt);
  }
  const samplesFile = path.join(evidenceDirectory, "samples.jsonl");
  writeCanonicalJsonl(samplesFile, acceptedLines);
  if (acceptedLines.length !== config.measurement.accepted_rounds * config.measurement.scenarios.length * 2) {
    fail("vm string memory protocol produced the wrong accepted sample count");
  }
  const analyzerArgs = [
    build.analyzerFile,
    "--input",
    samplesFile,
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
  const result = run("node", analyzerArgs, { cwd: SOURCE_HARNESS });
  let analysis;
  try {
    analysis = requireObject(JSON.parse(result.stdout), "vm string memory analysis");
  } catch (error) {
    fail(`vm string memory analyzer returned invalid JSON: ${error.message}`);
  }
  if (analysis.schema_version !== 1 || analysis.schema !== "rustia.bench.vm-string-memory-analysis.v1") {
    fail("vm string memory analyzer returned the wrong schema");
  }
  const analysisFile = path.join(evidenceDirectory, "analysis.json");
  fs.writeFileSync(analysisFile, `${JSON.stringify(analysis, null, 2)}\n`);
  return {
    protocolHash,
    samplesFile,
    analysisFile,
    analysis,
    analyzerCommand: [
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
    ],
    acceptedAttempts,
    warmupAttempts,
    acceptedSampleCount: acceptedLines.length,
    warmupSampleCount: warmupLines.length,
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

function pairedPilotOrder(round) {
  return [["baseline", "candidate"], ["candidate", "baseline"], ["candidate", "baseline"], ["baseline", "candidate"]][
    (round - 1) % 4
  ];
}

function relativeSpreadPercent(values) {
  if (!Array.isArray(values) || values.length < 2 || values.some((value) => !Number.isFinite(value) || value <= 0)) {
    fail("relative spread requires at least two positive finite values");
  }
  return ((Math.max(...values) - Math.min(...values)) / Math.min(...values)) * 100;
}

function pairedQualificationResult(config, pairs) {
  const ratios = pairs.map((pair) => pair.candidate_over_baseline_ratio);
  if (config.schema !== "rustia.digitalocean-fixed-benchmark.v6") {
    const spread = relativeSpreadPercent(ratios);
    return {
      schema: "rustia.bench.cloud-paired-qualification.v1",
      pilot_pairs: pairs,
      ratio_spread_percent: spread,
      maximum_ratio_spread_percent: config.qualification.max_ratio_spread_percent,
      passed: spread <= config.qualification.max_ratio_spread_percent,
      note: "qualification-only paired runs; never accepted as A/B evidence or public history",
    };
  }

  const orderGroups = {};
  for (const pair of pairs) {
    const key = pair.order.join("-then-");
    if (!orderGroups[key]) orderGroups[key] = [];
    orderGroups[key].push(pair.candidate_over_baseline_ratio);
  }
  const orderSpreadPercent = Object.fromEntries(
    Object.entries(orderGroups).map(([order, values]) => [
      order,
      values.length >= 2 ? relativeSpreadPercent(values) : null,
    ]),
  );
  const comparableSpreads = Object.values(orderSpreadPercent).filter(Number.isFinite);
  const spread = comparableSpreads.length > 0 ? Math.max(...comparableSpreads) : 0;
  return {
    schema: "rustia.bench.cloud-paired-qualification.v2",
    pilot_pairs: pairs,
    raw_ratio_spread_percent: ratios.length >= 2 ? relativeSpreadPercent(ratios) : 0,
    ratio_spread_by_order_percent: orderSpreadPercent,
    ratio_spread_percent: spread,
    maximum_ratio_spread_percent: config.qualification.max_ratio_spread_percent,
    passed: spread <= config.qualification.max_ratio_spread_percent,
    note: "qualification-only paired runs; spread compares repeated pairs with the same order so counterbalanced second-leg effects are not misclassified as host instability; never accepted as A/B evidence or public history",
  };
}

async function runPairedPilot(config, build, gateFile, gateRecords) {
  fs.mkdirSync(PILOT, { recursive: true });
  const pairs = [];
  for (let round = 1; round <= config.qualification.paired_pilot_rounds; round += 1) {
    const summaries = {};
    for (const [position, variant] of pairedPilotOrder(round).entries()) {
      const baseline = variant === "baseline";
      const binary = baseline ? build.baselineBinary : build.candidateBinary;
      const commit = baseline ? config.source.baseline_commit : config.source.candidate_commit;
      const file = path.join(PILOT, `paired-pilot-${round}-${variant}.json`);
      await runMonitoredBenchmark({
        config,
        label: `qualification-pair-${round}-${variant}`,
        binary,
        args: [...commonArgs(config), "--commit", commit, "--samples-json", file],
        cwd: SOURCE_B,
        gateFile,
        gateRecords,
        acceptedBlock: false,
        skipPreLegGate: position === 1,
      });
      summaries[variant] = sidecarSummary(file, {
        commit,
        target: "linux-x86_64",
        engine: "rustia",
        workload: config.measurement.workload,
        n: config.measurement.fixed_n,
        n_selection: "fixed",
        reps: config.measurement.repetitions,
        checksum: config.measurement.expected_checksum,
      });
    }
    pairs.push({
      round,
      order: pairedPilotOrder(round),
      baseline: summaries.baseline,
      candidate: summaries.candidate,
      candidate_over_baseline_ratio:
        summaries.candidate.harness_median / summaries.baseline.harness_median,
    });
    if (pairs.length >= 2) {
      const partialResult = pairedQualificationResult(config, pairs);
      if (!partialResult.passed) {
        writeJson(path.join(PILOT, "qualification.json"), partialResult);
        throw new ControlledStop(
          "host_qualification_failed",
          `within-order paired candidate/baseline ratio spread ${partialResult.ratio_spread_percent.toFixed(3)}% exceeded ` +
            `${config.qualification.max_ratio_spread_percent}% after ${pairs.length} pairs`,
        );
      }
    }
  }
  const result = pairedQualificationResult(config, pairs);
  writeJson(path.join(PILOT, "qualification.json"), result);
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
      skipPreLegGate:
        usesPairedDigitalOceanProtocol(config) && index % 2 === 1,
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
      finalSidecarName(config, index),
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

function vmStringMemoryEvidenceHashes(evidenceDirectory) {
  const actual = fs.readdirSync(evidenceDirectory).sort();
  assert.deepEqual(actual, VM_STRING_HASHED_EVIDENCE_FILES);
  const hashes = {};
  for (const file of actual) {
    const absolute = path.join(evidenceDirectory, file);
    const stat = fs.lstatSync(absolute);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
      fail(`vm string memory evidence contains non-regular entry ${file}`);
    }
    hashes[file] = sha256File(absolute);
  }
  return hashes;
}

function writeVmStringMemoryManifest({
  config,
  configFile,
  provider,
  providerFile,
  build,
  host,
  hostFile,
  gateFile,
  gateRecords,
  result,
  evidenceDirectory,
}) {
  fs.copyFileSync(RUN_LOG, path.join(evidenceDirectory, "remote-run.log"));
  const manifest = {
    schema_version: 1,
    schema: "rustia.bench.vm-string-memory-evidence.v1",
    created_at: new Date().toISOString(),
    provider,
    source: {
      baseline_commit: config.source.baseline_commit,
      candidate_commit: config.source.candidate_commit,
      harness_commit: config.source.harness_commit,
      baseline_original_bench_manifest_sha256: build.baselineOriginalManifestHash,
      candidate_original_bench_manifest_sha256: build.candidateOriginalManifestHash,
      overlay_bench_manifest_sha256: build.overlayManifestHash,
      sampler_sha256: build.samplerHash,
      analyzer_sha256: build.analyzerHash,
      overlay_disclosure:
        "bench/Cargo.toml and bench/src/bin/vm-string-memory.rs from the harness commit were copied byte-for-byte over both source trees before compilation",
      cargo_lock_sha256: config.source.expected_sha256.cargo_lock,
      toolchain_sha256: config.source.expected_sha256.toolchain,
      git_bundle_sha256: build.bundleHash,
      protocol_config_sha256: result.protocolHash,
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
      binary: "vm-string-memory",
      cargo_incremental: "0",
      rustc_wrapper: "unset",
      locked: true,
      offline_build_after_fetch: true,
      source_paths_retained_through_all_measurements: [SOURCE_A, SOURCE_B, SOURCE_HARNESS],
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
      kind: VM_STRING_MEMORY_PROTOCOL,
      vm_count: config.measurement.vm_count,
      scenarios: config.measurement.scenarios,
      warmup_rounds: config.measurement.warmup_rounds,
      accepted_rounds: config.measurement.accepted_rounds,
      round_retry_limit: config.measurement.round_retry_limit,
      warmup_sample_count: result.warmupSampleCount,
      accepted_sample_count: result.acceptedSampleCount,
      accepted_attempts: result.acceptedAttempts,
      warmup_attempts: result.warmupAttempts,
      warmup_order: memoryRoundOrder(1, config.measurement.scenarios),
      accepted_orders: Array.from(
        { length: config.measurement.accepted_rounds },
        (_, index) => memoryRoundOrder(index + 1, config.measurement.scenarios),
      ),
      order_rule:
        "ABBA/BAAB build-block alternation in consecutive opposite-role pairs; each pair shares one rotated first-block scenario order and the second block reverses it; every sample is a fresh process",
      bootstrap_repetitions: config.measurement.bootstrap_repetitions,
      bootstrap_seed: config.measurement.bootstrap_seed,
      analyzer_command: result.analyzerCommand,
      host_gate: config.host_gate,
      contaminating_process_comm_regex: VM_STRING_CONTAMINATING_PROCESS.source,
      exclusion_rule:
        "a host-gate violation discards all ten samples in the round; retries are capped before execution and never selected by measurement outcome",
      gate_log: { file: "gate.jsonl", sha256: sha256File(gateFile), records: gateRecords.length },
    },
    analysis: {
      file: "analysis.json",
      sha256: sha256File(result.analysisFile),
      schema: result.analysis.schema,
      identity: result.analysis.identity,
    },
    publication: {
      mode: "evidence_only",
      throughput_history_modified: false,
      imported_path: `bench/evidence/vm-string-memory/${config.source.candidate_commit}`,
    },
    evidence_sha256: vmStringMemoryEvidenceHashes(evidenceDirectory),
  };
  writeJson(path.join(evidenceDirectory, "manifest.json"), manifest);
  assert.deepEqual(fs.readdirSync(evidenceDirectory).sort(), VM_STRING_EVIDENCE_FILES);
  return manifest;
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
    const destination = isVmStringMemory(config)
      ? path.join(EXPORT, memoryEvidenceRelativePath(config.source.candidate_commit).replace(/^bench\//, ""))
      : path.join(EXPORT, "evidence", config.source.candidate_commit);
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
    if (isDedicatedHost(config)) {
      verifyDedicatedIsolation(config);
    }
    const bareMetalAttestation = verifyBareMetalHost(config, provider);
    if (bareMetalAttestation) log(`${provider.provider} physical-host topology attestation passed`);
    log("preparing exact source trees and frozen release binaries");
    const buildStarted = Date.now();
    const build = installAndBuild(config, bundle);
    phaseTimings.build_seconds = Number(((Date.now() - buildStarted) / 1000).toFixed(3));
    evidenceDirectory = isVmStringMemory(config)
      ? path.join(SOURCE_HARNESS, memoryEvidenceRelativePath(config.source.candidate_commit))
      : path.join(SOURCE_B, "bench/history/evidence", config.source.candidate_commit);
    fs.mkdirSync(evidenceDirectory, { recursive: isVmStringMemory(config) });
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

    if (isVmStringMemory(config)) {
      log("running one excluded warmup and sixteen accepted vm-string-memory rounds");
      const measurementStarted = Date.now();
      const result = await runVmStringMemoryProtocol(
        config,
        build,
        evidenceDirectory,
        gateFile,
        gateRecords,
        configFile,
      );
      phaseTimings.measurement_seconds = Number(((Date.now() - measurementStarted) / 1000).toFixed(3));
      writeVmStringMemoryManifest({
        config,
        configFile,
        provider,
        providerFile,
        build,
        host,
        hostFile: path.join(evidenceDirectory, "host.json"),
        gateFile,
        gateRecords,
        result,
        evidenceDirectory,
      });
      status = {
        ...status,
        status: "evidence_complete",
        detail: "vm string memory evidence completed; public throughput history was not changed",
        finished_at: new Date().toISOString(),
      };
    } else {
    const pairedQualification = usesPairedDigitalOceanProtocol(config);
    log(
      pairedQualification
        ? "running predeclared paired relative-stability qualification"
        : "running predeclared candidate-only host stability qualification",
    );
    const qualificationStarted = Date.now();
    let qualification;
    try {
      qualification = pairedQualification
        ? await runPairedPilot(config, build, gateFile, gateRecords)
        : await runPilot(config, build, gateFile, gateRecords);
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
    if (!isVmStringMemory(config) && evidenceDirectory && fs.existsSync(PILOT)) {
      const destination = path.join(evidenceDirectory, "qualification-sidecars");
      if (!fs.existsSync(destination)) fs.cpSync(PILOT, destination, { recursive: true });
    }
    exportRun(status, config, evidenceDirectory);
  }
  return ["accepted_and_published", "diagnostic_complete", "evidence_complete"].includes(status.status) ? 0 : 2;
}

function selfTest() {
  assert.equal(
    finalSidecarName({ measurement: { workload: "dense" } }, 1),
    "final-dense-1.json",
  );
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
  assert.throws(() => finalSidecarName({ measurement: { workload: "dense" } }, 0));
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
  // The dedicated provider section is validated before a build runs, so its
  // rules have to hold without a host. Each case here is a config that would
  // otherwise fail only after the box had been provisioned and compiled.
  const dedicatedHost = {
    name: "ci-bench",
    cpu_model: "AMD Ryzen 5 3600X",
    logical_cpus: 6,
    physical_cores: 6,
    pinned_cpus: "4",
    max_runtime_minutes: 180,
  };
  assert.doesNotThrow(() =>
    validateDedicatedProvider({ provider: { host: { ...dedicatedHost } } }),
  );
  // SMT on: logical CPUs exceed physical cores, so a sibling thread could
  // contend with a timed leg.
  assert.throws(
    () =>
      validateDedicatedProvider({
        provider: { host: { ...dedicatedHost, logical_cpus: 12 } },
      }),
    /SMT off/,
  );
  assert.throws(() =>
    validateDedicatedProvider({ provider: { host: { ...dedicatedHost, pinned_cpus: "" } } }),
  );
  assert.throws(() =>
    validateDedicatedProvider({ provider: { host: { ...dedicatedHost, extra: 1 } } }),
  );
  assert.throws(() => validateDedicatedProvider({ provider: { host: {} } }));
  assert.equal(isDedicatedHost({ schema: "rustia.dedicated-fixed-benchmark.v1" }), true);
  assert.equal(isDedicatedHost({ schema: "rustia.linode-fixed-benchmark.v2" }), false);
  // The dedicated host runs the same paired protocol as the accepted DO gates:
  // qualification, counterbalanced legs, conservative interval.
  assert.equal(
    usesPairedDigitalOceanProtocol({ schema: "rustia.dedicated-fixed-benchmark.v1" }),
    true,
  );

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
  assert.equal(
    timedStealGatePasses(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v4",
        host_gate: { maximum_steal_ticks: 0, maximum_timed_steal_percent: 0.1 },
      },
      { total_ticks: 1310, steal_ticks: 1 },
    ),
    true,
  );
  assert.equal(
    timedStealGatePasses(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v4",
        host_gate: { maximum_steal_ticks: 0, maximum_timed_steal_percent: 0.1 },
      },
      { total_ticks: 1000, steal_ticks: 2 },
    ),
    false,
  );
  assert.equal(
    timedStealGatePasses(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v5",
        host_gate: { maximum_steal_ticks: 0, maximum_timed_steal_percent: 0.5 },
      },
      { total_ticks: 1000, steal_ticks: 5 },
    ),
    true,
  );
  assert.equal(
    timedStealGatePasses(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v5",
        host_gate: { maximum_steal_ticks: 0, maximum_timed_steal_percent: 0.5 },
      },
      { total_ticks: 1000, steal_ticks: 6 },
    ),
    false,
  );
  assert.equal(quantileSorted([1, 2, 3, 4], 0.25), 1.75);
  assert.deepEqual(pairedPilotOrder(1), ["baseline", "candidate"]);
  assert.deepEqual(pairedPilotOrder(2), ["candidate", "baseline"]);
  assert.deepEqual(pairedPilotOrder(5), ["baseline", "candidate"]);
  assert.equal(relativeSpreadPercent([1, 1.01, 0.99]), (1.01 - 0.99) / 0.99 * 100);
  assert.throws(() => relativeSpreadPercent([1]));
  assert.equal(
    pairedQualificationResult(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v6",
        qualification: { max_ratio_spread_percent: 3 },
      },
      [
        { order: ["baseline", "candidate"], candidate_over_baseline_ratio: 0.99 },
        { order: ["candidate", "baseline"], candidate_over_baseline_ratio: 1.01 },
      ],
    ).passed,
    true,
  );
  assert.equal(
    pairedQualificationResult(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v6",
        qualification: { max_ratio_spread_percent: 3 },
      },
      [
        { order: ["baseline", "candidate"], candidate_over_baseline_ratio: 0.95 },
        { order: ["candidate", "baseline"], candidate_over_baseline_ratio: 1.05 },
        { order: ["candidate", "baseline"], candidate_over_baseline_ratio: 1.04 },
        { order: ["baseline", "candidate"], candidate_over_baseline_ratio: 0.96 },
      ],
    ).passed,
    true,
  );
  assert.equal(
    pairedQualificationResult(
      {
        schema: "rustia.digitalocean-fixed-benchmark.v6",
        qualification: { max_ratio_spread_percent: 3 },
      },
      [
        { order: ["baseline", "candidate"], candidate_over_baseline_ratio: 0.95 },
        { order: ["candidate", "baseline"], candidate_over_baseline_ratio: 1.05 },
        { order: ["candidate", "baseline"], candidate_over_baseline_ratio: 1.04 },
        { order: ["baseline", "candidate"], candidate_over_baseline_ratio: 1.01 },
      ],
    ).passed,
    false,
  );
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
  const hash = "a".repeat(64);
  const baselineCommit = "b".repeat(40);
  const candidateCommit = "c".repeat(40);
  const harnessCommit = "d".repeat(40);
  const memoryConfig = {
    schema: "rustia.linode-fixed-benchmark.v2",
    provider: {},
    source: {
      baseline_commit: baselineCommit,
      candidate_commit: candidateCommit,
      harness_commit: harnessCommit,
      expected_sha256: {
        baseline_original_bench_manifest: hash,
        candidate_original_bench_manifest: hash,
        overlay_bench_manifest: hash,
        sampler: hash,
        analyzer: hash,
        cargo_lock: hash,
        toolchain: hash,
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
    host_gate: {
      benchmark_cpu: 1,
      cooldown_seconds: 1,
      idle_window_seconds: 1,
      minimum_idle_percent: 95,
      maximum_load_1m: 0.5,
      maximum_steal_ticks: 0,
      maximum_timed_steal_percent: 0.1,
      wait_timeout_seconds: 30,
    },
    publication: { mode: "evidence_only" },
  };
  assert.equal(validateConfig(memoryConfig), memoryConfig);
  assert.throws(() => validateConfig({ ...memoryConfig, measurement: { ...memoryConfig.measurement, protocol: "other" } }));
  assert.throws(() => validateConfig({ ...memoryConfig, measurement: { ...memoryConfig.measurement, accepted_rounds: 15 } }));
  assert.throws(() => validateConfig({ ...memoryConfig, measurement: { ...memoryConfig.measurement, scenarios: [...VM_STRING_SCENARIOS].reverse() } }));
  assert.throws(() => validateConfig({ ...memoryConfig, qualification: {} }));
  assert.throws(() => validateConfig({ ...memoryConfig, publication: { mode: "acceptance" } }));
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
  assert.deepEqual(
    firstRoles,
    ["baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate",
      "baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate"],
  );
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
  assert.equal(memorySampleId("round", 3, "candidate", "short_loaded"), "round-03-candidate-short-loaded");
  assert.equal(
    memoryEvidenceRelativePath(candidateCommit),
    path.join("bench/evidence/vm-string-memory", candidateCommit),
  );
  assert.throws(() => memoryEvidenceRelativePath("short"));
  assert.equal(
    parseCanonicalJsonLine('{"schema":"fixture"}\n', "fixture").value.schema,
    "fixture",
  );
  assert.throws(() => parseCanonicalJsonLine('{ "schema": "fixture" }\n', "fixture"));
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
