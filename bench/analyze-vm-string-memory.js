#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");

const SAMPLE_SCHEMA = "rustia.bench.vm-string-sample.v1";
const OUTPUT_SCHEMA = "rustia.bench.vm-string-memory-analysis.v1";
const EXPECTED_ROUNDS = 16;
const EXPECTED_BOOTSTRAP_REPETITIONS = 100_000;
const EXPECTED_BOOTSTRAP_SEED = 0x52555354;
const EXPECTED_VM_COUNT = 512;
const EXPECTED_CONSTANT_SLOTS = 1_024;
const EXPECTED_INSTRUCTION_SLOTS = 1_025;

const ROLES = Object.freeze(["baseline", "candidate"]);
const SCENARIOS = Object.freeze([
  "empty",
  "short_unloaded",
  "short_loaded",
  "long_unloaded",
  "long_loaded",
]);

const SNAPSHOTS = Object.freeze(["clean", "program_ready", "vms_ready", "after_run"]);
const STATUS_FIELDS = Object.freeze([
  "vm_rss",
  "vm_hwm",
  "rss_anon",
  "rss_file",
  "rss_shmem",
]);
const SMAPS_FIELDS = Object.freeze([
  "rss",
  "pss",
  "private_clean",
  "private_dirty",
  "anonymous",
  "swap",
]);
const TIMING_FIELDS = Object.freeze(["program_build", "vm_create", "program_run"]);

const SCENARIO_SPEC = Object.freeze({
  empty: Object.freeze({
    executesConstants: false,
    stringKind: "none",
    stringCount: 0,
    bytesEach: 0,
    nilConstantCount: 1_024,
    expectedResultKind: "nil",
    expectedResultBytes: null,
    expectedResultFnv1a64: null,
  }),
  short_unloaded: Object.freeze({
    executesConstants: false,
    stringKind: "short",
    stringCount: 1_024,
    bytesEach: 13,
    nilConstantCount: 0,
    expectedResultKind: "nil",
    expectedResultBytes: null,
    expectedResultFnv1a64: null,
  }),
  short_loaded: Object.freeze({
    executesConstants: true,
    stringKind: "short",
    stringCount: 1_024,
    bytesEach: 13,
    nilConstantCount: 0,
    expectedResultKind: "string",
    expectedResultBytes: 13,
    expectedResultFnv1a64: expectedLoadedHash(1_023, 13, 37, 53),
  }),
  long_unloaded: Object.freeze({
    executesConstants: false,
    stringKind: "long",
    stringCount: 128,
    bytesEach: 2_048,
    nilConstantCount: 896,
    expectedResultKind: "nil",
    expectedResultBytes: null,
    expectedResultFnv1a64: null,
  }),
  long_loaded: Object.freeze({
    executesConstants: true,
    stringKind: "long",
    stringCount: 128,
    bytesEach: 2_048,
    nilConstantCount: 896,
    expectedResultKind: "string",
    expectedResultBytes: 2_048,
    expectedResultFnv1a64: expectedLoadedHash(127, 2_048, 29, 17),
  }),
});

const CONTRASTS = Object.freeze({
  short_unloaded_minus_empty: Object.freeze(["short_unloaded", "empty"]),
  long_unloaded_minus_empty: Object.freeze(["long_unloaded", "empty"]),
  short_loaded_minus_short_unloaded: Object.freeze(["short_loaded", "short_unloaded"]),
  long_loaded_minus_long_unloaded: Object.freeze(["long_loaded", "long_unloaded"]),
});

const CORROBORATING_MEMORY_METRICS = Object.freeze([
  "smaps_rollup_private_dirty",
  "smaps_rollup_rss",
  "status_rss_anon",
]);

const USAGE = `Usage:
  node bench/analyze-vm-string-memory.js \\
    --input samples.jsonl \\
    --baseline BASELINE-COMMIT \\
    --candidate CANDIDATE-COMMIT \\
    --rounds 16 --bootstrap-reps 100000 --seed 1381323604

Options:
  --input FILE             Canonical vm-string worker JSONL.
  --baseline COMMIT        Expected baseline commit (40 lowercase hex digits).
  --candidate COMMIT       Expected candidate commit (40 lowercase hex digits).
  --rounds N               Accepted paired rounds; fixed at 16.
  --bootstrap-reps N       Paired percentile bootstrap repetitions; fixed at 100000.
  --seed N                 Unsigned bootstrap seed; fixed at 1381323604 (0x52555354).
  --self-test              Run dependency-free fixture and rejection tests.
  --help                   Show this help.
`;

function fail(message) {
  throw new Error(message);
}

function isPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function requireObject(value, location) {
  if (!isPlainObject(value)) {
    fail(`${location} must be an object`);
  }
  return value;
}

function requireExactKeys(value, expectedKeys, location) {
  const object = requireObject(value, location);
  const expected = new Set(expectedKeys);
  const actual = Object.keys(object);
  const missing = expectedKeys.filter((key) => !Object.hasOwn(object, key));
  const unexpected = actual.filter((key) => !expected.has(key));
  if (missing.length > 0 || unexpected.length > 0) {
    const details = [];
    if (missing.length > 0) details.push(`missing ${missing.map(JSON.stringify).join(", ")}`);
    if (unexpected.length > 0) {
      details.push(`unexpected ${unexpected.map(JSON.stringify).join(", ")}`);
    }
    fail(`${location} has non-canonical keys: ${details.join("; ")}`);
  }
  return object;
}

