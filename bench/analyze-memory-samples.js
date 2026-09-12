#!/usr/bin/env node
"use strict";

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const RAW_SCHEMA = "rustia.bench.memory-raw.v1";
const ANALYSIS_SCHEMA = "rustia.bench.memory-analysis.v1";
const DEFAULT_BOOTSTRAP_REPETITIONS = 20_000;
const DEFAULT_BOOTSTRAP_SEED = 1_381_323_604;
const DEFAULT_ABSOLUTE_GROWTH_KIB_PER_EPOCH = 4;
const DEFAULT_PROPORTIONAL_GROWTH_PERCENT_PER_EPOCH = 0.05;
const RETAINED_CONTROLS = new Set(["memory_retained_control", "todo_retained_control"]);
// Provenance labels only ever name a machine in published row data; they are
// never resolved or connected to, so the character set is deliberately narrow.
const SAFE_HOST = /^[A-Za-z0-9._-]{1,128}$/;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { command: "self-test" };
  const inputs = [];
  let output = null;
  let minimumSamples = 1;
  let bootstrapRepetitions = DEFAULT_BOOTSTRAP_REPETITIONS;
  let bootstrapSeed = DEFAULT_BOOTSTRAP_SEED;
  let absoluteThreshold = DEFAULT_ABSOLUTE_GROWTH_KIB_PER_EPOCH;
  let proportionalThreshold = DEFAULT_PROPORTIONAL_GROWTH_PERCENT_PER_EPOCH;
  let host = null;
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (value == null) fail(`${flag} requires a value`);
    switch (flag) {
      case "--input":
        inputs.push(path.resolve(value));
        break;
      case "--output":
        if (output != null) fail("--output was supplied more than once");
        output = path.resolve(value);
        break;
      case "--minimum-samples":
        minimumSamples = positiveInteger(value, flag);
        break;
      case "--bootstrap-repetitions":
        bootstrapRepetitions = positiveInteger(value, flag);
        break;
      case "--bootstrap-seed":
        bootstrapSeed = nonnegativeInteger(value, flag);
        if (bootstrapSeed > 0xffffffff) fail("--bootstrap-seed must fit in u32");
        break;
      case "--absolute-growth-kib-per-epoch":
        absoluteThreshold = positiveNumber(value, flag);
        break;
      case "--proportional-growth-percent-per-epoch":
        proportionalThreshold = positiveNumber(value, flag);
        break;
      case "--host":
        if (host != null) fail("--host was supplied more than once");
        host = value;
        break;
      default:
        fail(`unknown argument ${JSON.stringify(flag)}`);
    }
    index += 1;
  }
  if (inputs.length === 0) fail("at least one --input is required");
  // Required, not defaulted. The samples themselves cannot say which machine
  // produced them, so this is the only place the answer can enter the row -- and
  // a default would be a guess published as provenance.
  if (host == null) fail("--host is required: name the machine these samples were measured on");
  if (!SAFE_HOST.test(host)) fail("--host must be a safe identifier");
  return {
    command: "analyze",
    inputs,
    output,
    minimumSamples,
    bootstrapRepetitions,
    bootstrapSeed,
    absoluteThreshold,
    proportionalThreshold,
    host,
  };
}

function positiveInteger(raw, label) {
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value <= 0) fail(`${label} must be a positive integer`);
  return value;
}

function nonnegativeInteger(raw, label) {
  const value = Number(raw);
  if (!Number.isSafeInteger(value) || value < 0) fail(`${label} must be a nonnegative integer`);
  return value;
}

function positiveNumber(raw, label) {
  const value = Number(raw);
  if (!Number.isFinite(value) || value <= 0) fail(`${label} must be a positive number`);
  return value;
}

function loadJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot parse ${file}: ${error.message}`);
  }
}

function finite(value, label) {
  if (!Number.isFinite(value)) fail(`${label} must be finite`);
  return value;
}

function nonnegative(value, label) {
  value = finite(value, label);
  if (value < 0) fail(`${label} must be nonnegative`);
  return value;
}

function phaseEvent(raw, phase) {
  const matches = raw.events.filter((entry) => entry.event.phase === phase);
  if (matches.length !== 1) fail(`${raw.sample_id} must contain exactly one ${phase} phase`);
  return matches[0];
}

function epochEvents(raw) {
  const events = raw.events.filter((entry) => entry.event.phase === "epoch");
  if (events.length !== raw.identity.epochs) {
    fail(`${raw.sample_id} epoch event count does not match identity.epochs`);
  }
  events.forEach((entry, index) => {
    if (entry.event.epoch !== index + 1) fail(`${raw.sample_id} epoch order is not contiguous`);
  });
  return events;
}

function anonKib(entry, label) {
  return nonnegative(entry.process_memory?.smaps_rollup_kib?.anonymous, `${label}.anonymous`);
}

function cgroupCurrent(entry, label) {
  return nonnegative(entry.cgroup_memory?.current_bytes, `${label}.current_bytes`);
}

function canonicalChecksum(value) {
  if (typeof value !== "string" || value === "") fail("checksum must be a nonempty string");
  const match = value.match(/^(?:number|f64|i64|u64):(-?[0-9]+)(?:\.0+)?$/);
  return match ? `integer:${BigInt(match[1]).toString()}` : value;
}

function theilSen(points) {
  if (points.length < 2) fail("Theil-Sen slope requires at least two points");
  const slopes = [];
  for (let left = 0; left < points.length; left += 1) {
    for (let right = left + 1; right < points.length; right += 1) {
      const dx = points[right].x - points[left].x;
      if (dx > 0) slopes.push((points[right].y - points[left].y) / dx);
    }
  }
  return median(slopes);
}

function memoryAreaKibSeconds(timeline, baselineBytes, startMs, endMs) {
  const points = timeline
    .filter((point) => point.elapsed_ms >= startMs && point.elapsed_ms <= endMs)
    .map((point) => ({
      x: finite(point.elapsed_ms, "timeline.elapsed_ms"),
      y: Math.max(0, nonnegative(point.current_bytes, "timeline.current_bytes") - baselineBytes) / 1024,
    }));
  if (points.length < 2) return 0;
  let area = 0;
  for (let index = 1; index < points.length; index += 1) {
    const seconds = (points[index].x - points[index - 1].x) / 1000;
    area += ((points[index - 1].y + points[index].y) / 2) * seconds;
  }
  return area;
}

function workloadPeakDeltaKib(raw, spawnedBaselineBytes) {
  const phases = raw.events;
  const preparedIndex = phases.findIndex((entry) => entry.event?.phase === "prepared");
  const quiescentIndex = phases.findIndex((entry) => entry.event?.phase === "quiescent");
  if (
    raw.platform?.resettable_memory_peak === true &&
    preparedIndex >= 0 &&
    quiescentIndex > preparedIndex &&
    phases.slice(preparedIndex + 1, quiescentIndex + 1).every(
      (entry) => Number.isSafeInteger(entry.phase_peak_bytes) && entry.phase_peak_bytes >= 0,
    )
  ) {
    let maximum = 0;
    for (let index = preparedIndex + 1; index <= quiescentIndex; index += 1) {
      const phaseStart = cgroupCurrent(phases[index - 1], "phase peak start");
      maximum = Math.max(maximum, phases[index].phase_peak_bytes - phaseStart);
    }
    return Math.max(0, maximum) / 1024;
  }
  const wholeSamplePeak = nonnegative(
    raw.final_cgroup?.peak_bytes,
    `${raw.sample_id}.final_cgroup.peak_bytes`,
  );
  return Math.max(0, wholeSamplePeak - spawnedBaselineBytes) / 1024;
}

function sampleMetrics(raw, file) {
  if (raw?.schema_version !== 1 || raw?.schema !== RAW_SCHEMA) fail(`${file} has the wrong raw schema`);
  if (!Array.isArray(raw.events) || !Array.isArray(raw.timeline)) fail(`${file} lacks events/timeline`);
  if (typeof raw.sample_id !== "string" || raw.sample_id === "") fail(`${file} lacks sample_id`);
  const { identity } = raw;
  for (const field of ["commit", "engine", "workload"]) {
    if (typeof identity?.[field] !== "string" || identity[field] === "") fail(`${file} identity.${field}`);
  }
  if (!Number.isSafeInteger(identity.n) || identity.n <= 0) fail(`${file} identity.n`);
  if (!Number.isSafeInteger(identity.epochs) || identity.epochs < 2) {
    fail(`${file} identity.epochs must be at least two for growth analysis`);
  }
  const spawned = phaseEvent(raw, "spawned");
  const prepared = phaseEvent(raw, "prepared");
  const warmed = phaseEvent(raw, "warmed");
  const quiescent = phaseEvent(raw, "quiescent");
  const dropped = phaseEvent(raw, "dropped");
  const epochs = epochEvents(raw);
  const spawnedAnon = anonKib(spawned, `${raw.sample_id}.spawned`);
  const warmedAnon = anonKib(warmed, `${raw.sample_id}.warmed`);
  const epochPoints = epochs.map((entry) => ({
    x: entry.event.epoch,
    y: anonKib(entry, `${raw.sample_id}.epoch.${entry.event.epoch}`) - warmedAnon,
  }));
  const tailStart = Math.floor(epochPoints.length / 2);
  const tail = epochPoints.slice(tailStart);
  const slope = theilSen(tail);
  const baselineCgroup = cgroupCurrent(spawned, `${raw.sample_id}.spawned`);
  const retainedKib = anonKib(quiescent, `${raw.sample_id}.quiescent`) - warmedAnon;
  const peakDeltaKib = workloadPeakDeltaKib(raw, baselineCgroup);
  const recoveryRatio = peakDeltaKib > 0
    ? Math.max(-1, Math.min(1, (peakDeltaKib - Math.max(0, retainedKib)) / peakDeltaKib))
    : 0;
  const checksumSequence = [
    canonicalChecksum(warmed.event.checksum),
    ...epochs.map((entry) => canonicalChecksum(entry.event.checksum)),
  ];
  const endMs = quiescent.elapsed_ms;
  const memoryArea = memoryAreaKibSeconds(
    raw.timeline,
    baselineCgroup,
    warmed.elapsed_ms,
    endMs,
  );
  return {
    file,
    sample_id: raw.sample_id,
    commit: identity.commit,
    engine: identity.engine,
    workload: identity.workload,
    n: identity.n,
    epochs: identity.epochs,
    execution_mode: epochs[0].event.execution_mode,
    dialect: epochs[0].event.dialect,
    target: epochs[0].event.target,
    spawned_anon_kib: spawnedAnon,
    spawned_pss_kib: nonnegative(spawned.process_memory.smaps_rollup_kib.pss, "spawned.pss"),
    prepared_delta_anon_kib: anonKib(prepared, "prepared") - spawnedAnon,
    peak_delta_kib: peakDeltaKib,
    quiescent_retained_delta_anon_kib: retainedKib,
    dropped_delta_anon_kib: anonKib(dropped, "dropped") - spawnedAnon,
    recovery_ratio: recoveryRatio,
    tail_growth_kib_per_epoch: slope,
    tail_growth_percent_per_epoch: warmedAnon > 0 ? (slope / warmedAnon) * 100 : 0,
    memory_area_kib_seconds_per_unit: memoryArea / (identity.n * identity.epochs),
    checksum_sequence: checksumSequence,
  };
}

function median(values) {
  if (values.length === 0) fail("median requires values");
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}

function quantile(values, probability) {
  const sorted = [...values].sort((left, right) => left - right);
  if (sorted.length === 1) return sorted[0];
  const index = (sorted.length - 1) * probability;
  const lower = Math.floor(index);
  const fraction = index - lower;
  return sorted[lower] + (sorted[Math.min(lower + 1, sorted.length - 1)] - sorted[lower]) * fraction;
}

function stats(values) {
  return {
    sample_count: values.length,
    min: Math.min(...values),
    q1: quantile(values, 0.25),
    median: median(values),
    q3: quantile(values, 0.75),
    max: Math.max(...values),
  };
}

function mulberry32(seed) {
  let value = seed >>> 0;
  return () => {
    value = (value + 0x6d2b79f5) >>> 0;
    let result = value;
    result = Math.imul(result ^ (result >>> 15), result | 1);
    result ^= result + Math.imul(result ^ (result >>> 7), result | 61);
    return ((result ^ (result >>> 14)) >>> 0) / 0x100000000;
  };
}

function bootstrapMedianInterval(values, repetitions, seed) {
  const random = mulberry32(seed);
  const estimates = [];
  for (let repetition = 0; repetition < repetitions; repetition += 1) {
    const resample = [];
    for (let index = 0; index < values.length; index += 1) {
      resample.push(values[Math.floor(random() * values.length)]);
    }
    estimates.push(median(resample));
  }
  return {
    lower: quantile(estimates, 0.025),
    upper: quantile(estimates, 0.975),
  };
}

function groupBy(samples, key) {
  const groups = new Map();
  for (const sample of samples) {
    const value = key(sample);
    const group = groups.get(value) ?? [];
    group.push(sample);
    groups.set(value, group);
  }
  return groups;
}

function groupKey(sample) {
  return [
    sample.commit,
    sample.target,
    sample.engine,
    sample.execution_mode,
    sample.workload,
    sample.n,
    sample.epochs,
  ].join("\0");
}

function summarizeGroup(samples, config) {
  if (samples.length < config.minimumSamples) {
    fail(`${samples[0].engine}/${samples[0].workload} has ${samples.length} samples; expected ${config.minimumSamples}`);
  }
  const firstChecksums = JSON.stringify(samples[0].checksum_sequence);
  for (const sample of samples.slice(1)) {
    if (JSON.stringify(sample.checksum_sequence) !== firstChecksums) {
      fail(`${sample.engine}/${sample.workload} checksum sequence changed across fresh processes`);
    }
  }
  const slopes = samples.map((sample) => sample.tail_growth_kib_per_epoch);
  const percentSlopes = samples.map((sample) => sample.tail_growth_percent_per_epoch);
  const slopeInterval = bootstrapMedianInterval(slopes, config.bootstrapRepetitions, config.bootstrapSeed);
  const percentInterval = bootstrapMedianInterval(
    percentSlopes,
    config.bootstrapRepetitions,
    config.bootstrapSeed,
  );
  const expectedRetention = RETAINED_CONTROLS.has(samples[0].workload);
  const lowerExceeds =
    slopeInterval.lower > config.absoluteThreshold ||
    percentInterval.lower > config.proportionalThreshold;
  const upperStable =
    slopeInterval.upper <= config.absoluteThreshold &&
    percentInterval.upper <= config.proportionalThreshold;
  let classification;
  if (expectedRetention) {
    classification = lowerExceeds ? "expected_retention" : "detector_failed";
  } else if (lowerExceeds) {
    classification = "growth_signal";
  } else if (upperStable) {
    classification = "stable_in_tested_window";
  } else {
    classification = "inconclusive";
  }
  const metric = (name) => stats(samples.map((sample) => sample[name]));
  return {
    commit: samples[0].commit,
    target: samples[0].target,
    host: config.host,
    engine: samples[0].engine,
    dialect: samples[0].dialect,
    execution_mode: samples[0].execution_mode,
    workload: samples[0].workload,
    n: samples[0].n,
    epochs: samples[0].epochs,
    sample_ids: samples.map((sample) => sample.sample_id).sort(),
    checksum_sequence: samples[0].checksum_sequence,
    metrics: {
      spawned_anon_kib: metric("spawned_anon_kib"),
      spawned_pss_kib: metric("spawned_pss_kib"),
      prepared_delta_anon_kib: metric("prepared_delta_anon_kib"),
      peak_delta_kib: metric("peak_delta_kib"),
      quiescent_retained_delta_anon_kib: metric("quiescent_retained_delta_anon_kib"),
      dropped_delta_anon_kib: metric("dropped_delta_anon_kib"),
      recovery_ratio: metric("recovery_ratio"),
      tail_growth_kib_per_epoch: metric("tail_growth_kib_per_epoch"),
      tail_growth_percent_per_epoch: metric("tail_growth_percent_per_epoch"),
      memory_area_kib_seconds_per_unit: metric("memory_area_kib_seconds_per_unit"),
    },
    tail_growth_ci_95: {
      kib_per_epoch: slopeInterval,
      percent_per_epoch: percentInterval,
    },
    classification,
  };
}

function validatePortableChecksums(rows) {
  const portable = new Set([
    "memory_burst",
    "memory_churn",
    "memory_retained_control",
    "todo_steady",
    "todo_history",
    "todo_retained_control",
  ]);
  for (const group of groupBy(
    rows.filter((row) => portable.has(row.workload)),
    (row) => `${row.commit}\0${row.target}\0${row.workload}\0${row.n}\0${row.epochs}`,
  ).values()) {
    const expected = JSON.stringify(group[0].checksum_sequence);
    for (const row of group.slice(1)) {
      if (JSON.stringify(row.checksum_sequence) !== expected) {
        fail(`portable checksum mismatch for ${row.workload}: ${group[0].engine} vs ${row.engine}`);
      }
    }
  }
}

function detectorCoverage(rows) {
  const byEngine = groupBy(rows, (row) => `${row.commit}\0${row.target}\0${row.engine}`);
  return [...byEngine.values()].map((engineRows) => {
    const controls = engineRows.filter((row) => RETAINED_CONTROLS.has(row.workload));
    return {
      commit: engineRows[0].commit,
      target: engineRows[0].target,
      engine: engineRows[0].engine,
      retained_controls: controls.map((row) => ({
        workload: row.workload,
        classification: row.classification,
      })),
      detector_validated:
        controls.length > 0 && controls.every((row) => row.classification === "expected_retention"),
    };
  });
}

function analyze(config) {
  const samples = config.inputs.map((file) => sampleMetrics(loadJson(file), file));
  const rows = [...groupBy(samples, groupKey).values()]
    .map((group) => summarizeGroup(group, config))
    .sort(
      (left, right) =>
        left.target.localeCompare(right.target) ||
        left.workload.localeCompare(right.workload) ||
        left.engine.localeCompare(right.engine),
    );
  validatePortableChecksums(rows);
  return {
    schema_version: 1,
    schema: ANALYSIS_SCHEMA,
    generated_at: new Date().toISOString(),
    method: {
      primary_phase_memory: "smaps_rollup Anonymous KiB",
      primary_short_spike_memory:
        "resettable cgroup-v2 phase memory.peak when available; whole-sample memory.peak fallback",
      independent_unit: "fresh process/cgroup sample",
      tail_window: "last half of epoch markers",
      tail_estimator: "Theil-Sen within sample; median across samples",
      confidence_interval: "seeded fresh-process percentile bootstrap of the median",
      bootstrap_repetitions: config.bootstrapRepetitions,
      bootstrap_seed_uint32: config.bootstrapSeed,
      absolute_growth_kib_per_epoch: config.absoluteThreshold,
      proportional_growth_percent_per_epoch: config.proportionalThreshold,
      rss_hwm_role: "diagnostic only",
    },
    host: config.host,
    raw_sample_count: samples.length,
    row_count: rows.length,
    detector_coverage: detectorCoverage(rows),
    rows,
  };
}

function selfTest() {
  assert.equal(theilSen([{ x: 1, y: 2 }, { x: 2, y: 4 }, { x: 4, y: 8 }]), 2);
  assert.equal(canonicalChecksum("number:12.000000000000"), "integer:12");
  assert.equal(canonicalChecksum("i64:12"), "integer:12");
  assert.deepEqual(stats([1, 2, 3, 4]), {
    sample_count: 4,
    min: 1,
    q1: 1.75,
    median: 2.5,
    q3: 3.25,
    max: 4,
  });
  const interval = bootstrapMedianInterval([10, 10, 10], 100, 7);
  assert.deepEqual(interval, { lower: 10, upper: 10 });
  process.stdout.write("memory analyzer self-test passed\n");
}

function main() {
  const config = parseArgs(process.argv.slice(2));
  if (config.command === "self-test") {
    selfTest();
    return;
  }
  const result = analyze(config);
  const text = `${JSON.stringify(result, null, 2)}\n`;
  if (config.output) {
    if (fs.existsSync(config.output)) fail(`refusing to overwrite ${config.output}`);
    fs.mkdirSync(path.dirname(config.output), { recursive: true });
    fs.writeFileSync(config.output, text, { flag: "wx" });
  } else {
    process.stdout.write(text);
  }
}

try {
  main();
} catch (error) {
  process.stderr.write(`analyze-memory-samples: ${error.message}\n`);
  process.exitCode = 1;
}
