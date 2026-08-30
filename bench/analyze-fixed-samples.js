#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");

const RAW_SAMPLES_SCHEMA = "rustia.bench.raw-samples.v1";
const OUTPUT_SCHEMA = "rustia.bench.fixed-sample-analysis.v1";
const DEFAULT_BOOTSTRAP_ITERATIONS = 20_000;
const DEFAULT_MATERIAL_THRESHOLD_PERCENT = 5;
const DEFAULT_SEED = 0x52555354;
const MIN_BOOTSTRAP_ITERATIONS = 1_000;
const MAX_BOOTSTRAP_ITERATIONS = 1_000_000;
const MAX_EXPECTED_LEGS = 1_000_000;
const DEFAULT_CONFIDENCE_LEVEL = 0.95;
const COMPATIBILITY_CONFIDENCE_LEVEL = 0.95;
// A percentile interval is only as resolved as the number of resamples in its
// tails. At 99% with 1000 iterations each tail holds five values, so the bound
// is whichever five the seed happened to draw. Require ten per tail and say so,
// rather than emitting a confidently-worded interval nobody can rely on.
const MIN_BOOTSTRAP_TAIL_SAMPLES = 10;

const RAW_PROTOCOL_METADATA = Object.freeze({
  description:
    "Raw per-repetition timings in execution order. This sidecar is separate from the append-only public history schema.",
  measurement_isolation: "fresh prepared workload per repetition",
  warmup: "one untimed invocation immediately before each recorded sample",
  sample_order: "execution order (unsorted)",
});

const USAGE = `Usage:
  node bench/analyze-fixed-samples.js \\
    --baseline BASELINE-SIDECAR.json [--baseline ...] \\
    --candidate CANDIDATE-SIDECAR.json [--candidate ...] \\
    [--bootstrap-reps 20000] [--seed 1381323604] \\
    [--material-threshold 5] [--confidence 0.95] \\
    [--expected-legs N] [--require-full-commits]

Options:
  --baseline FILE            Add one baseline raw-sample sidecar (repeatable).
  --candidate FILE           Add one candidate raw-sample sidecar (repeatable).
  --bootstrap-reps N         Bootstrap iterations, 1000..1000000 (default: 20000).
  --seed N                   Unsigned 32-bit bootstrap seed (default: 1381323604).
  --material-threshold PCT   Predeclared material-regression threshold (default: 5).
  --confidence P             Interval level, 0.5 < P < 1 (default: 0.95). Screening
                             several workloads at once wants a higher level: five
                             independent 95% tests give about a 23% chance one of
                             them moves on a change that did nothing, against
                             about 5% at 0.99. More repetitions do not fix that --
                             they narrow each interval, they do not change the
                             per-test error rate.
  --expected-legs N          Require exactly N baseline and N candidate legs.
  --require-full-commits     Require distinct lowercase 40-hex commit IDs.
  --self-test                Run dependency-free in-memory tests.
  --help                     Show this help.
`;

function fail(message) {
  throw new Error(message);
}

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function requireObject(value, location) {
  if (!isPlainObject(value)) {
    fail(`${location} must be an object`);
  }
  return value;
}

function requireArray(value, location) {
  if (!Array.isArray(value)) {
    fail(`${location} must be an array`);
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
    fail(`${location} must be a safe integer >= ${minimum}`);
  }
  return value;
}

function requirePositiveNumber(value, location) {
  if (typeof value !== "number" || !Number.isFinite(value) || value <= 0) {
    fail(`${location} must be a finite number > 0`);
  }
  return value;
}

function requireFiniteNumber(value, location) {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    fail(`${location} must be a finite number`);
  }
  return value;
}

function requireIntegerRange(value, location, minimum, maximum) {
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    fail(`${location} must be a safe integer between ${minimum} and ${maximum}`);
  }
  return value;
}

// A confidence level is only usable if the bootstrap has enough resamples to
// resolve both of its tails. Checking the pair together is what makes the error
// actionable: it names the iteration count the requested level needs instead of
// rejecting the level on its own.
function requireConfidenceLevel(value, iterations, location) {
  requireFiniteNumber(value, location);
  if (!(value > 0.5) || !(value < 1)) {
    fail(`${location} must be greater than 0.5 and less than 1`);
  }
  const tailSamples = (iterations * (1 - value)) / 2;
  if (tailSamples < MIN_BOOTSTRAP_TAIL_SAMPLES) {
    const needed = Math.ceil((2 * MIN_BOOTSTRAP_TAIL_SAMPLES) / (1 - value));
    fail(
      `${location} ${value} needs at least ${needed} bootstrap iterations to resolve ` +
        `${MIN_BOOTSTRAP_TAIL_SAMPLES} resamples per tail, but ${iterations} were requested`,
    );
  }
  return value;
}

function sorted(values) {
  return values.slice().sort((left, right) => left - right);
}

// Matches script-bench-harness: for an even sample count this is the upper
// middle order statistic, not the average of the middle pair.
function harnessMedian(values) {
  const ordered = sorted(values);
  return ordered[Math.floor(ordered.length / 2)];
}

// Hyndman/Fan type 7 (the default used by R and many analysis tools).
function quantileSorted(ordered, probability) {
  assert(ordered.length > 0);
  assert(probability >= 0 && probability <= 1);
  if (ordered.length === 1) {
    return ordered[0];
  }
  const position = (ordered.length - 1) * probability;
  const lowerIndex = Math.floor(position);
  const upperIndex = Math.ceil(position);
  if (lowerIndex === upperIndex) {
    return ordered[lowerIndex];
  }
  const fraction = position - lowerIndex;
  return ordered[lowerIndex] * (1 - fraction) + ordered[upperIndex] * fraction;
}

function quantile(values, probability) {
  return quantileSorted(sorted(values), probability);
}

function pooledSummary(values) {
  const ordered = sorted(values);
  return {
    sample_count: ordered.length,
    min: cleanNumber(ordered[0]),
    q1: cleanNumber(quantileSorted(ordered, 0.25)),
    median: cleanNumber(quantileSorted(ordered, 0.5)),
    q3: cleanNumber(quantileSorted(ordered, 0.75)),
    max: cleanNumber(ordered[ordered.length - 1]),
  };
}

