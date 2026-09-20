#!/usr/bin/env node
"use strict";

// Validates the frozen owned-host config, bench/cloud/dedicated-core2-tape-sparse.json.
//
// That file is the one place the repository records what the owned benchmark
// host guarantees: its provenance label, CPU model, core counts, the cores held
// off the scheduler, and the core a timed leg runs on. Three live tools copy
// its provider block or cross-check against it -- the matrix and memory config
// generators and the history-backfill freezer -- so a config that disagreed
// with itself would reach every lane before any host saw it.
//
// The rules came from the retired fixed-throughput runner's `validate` command
// and are kept whole rather than trimmed to what today's readers consume: the
// file is also the frozen A/B protocol, and "validates" has to keep meaning
// what it meant when the file was frozen.
//
// tools/run-linux-history-backfill.js carries the same provider-block rules.
// It cannot require this file: the on-host runners ship to the machine as
// single scripts, so each holds the rules it needs.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const REPO = path.resolve(__dirname, "..");
const CONFIG = "bench/cloud/dedicated-core2-tape-sparse.json";
const SCHEMA = "rustia.dedicated-fixed-benchmark.v1";
const FIXED_THROUGHPUT_PROTOCOL = "fixed_throughput_v1";
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
// provider.host.name is the provenance label every owned-host timing and memory
// row carries into published data, copied from this file by the config
// generators. A label, never an address: no dots and no colons, so it can be
// neither a fully qualified name nor an IPv4 or IPv6 address. The same literal
// is carried by every emitter and validator of that label; the on-host runners
// refuse a bad one too, but only after the block has been copied into a lane
// and shipped to the machine.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

function fail(message) {
  throw new Error(message);
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
  const actual = Object.keys(requireObject(value, location)).sort();
  const wanted = expected.slice().sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(`${location} keys must be ${wanted.join(", ")}; got ${actual.join(", ")}`);
  }
}

