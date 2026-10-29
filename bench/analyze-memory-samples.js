#!/usr/bin/env node
"use strict";

const assert = require("assert");
const fs = require("fs");
const path = require("path");

const RAW_SCHEMA = "rustia.bench.memory-raw.v1";
const ANALYSIS_SCHEMA = "rustia.bench.memory-analysis.v1";
const { MEMORY_PROTOCOL, QUIESCENCE_MS } = require("./memory-protocol.js");
const DEFAULT_BOOTSTRAP_REPETITIONS = 20_000;
const DEFAULT_BOOTSTRAP_SEED = 1_381_323_604;
const DEFAULT_ABSOLUTE_GROWTH_KIB_PER_EPOCH = 4;
const DEFAULT_PROPORTIONAL_GROWTH_PERCENT_PER_EPOCH = 0.05;
const RETAINED_CONTROLS = new Set(["memory_retained_control", "todo_retained_control"]);
// Provenance labels only ever name a machine in published row data; they are
// never resolved or connected to, so the character set is deliberately narrow:
// no dots and no colons, so a label can be neither a fully qualified name nor
// an IPv4 or IPv6 address, whatever an operator types.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

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
  if (!HOST_LABEL.test(host)) fail("--host must be a label of [A-Za-z0-9_-] characters, never an address");
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