function requireString(value, location) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${location} must be a non-empty string`);
  }
  return value;
}

function requireExactString(value, expected, location) {
  if (value !== expected) {
    fail(`${location} must be ${JSON.stringify(expected)}, got ${JSON.stringify(value)}`);
  }
  return value;
}

function requireBoolean(value, expected, location) {
  if (value !== expected) {
    fail(`${location} must be ${expected}`);
  }
  return value;
}

function requireSafeInteger(value, location, minimum = 0) {
  if (!Number.isSafeInteger(value) || value < minimum) {
    fail(`${location} must be a safe integer >= ${minimum}`);
  }
  return value;
}

function requireExactInteger(value, expected, location) {
  requireSafeInteger(value, location, 0);
  if (value !== expected) {
    fail(`${location} must be ${expected}, got ${value}`);
  }
  return value;
}

function requireNullableExact(value, expected, location) {
  if (value !== expected) {
    fail(`${location} must be ${JSON.stringify(expected)}, got ${JSON.stringify(value)}`);
  }
  return value;
}

function requireLowerHex(value, length, location) {
  if (typeof value !== "string" || !new RegExp(`^[0-9a-f]{${length}}$`).test(value)) {
    fail(`${location} must be exactly ${length} lowercase hexadecimal digits`);
  }
  return value;
}

function requireSafeId(value, location) {
  if (
    typeof value !== "string" ||
    value.length < 1 ||
    value.length > 128 ||
    !/^[A-Za-z0-9._-]+$/.test(value)
  ) {
    fail(`${location} must be 1..128 ASCII letters, digits, '.', '_', or '-'`);
  }
  return value;
}

function cleanNumber(value) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    fail(`analysis output contains a non-finite number: ${value}`);
  }
  const cleaned = Number(value.toPrecision(15));
  return Object.is(cleaned, -0) ? 0 : cleaned;
}

function sorted(values) {
  return values.slice().sort((left, right) => left - right);
}

// Hyndman/Fan type 7, matching the default quantile definition in R.
function quantileSorted(ordered, probability) {
  assert(ordered.length > 0);
  assert(probability >= 0 && probability <= 1);
  if (ordered.length === 1) return ordered[0];
  const position = (ordered.length - 1) * probability;
  const lowerIndex = Math.floor(position);
  const upperIndex = Math.ceil(position);
  if (lowerIndex === upperIndex) return ordered[lowerIndex];
  const fraction = position - lowerIndex;
  return ordered[lowerIndex] * (1 - fraction) + ordered[upperIndex] * fraction;
}

function quantile(values, probability) {
  return quantileSorted(sorted(values), probability);
}

function summary(values) {
  if (!Array.isArray(values) || values.length === 0) {
    fail("cannot summarize an empty value set");
  }
  for (const [index, value] of values.entries()) {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      fail(`summary value[${index}] must be finite`);
    }
  }
  const ordered = sorted(values);
  return {
    sample_count: ordered.length,
    median: cleanNumber(quantileSorted(ordered, 0.5)),
    q1: cleanNumber(quantileSorted(ordered, 0.25)),
    q3: cleanNumber(quantileSorted(ordered, 0.75)),
    min: cleanNumber(ordered[0]),
    max: cleanNumber(ordered[ordered.length - 1]),
  };
}

function indexedBytes(index, length, multiplier, stride) {
  const bytes = Buffer.alloc(length);
  let remaining = BigInt(index);
  for (let offset = 0; offset < 8; offset += 1) {
    bytes[offset] = Number(remaining & 0xffn);
    remaining >>= 8n;
  }
  for (let offset = 0; offset < length - 8; offset += 1) {
    bytes[8 + offset] = (index * multiplier + offset * stride) & 0xff;
  }
  return bytes;
}

function fnv1a64Hex(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = BigInt.asUintN(64, hash * 0x100000001b3n);
  }
  return hash.toString(16).padStart(16, "0");
}

function expectedLoadedHash(index, length, multiplier, stride) {
  return fnv1a64Hex(indexedBytes(index, length, multiplier, stride));
}

function validateStatus(value, location) {
  const status = requireExactKeys(value, STATUS_FIELDS, location);
  const normalized = {};
  for (const field of STATUS_FIELDS) {
    normalized[field] = requireSafeInteger(status[field], `${location}.${field}`, 0);
  }
  return normalized;
}

function validateSmaps(value, location) {
  const smaps = requireExactKeys(value, SMAPS_FIELDS, location);
  const normalized = {};
  for (const field of SMAPS_FIELDS) {
    normalized[field] = requireSafeInteger(smaps[field], `${location}.${field}`, 0);
  }
  if (normalized.swap !== 0) {
    fail(`${location}.swap must be 0 KiB, got ${normalized.swap}`);
  }
  return normalized;
}

function validateSnapshot(value, location) {
  const snapshot = requireExactKeys(value, ["status", "smaps_rollup"], location);
  return {
    status: validateStatus(snapshot.status, `${location}.status`),
    smapsRollup: validateSmaps(snapshot.smaps_rollup, `${location}.smaps_rollup`),
  };
}

function validateScenario(value, location) {
  const scenario = requireExactKeys(
    value,
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
    location,
  );
  const id = requireString(scenario.id, `${location}.id`);
  const specification = SCENARIO_SPEC[id];
  if (specification === undefined) {
    fail(`${location}.id must be one of ${SCENARIOS.map(JSON.stringify).join(", ")}`);
  }
  requireExactInteger(scenario.vm_count, EXPECTED_VM_COUNT, `${location}.vm_count`);
  requireExactString(scenario.profile, "reproducible-standard", `${location}.profile`);
  requireExactInteger(
    scenario.constant_slots,
    EXPECTED_CONSTANT_SLOTS,
    `${location}.constant_slots`,
  );
  requireExactInteger(
    scenario.instruction_slots,
    EXPECTED_INSTRUCTION_SLOTS,
    `${location}.instruction_slots`,
  );
  requireBoolean(
    scenario.executes_constants,
    specification.executesConstants,
    `${location}.executes_constants`,
  );
  requireExactInteger(
    scenario.nil_constant_count,
    specification.nilConstantCount,
    `${location}.nil_constant_count`,
  );

  const strings = requireExactKeys(
    scenario.strings,
    ["kind", "count", "bytes_each", "generator"],
    `${location}.strings`,
  );
  requireExactString(strings.kind, specification.stringKind, `${location}.strings.kind`);
  requireExactInteger(strings.count, specification.stringCount, `${location}.strings.count`);
  requireExactInteger(
    strings.bytes_each,
    specification.bytesEach,
    `${location}.strings.bytes_each`,
  );
  requireExactString(
    strings.generator,
    "indexed-binary-v1",
    `${location}.strings.generator`,
  );
  return id;
}

function validateValidation(value, scenarioId, location) {
  const validation = requireExactKeys(
    value,
    [
      "created_vms",
      "ran_vms",
      "expected_result_kind",
      "expected_result_bytes",
      "expected_result_fnv1a64",
      "all_results_matched",
    ],
    location,
  );
  const specification = SCENARIO_SPEC[scenarioId];
  requireExactInteger(validation.created_vms, EXPECTED_VM_COUNT, `${location}.created_vms`);
  requireExactInteger(validation.ran_vms, EXPECTED_VM_COUNT, `${location}.ran_vms`);
  requireExactString(
    validation.expected_result_kind,
    specification.expectedResultKind,
    `${location}.expected_result_kind`,
  );
  requireNullableExact(
    validation.expected_result_bytes,
    specification.expectedResultBytes,
    `${location}.expected_result_bytes`,
  );
  requireNullableExact(
    validation.expected_result_fnv1a64,
    specification.expectedResultFnv1a64,
    `${location}.expected_result_fnv1a64`,
  );
  requireBoolean(validation.all_results_matched, true, `${location}.all_results_matched`);
}

function validateSample(value, location) {
  const root = requireExactKeys(
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
  requireExactInteger(root.schema_version, 1, `${location}.schema_version`);
  requireExactString(root.schema, SAMPLE_SCHEMA, `${location}.schema`);
  const sampleId = requireSafeId(root.sample_id, `${location}.sample_id`);
  const protocolSha256 = requireLowerHex(
    root.protocol_sha256,
    64,
    `${location}.protocol_sha256`,
  );
  const timestampUnix = requireSafeInteger(root.timestamp_unix, `${location}.timestamp_unix`, 1);

  const build = requireExactKeys(root.build, ["role", "commit"], `${location}.build`);
  const role = requireString(build.role, `${location}.build.role`);
  if (!ROLES.includes(role)) {
    fail(`${location}.build.role must be "baseline" or "candidate"`);
  }
  const commit = requireLowerHex(build.commit, 40, `${location}.build.commit`);
  const round = requireSafeInteger(root.round, `${location}.round`, 1);
  const scenarioId = validateScenario(root.scenario, `${location}.scenario`);

  const platform = requireExactKeys(
    root.platform,
    ["os", "arch", "pid", "memory_probe_warmups"],
    `${location}.platform`,
  );
  requireExactString(platform.os, "linux", `${location}.platform.os`);
  requireExactString(platform.arch, "x86_64", `${location}.platform.arch`);
  const pid = requireSafeInteger(platform.pid, `${location}.platform.pid`, 1);
  requireExactInteger(
    platform.memory_probe_warmups,
    2,
    `${location}.platform.memory_probe_warmups`,
  );

  const layout = requireExactKeys(
    root.layout_bytes,
    ["value", "instruction", "constant", "vm"],
    `${location}.layout_bytes`,
  );
  const layoutBytes = {};
  for (const field of ["value", "instruction", "constant", "vm"]) {
    layoutBytes[field] = requireSafeInteger(
      layout[field],
      `${location}.layout_bytes.${field}`,
      1,
    );
  }

  const timingObject = requireExactKeys(root.timing_ns, TIMING_FIELDS, `${location}.timing_ns`);
  const timingNs = {};
  for (const field of TIMING_FIELDS) {
    timingNs[field] = requireSafeInteger(
      timingObject[field],
      `${location}.timing_ns.${field}`,
      1,
    );
  }

  const memoryObject = requireExactKeys(root.memory_kib, SNAPSHOTS, `${location}.memory_kib`);
  const memoryKib = {};
  for (const snapshot of SNAPSHOTS) {
    memoryKib[snapshot] = validateSnapshot(
      memoryObject[snapshot],
      `${location}.memory_kib.${snapshot}`,
    );
  }
  validateValidation(root.validation, scenarioId, `${location}.validation`);

  return {
    location,
    sampleId,
    protocolSha256,
    timestampUnix,
    role,
    commit,
    round,
    scenarioId,
    pid,
    layoutBytes,
    timingNs,
    memoryKib,
  };
}

function loadJsonl(inputFile) {
  let bytes;
  try {
    const status = fs.statSync(inputFile);
    if (!status.isFile()) fail(`--input ${JSON.stringify(inputFile)} is not a regular file`);
    bytes = fs.readFileSync(inputFile);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("--input ")) throw error;
    fail(`cannot read --input ${JSON.stringify(inputFile)}: ${error.message}`);
  }
  const text = bytes.toString("utf8");
  return {
    inputSha256: crypto.createHash("sha256").update(bytes).digest("hex"),
    records: parseJsonlText(text, inputFile),
  };
}

function parseJsonlText(text, sourceName) {
  if (text.length === 0) fail(`${JSON.stringify(sourceName)} is empty`);
  const lines = text.split("\n");
  if (lines.at(-1) === "") lines.pop();
  const records = [];
  for (const [index, rawLine] of lines.entries()) {
    const lineNumber = index + 1;
    const line = rawLine.endsWith("\r") ? rawLine.slice(0, -1) : rawLine;
    if (line.trim().length === 0) {
      fail(`${JSON.stringify(sourceName)}:${lineNumber} is a blank JSONL record`);
    }
    if (line !== line.trim()) {
      fail(`${JSON.stringify(sourceName)}:${lineNumber} has non-canonical surrounding whitespace`);
    }
    let value;
    try {
      value = JSON.parse(line);
    } catch (error) {
      fail(`${JSON.stringify(sourceName)}:${lineNumber} is invalid JSON: ${error.message}`);
    }
    records.push(validateSample(value, `${JSON.stringify(sourceName)}:${lineNumber}`));
  }
  return records;
}

function sameObject(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function validateProtocol(records, options) {
  const expectedSampleCount = ROLES.length * options.rounds * SCENARIOS.length;
  if (records.length !== expectedSampleCount) {
    fail(
      `input must contain exactly ${expectedSampleCount} samples ` +
        `(2 builds x ${options.rounds} rounds x ${SCENARIOS.length} scenarios), ` +
        `got ${records.length}`,
    );
  }

  const sampleIds = new Map();
  const tuples = new Map();
  let protocolSha256 = null;
  const layouts = { baseline: null, candidate: null };
  const grouped = {
    baseline: new Map(),
    candidate: new Map(),
  };

  for (const record of records) {
    const expectedCommit = record.role === "baseline" ? options.baselineCommit : options.candidateCommit;
    if (record.commit !== expectedCommit) {
      fail(
        `${record.location}.build.commit is ${record.commit}, but --${record.role} is ` +
          `${expectedCommit}`,
      );
    }
    if (record.round > options.rounds) {
      fail(`${record.location}.round must be between 1 and ${options.rounds}, got ${record.round}`);
    }
    if (protocolSha256 === null) protocolSha256 = record.protocolSha256;
    if (record.protocolSha256 !== protocolSha256) {
      fail(
        `${record.location}.protocol_sha256 is ${record.protocolSha256}, expected the common ` +
          `protocol identity ${protocolSha256}`,
      );
    }

    const previousId = sampleIds.get(record.sampleId);
    if (previousId !== undefined) {
      fail(
        `${record.location}.sample_id duplicates ${JSON.stringify(record.sampleId)} from ` +
          previousId,
      );
    }
    sampleIds.set(record.sampleId, record.location);

    const tuple = `${record.role}/${record.round}/${record.scenarioId}`;
    const previousTuple = tuples.get(tuple);
    if (previousTuple !== undefined) {
      fail(`${record.location} duplicates protocol cell ${tuple} from ${previousTuple}`);
    }
    tuples.set(tuple, record.location);

    if (layouts[record.role] === null) layouts[record.role] = record.layoutBytes;
    if (!sameObject(layouts[record.role], record.layoutBytes)) {
      fail(
        `${record.location}.layout_bytes differs within the ${record.role} build: ` +
          `${JSON.stringify(record.layoutBytes)} versus ${JSON.stringify(layouts[record.role])}`,
      );
    }

    let round = grouped[record.role].get(record.round);
    if (round === undefined) {
      round = new Map();
      grouped[record.role].set(record.round, round);
    }
    round.set(record.scenarioId, record);
  }

  for (const role of ROLES) {
    for (let round = 1; round <= options.rounds; round += 1) {
      const samples = grouped[role].get(round);
      if (samples === undefined) fail(`missing all ${role} samples for accepted round ${round}`);
      for (const scenario of SCENARIOS) {
        if (!samples.has(scenario)) {
          fail(`missing ${role} accepted round ${round} scenario ${scenario}`);
        }
      }
    }
  }

  return { grouped, layouts, protocolSha256 };
}

function anonymous(record, snapshot) {
  return record.memoryKib[snapshot].smapsRollup.anonymous;
}

function corroboratingMemory(record, snapshot, metric) {
  switch (metric) {
    case "smaps_rollup_private_dirty":
      return record.memoryKib[snapshot].smapsRollup.private_dirty;
    case "smaps_rollup_rss":
      return record.memoryKib[snapshot].smapsRollup.rss;
    case "status_rss_anon":
      return record.memoryKib[snapshot].status.rss_anon;
    default:
      fail(`unknown corroborating memory metric ${JSON.stringify(metric)}`);
  }
}

function memoryMeasures(record) {
  const clean = anonymous(record, "clean");
  const programReady = anonymous(record, "program_ready");
  const afterRun = anonymous(record, "after_run");
  const corroboratingGrowthKib = {};
  for (const metric of CORROBORATING_MEMORY_METRICS) {
    corroboratingGrowthKib[metric] =
      corroboratingMemory(record, "after_run", metric) -
      corroboratingMemory(record, "program_ready", metric);
  }
  return {
    primaryGrowthKib: afterRun - programReady,
    oneTimeProgramGrowthKib: programReady - clean,
    wholeProcessGrowthKib: afterRun - clean,
    corroboratingGrowthKib,
  };
}

function timingMeasures(record) {
  const { program_build: programBuild, vm_create: vmCreate, program_run: programRun } =
    record.timingNs;
  return {
    one_time_program_build: programBuild,
    vm_create: vmCreate / EXPECTED_VM_COUNT,
    program_run: programRun / EXPECTED_VM_COUNT,
    vm_create_plus_program_run: (vmCreate + programRun) / EXPECTED_VM_COUNT,
    amortized_total_including_program_build:
      (programBuild + vmCreate + programRun) / EXPECTED_VM_COUNT,
  };
}

function roleRoundMetrics(groupedRole, rounds) {
  const output = new Map();
  for (let round = 1; round <= rounds; round += 1) {
    const records = groupedRole.get(round);
    const scenarios = {};
    for (const scenario of SCENARIOS) {
      const record = records.get(scenario);
      scenarios[scenario] = {
        memory: memoryMeasures(record),
        timing: timingMeasures(record),
      };
    }
    const contrasts = {};
    const corroboratingContrasts = Object.fromEntries(
      CORROBORATING_MEMORY_METRICS.map((metric) => [metric, {}]),
    );
    for (const [name, [left, right]] of Object.entries(CONTRASTS)) {
      contrasts[name] =
        (scenarios[left].memory.primaryGrowthKib - scenarios[right].memory.primaryGrowthKib) /
        EXPECTED_VM_COUNT;
      for (const metric of CORROBORATING_MEMORY_METRICS) {
        corroboratingContrasts[metric][name] =
          (scenarios[left].memory.corroboratingGrowthKib[metric] -
            scenarios[right].memory.corroboratingGrowthKib[metric]) /
          EXPECTED_VM_COUNT;
      }
    }
    output.set(round, { scenarios, contrasts, corroboratingContrasts });
  }
  return output;
}

function collectRounds(roundMetrics, rounds, selector) {
  const values = [];
  for (let round = 1; round <= rounds; round += 1) {
    values.push(selector(roundMetrics.get(round)));
  }
  return values;
}

function summarizeRole(roundMetrics, rounds) {
  const primaryGrowth = {};
  const oneTimeProgramGrowth = {};
  const wholeProcessGrowth = {};
  const oneTimeProgramBuild = {};
  const timingPerVm = {};
  for (const scenario of SCENARIOS) {
    primaryGrowth[scenario] = summary(
      collectRounds(
        roundMetrics,
        rounds,
        (metrics) => metrics.scenarios[scenario].memory.primaryGrowthKib / EXPECTED_VM_COUNT,
      ),
    );
    oneTimeProgramGrowth[scenario] = summary(
      collectRounds(
        roundMetrics,
        rounds,
        (metrics) => metrics.scenarios[scenario].memory.oneTimeProgramGrowthKib,
      ),
    );
    wholeProcessGrowth[scenario] = summary(
      collectRounds(
        roundMetrics,
        rounds,
        (metrics) => metrics.scenarios[scenario].memory.wholeProcessGrowthKib / EXPECTED_VM_COUNT,
      ),
    );
    oneTimeProgramBuild[scenario] = summary(
      collectRounds(
        roundMetrics,
        rounds,
        (metrics) => metrics.scenarios[scenario].timing.one_time_program_build,
      ),
    );
    timingPerVm[scenario] = {};
    for (const field of [
      "vm_create",
      "program_run",
      "vm_create_plus_program_run",
      "amortized_total_including_program_build",
    ]) {
      timingPerVm[scenario][field] = summary(
        collectRounds(roundMetrics, rounds, (metrics) => metrics.scenarios[scenario].timing[field]),
      );
    }
  }
  const contrasts = {};
  for (const name of Object.keys(CONTRASTS)) {
    contrasts[name] = summary(
      collectRounds(roundMetrics, rounds, (metrics) => metrics.contrasts[name]),
    );
  }
  const corroboratingContrasts = {};
  for (const metric of CORROBORATING_MEMORY_METRICS) {
    corroboratingContrasts[metric] = {};
    for (const name of Object.keys(CONTRASTS)) {
      corroboratingContrasts[metric][name] = summary(
        collectRounds(
          roundMetrics,
          rounds,
          (metrics) => metrics.corroboratingContrasts[metric][name],
        ),
      );
    }
  }
  return {
    memory: {
      primary_after_run_minus_program_ready_kib_per_vm: primaryGrowth,
      one_time_program_ready_minus_clean_kib: oneTimeProgramGrowth,
      diagnostic_after_run_minus_clean_kib_per_vm: wholeProcessGrowth,
      matched_primary_contrasts_kib_per_vm: contrasts,
      corroborating_matched_contrasts_kib_per_vm: corroboratingContrasts,
    },
    one_time_program_build_ns: oneTimeProgramBuild,
    timing_ns_per_vm: timingPerVm,
  };
}

function mulberry32(seed) {
  let state = seed >>> 0;
  return function random() {
    state = (state + 0x6d2b79f5) >>> 0;
    let value = state;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 0x1_0000_0000;
  };
}

function pairedPercentileBootstrap(values, repetitions, seed) {
  const random = mulberry32(seed);
  const resampled = new Float64Array(repetitions);
  const sample = new Array(values.length);
  for (let repetition = 0; repetition < repetitions; repetition += 1) {
    for (let index = 0; index < values.length; index += 1) {
      sample[index] = values[Math.floor(random() * values.length)];
    }
    sample.sort((left, right) => left - right);
    resampled[repetition] = quantileSorted(sample, 0.5);
  }
  resampled.sort();
  return {
    lower: cleanNumber(quantileSorted(resampled, 0.025)),
    upper: cleanNumber(quantileSorted(resampled, 0.975)),
  };
}

function pairedDifferences(baselineMetrics, candidateMetrics, rounds, selector) {
  const values = [];
  for (let round = 1; round <= rounds; round += 1) {
    values.push(selector(candidateMetrics.get(round)) - selector(baselineMetrics.get(round)));
  }
  return values;
}

function summarizeComparison(baselineMetrics, candidateMetrics, options) {
  const memoryContrasts = {};
  for (const name of Object.keys(CONTRASTS)) {
    const differences = pairedDifferences(
      baselineMetrics,
      candidateMetrics,
      options.rounds,
      (metrics) => metrics.contrasts[name],
    );
    memoryContrasts[name] = {
      paired_round_difference: summary(differences),
      paired_percentile_bootstrap_ci_95: pairedPercentileBootstrap(
        differences,
        options.bootstrapRepetitions,
        options.seed,
      ),
    };
  }

  const corroboratingMemoryContrasts = {};
  for (const metric of CORROBORATING_MEMORY_METRICS) {
    corroboratingMemoryContrasts[metric] = {};
    for (const name of Object.keys(CONTRASTS)) {
      corroboratingMemoryContrasts[metric][name] = {
        paired_round_difference: summary(
          pairedDifferences(
            baselineMetrics,
            candidateMetrics,
            options.rounds,
            (metrics) => metrics.corroboratingContrasts[metric][name],
          ),
        ),
      };
    }
  }

  const oneTimeProgram = {};
  const wholeProcess = {};
  const oneTimeProgramBuild = {};
  const timingPerVm = {};
  for (const scenario of SCENARIOS) {
    oneTimeProgram[scenario] = summary(
      pairedDifferences(
        baselineMetrics,
        candidateMetrics,
        options.rounds,
        (metrics) => metrics.scenarios[scenario].memory.oneTimeProgramGrowthKib,
      ),
    );
    wholeProcess[scenario] = summary(
      pairedDifferences(
        baselineMetrics,
        candidateMetrics,
        options.rounds,
        (metrics) => metrics.scenarios[scenario].memory.wholeProcessGrowthKib / EXPECTED_VM_COUNT,
      ),
    );
    oneTimeProgramBuild[scenario] = summary(
      pairedDifferences(
        baselineMetrics,
        candidateMetrics,
        options.rounds,
        (metrics) => metrics.scenarios[scenario].timing.one_time_program_build,
      ),
    );
    timingPerVm[scenario] = {};
    for (const field of [
      "vm_create",
      "program_run",
      "vm_create_plus_program_run",
      "amortized_total_including_program_build",
    ]) {
      timingPerVm[scenario][field] = summary(
        pairedDifferences(
          baselineMetrics,
          candidateMetrics,
          options.rounds,
          (metrics) => metrics.scenarios[scenario].timing[field],
        ),
      );
    }
  }

  return {
    matched_primary_contrast_candidate_minus_baseline_kib_per_vm: memoryContrasts,
    corroborating_matched_contrast_candidate_minus_baseline_kib_per_vm:
      corroboratingMemoryContrasts,
    one_time_program_candidate_minus_baseline_kib: oneTimeProgram,
    diagnostic_whole_process_candidate_minus_baseline_kib_per_vm: wholeProcess,
    one_time_program_build_candidate_minus_baseline_ns: oneTimeProgramBuild,
    timing_candidate_minus_baseline_ns_per_vm: timingPerVm,
  };
}

function analyzeRecords(records, options, inputSha256) {
  const protocol = validateProtocol(records, options);
  const baselineMetrics = roleRoundMetrics(protocol.grouped.baseline, options.rounds);
  const candidateMetrics = roleRoundMetrics(protocol.grouped.candidate, options.rounds);
  return {
    schema_version: 1,
    schema: OUTPUT_SCHEMA,
    identity: {
      sample_schema: SAMPLE_SCHEMA,
      baseline_commit: options.baselineCommit,
      candidate_commit: options.candidateCommit,
      accepted_rounds: options.rounds,
      accepted_round_ids: Array.from({ length: options.rounds }, (_, index) => index + 1),
      sample_count: records.length,
      protocol_sha256: protocol.protocolSha256,
      input_sha256: inputSha256,
      platform: { os: "linux", arch: "x86_64" },
      vm_count: EXPECTED_VM_COUNT,
      constant_slots: EXPECTED_CONSTANT_SLOTS,
      instruction_slots: EXPECTED_INSTRUCTION_SLOTS,
      scenarios: SCENARIOS.slice(),
      layout_bytes_by_build: protocol.layouts,
    },
    method: {
      pairing: "candidate and baseline samples matched by accepted round and scenario",
      primary_memory_source: "memory_kib.*.smaps_rollup.anonymous",
      corroborating_memory_sources: [
        "memory_kib.*.smaps_rollup.private_dirty",
        "memory_kib.*.smaps_rollup.rss",
        "memory_kib.*.status.rss_anon",
      ],
      primary_sample_growth: "after_run - program_ready",
      primary_contrasts: Object.fromEntries(
        Object.entries(CONTRASTS).map(([name, [left, right]]) => [name, `${left} - ${right}`]),
      ),
      comparison: "per-round candidate contrast - paired baseline contrast",
      memory_unit: "KiB per VM",
      one_time_program_memory: "program_ready - clean, KiB (not divided by VM count)",
      diagnostic_whole_process_memory: "after_run - clean, KiB per VM",
      one_time_timing: "program_build, nanoseconds (not divided by VM count)",
      per_vm_timing:
        "vm_create, program_run, their combined cost, and explicitly amortized total; nanoseconds per VM",
      summary_quantiles: "Hyndman/Fan type 7 linear interpolation",
      point_estimator: "median of paired round-level candidate-minus-baseline differences",
      confidence_interval: "seeded paired round-level percentile bootstrap",
      confidence_interval_scope:
        "primary smaps_rollup.anonymous matched contrasts only; corroborating fields are descriptive cross-checks",
      nominal_confidence_level: 0.95,
      bootstrap_repetitions: options.bootstrapRepetitions,
      bootstrap_seed_uint32: options.seed,
      bootstrap_rng: "mulberry32",
      bootstrap_resampling_unit:
        "accepted paired rounds with replacement; the seed is reset per contrast so all contrasts use identical round-index resamples",
      coverage_caveat:
        "95% is nominal and describes the observed host/run protocol; it cannot cover unobserved host states or systematic process-order effects",
    },
    baseline: {
      commit: options.baselineCommit,
      ...summarizeRole(baselineMetrics, options.rounds),
    },
    candidate: {
      commit: options.candidateCommit,
      ...summarizeRole(candidateMetrics, options.rounds),
    },
    comparison: summarizeComparison(baselineMetrics, candidateMetrics, options),
  };
}

function parseUnsignedOption(raw, option) {
  if (!/^(0|[1-9][0-9]*)$/.test(raw)) {
    fail(`${option} must be an unsigned decimal integer`);
  }
  const value = Number(raw);
  if (!Number.isSafeInteger(value)) fail(`${option} is outside the safe integer range`);
  return value;
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "--self-test") return { mode: "self-test" };
  if (argv.length === 1 && (argv[0] === "--help" || argv[0] === "-h")) {
    return { mode: "help" };
  }
  if (argv.includes("--self-test")) fail("--self-test must be used alone");
  if (argv.includes("--help") || argv.includes("-h")) fail("--help must be used alone");
  const raw = {};
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index];
    const value = argv[index + 1];
    if (value === undefined) fail(`${option} requires a value`);
    if (!["--input", "--baseline", "--candidate", "--rounds", "--bootstrap-reps", "--seed"].includes(option)) {
      fail(`unknown option ${JSON.stringify(option)}`);
    }
    if (Object.hasOwn(raw, option)) fail(`${option} was supplied more than once`);
    raw[option] = value;
  }
  for (const option of [
    "--input",
    "--baseline",
    "--candidate",
    "--rounds",
    "--bootstrap-reps",
    "--seed",
  ]) {
    if (!Object.hasOwn(raw, option)) fail(`missing required ${option}`);
  }
  const baselineCommit = requireLowerHex(raw["--baseline"], 40, "--baseline");
  const candidateCommit = requireLowerHex(raw["--candidate"], 40, "--candidate");
  if (baselineCommit === candidateCommit) fail("--baseline and --candidate commits must differ");
  const rounds = parseUnsignedOption(raw["--rounds"], "--rounds");
  const bootstrapRepetitions = parseUnsignedOption(
    raw["--bootstrap-reps"],
    "--bootstrap-reps",
  );
  const seed = parseUnsignedOption(raw["--seed"], "--seed");
  if (rounds !== EXPECTED_ROUNDS) fail(`--rounds must be ${EXPECTED_ROUNDS}, got ${rounds}`);
  if (bootstrapRepetitions !== EXPECTED_BOOTSTRAP_REPETITIONS) {
    fail(
      `--bootstrap-reps must be ${EXPECTED_BOOTSTRAP_REPETITIONS}, got ${bootstrapRepetitions}`,
    );
  }
  if (seed !== EXPECTED_BOOTSTRAP_SEED) {
    fail(`--seed must be ${EXPECTED_BOOTSTRAP_SEED} (0x52555354), got ${seed}`);
  }
  return {
    mode: "analyze",
    inputFile: raw["--input"],
    baselineCommit,
    candidateCommit,
    rounds,
    bootstrapRepetitions,
    seed,
  };
}

function fixtureSnapshot(anonymousKib, swap = 0) {
  return {
    status: {
      vm_rss: anonymousKib + 2_000,
      vm_hwm: anonymousKib + 2_100,
      rss_anon: anonymousKib,
      rss_file: 2_000,
      rss_shmem: 0,
    },
    smaps_rollup: {
      rss: anonymousKib + 2_000,
      pss: anonymousKib + 1_000,
      private_clean: 1_000,
      private_dirty: anonymousKib,
      anonymous: anonymousKib,
      swap,
    },
  };
}

function fixtureScenario(scenarioId) {
  const specification = SCENARIO_SPEC[scenarioId];
  return {
    id: scenarioId,
    vm_count: EXPECTED_VM_COUNT,
    profile: "reproducible-standard",
    constant_slots: EXPECTED_CONSTANT_SLOTS,
    instruction_slots: EXPECTED_INSTRUCTION_SLOTS,
    executes_constants: specification.executesConstants,
    strings: {
      kind: specification.stringKind,
      count: specification.stringCount,
      bytes_each: specification.bytesEach,
      generator: "indexed-binary-v1",
    },
    nil_constant_count: specification.nilConstantCount,
  };
}

function fixtureRecords() {
  const commits = {
    baseline: "a".repeat(40),
    candidate: "b".repeat(40),
  };
  const protocolSha256 = "c".repeat(64);
  const records = [];
  const contrastPerVm = {
    baseline: {
      empty: 0,
      short_unloaded: 20,
      short_loaded: 28,
      long_unloaded: 40,
      long_loaded: 56,
    },
    candidate: {
      empty: 0,
      short_unloaded: 12,
      short_loaded: 14,
      long_unloaded: 10,
      long_loaded: 14,
    },
  };
  for (const role of ROLES) {
    for (let round = 1; round <= EXPECTED_ROUNDS; round += 1) {
      for (const [scenarioIndex, scenarioId] of SCENARIOS.entries()) {
        const specification = SCENARIO_SPEC[scenarioId];
        const clean = 10_000 + round;
        const programGrowth = 40 + scenarioIndex * 20 + (role === "candidate" ? 3 : 0);
        const programReady = clean + programGrowth;
        const baseVmGrowth = 5_120 + round;
        const primaryGrowth =
          baseVmGrowth + contrastPerVm[role][scenarioId] * EXPECTED_VM_COUNT;
        const vmsReady = programReady + Math.floor(primaryGrowth / 2);
        const afterRun = programReady + primaryGrowth;
        records.push({
          schema_version: 1,
          schema: SAMPLE_SCHEMA,
          sample_id: `${role}-round-${round}-${scenarioId}`,
          protocol_sha256: protocolSha256,
          timestamp_unix: 1_800_000_000 + round,
          build: { role, commit: commits[role] },
          round,
          scenario: fixtureScenario(scenarioId),
          platform: {
            os: "linux",
            arch: "x86_64",
            pid: 10_000 + records.length,
            memory_probe_warmups: 2,
          },
          layout_bytes: {
            value: role === "baseline" ? 24 : 16,
            instruction: 12,
            constant: 32,
            vm: 1_024,
          },
          timing_ns: {
            program_build: 100_000 + round + scenarioIndex,
            vm_create: 2_000_000 + round * 512 + scenarioIndex * 100,
            program_run:
              3_000_000 + round * 256 + scenarioIndex * 200 + (role === "candidate" ? -512 : 0),
          },
          memory_kib: {
            clean: fixtureSnapshot(clean),
            program_ready: fixtureSnapshot(programReady),
            vms_ready: fixtureSnapshot(vmsReady),
            after_run: fixtureSnapshot(afterRun),
          },
          validation: {
            created_vms: EXPECTED_VM_COUNT,
            ran_vms: EXPECTED_VM_COUNT,
            expected_result_kind: specification.expectedResultKind,
            expected_result_bytes: specification.expectedResultBytes,
            expected_result_fnv1a64: specification.expectedResultFnv1a64,
            all_results_matched: true,
          },
        });
      }
    }
  }
  return { commits, protocolSha256, records };
}

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function expectFailure(label, fixture, pattern) {
  let thrown = null;
  try {
    const records = fixture.records.map((record, index) =>
      validateSample(record, `negative ${label}[${index}]`),
    );
    analyzeRecords(
      records,
      {
        baselineCommit: fixture.commits.baseline,
        candidateCommit: fixture.commits.candidate,
        rounds: EXPECTED_ROUNDS,
        bootstrapRepetitions: EXPECTED_BOOTSTRAP_REPETITIONS,
        seed: EXPECTED_BOOTSTRAP_SEED,
      },
      "d".repeat(64),
    );
  } catch (error) {
    thrown = error;
  }
  assert(thrown instanceof Error, `${label} should fail`);
  assert.match(thrown.message, pattern, `${label} should have a precise diagnostic`);
}

function runSelfTest() {
  const fixture = fixtureRecords();
  const text = fixture.records.map((record) => JSON.stringify(record)).join("\n") + "\n";
  const records = parseJsonlText(text, "self-test.jsonl");
  const analysis = analyzeRecords(
    records,
    {
      baselineCommit: fixture.commits.baseline,
      candidateCommit: fixture.commits.candidate,
      rounds: EXPECTED_ROUNDS,
      bootstrapRepetitions: EXPECTED_BOOTSTRAP_REPETITIONS,
      seed: EXPECTED_BOOTSTRAP_SEED,
    },
    crypto.createHash("sha256").update(text).digest("hex"),
  );
  assert.equal(analysis.schema, OUTPUT_SCHEMA);
  assert.equal(analysis.identity.sample_count, 160);
  assert.equal(analysis.identity.accepted_rounds, 16);
  const comparisons = analysis.comparison.matched_primary_contrast_candidate_minus_baseline_kib_per_vm;
  for (const [name, expected] of [
    ["short_unloaded_minus_empty", -8],
    ["long_unloaded_minus_empty", -30],
    ["short_loaded_minus_short_unloaded", -6],
    ["long_loaded_minus_long_unloaded", -12],
  ]) {
    assert.equal(comparisons[name].paired_round_difference.median, expected, name);
    assert.deepEqual(comparisons[name].paired_percentile_bootstrap_ci_95, {
      lower: expected,
      upper: expected,
    });
  }
  assert.equal(
    analysis.comparison.timing_candidate_minus_baseline_ns_per_vm.empty.program_run.median,
    -1,
  );
  assert.equal(analysis.baseline.one_time_program_build_ns.empty.median, 100_008.5);
  assert.equal(
    analysis.comparison.one_time_program_build_candidate_minus_baseline_ns.empty.median,
    0,
  );
  for (const metric of CORROBORATING_MEMORY_METRICS) {
    assert.equal(
      analysis.baseline.memory.corroborating_matched_contrasts_kib_per_vm[metric]
        .short_unloaded_minus_empty.median,
      20,
      metric,
    );
    assert.equal(
      analysis.comparison.corroborating_matched_contrast_candidate_minus_baseline_kib_per_vm[
        metric
      ].short_unloaded_minus_empty.paired_round_difference.median,
      -8,
      metric,
    );
  }

  const duplicateId = clone(fixture);
  duplicateId.records[1].sample_id = duplicateId.records[0].sample_id;
  expectFailure("duplicate sample id", duplicateId, /sample_id duplicates/);

  const badVmCount = clone(fixture);
  badVmCount.records[0].scenario.vm_count = 511;
  expectFailure("VM count", badVmCount, /scenario\.vm_count must be 512/);

  const swapped = clone(fixture);
  swapped.records[0].memory_kib.vms_ready.smaps_rollup.swap = 1;
  expectFailure("swap", swapped, /vms_ready\.smaps_rollup\.swap must be 0 KiB/);

  const badArch = clone(fixture);
  badArch.records[0].platform.arch = "aarch64";
  expectFailure("architecture", badArch, /platform\.arch must be "x86_64"/);

  const badCommit = clone(fixture);
  badCommit.records[0].build.commit = "abc";
  expectFailure("full commit", badCommit, /build\.commit must be exactly 40/);

  const mixedProtocol = clone(fixture);
  mixedProtocol.records.at(-1).protocol_sha256 = "e".repeat(64);
  expectFailure("protocol identity", mixedProtocol, /expected the common protocol identity/);

  const duplicateCell = clone(fixture);
  duplicateCell.records[2].scenario = clone(duplicateCell.records[1].scenario);
  duplicateCell.records[2].validation = clone(duplicateCell.records[1].validation);
  expectFailure("duplicate protocol cell", duplicateCell, /duplicates protocol cell/);

  assert.throws(
    () => parseJsonlText(`${JSON.stringify(fixture.records[0])}\n\n`, "blank.jsonl"),
    /blank JSONL record/,
  );
  assert.throws(
    () =>
      parseArgs([
        "--input",
        "x",
        "--baseline",
        fixture.commits.baseline,
        "--candidate",
        fixture.commits.candidate,
        "--rounds",
        "15",
        "--bootstrap-reps",
        "100000",
        "--seed",
        "1381323604",
      ]),
    /--rounds must be 16/,
  );
  process.stdout.write("analyze-vm-string-memory self-test passed\n");
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.mode === "help") {
    process.stdout.write(USAGE);
    return;
  }
  if (options.mode === "self-test") {
    runSelfTest();
    return;
  }
  const { records, inputSha256 } = loadJsonl(options.inputFile);
  const analysis = analyzeRecords(records, options, inputSha256);
  process.stdout.write(`${JSON.stringify(analysis, null, 2)}\n`);
}

try {
  main();
} catch (error) {
  process.stderr.write(`analyze-vm-string-memory: ${error.message}\n`);
  process.exitCode = 1;
}