function numbersClose(left, right) {
  if (!Number.isFinite(left) || !Number.isFinite(right)) {
    return false;
  }
  const scale = Math.max(1, Math.abs(left), Math.abs(right));
  return Math.abs(left - right) <= scale * 1e-12;
}

function requireSummaryMatch(actual, expected, location) {
  if (!numbersClose(actual, expected)) {
    fail(`${location} is ${actual}, expected ${expected} from samples`);
  }
}

function validateSidecar(value, location) {
  const root = requireObject(value, location);
  if (root.schema !== RAW_SAMPLES_SCHEMA) {
    fail(`${location}.schema must be ${JSON.stringify(RAW_SAMPLES_SCHEMA)}`);
  }
  if (root.schema_version !== 1) {
    fail(`${location}.schema_version must be 1`);
  }
  if (root.source !== "script-bench-harness") {
    fail(`${location}.source must be "script-bench-harness"`);
  }

  const metadata = requireObject(root.metadata, `${location}.metadata`);
  for (const [field, expected] of Object.entries(RAW_PROTOCOL_METADATA)) {
    const actual = requireString(metadata[field], `${location}.metadata.${field}`);
    if (actual !== expected) {
      fail(
        `${location}.metadata.${field} must be ${JSON.stringify(expected)}, ` +
          `got ${JSON.stringify(actual)}`,
      );
    }
  }
  const measurementCount = requireInteger(
    metadata.measurement_count,
    `${location}.metadata.measurement_count`,
    0,
  );
  const metadataSampleCount = requireInteger(
    metadata.sample_count,
    `${location}.metadata.sample_count`,
    0,
  );
  const measurements = requireArray(root.measurements, `${location}.measurements`);
  if (measurementCount !== measurements.length) {
    fail(
      `${location}.metadata.measurement_count is ${measurementCount}, ` +
        `but measurements has length ${measurements.length}`,
    );
  }
  if (measurements.length !== 1) {
    fail(`${location} must contain exactly one measurement`);
  }

  const measurementLocation = `${location}.measurements[0]`;
  const measurement = requireObject(measurements[0], measurementLocation);
  const timestampUnix = requireInteger(
    measurement.timestamp_unix,
    `${measurementLocation}.timestamp_unix`,
    0,
  );
  const commit = requireString(measurement.commit, `${measurementLocation}.commit`);
  const target = requireString(measurement.target, `${measurementLocation}.target`);
  const engine = requireString(measurement.engine, `${measurementLocation}.engine`);
  const dialect = requireString(measurement.dialect, `${measurementLocation}.dialect`);
  const workload = requireString(measurement.workload, `${measurementLocation}.workload`);
  const unit = requireString(measurement.unit, `${measurementLocation}.unit`);
  const n = requireInteger(measurement.n, `${measurementLocation}.n`, 1);
  if (measurement.n_selection !== "fixed") {
    fail(`${measurementLocation}.n_selection must be "fixed"`);
  }
  const reps = requireInteger(measurement.reps, `${measurementLocation}.reps`, 1);
  const reportedMin = requirePositiveNumber(
    measurement.ns_per_unit_min,
    `${measurementLocation}.ns_per_unit_min`,
  );
  const reportedMedian = requirePositiveNumber(
    measurement.ns_per_unit_median,
    `${measurementLocation}.ns_per_unit_median`,
  );
  const unitsPerSecond = requirePositiveNumber(
    measurement.units_per_sec,
    `${measurementLocation}.units_per_sec`,
  );
  const checksum = requireString(measurement.checksum, `${measurementLocation}.checksum`);
  const rawSamples = requireArray(
    measurement.samples_ns_per_unit,
    `${measurementLocation}.samples_ns_per_unit`,
  );
  if (rawSamples.length !== reps) {
    fail(
      `${measurementLocation}.samples_ns_per_unit has length ${rawSamples.length}, ` +
        `but reps is ${reps}`,
    );
  }
  if (metadataSampleCount !== rawSamples.length) {
    fail(
      `${location}.metadata.sample_count is ${metadataSampleCount}, ` +
        `but the measurement contains ${rawSamples.length} samples`,
    );
  }
  const samples = rawSamples.map((sample, index) =>
    requirePositiveNumber(sample, `${measurementLocation}.samples_ns_per_unit[${index}]`),
  );
  const ordered = sorted(samples);
  requireSummaryMatch(reportedMin, ordered[0], `${measurementLocation}.ns_per_unit_min`);
  requireSummaryMatch(
    reportedMedian,
    harnessMedian(samples),
    `${measurementLocation}.ns_per_unit_median`,
  );
  requireSummaryMatch(
    unitsPerSecond,
    1e9 / reportedMin,
    `${measurementLocation}.units_per_sec`,
  );

  return {
    timestampUnix,
    commit,
    target,
    engine,
    dialect,
    workload,
    unit,
    n,
    reps,
    checksum,
    samples,
    reportedMedian,
  };
}

function loadSidecar(file, group, index) {
  const location = `${group}[${index}] ${JSON.stringify(file)}`;
  let canonicalFile;
  try {
    canonicalFile = fs.realpathSync(file);
  } catch (error) {
    fail(`${location}: cannot resolve file: ${error.message}`);
  }
  let bytes;
  try {
    bytes = fs.readFileSync(canonicalFile);
  } catch (error) {
    fail(`${location}: cannot read file: ${error.message}`);
  }
  const text = bytes.toString("utf8");
  let value;
  try {
    value = JSON.parse(text);
  } catch (error) {
    fail(`${location}: invalid JSON: ${error.message}`);
  }
  return {
    file,
    canonicalFile,
    contentSha256: crypto.createHash("sha256").update(bytes).digest("hex"),
    ...validateSidecar(value, location),
  };
}