// A taskset-style CPU list: "4", "4,5", "4-5".
function parseCpuList(text, location) {
  const cpus = new Set();
  for (const token of String(text).trim().split(",")) {
    const match = token.match(/^(\d+)(?:-(\d+))?$/);
    if (!match) fail(`${location} has invalid CPU-list syntax`);
    const start = Number(match[1]);
    const end = match[2] === undefined ? start : Number(match[2]);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || end < start || end - start > 4096) {
      fail(`${location} has an invalid CPU range`);
    }
    for (let cpu = start; cpu <= end; cpu += 1) cpus.add(cpu);
  }
  return [...cpus].sort((left, right) => left - right);
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
  if (!HOST_LABEL.test(host.name)) {
    fail(
      `config.provider.host.name ${JSON.stringify(host.name)} is not a label: ` +
        `it may contain only [A-Za-z0-9_-], never an address`,
    );
  }
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
  if (config.schema !== SCHEMA) fail(`config.schema must be ${SCHEMA}`);
  validateDedicatedProvider(config);
  for (const field of ["source", "measurement", "qualification", "host_gate", "publication"]) {
    requireObject(config[field], `config.${field}`);
  }
  const { source, measurement } = config;
  if (measurement.protocol !== undefined && measurement.protocol !== FIXED_THROUGHPUT_PROTOCOL) {
    fail(`measurement.protocol must be absent or ${FIXED_THROUGHPUT_PROTOCOL}`);
  }

  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    if (!FULL_COMMIT.test(source[field])) fail(`config.source.${field} must be a full commit`);
  }
  if (source.baseline_commit === source.candidate_commit) {
    fail("baseline and candidate commits must differ");
  }
  const hashes = requireObject(source.expected_sha256, "config.source.expected_sha256");
  if (Object.keys(hashes).length === 0) fail("config.source.expected_sha256 must name at least one file");
  for (const [field, value] of Object.entries(hashes)) {
    if (!SHA256.test(value)) fail(`config.source.expected_sha256.${field} must be a SHA-256`);
  }

  requireString(measurement.workload, "measurement.workload");
  if (!Array.isArray(measurement.run_order)) fail("measurement.run_order must be an array");
  const expectedLegs = requireInteger(
    measurement.expected_legs_per_group,
    "measurement.expected_legs_per_group",
    1,
  );
  if (measurement.run_order.length !== expectedLegs * 2) {
    fail("measurement.run_order has the wrong length");
  }
  const counts = { baseline: 0, candidate: 0 };
  const files = new Set();
  for (const leg of measurement.run_order) {
    requireObject(leg, "measurement.run_order[]");
    if (!Object.hasOwn(counts, leg.variant)) fail("run-order variant must be baseline or candidate");
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
  // The paired protocol: every adjacent pair holds one leg of each side, so a
  // drift across the run lands on both sides equally.
  for (let index = 0; index < measurement.run_order.length; index += 2) {
    if (measurement.run_order[index].variant === measurement.run_order[index + 1].variant) {
      fail("run-order pairs must contain one baseline and one candidate leg");
    }
  }
  requireInteger(measurement.fixed_n, "measurement.fixed_n", 1);
  requireInteger(measurement.repetitions, "measurement.repetitions", 1);

  requireExactKeys(
    config.qualification,
    ["paired_pilot_rounds", "max_ratio_spread_percent"],
    "qualification",
  );
  requireInteger(config.qualification.paired_pilot_rounds, "qualification.paired_pilot_rounds", 2);
  requireNumber(config.qualification.max_ratio_spread_percent, "qualification.max_ratio_spread_percent", 0);

  const benchmarkCpu = requireInteger(config.host_gate.benchmark_cpu, "host_gate.benchmark_cpu", 0);
  requireInteger(config.host_gate.maximum_steal_ticks, "host_gate.maximum_steal_ticks", 0);
  // The timed core must be one the host holds off the scheduler. The two
  // config generators copy both fields from this file, so a disagreement here
  // would be copied into every lane.
  const pinned = parseCpuList(config.provider.host.pinned_cpus, "config.provider.host.pinned_cpus");
  if (!pinned.includes(benchmarkCpu)) {
    fail(
      `host_gate.benchmark_cpu ${benchmarkCpu} is not in provider.host.pinned_cpus ` +
        `${config.provider.host.pinned_cpus}`,
    );
  }

  if (!["acceptance", "diagnostic"].includes(config.publication.mode)) {
    fail("publication.mode must be acceptance or diagnostic");
  }
  requireInteger(config.publication.final_measurements, "publication.final_measurements", 3);
  return config;
}