function memoryAreaKibSeconds(timeline, baselineBytes, startMs, endMs, field = "current_bytes") {
  const points = timeline
    .filter((point) => point.elapsed_ms >= startMs && point.elapsed_ms <= endMs)
    .map((point) => ({
      x: finite(point.elapsed_ms, "timeline.elapsed_ms"),
      y: Math.max(0, nonnegative(point[field], `timeline.${field}`) - baselineBytes) / 1024,
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
  const corrected = raw?.schema_version === 2 && raw?.schema === "rustia.bench.memory-raw.v2";
  if (!corrected && (raw?.schema_version !== 1 || raw?.schema !== RAW_SCHEMA)) fail(`${file} has the wrong raw schema`);
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
  if (corrected) validateCorrectedSample(raw, file, warmed, quiescent, epochs);
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
    ...(corrected ? {
      measurement_protocol: MEMORY_PROTOCOL,
      lane: identity.lane,
      role: identity.role,
      round: identity.round,
      binary_sha256: identity.binary_sha256,
      anonymous_memory_area_kib_seconds_per_unit: memoryAreaKibSeconds(
        raw.timeline, spawned.cgroup_memory.stat_bytes.anon, warmed.elapsed_ms, endMs, "anon_bytes",
      ) / (identity.n * identity.epochs),
    } : {}),
  };
}

function validateCorrectedSample(raw, file, warmed, quiescent, epochs) {
  const identity = raw.identity;
  if (!/^[0-9a-f]{40}$/.test(identity.commit) || !/^[0-9a-f]{64}$/.test(identity.binary_sha256)) fail(`${file} lacks exact source/binary identity`);
  if (!["workload", "null-control"].includes(identity.lane) || !["warmup", "measurement"].includes(identity.role)) fail(`${file} has invalid sampling role/lane`);
  if (!Number.isSafeInteger(identity.round) || identity.round < 0 || (identity.role === "warmup") !== (identity.round === 0)) fail(`${file} has invalid sampling round`);
  if (raw.protocol?.quiescence_ms !== QUIESCENCE_MS || raw.protocol.forced_collection !== false || raw.protocol.allocator_purge !== false) fail(`${file} has the wrong quiescence policy`);
  const shape = ["spawned", "prepared", "warmed", ...epochs.map(() => "epoch"), "quiescent", "dropped"];
  if (JSON.stringify(raw.events.map(entry => entry.event.phase)) !== JSON.stringify(shape)) fail(`${file} has the wrong phase order`);
  let previous = -1;
  for (const entry of raw.events) {
    for (const key of ["target", "dialect", "execution_mode"]) {
      if (typeof entry.event[key] !== "string" || !entry.event[key] || entry.event[key] !== raw.events[0].event[key]) fail(`${file} changes event ${key}`);
    }
    if (nonnegative(entry.elapsed_ms, "phase time") <= previous) fail(`${file} has unordered phase times`);
    previous = entry.elapsed_ms;
    if (entry.event.schema !== "rustia.bench.memory-child-event.v2" || entry.event.schema_version !== 2 || entry.event.null_control !== (identity.lane === "null-control")) fail(`${file} child event lane/schema differs`);
    for (const key of ["engine", "workload", "commit", "n", "epochs"]) {
      if (entry.event[key] !== identity[key]) fail(`${file} child event ${key} differs`);
    }
    nonnegative(entry.cgroup_memory?.stat_bytes?.anon, "phase anonymous bytes");
    if (entry.process_memory?.smaps_rollup_kib?.swap !== 0 || entry.cgroup_memory?.swap_current_bytes !== 0) fail(`${file} phase used swap`);
  }
  if (quiescent.elapsed_ms - epochs.at(-1).elapsed_ms < QUIESCENCE_MS) fail(`${file} lacks the bounded quiescence interval`);
  previous = -1;
  for (const point of raw.timeline) {
    if (nonnegative(point.elapsed_ms, "timeline time") <= previous) fail(`${file} has unordered timeline times`);
    previous = point.elapsed_ms;
    nonnegative(point.anon_bytes, "timeline anonymous bytes");
    nonnegative(point.current_bytes, "timeline current charge");
  }
  for (const entry of [warmed, quiescent]) {
    const endpoint = raw.timeline.find(point => point.elapsed_ms === entry.elapsed_ms);
    if (!endpoint || endpoint.anon_bytes !== entry.cgroup_memory.stat_bytes.anon || endpoint.current_bytes !== entry.cgroup_memory.current_bytes) fail(`${file} lacks an exact phase timeline endpoint`);
  }
  for (const key of ["oom", "oom_kill", "oom_group_kill", "high", "max"]) {
    if ((raw.final_cgroup?.events?.[key] ?? 0) !== 0) fail(`${file} has contaminated cgroup events`);
  }
  if (raw.final_cgroup?.swap_current_bytes !== 0 || raw.final_cgroup?.pressure?.some?.total !== 0 || raw.final_cgroup?.pressure?.full?.total !== 0) fail(`${file} has swap/pressure contamination`);
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

function summarizeCorrectedGroup(samples, config) {
  const lanes = ["workload", "null-control"];
  const byLane = new Map();
  const binaries = new Set(samples.map(sample => sample.binary_sha256));
  if (binaries.size !== 1) fail("matched memory samples use different binaries");
  if (new Set(samples.map(sample => sample.dialect)).size !== 1) fail("matched memory samples use different dialects");
  for (const lane of lanes) {
    const laneSamples = samples.filter(sample => sample.lane === lane);
    const warmups = laneSamples.filter(sample => sample.role === "warmup");
    const measured = laneSamples.filter(sample => sample.role === "measurement").sort((a, b) => a.round - b.round);
    if (warmups.length !== 1) fail(`${lane} requires exactly one separate warmup process`);
    if (measured.length < Math.max(9, config.minimumSamples)) fail(`${lane} needs at least nine measurement processes`);
    measured.forEach((sample, index) => {
      if (sample.round !== index + 1) fail(`${lane} has duplicate or missing measurement rounds`);
      if (JSON.stringify(sample.checksum_sequence) !== JSON.stringify(warmups[0].checksum_sequence)) fail(`${lane} warmup/measurement checksum sequence differs`);
    });
    byLane.set(lane, measured);
  }
  const work = byLane.get("workload");
  const controls = byLane.get("null-control");
  if (work.length !== controls.length) fail("memory workload/control sample counts differ");
  // The null lane warms normally, then retains the callable without executing
  // guest epochs. Its checksum must remain that of the warm invocation.
  for (const control of controls) {
    if (!control.checksum_sequence.every(value => value === control.checksum_sequence[0])) fail("null-control executed epoch work");
  }
  if (work.some((sample, index) => sample.checksum_sequence[0] !== controls[index].checksum_sequence[0])) fail("paired warm invocation checksums differ");
  const row = summarizeGroup(work, config);
  row.measurement_protocol = MEMORY_PROTOCOL;
  row.control_sample_ids = controls.map(sample => sample.sample_id);
  row.metrics.null_retained_delta_anon_kib = stats(controls.map(sample => sample.quiescent_retained_delta_anon_kib));
  row.metrics.control_adjusted_retained_delta_anon_kib = stats(work.map((sample, index) =>
    sample.quiescent_retained_delta_anon_kib - controls[index].quiescent_retained_delta_anon_kib));
  row.metrics.anonymous_memory_area_kib_seconds_per_unit = stats(work.map(sample => sample.anonymous_memory_area_kib_seconds_per_unit));
  return row;
}

function analyze(config) {
  const samples = config.inputs.map((file) => sampleMetrics(loadJson(file), file));
  if (new Set(samples.map(sample => sample.sample_id)).size !== samples.length) fail("duplicate memory sample identity");
  const corrected = samples[0]?.measurement_protocol === MEMORY_PROTOCOL;
  if (samples.some(sample => (sample.measurement_protocol === MEMORY_PROTOCOL) !== corrected)) fail("cannot mix legacy and v2 raw memory protocols");
  const rows = [...groupBy(samples, groupKey).values()]
    .map((group) => corrected ? summarizeCorrectedGroup(group, config) : summarizeGroup(group, config))
    .sort(
      (left, right) =>
        left.target.localeCompare(right.target) ||
        left.workload.localeCompare(right.workload) ||
        left.engine.localeCompare(right.engine),
    );
  validatePortableChecksums(rows);
  return {
    schema_version: corrected ? 2 : 1,
    schema: corrected ? "rustia.bench.memory-analysis.v2" : ANALYSIS_SCHEMA,
    generated_at: new Date().toISOString(),
    method: {
      ...(corrected ? { measurement_protocol: MEMORY_PROTOCOL,
        quiescence_ms: QUIESCENCE_MS,
        retained_adjustment: "paired workload minus same-round null-control anonymous delta",
        anonymous_memory_area: "excess cgroup memory.stat anon above spawned baseline, warmed through quiescent",
        legacy_memory_area: "total cgroup charge; retained as a separate corroborating metric",
        warmup_processes_per_case: 2,
      } : {}),
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
    ...(corrected ? {
      warmup_sample_count: samples.filter(sample => sample.role === "warmup").length,
      control_sample_count: samples.filter(sample => sample.role === "measurement" && sample.lane === "null-control").length,
    } : {}),
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

  // The host label is the only provenance a row gets, so it is required, must
  // be a label rather than anything resolvable, and may be given once.
  const input = ["--input", "samples.json"];
  assert.equal(parseArgs([...input, "--host", "ci-bench"]).host, "ci-bench");
  assert.throws(() => parseArgs(input), /--host is required/);
  assert.equal(parseArgs([...input, "--host", "linode-ap-southeast-g6-dedicated-4"]).host, "linode-ap-southeast-g6-dedicated-4");
  assert.throws(() => parseArgs([...input, "--host", ""]), /requires a value|never an address/);
  for (const address of ["bench.example.com:22", "bench.example.com", "10.0.0.5", "fe80::1", "a b", "user@box"]) {
    assert.throws(() => parseArgs([...input, "--host", address]), /never an address/);
  }
  assert.throws(() => parseArgs([...input, "--host", "ci-bench", "--host", "ci-bench"]), /more than once/);
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

if (require.main === module) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`analyze-memory-samples: ${error.message}\n`);
    process.exitCode = 1;
  }
}
module.exports = { analyze, sampleMetrics, memoryAreaKibSeconds, summarizeCorrectedGroup };