function uniqueValue(records, field, description) {
  const first = records[0][field];
  for (let index = 1; index < records.length; index += 1) {
    if (records[index][field] !== first) {
      fail(
        `${description} must be consistent: ${JSON.stringify(records[0].file)} has ` +
          `${JSON.stringify(first)}, but ${JSON.stringify(records[index].file)} has ` +
          `${JSON.stringify(records[index][field])}`,
      );
    }
  }
  return first;
}

function validateUniqueInputs(records) {
  const seenCanonicalFiles = new Map();
  const seenContentDigests = new Map();
  for (const [index, recordValue] of records.entries()) {
    const record = requireObject(recordValue, `comparison record[${index}]`);
    const file = requireString(record.file, `comparison record[${index}].file`);
    const canonicalFile = requireString(
      record.canonicalFile,
      `comparison record[${index}].canonicalFile`,
    );
    const contentSha256 = requireString(
      record.contentSha256,
      `comparison record[${index}].contentSha256`,
    );
    if (!/^[0-9a-f]{64}$/.test(contentSha256)) {
      fail(`comparison record[${index}].contentSha256 must be 64 lowercase hex digits`);
    }

    const previousPath = seenCanonicalFiles.get(canonicalFile);
    if (previousPath !== undefined) {
      fail(
        `duplicate sidecar path: ${JSON.stringify(file)} resolves to ` +
          `${JSON.stringify(canonicalFile)}, already used by ${JSON.stringify(previousPath)}`,
      );
    }
    seenCanonicalFiles.set(canonicalFile, file);

    const previousDigest = seenContentDigests.get(contentSha256);
    if (previousDigest !== undefined) {
      fail(
        `duplicate sidecar content: ${JSON.stringify(file)} has SHA-256 ${contentSha256}, ` +
          `already used by ${JSON.stringify(previousDigest)}`,
      );
    }
    seenContentDigests.set(contentSha256, file);
  }
}

function validateComparison(
  baselineValue,
  candidateValue,
  expectedLegs = null,
  requireFullCommits = false,
) {
  const baseline = requireArray(baselineValue, "baseline records");
  const candidate = requireArray(candidateValue, "candidate records");
  if (baseline.length === 0) {
    fail("at least one --baseline FILE is required");
  }
  if (candidate.length === 0) {
    fail("at least one --candidate FILE is required");
  }
  if (baseline.length !== candidate.length) {
    fail(
      `baseline and candidate leg counts must match, got ${baseline.length} and ` +
        `${candidate.length}`,
    );
  }
  if (expectedLegs !== null && baseline.length !== expectedLegs) {
    fail(
      `expected exactly ${expectedLegs} baseline and candidate legs, got ` +
        `${baseline.length} each`,
    );
  }

  validateUniqueInputs(baseline.concat(candidate));

  const baselineCommit = uniqueValue(baseline, "commit", "baseline commit");
  const candidateCommit = uniqueValue(candidate, "commit", "candidate commit");
  if (baselineCommit === candidateCommit) {
    fail(`baseline and candidate commits must differ, both are ${JSON.stringify(baselineCommit)}`);
  }
  if (requireFullCommits) {
    for (const [description, commit] of [
      ["baseline commit", baselineCommit],
      ["candidate commit", candidateCommit],
    ]) {
      if (!/^[0-9a-f]{40}$/.test(commit)) {
        fail(`${description} must be exactly 40 lowercase hexadecimal digits`);
      }
    }
  }
  const all = baseline.concat(candidate);
  const identity = {};
  for (const [field, description] of [
    ["target", "target"],
    ["engine", "engine"],
    ["dialect", "dialect"],
    ["workload", "workload"],
    ["unit", "unit"],
    ["n", "fixed n"],
    ["reps", "repetitions"],
    ["checksum", "checksum"],
  ]) {
    identity[field] = uniqueValue(all, field, description);
  }
  return { baselineCommit, candidateCommit, identity };
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

function resampledMedian(values, random) {
  const sample = new Array(values.length);
  for (let index = 0; index < values.length; index += 1) {
    sample[index] = values[Math.floor(random() * values.length)];
  }
  return quantile(sample, 0.5);
}

function medianDeltaPercent(baselineMedian, candidateMedian) {
  const delta = (candidateMedian / baselineMedian - 1) * 100;
  return requireFiniteNumber(delta, "derived median delta percent");
}

// Both tails of a two-sided percentile interval at the given level. Split out
// so the operative interval and the retained 95% one come from one sorted array
// of resamples: two different levels over the same draws, not two experiments.
function percentileInterval(sortedDeltas, confidence) {
  const tail = (1 - confidence) / 2;
  return {
    lower: quantileSorted(sortedDeltas, tail),
    upper: quantileSorted(sortedDeltas, 1 - tail),
  };
}

function bootstrapIntervals(sortedDeltas, confidence) {
  return {
    operative: percentileInterval(sortedDeltas, confidence),
    compatibility: percentileInterval(sortedDeltas, COMPATIBILITY_CONFIDENCE_LEVEL),
  };
}

function pooledBootstrap(baseline, candidate, iterations, seed, confidence = DEFAULT_CONFIDENCE_LEVEL) {
  requireIntegerRange(
    iterations,
    "bootstrap iterations",
    MIN_BOOTSTRAP_ITERATIONS,
    MAX_BOOTSTRAP_ITERATIONS,
  );
  requireIntegerRange(seed, "bootstrap seed", 0, 0xffff_ffff);
  requireConfidenceLevel(confidence, iterations, "pooled bootstrap confidence");
  const random = mulberry32(seed);
  const deltas = new Array(iterations);
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const baselineMedian = resampledMedian(baseline, random);
    const candidateMedian = resampledMedian(candidate, random);
    deltas[iteration] = medianDeltaPercent(baselineMedian, candidateMedian);
  }
  deltas.sort((left, right) => left - right);
  const intervals = bootstrapIntervals(deltas, confidence);
  return { ...intervals.operative, compatibility: intervals.compatibility };
}