function loadConfig(relative) {
  const file = path.join(REPO, relative);
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot load config ${relative}: ${error.message}`);
  }
}

function selfTest() {
  // The provider rules have to hold without a host. Each case is a config
  // that would otherwise fail only on the machine, after a build.
  const dedicatedHost = {
    name: "ci-bench",
    cpu_model: "AMD Ryzen 5 3600X",
    logical_cpus: 6,
    physical_cores: 6,
    pinned_cpus: "4",
    max_runtime_minutes: 180,
  };
  assert.doesNotThrow(() => validateDedicatedProvider({ provider: { host: { ...dedicatedHost } } }));
  // SMT on: logical CPUs exceed physical cores, so a sibling thread could
  // contend with a timed leg.
  assert.throws(
    () => validateDedicatedProvider({ provider: { host: { ...dedicatedHost, logical_cpus: 12 } } }),
    /SMT off/,
  );
  assert.throws(() => validateDedicatedProvider({ provider: { host: { ...dedicatedHost, pinned_cpus: "" } } }));
  assert.throws(() => validateDedicatedProvider({ provider: { host: { ...dedicatedHost, extra: 1 } } }));
  assert.throws(() => validateDedicatedProvider({ provider: { host: {} } }));
  assert.throws(() => validateDedicatedProvider({ provider: { host: dedicatedHost, region: "x" } }));
  // The name becomes the `host` label on published rows, so it must be a label
  // and never an address -- refused here, before a generator copies it.
  for (const name of ["bench.example.com", "10.0.0.5", "fe80::1", "ci bench", "x".repeat(129)]) {
    assert.throws(
      () => validateDedicatedProvider({ provider: { host: { ...dedicatedHost, name } } }),
      /is not a label/,
      `host name ${JSON.stringify(name)} must be refused`,
    );
  }
  assert.doesNotThrow(() => validateDedicatedProvider({ provider: { host: { ...dedicatedHost, name: "x".repeat(128) } } }));

  // The committed config is the one that matters, so the rules are exercised
  // on it and on one-edit variants of it rather than on a lookalike fixture.
  const committed = loadConfig(CONFIG);
  assert.equal(validateConfig(committed).schema, SCHEMA);
  const variant = (edit) => {
    const copy = JSON.parse(JSON.stringify(committed));
    edit(copy);
    return copy;
  };
  const rejects = (edit, pattern) => assert.throws(() => validateConfig(variant(edit)), pattern);
  rejects((config) => {
    config.schema = "rustia.linode-fixed-benchmark.v2";
  }, /config\.schema/);
  rejects((config) => {
    config.provider.host.logical_cpus = 12;
  }, /SMT off/);
  rejects((config) => {
    config.provider.host.name = "bench.example.com";
  }, /provider\.host\.name .* is not a label/);
  rejects((config) => {
    config.source.candidate_commit = config.source.baseline_commit;
  }, /must differ/);
  rejects((config) => {
    config.source.harness_commit = "daaf422";
  }, /harness_commit must be a full commit/);
  rejects((config) => {
    config.source.expected_sha256.workload = "abc";
  }, /SHA-256/);
  // A missing hash table is a structured failure, not a TypeError.
  rejects((config) => {
    delete config.source.expected_sha256;
  }, /expected_sha256 must be an object/);
  rejects((config) => {
    config.measurement.protocol = "vm_string_memory_v1";
  }, /measurement\.protocol/);
  rejects((config) => {
    config.measurement.run_order.pop();
  }, /wrong length/);
  rejects((config) => {
    config.measurement.run_order[1].variant = "baseline";
    config.measurement.run_order[3].variant = "candidate";
  }, /one baseline and one candidate/);
  rejects((config) => {
    config.measurement.run_order[0].file = config.measurement.run_order[1].file;
  }, /duplicate run-order file/);
  rejects((config) => {
    config.measurement.run_order[0].file = "../escape.json";
  }, /JSON basenames/);
  rejects((config) => {
    config.measurement.fixed_n = 0;
  }, /fixed_n/);
  rejects((config) => {
    config.qualification.paired_pilot_rounds = 1;
  }, /paired_pilot_rounds/);
  rejects((config) => {
    config.qualification.extra = 1;
  }, /qualification keys/);
  rejects((config) => {
    config.host_gate.benchmark_cpu = 5;
  }, /not in provider\.host\.pinned_cpus/);
  rejects((config) => {
    config.publication.mode = "screening";
  }, /publication\.mode/);
  rejects((config) => {
    config.publication.final_measurements = 2;
  }, /final_measurements/);
  process.stdout.write("dedicated config checker self-test passed\n");
}

function main() {
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "--self-test") {
    selfTest();
    return;
  }
  if (args.length !== 0) {
    process.stderr.write("Usage: node tools/check-dedicated-config.js [--self-test]\n");
    process.exitCode = 2;
    return;
  }
  const config = validateConfig(loadConfig(CONFIG));
  process.stdout.write(
    `${CONFIG}: ${config.schema} validates ` +
      `(${config.measurement.workload} n=${config.measurement.fixed_n} ` +
      `reps=${config.measurement.repetitions} legs=${config.measurement.run_order.length})\n`,
  );
}

try {
  main();
} catch (error) {
  process.stderr.write(`check-dedicated-config: ${error.message}\n`);
  process.exitCode = 1;
}