function resampledHierarchicalMedian(records, random) {
  const sample = [];
  for (let legIndex = 0; legIndex < records.length; legIndex += 1) {
    const leg = records[Math.floor(random() * records.length)].samples;
    for (let repetition = 0; repetition < leg.length; repetition += 1) {
      sample.push(leg[Math.floor(random() * leg.length)]);
    }
  }
  return quantile(sample, 0.5);
}

function hierarchicalBootstrap(
  baseline,
  candidate,
  iterations,
  seed,
  confidence = DEFAULT_CONFIDENCE_LEVEL,
) {
  requireIntegerRange(
    iterations,
    "bootstrap iterations",
    MIN_BOOTSTRAP_ITERATIONS,
    MAX_BOOTSTRAP_ITERATIONS,
  );
  requireIntegerRange(seed, "bootstrap seed", 0, 0xffff_ffff);
  requireConfidenceLevel(confidence, iterations, "hierarchical bootstrap confidence");
  const random = mulberry32(seed);
  const deltas = new Array(iterations);
  for (let iteration = 0; iteration < iterations; iteration += 1) {
    const baselineMedian = resampledHierarchicalMedian(baseline, random);
    const candidateMedian = resampledHierarchicalMedian(candidate, random);
    deltas[iteration] = medianDeltaPercent(baselineMedian, candidateMedian);
  }
  deltas.sort((left, right) => left - right);
  const intervals = bootstrapIntervals(deltas, confidence);
  return { ...intervals.operative, compatibility: intervals.compatibility };
}

// The verdict answers one question -- "is a regression larger than the declared
// threshold excluded?" -- and its threshold is one-sided. At a threshold of zero
// a genuine speedup therefore reports material_regression_excluded, which is
// true and reads as if nothing happened. The direction is a separate, additive
// reading of the same interval against zero, so a PR summary can lead with what
// actually moved without the verdict having to mean two things at once.
function classifyDirection(lower, upper) {
  if (upper < 0) {
    return { status: "faster", rule: "ci_upper_percent < 0" };
  }
  if (lower > 0) {
    return { status: "slower", rule: "ci_lower_percent > 0" };
  }
  return { status: "indistinguishable", rule: "confidence_interval_contains_or_touches_zero" };
}

function classifyVerdict(lower, upper, threshold) {
  if (upper < threshold) {
    return {
      status: "material_regression_excluded",
      rule: "ci_upper_percent < material_regression_threshold_percent",
    };
  }
  if (lower > threshold) {
    return {
      status: "regression",
      rule: "ci_lower_percent > material_regression_threshold_percent",
    };
  }
  return {
    status: "inconclusive",
    rule: "confidence_interval_contains_or_touches_material_threshold",
  };
}

function cleanNumber(value) {
  requireFiniteNumber(value, "analysis output number");
  const cleaned = Number(value.toPrecision(15));
  return Object.is(cleaned, -0) ? 0 : cleaned;
}

function validateAnalysisOptions(value) {
  const options = requireObject(value, "analysis options");
  const bootstrapIterations = requireIntegerRange(
    options.bootstrapIterations,
    "analysis options.bootstrapIterations",
    MIN_BOOTSTRAP_ITERATIONS,
    MAX_BOOTSTRAP_ITERATIONS,
  );
  const seed = requireIntegerRange(
    options.seed,
    "analysis options.seed",
    0,
    0xffff_ffff,
  );
  const materialThresholdPercent = requireFiniteNumber(
    options.materialThresholdPercent,
    "analysis options.materialThresholdPercent",
  );
  if (materialThresholdPercent < 0) {
    fail("analysis options.materialThresholdPercent must be >= 0");
  }
  const expectedLegs = options.expectedLegs === undefined ? null : options.expectedLegs;
  if (expectedLegs !== null) {
    requireIntegerRange(
      expectedLegs,
      "analysis options.expectedLegs",
      1,
      MAX_EXPECTED_LEGS,
    );
  }
  const requireFullCommits = options.requireFullCommits ?? false;
  if (typeof requireFullCommits !== "boolean") {
    fail("analysis options.requireFullCommits must be a boolean");
  }
  const confidenceLevel = requireConfidenceLevel(
    options.confidenceLevel ?? DEFAULT_CONFIDENCE_LEVEL,
    bootstrapIterations,
    "analysis options.confidenceLevel",
  );
  return {
    bootstrapIterations,
    seed,
    materialThresholdPercent,
    expectedLegs,
    requireFullCommits,
    confidenceLevel,
  };
}

function groupOutput(records, commit) {
  const pooled = records.flatMap((record) => record.samples);
  return {
    commit,
    leg_count: records.length,
    sample_count: pooled.length,
    legs: records.map((record, index) => ({
      leg: index + 1,
      file: record.file,
      timestamp_unix: record.timestampUnix,
      median_ns_per_unit: cleanNumber(record.reportedMedian),
    })),
    pooled_ns_per_unit: pooledSummary(pooled),
  };
}

function analyzeRecords(baseline, candidate, options) {
  const validatedOptions = validateAnalysisOptions(options);
  const { baselineCommit, candidateCommit, identity } = validateComparison(
    baseline,
    candidate,
    validatedOptions.expectedLegs,
    validatedOptions.requireFullCommits,
  );
  const baselineSamples = baseline.flatMap((record) => record.samples);
  const candidateSamples = candidate.flatMap((record) => record.samples);
  const baselineMedian = quantile(baselineSamples, 0.5);
  const candidateMedian = quantile(candidateSamples, 0.5);
  const delta = medianDeltaPercent(baselineMedian, candidateMedian);
  const pooledInterval = pooledBootstrap(
    baselineSamples,
    candidateSamples,
    validatedOptions.bootstrapIterations,
    validatedOptions.seed,
    validatedOptions.confidenceLevel,
  );
  const hierarchicalInterval = hierarchicalBootstrap(
    baseline,
    candidate,
    validatedOptions.bootstrapIterations,
    validatedOptions.seed,
    validatedOptions.confidenceLevel,
  );
  const envelope = (pooled, hierarchical) => ({
    lower: cleanNumber(Math.min(pooled.lower, hierarchical.lower)),
    upper: cleanNumber(Math.max(pooled.upper, hierarchical.upper)),
  });
  const clean = (value) => ({ lower: cleanNumber(value.lower), upper: cleanNumber(value.upper) });
  const reportedPooledInterval = clean(pooledInterval);
  const reportedHierarchicalInterval = clean(hierarchicalInterval);
  const reportedInterval = envelope(pooledInterval, hierarchicalInterval);
  // The three *_95_percent fields below are always the 95% interval whatever
  // --confidence asks for, so their names never lie and every consumer written
  // before this option keeps reading exactly what it always read. The operative
  // interval -- the one the verdict is drawn from -- is the unsuffixed field.
  const compatibilityPooled = clean(pooledInterval.compatibility);
  const compatibilityHierarchical = clean(hierarchicalInterval.compatibility);
  const compatibilityInterval = envelope(
    pooledInterval.compatibility,
    hierarchicalInterval.compatibility,
  );
  const reportedThreshold = cleanNumber(validatedOptions.materialThresholdPercent);
  const verdict = classifyVerdict(
    reportedInterval.lower,
    reportedInterval.upper,
    reportedThreshold,
  );
  const direction = classifyDirection(reportedInterval.lower, reportedInterval.upper);

  return {
    schema_version: 1,
    schema: OUTPUT_SCHEMA,
    identity: {
      target: identity.target,
      engine: identity.engine,
      dialect: identity.dialect,
      workload: identity.workload,
      unit: identity.unit,
      fixed_n: identity.n,
      reps_per_leg: identity.reps,
      legs_per_group: baseline.length,
      checksum: identity.checksum,
    },
    method: {
      leg_median: "script-bench-harness upper-middle order statistic",
      pooled_quantiles: "Hyndman/Fan type 7 linear interpolation",
      comparison: "candidate pooled median / baseline pooled median - 1",
      confidence_interval:
        "conservative envelope of seeded pooled and hierarchical percentile intervals",
      nominal_confidence_level: validatedOptions.confidenceLevel,
      bootstrap_iterations: validatedOptions.bootstrapIterations,
      bootstrap_seed_uint32: validatedOptions.seed,
      pooled_resampling_unit: "samples independently within each pooled group",
      hierarchical_resampling_unit:
        "legs with replacement, then repetitions with replacement within each selected leg",
      compatibility_intervals:
        "comparison.*_ci_95_percent are always the 95% interval regardless of --confidence, for consumers written before it existed; comparison.*_ci_percent are at comparison.confidence_level and are what the verdict and direction are drawn from",
      coverage_caveat:
        "the stated level is nominal: the pooled interval assumes IID repetitions; the hierarchical sensitivity interval models observed leg clustering but cannot cover unobserved host states",
      expected_legs: validatedOptions.expectedLegs,
      require_full_commits: validatedOptions.requireFullCommits,
    },
    baseline: groupOutput(baseline, baselineCommit),
    candidate: groupOutput(candidate, candidateCommit),
    comparison: {
      candidate_delta_percent: cleanNumber(delta),
      confidence_level: validatedOptions.confidenceLevel,
      pooled_bootstrap_ci_percent: reportedPooledInterval,
      hierarchical_bootstrap_ci_percent: reportedHierarchicalInterval,
      bootstrap_ci_percent: reportedInterval,
      pooled_bootstrap_ci_95_percent: compatibilityPooled,
      hierarchical_bootstrap_ci_95_percent: compatibilityHierarchical,
      bootstrap_ci_95_percent: compatibilityInterval,
      material_regression_threshold_percent: reportedThreshold,
      direction,
      verdict,
    },
  };
}

function parseIntegerOption(value, option, minimum, maximum) {
  if (!/^(0|[1-9][0-9]*)$/.test(value)) {
    fail(`${option} requires an integer`);
  }
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    fail(`${option} must be between ${minimum} and ${maximum}`);
  }
  return parsed;
}

function parseArgs(args) {
  const config = {
    baselineFiles: [],
    candidateFiles: [],
    bootstrapIterations: DEFAULT_BOOTSTRAP_ITERATIONS,
    expectedLegs: null,
    confidenceLevel: DEFAULT_CONFIDENCE_LEVEL,
    materialThresholdPercent: DEFAULT_MATERIAL_THRESHOLD_PERCENT,
    requireFullCommits: false,
    seed: DEFAULT_SEED,
    selfTest: false,
    help: false,
  };
  let analysisOptionSeen = false;

  for (let index = 0; index < args.length; index += 1) {
    const option = args[index];
    if (option === "--help" || option === "-h") {
      config.help = true;
      continue;
    }
    if (option === "--self-test") {
      config.selfTest = true;
      continue;
    }
    if (option === "--require-full-commits") {
      config.requireFullCommits = true;
      analysisOptionSeen = true;
      continue;
    }
    const value = args[index + 1];
    if (value === undefined) {
      fail(`${option} requires a value`);
    }
    if (option === "--baseline") {
      config.baselineFiles.push(value);
    } else if (option === "--candidate") {
      config.candidateFiles.push(value);
    } else if (option === "--bootstrap-reps" || option === "--bootstrap-iterations") {
      config.bootstrapIterations = parseIntegerOption(
        value,
        option,
        MIN_BOOTSTRAP_ITERATIONS,
        MAX_BOOTSTRAP_ITERATIONS,
      );
    } else if (option === "--seed") {
      config.seed = parseIntegerOption(value, option, 0, 0xffff_ffff);
    } else if (
      option === "--material-threshold" ||
      option === "--material-threshold-percent"
    ) {
      const parsed = Number(value);
      if (value.trim() === "" || !Number.isFinite(parsed) || parsed < 0) {
        fail(`${option} must be a finite number >= 0`);
      }
      config.materialThresholdPercent = parsed;
    } else if (option === "--confidence" || option === "--confidence-level") {
      const parsed = Number(value);
      if (value.trim() === "" || !Number.isFinite(parsed)) {
        fail(`${option} must be a finite number`);
      }
      config.confidenceLevel = parsed;
    } else if (option === "--expected-legs") {
      config.expectedLegs = parseIntegerOption(value, option, 1, MAX_EXPECTED_LEGS);
    } else {
      fail(`unknown option ${JSON.stringify(option)}`);
    }
    analysisOptionSeen = true;
    index += 1;
  }

  if (config.selfTest && analysisOptionSeen) {
    fail("--self-test cannot be combined with analysis options");
  }
  return config;
}

function fixtureMeasurement(overrides = {}) {
  const samples = overrides.samples ?? [99, 100, 101, 102, 103];
  const ordered = sorted(samples);
  const minimum = ordered[0];
  return {
    timestamp_unix: overrides.timestampUnix ?? 1_700_000_000,
    commit: overrides.commit ?? "baseline-commit",
    target: overrides.target ?? "test-os-test-arch",
    engine: overrides.engine ?? "rustia",
    dialect: overrides.dialect ?? "luau",
    workload: overrides.workload ?? "dense_tables",
    unit: overrides.unit ?? "build+sum+sort of 256-elem array",
    n: overrides.n ?? 1500,
    n_selection: overrides.nSelection ?? "fixed",
    reps: overrides.reps ?? samples.length,
    ns_per_unit_min: overrides.reportedMin ?? minimum,
    ns_per_unit_median: overrides.reportedMedian ?? harnessMedian(samples),
    units_per_sec: overrides.unitsPerSecond ?? 1e9 / minimum,
    checksum: overrides.checksum ?? "number:193302491.00000000000000000",
    samples_ns_per_unit: samples,
  };
}

function fixtureSidecar(overrides = {}) {
  const measurements = overrides.measurements ?? [fixtureMeasurement(overrides)];
  const sampleCount = measurements.reduce(
    (total, measurement) => total + measurement.samples_ns_per_unit.length,
    0,
  );
  return {
    schema_version: overrides.schemaVersion === undefined ? 1 : overrides.schemaVersion,
    schema: overrides.schema ?? RAW_SAMPLES_SCHEMA,
    source: overrides.source ?? "script-bench-harness",
    metadata: {
      description: overrides.description ?? RAW_PROTOCOL_METADATA.description,
      measurement_isolation:
        overrides.measurementIsolation ?? RAW_PROTOCOL_METADATA.measurement_isolation,
      warmup: overrides.warmup ?? RAW_PROTOCOL_METADATA.warmup,
      sample_order: overrides.sampleOrder ?? RAW_PROTOCOL_METADATA.sample_order,
      measurement_count:
        overrides.measurementCount === undefined
          ? measurements.length
          : overrides.measurementCount,
      sample_count:
        overrides.sampleCount === undefined ? sampleCount : overrides.sampleCount,
    },
    measurements,
  };
}

function fixtureRecord(file, overrides = {}) {
  const sidecar = fixtureSidecar(overrides);
  const text = JSON.stringify(sidecar);
  return {
    file,
    canonicalFile: `/synthetic/${file}`,
    contentSha256: crypto.createHash("sha256").update(text).digest("hex"),
    ...validateSidecar(sidecar, file),
  };
}

function assertThrowsMessage(callback, pattern) {
  assert.throws(callback, (error) => {
    assert.match(error.message, pattern);
    return true;
  });
}

function runSelfTest() {
  const baseline1 = fixtureRecord("A1.json", {
    samples: [99, 100, 101, 102, 103],
    timestampUnix: 1,
  });
  const baseline2 = fixtureRecord("A2.json", {
    samples: [98, 100, 102, 104, 106],
    timestampUnix: 2,
  });
  const candidate1 = fixtureRecord("B1.json", {
    samples: [100, 102, 103, 104, 106],
    commit: "candidate-commit",
    timestampUnix: 3,
  });
  const candidate2 = fixtureRecord("B2.json", {
    samples: [99, 101, 103, 105, 107],
    commit: "candidate-commit",
    timestampUnix: 4,
  });
  const options = {
    bootstrapIterations: 1_000,
    expectedLegs: 2,
    materialThresholdPercent: 5,
    requireFullCommits: false,
    seed: 123456,
  };
  const first = analyzeRecords([baseline1, baseline2], [candidate1, candidate2], options);
  const second = analyzeRecords([baseline1, baseline2], [candidate1, candidate2], options);
  assert.deepEqual(first, second, "the seeded analysis must be deterministic");
  assert.equal(first.schema, OUTPUT_SCHEMA);
  assert.equal(first.identity.fixed_n, 1500);
  assert.equal(first.baseline.leg_count, 2);
  assert.equal(first.candidate.leg_count, 2);
  assert.equal(first.baseline.sample_count, 10);
  assert.equal(first.baseline.legs[0].median_ns_per_unit, 101);
  assert.deepEqual(first.baseline.pooled_ns_per_unit, {
    sample_count: 10,
    min: 98,
    q1: 100,
    median: 101.5,
    q3: 102.75,
    max: 106,
  });
  assert.equal(first.identity.legs_per_group, 2);
  assert.equal(first.method.nominal_confidence_level, 0.95);
  assert.equal(first.method.expected_legs, 2);
  assert.equal(first.method.require_full_commits, false);
  assert.equal(
    first.comparison.bootstrap_ci_95_percent.lower,
    Math.min(
      first.comparison.pooled_bootstrap_ci_95_percent.lower,
      first.comparison.hierarchical_bootstrap_ci_95_percent.lower,
    ),
  );
  assert.equal(
    first.comparison.bootstrap_ci_95_percent.upper,
    Math.max(
      first.comparison.pooled_bootstrap_ci_95_percent.upper,
      first.comparison.hierarchical_bootstrap_ci_95_percent.upper,
    ),
  );
  assert(first.comparison.bootstrap_ci_95_percent.lower <= first.comparison.candidate_delta_percent);
  assert(first.comparison.bootstrap_ci_95_percent.upper >= first.comparison.candidate_delta_percent);

  assert.deepEqual(classifyVerdict(-2, 4.999, 5).status, "material_regression_excluded");
  assert.deepEqual(classifyVerdict(5.001, 8, 5).status, "regression");
  assert.deepEqual(classifyVerdict(5, 8, 5).status, "inconclusive");
  assert.deepEqual(classifyVerdict(-2, 5, 5).status, "inconclusive");

  assertThrowsMessage(
    () => validateSidecar(fixtureSidecar({ schema: "wrong" }), "bad.json"),
    /\.schema must be/,
  );
  assertThrowsMessage(
    () => validateSidecar(fixtureSidecar({ warmup: "different protocol" }), "bad.json"),
    /metadata\.warmup must be/,
  );
  assertThrowsMessage(
    () =>
      validateSidecar(
        fixtureSidecar({
          measurements: [fixtureMeasurement(), fixtureMeasurement()],
        }),
        "bad.json",
      ),
    /exactly one measurement/,
  );
  assertThrowsMessage(
    () => validateSidecar(fixtureSidecar({ nSelection: "calibrated" }), "bad.json"),
    /n_selection must be "fixed"/,
  );
  assertThrowsMessage(
    () => validateSidecar(fixtureSidecar({ reps: 6 }), "bad.json"),
    /samples_ns_per_unit has length 5, but reps is 6/,
  );
  assertThrowsMessage(
    () => validateSidecar(fixtureSidecar({ sampleCount: 4 }), "bad.json"),
    /metadata\.sample_count is 4/,
  );
  assertThrowsMessage(
    () =>
      validateSidecar(
        fixtureSidecar({
          samples: [1e-300],
          unitsPerSecond: 1,
        }),
        "bad.json",
      ),
    /expected Infinity from samples/,
  );
  assertThrowsMessage(
    () =>
      validateComparison(
        [baseline1, { ...baseline2, commit: "another-baseline" }],
        [candidate1, candidate2],
      ),
    /baseline commit must be consistent/,
  );
  assertThrowsMessage(
    () =>
      validateComparison(
        [baseline1, baseline2],
        [candidate1, { ...candidate2, commit: "another-candidate" }],
      ),
    /candidate commit must be consistent/,
  );
  assertThrowsMessage(
    () =>
      validateComparison(
        [baseline1, baseline2],
        [candidate1, { ...candidate2, checksum: "wrong" }],
      ),
    /checksum must be consistent/,
  );
  assertThrowsMessage(
    () => validateComparison([baseline1], [{ ...candidate1, n: 1600 }]),
    /fixed n must be consistent/,
  );
  assertThrowsMessage(
    () => validateComparison([baseline1, baseline2], [candidate1]),
    /leg counts must match/,
  );
  assertThrowsMessage(
    () => validateComparison([baseline1, baseline2], [candidate1, candidate2], 3),
    /expected exactly 3/,
  );
  assertThrowsMessage(
    () =>
      validateComparison(
        [baseline1, { ...baseline2, canonicalFile: baseline1.canonicalFile }],
        [candidate1, candidate2],
      ),
    /duplicate sidecar path/,
  );
  assertThrowsMessage(
    () =>
      validateComparison(
        [baseline1, { ...baseline2, contentSha256: baseline1.contentSha256 }],
        [candidate1, candidate2],
      ),
    /duplicate sidecar content/,
  );
  assertThrowsMessage(
    () =>
      validateComparison(
        [baseline1, baseline2],
        [
          { ...candidate1, commit: baseline1.commit },
          { ...candidate2, commit: baseline1.commit },
        ],
      ),
    /commits must differ/,
  );
  assertThrowsMessage(
    () => validateComparison([baseline1, baseline2], [candidate1, candidate2], 2, true),
    /baseline commit must be exactly 40/,
  );
  const fullBaseline = [baseline1, baseline2].map((record) => ({
    ...record,
    commit: "a".repeat(40),
  }));
  const fullCandidate = [candidate1, candidate2].map((record) => ({
    ...record,
    commit: "b".repeat(40),
  }));
  assert.doesNotThrow(() => validateComparison(fullBaseline, fullCandidate, 2, true));
  assertThrowsMessage(
    () => parseArgs(["--baseline", "A.json", "--seed", "4294967296"]),
    /--seed must be between/,
  );
  assertThrowsMessage(
    () => parseArgs(["--bootstrap-reps", "999"]),
    /--bootstrap-reps must be between 1000 and 1000000/,
  );
  assertThrowsMessage(
    () => parseArgs(["--bootstrap-reps", "1000001"]),
    /--bootstrap-reps must be between 1000 and 1000000/,
  );
  const parsed = parseArgs([
    "--baseline",
    "A1.json",
    "--baseline",
    "A2.json",
    "--candidate",
    "B1.json",
    "--bootstrap-reps",
    "1000",
    "--material-threshold",
    "3.5",
    "--expected-legs",
    "2",
    "--require-full-commits",
    "--seed",
    "7",
  ]);
  assert.deepEqual(parsed.baselineFiles, ["A1.json", "A2.json"]);
  assert.deepEqual(parsed.candidateFiles, ["B1.json"]);
  assert.equal(parsed.bootstrapIterations, 1000);
  assert.equal(parsed.expectedLegs, 2);
  assert.equal(parsed.materialThresholdPercent, 3.5);
  assert.equal(parsed.requireFullCommits, true);
  assert.equal(parsed.seed, 7);

  assertThrowsMessage(
    () =>
      analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
        ...options,
        bootstrapIterations: 999,
      }),
    /bootstrapIterations must be a safe integer between 1000 and 1000000/,
  );
  assertThrowsMessage(
    () =>
      analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
        ...options,
        materialThresholdPercent: Number.POSITIVE_INFINITY,
      }),
    /materialThresholdPercent must be a finite number/,
  );
  assertThrowsMessage(
    () =>
      analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
        ...options,
        requireFullCommits: "yes",
      }),
    /requireFullCommits must be a boolean/,
  );

  const extremeBaseline = [baseline1, baseline2].map((record) => ({
    ...record,
    samples: Array(5).fill(1e-299),
    reportedMedian: 1e-299,
  }));
  const extremeCandidate = [candidate1, candidate2].map((record) => ({
    ...record,
    samples: Array(5).fill(1e308),
    reportedMedian: 1e308,
  }));
  assertThrowsMessage(
    () => analyzeRecords(extremeBaseline, extremeCandidate, options),
    /derived median delta percent must be a finite number/,
  );

  // Direction reads the same interval as the verdict but against zero, so a
  // speedup stops being reported as "no material regression".
  assert.equal(classifyDirection(-4, -1).status, "faster");
  assert.equal(classifyDirection(1, 4).status, "slower");
  assert.equal(classifyDirection(-1, 4).status, "indistinguishable");
  assert.equal(classifyDirection(-4, 0).status, "indistinguishable");
  assert.equal(classifyDirection(0, 4).status, "indistinguishable");
  assert.equal(first.comparison.direction.status, classifyDirection(
    first.comparison.bootstrap_ci_percent.lower,
    first.comparison.bootstrap_ci_percent.upper,
  ).status);

  // A speedup large enough to clear zero must read as faster while the verdict,
  // which is one-sided against the threshold, still says a regression is
  // excluded. Both statements are true; only one of them is a headline.
  const fasterCandidate = [candidate1, candidate2].map((record, index) => ({
    ...record,
    samples: [50, 51, 52, 53, 54].map((value) => value + index),
    reportedMedian: 52 + index,
  }));
  const speedup = analyzeRecords([baseline1, baseline2], fasterCandidate, options);
  assert.equal(speedup.comparison.direction.status, "faster");
  assert.equal(speedup.comparison.verdict.status, "material_regression_excluded");

  // The retained 95% fields must not move when --confidence does. Run the same
  // records at both levels and require the 0.99 run's compatibility fields to
  // equal the 0.95 run's operative fields exactly -- that equality is the whole
  // promise those field names make to consumers written before the option.
  const wideOptions = { ...options, bootstrapIterations: 4_000 };
  const atNinetyFive = analyzeRecords([baseline1, baseline2], [candidate1, candidate2], wideOptions);
  const atNinetyNine = analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
    ...wideOptions,
    confidenceLevel: 0.99,
  });
  assert.equal(atNinetyFive.comparison.confidence_level, 0.95);
  assert.equal(atNinetyNine.comparison.confidence_level, 0.99);
  assert.deepEqual(
    atNinetyNine.comparison.bootstrap_ci_95_percent,
    atNinetyFive.comparison.bootstrap_ci_percent,
  );
  assert.deepEqual(
    atNinetyNine.comparison.pooled_bootstrap_ci_95_percent,
    atNinetyFive.comparison.pooled_bootstrap_ci_percent,
  );
  assert.deepEqual(
    atNinetyNine.comparison.hierarchical_bootstrap_ci_95_percent,
    atNinetyFive.comparison.hierarchical_bootstrap_ci_percent,
  );
  // And at the default level the two are the same object value, so nothing
  // reading either field can disagree with the verdict.
  assert.deepEqual(
    atNinetyFive.comparison.bootstrap_ci_95_percent,
    atNinetyFive.comparison.bootstrap_ci_percent,
  );
  // A higher level can only widen the operative interval.
  assert(
    atNinetyNine.comparison.bootstrap_ci_percent.lower <=
      atNinetyFive.comparison.bootstrap_ci_percent.lower,
  );
  assert(
    atNinetyNine.comparison.bootstrap_ci_percent.upper >=
      atNinetyFive.comparison.bootstrap_ci_percent.upper,
  );
  assert.equal(atNinetyNine.method.nominal_confidence_level, 0.99);

  assertThrowsMessage(
    () => analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
      ...options,
      confidenceLevel: 0.5,
    }),
    /confidenceLevel must be greater than 0\.5 and less than 1/,
  );
  assertThrowsMessage(
    () => analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
      ...options,
      confidenceLevel: 1,
    }),
    /confidenceLevel must be greater than 0\.5 and less than 1/,
  );
  // 0.99 over 1000 iterations leaves five resamples per tail, which is not an
  // interval so much as a coin flip. It must be refused, and the message must
  // name the iteration count that would work.
  assertThrowsMessage(
    () => analyzeRecords([baseline1, baseline2], [candidate1, candidate2], {
      ...options,
      confidenceLevel: 0.99,
    }),
    /needs at least 2000 bootstrap iterations to resolve 10 resamples per tail, but 1000 were requested/,
  );
  assert.equal(requireConfidenceLevel(0.99, 2_000, "probe"), 0.99);

  assert.equal(parseArgs(["--confidence", "0.99"]).confidenceLevel, 0.99);
  assert.equal(parseArgs([]).confidenceLevel, 0.95);
  assertThrowsMessage(() => parseArgs(["--confidence", "nope"]), /--confidence must be a finite number/);

  process.stdout.write("analyze-fixed-samples self-test: ok\n");
}

function main(args) {
  const config = parseArgs(args);
  if (config.help) {
    process.stdout.write(USAGE);
    return;
  }
  if (config.selfTest) {
    runSelfTest();
    return;
  }
  if (config.baselineFiles.length === 0 || config.candidateFiles.length === 0) {
    fail("at least one --baseline FILE and one --candidate FILE are required; use --help");
  }
  const baseline = config.baselineFiles.map((file, index) =>
    loadSidecar(file, "baseline", index),
  );
  const candidate = config.candidateFiles.map((file, index) =>
    loadSidecar(file, "candidate", index),
  );
  const output = analyzeRecords(baseline, candidate, config);
  process.stdout.write(`${JSON.stringify(output, null, 2)}\n`);
}

if (require.main === module) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    process.stderr.write(`analyze-fixed-samples: ${error.message}\n`);
    process.exitCode = 1;
  }
}

module.exports = {
  analyzeRecords,
  classifyDirection,
  classifyVerdict,
  hierarchicalBootstrap,
  parseArgs,
  pooledBootstrap,
  pooledSummary,
  requireConfidenceLevel,
  validateAnalysisOptions,
  validateComparison,
  validateSidecar,
};
