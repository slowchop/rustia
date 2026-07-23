#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = path.join(__dirname, "current-results.json");
const historyPath = path.join(__dirname, "history", "runs.jsonl");
const wasmResultsPath = path.join(__dirname, "wasm", "current-results.json");
const wasmDispatchPath = path.join(__dirname, "wasm", "dispatch-current-results.json");
const cohortEvidencePath = path.join(__dirname, "history", "evidence");
const outputPath = path.join(__dirname, "current-results.html");
const checkOnly = process.argv.includes("--check");
const REQUIRED_COMMON_WORKLOADS = Object.freeze([
  "closures_upvalues",
  "dense_tables",
  "generic_iteration",
  "sparse_tables",
  "stdlib_math_table",
]);
const REQUIRED_COMMON_WORKLOAD_SET = new Set(REQUIRED_COMMON_WORKLOADS);

function fail(message) {
  console.error(`render-current-results: ${message}`);
  process.exit(1);
}

function readData() {
  let data;
  try {
    data = JSON.parse(fs.readFileSync(sourcePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(sourcePath)}: ${error.message}`);
  }

  if (data.schema_version !== 1) {
    fail("schema_version must be 1");
  }
  if (data.schema !== "rustia.bench.current-results.v1") {
    fail('schema must be "rustia.bench.current-results.v1"');
  }
  if (data.source !== "script-bench-harness") {
    fail('source must be "script-bench-harness"');
  }
  if (data.metadata == null || typeof data.metadata !== "object" || Array.isArray(data.metadata)) {
    fail("metadata must be an object");
  }
  for (const key of ["targets", "engines", "workloads", "commits"]) {
    if (!Array.isArray(data.metadata[key])) {
      fail(`metadata.${key} must be an array`);
    }
  }
  if (!Array.isArray(data.rows)) {
    fail("rows must be an array");
  }
  if (!Number.isInteger(data.metadata.row_count) || data.metadata.row_count !== data.rows.length) {
    fail("metadata.row_count must match rows.length");
  }
  if (data.rows.length === 0) {
    fail("rows must contain at least one result");
  }
  if (typeof data.metadata.history !== "string" || data.metadata.history.trim() === "") {
    fail("metadata.history must be a non-empty string");
  }
  if (typeof data.metadata.history_schema !== "string" || data.metadata.history_schema.trim() === "") {
    fail("metadata.history_schema must be a non-empty string");
  }
  const window = data.metadata.measurement_window_unix;
  if (window == null || typeof window !== "object" || Array.isArray(window)) {
    fail("metadata.measurement_window_unix must be an object");
  }
  if (!Number.isFinite(window.start) || !Number.isFinite(window.end) || window.start <= 0 || window.end < window.start) {
    fail("metadata.measurement_window_unix must contain positive start/end values");
  }

  data.rows.forEach(validateRow);
  validateMetadataMatchesRows(data);
  return data;
}

function readHistory() {
  let text;
  try {
    text = fs.readFileSync(historyPath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(historyPath)}: ${error.message}`);
  }

  const records = [];
  for (const [index, rawLine] of text.split(/\r?\n/).entries()) {
    const line = rawLine.trim();
    if (line === "") {
      continue;
    }
    let record;
    try {
      record = JSON.parse(line);
    } catch (error) {
      fail(`${relative(historyPath)} line ${index + 1} is not JSON: ${error.message}`);
    }
    records.push(normalizeHistoryRecord(record, index + 1));
  }
  if (records.length === 0) {
    fail(`${relative(historyPath)} must contain at least one benchmark record`);
  }
  return records.sort(compareRunTime);
}

function normalizeHistoryRecord(record, lineNumber) {
  const label = `${relative(historyPath)} line ${lineNumber}`;
  for (const key of ["commit", "target", "engine", "dialect", "workload", "unit", "checksum"]) {
    if (typeof record[key] !== "string" || record[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  for (const key of ["timestamp_unix", "n", "reps", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
    if (!Number.isFinite(Number(record[key])) || Number(record[key]) <= 0) {
      fail(`${label}.${key} must be a positive finite number`);
    }
  }
  return {
    timestamp_unix: Number(record.timestamp_unix),
    commit: record.commit,
    target: record.target,
    engine: record.engine,
    dialect: record.dialect,
    workload: record.workload,
    unit: record.unit,
    n: Number(record.n),
    reps: Number(record.reps),
    ns_per_unit_min: Number(record.ns_per_unit_min),
    ns_per_unit_median: Number(record.ns_per_unit_median),
    units_per_sec: Number(record.units_per_sec),
    checksum: record.checksum,
    prepare_ns_min: optionalPositive(record.prepare_ns_min),
    prepare_ns_median: optionalPositive(record.prepare_ns_median),
    first_call_ns_min: optionalPositive(record.first_call_ns_min),
    first_call_ns_median: optionalPositive(record.first_call_ns_median),
    cold_start_ns_min: optionalPositive(record.cold_start_ns_min),
    cold_start_ns_median: optionalPositive(record.cold_start_ns_median),
    mode_family: typeof record.mode_family === "string" ? record.mode_family : modeFamily(record.engine),
    execution_mode: typeof record.execution_mode === "string" ? record.execution_mode : executionMode(record.engine),
    native_verified: record.native_verified === true,
    native_verification: typeof record.native_verification === "string" ? record.native_verification : "not-recorded",
    native_verification_ratio: optionalPositive(record.native_verification_ratio),
  };
}

function optionalPositive(value) {
  return Number.isFinite(Number(value)) && Number(value) > 0 ? Number(value) : null;
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(filePath)}: ${error.message}`);
  }
}

function readWasmResults() {
  const data = readJson(wasmResultsPath);
  if (data.schema_version !== 1) {
    fail("wasm results schema_version must be 1");
  }
  if (data.schema !== "rustia.bench.wasm-gate-current-results.v1") {
    fail('wasm results schema must be "rustia.bench.wasm-gate-current-results.v1"');
  }
  if (!Array.isArray(data.runs) || data.runs.length === 0) {
    fail("wasm results runs must be a non-empty array");
  }
  data.runs.forEach(validateWasmRun);
  return data;
}

function readWasmDispatchResults() {
  const data = readJson(wasmDispatchPath);
  if (data.schema_version !== 1) {
    fail("wasm dispatch schema_version must be 1");
  }
  if (data.schema !== "rustia.bench.wasm-dispatch-current-results.v1") {
    fail('wasm dispatch schema must be "rustia.bench.wasm-dispatch-current-results.v1"');
  }
  if (!Array.isArray(data.runs) || data.runs.length === 0) {
    fail("wasm dispatch runs must be a non-empty array");
  }
  data.runs.forEach(validateWasmDispatchRun);
  return data;
}

function readCohortEvidence() {
  let entries;
  try {
    entries = fs.readdirSync(cohortEvidencePath, { withFileTypes: true });
  } catch (error) {
    fail(`could not read ${relative(cohortEvidencePath)}: ${error.message}`);
  }
  return entries
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(cohortEvidencePath, entry.name, "manifest.json"))
    .filter((filePath) => fs.existsSync(filePath))
    .map((filePath) => normalizeCohortEvidence(readJson(filePath), filePath))
    .filter((cohort) => cohort.published)
    .sort((left, right) => left.createdAt.localeCompare(right.createdAt));
}

function normalizeCohortEvidence(data, filePath) {
  const label = relative(filePath);
  if (data.schema !== "rustia.bench.cloud-evidence.v1") {
    fail(`${label}.schema must be rustia.bench.cloud-evidence.v1`);
  }
  if (!data.comparison || !Array.isArray(data.comparison.legs) || data.comparison.legs.length === 0) {
    fail(`${label}.comparison.legs must be a non-empty array`);
  }
  const interval = data.comparison.conservative_bootstrap_ci_95_percent;
  for (const [key, value] of Object.entries({
    delta: data.comparison.candidate_delta_percent,
    lower: interval?.lower,
    upper: interval?.upper,
  })) {
    if (!Number.isFinite(value)) fail(`${label} ${key} must be finite`);
  }
  const workload = data.comparison.legs[0].workload;
  if (typeof workload !== "string" || workload === "") fail(`${label} workload must be present`);
  const sampleCount = data.comparison.legs.reduce((sum, leg) => sum + Number(leg.sample_count ?? 0), 0);
  if (!Number.isSafeInteger(sampleCount) || sampleCount <= 0) fail(`${label} sample count must be positive`);
  return {
    workload,
    createdAt: data.created_at,
    candidateCommit: data.source?.candidate_commit,
    delta: data.comparison.candidate_delta_percent,
    lower: interval.lower,
    upper: interval.upper,
    legCount: data.comparison.legs.length,
    sampleCount,
    verdict: data.comparison.verdict?.status,
    published: data.publication?.performed === true,
  };
}

function validateRow(row, index) {
  const label = `rows[${index}]`;
  for (const key of ["commit", "target", "engine", "dialect", "workload", "unit", "checksum"]) {
    if (typeof row[key] !== "string" || row[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  for (const key of ["timestamp_unix", "n", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
    if (!Number.isFinite(row[key]) || row[key] <= 0) {
      fail(`${label}.${key} must be a positive finite number`);
    }
  }
  if (row.speed_vs_rustia != null && (!Number.isFinite(row.speed_vs_rustia) || row.speed_vs_rustia <= 0)) {
    fail(`${label}.speed_vs_rustia must be null or a positive finite number`);
  }
  for (const key of ["prepare_ns_min", "prepare_ns_median", "first_call_ns_min", "first_call_ns_median", "cold_start_ns_min", "cold_start_ns_median"]) {
    if (row[key] != null && (!Number.isFinite(row[key]) || row[key] <= 0)) {
      fail(`${label}.${key} must be null or a positive finite number`);
    }
  }
}

function validateWasmRun(run, index) {
  const label = `wasm runs[${index}]`;
  if (run.schema !== "rustia.bench.wasm-runtime-results.v1") {
    fail(`${label}.schema must be "rustia.bench.wasm-runtime-results.v1"`);
  }
  for (const key of ["commit", "target"]) {
    if (typeof run[key] !== "string" || run[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  if (!Number.isFinite(run.timestamp_unix) || run.timestamp_unix <= 0) {
    fail(`${label}.timestamp_unix must be a positive finite number`);
  }
  if (run.runtime == null || typeof run.runtime !== "object" || Array.isArray(run.runtime)) {
    fail(`${label}.runtime must be an object`);
  }
  if (run.adapter == null || typeof run.adapter !== "object" || Array.isArray(run.adapter)) {
    fail(`${label}.adapter must be an object`);
  }
  for (const key of ["name", "family"]) {
    if (typeof run.runtime[key] !== "string" || run.runtime[key].trim() === "") {
      fail(`${label}.runtime.${key} must be a non-empty string`);
    }
  }
  for (const key of ["id", "dialect"]) {
    if (typeof run.adapter[key] !== "string" || run.adapter[key].trim() === "") {
      fail(`${label}.adapter.${key} must be a non-empty string`);
    }
  }
  if (!Array.isArray(run.rows) || run.rows.length === 0) {
    fail(`${label}.rows must be a non-empty array`);
  }
  run.rows.forEach((row, rowIndex) => {
    const rowLabel = `${label}.rows[${rowIndex}]`;
    for (const key of ["engine", "dialect", "workload", "unit", "checksum"]) {
      if (typeof row[key] !== "string" || row[key].trim() === "") {
        fail(`${rowLabel}.${key} must be a non-empty string`);
      }
    }
    for (const key of ["n", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
      if (!Number.isFinite(row[key]) || row[key] <= 0) {
        fail(`${rowLabel}.${key} must be a positive finite number`);
      }
    }
  });
}

function validateWasmDispatchRun(run, index) {
  const label = `wasm dispatch runs[${index}]`;
  if (run.schema !== "rustia.bench.wasm-dispatch-results.v1") {
    fail(`${label}.schema must be "rustia.bench.wasm-dispatch-results.v1"`);
  }
  if (run.runtime == null || typeof run.runtime !== "object" || Array.isArray(run.runtime)) {
    fail(`${label}.runtime must be an object`);
  }
  if (run.comparison == null || typeof run.comparison !== "object" || Array.isArray(run.comparison)) {
    fail(`${label}.comparison must be an object`);
  }
  if (run.comparison.status !== "measured") {
    fail(`${label}.comparison.status must be measured`);
  }
  for (const key of ["name", "family"]) {
    if (typeof run.runtime[key] !== "string" || run.runtime[key].trim() === "") {
      fail(`${label}.runtime.${key} must be a non-empty string`);
    }
  }
  if (!Array.isArray(run.rows) || run.rows.length !== 2) {
    fail(`${label}.rows must contain br_table and call_ref`);
  }
  run.rows.forEach((row, rowIndex) => {
    const rowLabel = `${label}.rows[${rowIndex}]`;
    for (const key of ["style", "feature", "checksum"]) {
      if (typeof row[key] !== "string" || row[key].trim() === "") {
        fail(`${rowLabel}.${key} must be a non-empty string`);
      }
    }
    for (const key of ["n", "ns_per_dispatch_min", "ns_per_dispatch_median", "dispatches_per_sec"]) {
      if (!Number.isFinite(row[key]) || row[key] <= 0) {
        fail(`${rowLabel}.${key} must be a positive finite number`);
      }
    }
  });
}

function validateMetadataMatchesRows(data) {
  const expected = {
    targets: unique(data.rows.map((row) => row.target)),
    engines: unique(data.rows.map((row) => row.engine)),
    workloads: unique(data.rows.map((row) => row.workload)),
    commits: unique(data.rows.map((row) => row.commit)),
  };

  for (const [key, values] of Object.entries(expected)) {
    const actual = data.metadata[key].slice().sort((left, right) => left.localeCompare(right));
    if (JSON.stringify(actual) !== JSON.stringify(values)) {
      fail(`metadata.${key} must match rows`);
    }
  }

  const timestamps = data.rows.map((row) => row.timestamp_unix);
  if (
    data.metadata.measurement_window_unix.start !== Math.min(...timestamps) ||
    data.metadata.measurement_window_unix.end !== Math.max(...timestamps)
  ) {
    fail("metadata.measurement_window_unix must match rows");
  }
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function relative(filePath) {
  return path.relative(root, filePath);
}

function unique(values) {
  return [...new Set(values)].sort((left, right) => left.localeCompare(right));
}

function formatNumber(value) {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: value >= 100 ? 0 : value >= 10 ? 1 : 3,
  }).format(value);
}

function formatInteger(value) {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(value);
}

function formatTime(timestampUnix) {
  return new Date(timestampUnix * 1000).toISOString().replace("T", " ").replace(".000Z", " UTC");
}

function formatSpeed(value) {
  return Number.isFinite(value) ? `${formatNumber(value)}x` : "n/a";
}

function runtimeLabel(runtime) {
  return `${runtime.family}/${runtime.name}`;
}

function targetKind(target) {
  const value = String(target);
  if (value.startsWith("wasm32")) {
    return "wasm32";
  }
  for (const candidate of ["aarch64", "arm64", "x86_64", "x86", "arm32", "armv7"]) {
    if (value.includes(candidate)) {
      return candidate;
    }
  }
  const parts = value.split("-").filter(Boolean);
  return parts.length === 0 ? value : parts[parts.length - 1];
}

function slug(value) {
  return String(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function groupBy(values, keyFn) {
  const groups = new Map();
  for (const value of values) {
    const key = keyFn(value);
    const group = groups.get(key);
    if (group) {
      group.push(value);
    } else {
      groups.set(key, [value]);
    }
  }
  return groups;
}

function geometricMean(values) {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  if (finite.length === 0) {
    return null;
  }
  return Math.exp(finite.reduce((sum, value) => sum + Math.log(value), 0) / finite.length);
}

function languageLabel(dialect) {
  const labels = {
    javascript: "JavaScript",
    koto: "Koto",
    lua54: "Lua 5.4",
    luau: "Luau",
    rhai: "Rhai",
    roto: "Roto",
    wasm: "WebAssembly",
  };
  return labels[dialect] ?? String(dialect)
    .split(/[-_]+/)
    .filter(Boolean)
    .map((part) => `${part[0].toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function implementationLabel(engine) {
  if (engine === "mlua-luau" || engine === "mlua-luau-jit" || engine === "mlua-lua54") return "mlua";
  return engine;
}

function modeFamily(engine) {
  return engine === "mlua-luau-jit" ? "mlua-luau" : engine;
}

function executionMode(engine) {
  if (engine === "mlua-luau-jit") return "jit";
  if (engine === "roto" || engine === "wasmtime") return "native";
  return "interpreter";
}

function bestAvailableRows(rows) {
  const priority = { native: 3, jit: 3, "wasm-runtime": 2, interpreter: 1 };
  const selected = new Map();
  for (const row of rows) {
    const key = `${row.target}\0${row.mode_family}\0${row.workload}\0${row.runtime ?? ""}`;
    const existing = selected.get(key);
    const rowPriority = priority[row.execution_mode] ?? 0;
    const existingPriority = priority[existing?.execution_mode] ?? -1;
    if (!existing || rowPriority > existingPriority ||
        (rowPriority === existingPriority && compareRunTime(existing, row) < 0)) {
      selected.set(key, row);
    }
  }
  return [...selected.values()];
}

function compareRows(left, right) {
  return (
    left.target.localeCompare(right.target) ||
    left.workload.localeCompare(right.workload) ||
    left.ns_per_unit_min - right.ns_per_unit_min ||
    left.engine.localeCompare(right.engine)
  );
}

function compareRunTime(left, right) {
  return (
    left.timestamp_unix - right.timestamp_unix ||
    left.commit.localeCompare(right.commit) ||
    left.engine.localeCompare(right.engine) ||
    left.workload.localeCompare(right.workload)
  );
}

function latestBy(values, keyFn) {
  const latest = new Map();
  for (const value of values) {
    const key = keyFn(value);
    const existing = latest.get(key);
    if (!existing || compareRunTime(existing, value) < 0) {
      latest.set(key, value);
    }
  }
  return [...latest.values()];
}

function summarizeLanguageTargets(summaryRows) {
  const commonRows = summaryRows
    .filter((row) => REQUIRED_COMMON_WORKLOAD_SET.has(row.workload))
    .sort(compareRows);

  return [...groupBy(commonRows, (row) => row.target)]
    .map(([target, targetValues]) => {
      const implementationRows = [...groupBy(
        targetValues,
        (row) => `${row.dialect}\0${row.engine}`,
      )]
        .map(([key, values]) => {
          const [dialect, engine] = key.split("\0");
          const measuredRows = latestBy(values, (row) => row.workload)
            .sort((left, right) => left.workload.localeCompare(right.workload));
          const measuredWorkloads = unique(measuredRows.map((row) => row.workload));
          return {
            dialect,
            language: languageLabel(dialect),
            engine,
            mode: values[0].execution_mode,
            nativeVerified: values[0].native_verified,
            nativeVerification: values[0].native_verification,
            nativeVerificationRatio: values[0].native_verification_ratio,
            measuredRows,
            measuredWorkloads,
            coverage: measuredWorkloads.length,
            geoSpeed: null,
            geoTime: null,
            bestCount: null,
            rank: null,
          };
        })
        .sort((left, right) =>
          left.language.localeCompare(right.language) ||
          left.engine.localeCompare(right.engine)
        );
      const rustia = implementationRows.find((row) => row.engine === "rustia") ?? null;
      const baselineComplete = rustia != null &&
        rustia.coverage === REQUIRED_COMMON_WORKLOADS.length &&
        rustia.measuredRows.every((row) => Number.isFinite(row.speed_vs_rustia));
      const complete = implementationRows.length > 1 &&
        baselineComplete &&
        implementationRows.every((row) =>
          row.coverage === REQUIRED_COMMON_WORKLOADS.length &&
          row.measuredRows.every((measurement) => Number.isFinite(measurement.speed_vs_rustia))
        );

      if (complete) {
        for (const row of implementationRows) {
          row.geoSpeed = geometricMean(row.measuredRows.map((measurement) => measurement.speed_vs_rustia));
          row.geoTime = geometricMean(row.measuredRows.map((measurement) => measurement.ns_per_unit_min));
          row.bestCount = 0;
        }
        for (const workload of REQUIRED_COMMON_WORKLOADS) {
          const candidates = implementationRows.map((row) => ({
            row,
            measurement: row.measuredRows.find((measurement) => measurement.workload === workload),
          }));
          const minimum = Math.min(...candidates.map((candidate) => candidate.measurement.ns_per_unit_min));
          for (const candidate of candidates) {
            if (candidate.measurement.ns_per_unit_min === minimum) {
              candidate.row.bestCount += 1;
            }
          }
        }
        implementationRows
          .slice()
          .sort((left, right) =>
            right.geoSpeed - left.geoSpeed ||
            left.language.localeCompare(right.language) ||
            left.engine.localeCompare(right.engine)
          )
          .forEach((row, index) => {
            row.rank = index + 1;
          });
      }

      implementationRows.sort((left, right) =>
        (left.rank ?? Number.POSITIVE_INFINITY) - (right.rank ?? Number.POSITIVE_INFINITY) ||
        left.language.localeCompare(right.language) ||
        left.engine.localeCompare(right.engine)
      );

      const timestamps = targetValues.map((row) => row.timestamp_unix);
      const commits = unique(targetValues.map((row) => row.commit));
      const runtimes = unique(targetValues.map((row) => row.runtime).filter(Boolean));
      return {
        target,
        targetKind: targetValues[0].targetKind,
        runtimes,
        complete,
        status: complete ? "Complete" : "Partial",
        revisionStatus: commits.length === 1 ? "Single revision" : "Mixed revisions",
        commits,
        minTimestamp: Math.min(...timestamps),
        maxTimestamp: Math.max(...timestamps),
        implementationRows,
      };
    })
    .sort((left, right) => left.target.localeCompare(right.target));
}

function summarize(data, wasmRows) {
  const rows = data.rows.slice().sort(compareRows);
  const summaryRows = [
    ...rows.map((row) => ({
      engine: row.engine,
      dialect: row.dialect,
      target: row.target,
      targetKind: targetKind(row.target),
      runtime: null,
      workload: row.workload,
      timestamp_unix: row.timestamp_unix,
      commit: row.commit,
      ns_per_unit_min: row.ns_per_unit_min,
      speed_vs_rustia: row.speed_vs_rustia,
      prepare_ns_median: optionalPositive(row.prepare_ns_median),
      first_call_ns_median: optionalPositive(row.first_call_ns_median),
      cold_start_ns_median: optionalPositive(row.cold_start_ns_median),
      mode_family: row.mode_family ?? modeFamily(row.engine),
      execution_mode: row.execution_mode ?? executionMode(row.engine),
      native_verified: row.native_verified === true,
      native_verification: row.native_verification ?? "not-recorded",
      native_verification_ratio: optionalPositive(row.native_verification_ratio),
    })),
    ...wasmRows.map((row) => ({
      engine: row.adapter,
      dialect: row.dialect,
      target: row.target,
      targetKind: targetKind(row.target),
      runtime: row.runtime,
      workload: row.workload,
      timestamp_unix: row.timestamp_unix,
      commit: row.commit,
      ns_per_unit_min: row.ns_per_unit_min,
      speed_vs_rustia: row.speed_vs_rustia,
      prepare_ns_median: null,
      first_call_ns_median: null,
      cold_start_ns_median: null,
      mode_family: row.adapter,
      execution_mode: "wasm-runtime",
      native_verified: true,
      native_verification: `runtime-${row.runtime}`,
      native_verification_ratio: null,
    })),
  ].sort(compareRows);
  const timestamps = rows.map((row) => row.timestamp_unix);
  const targetRows = [...groupBy(rows, (row) => row.target)]
    .map(([target, values]) => ({ target, values }))
    .sort((left, right) => left.target.localeCompare(right.target));
  const linuxComparisonRows = summaryRows.filter((row) => row.targetKind !== "wasm32");

  return {
    rows,
    summaryRows,
    targetRows,
    linuxComparisonViews: {
      all: summarizeLanguageTargets(linuxComparisonRows),
      interpreter: summarizeLanguageTargets(
        linuxComparisonRows.filter((row) => row.execution_mode === "interpreter"),
      ),
      accelerated: summarizeLanguageTargets(
        linuxComparisonRows.filter((row) =>
          row.execution_mode === "jit" ||
          row.execution_mode === "native" ||
          row.engine === "rustia"
        ),
      ),
    },
    coldStartRows: summarizeColdStarts(summaryRows),
    minTimestamp: Math.min(...timestamps),
    maxTimestamp: Math.max(...timestamps),
  };
}

function summarizeColdStarts(rows) {
  return [...groupBy(
    rows.filter((row) => row.runtime == null && REQUIRED_COMMON_WORKLOAD_SET.has(row.workload) && Number.isFinite(row.cold_start_ns_median)),
    (row) => `${row.target}\0${row.engine}`,
  )].map(([key, values]) => {
    const [target, engine] = key.split("\0");
    const latest = latestBy(values, (row) => row.workload);
    return {
      target,
      engine,
      dialect: latest[0].dialect,
      mode: latest[0].execution_mode,
      prepare: geometricMean(latest.map((row) => row.prepare_ns_median)),
      firstCall: geometricMean(latest.map((row) => row.first_call_ns_median)),
      total: geometricMean(latest.map((row) => row.cold_start_ns_median)),
      coverage: latest.length,
      nativeVerified: latest[0].native_verified,
      nativeVerification: latest[0].native_verification,
      nativeVerificationRatio: latest[0].native_verification_ratio,
    };
  }).sort((left, right) => left.total - right.total || left.engine.localeCompare(right.engine));
}

function summarizeHistory(records) {
  const rows = records.slice().sort(compareRunTime);
  const timestamps = rows.map((row) => row.timestamp_unix);
  const targetRows = [...groupBy(rows, (row) => row.target)]
    .map(([target, values]) => ({ target, values }))
    .sort((left, right) => left.target.localeCompare(right.target));
  return {
    rows,
    targetRows,
    engineCount: unique(rows.map((row) => row.engine)).length,
    workloadCount: unique(rows.map((row) => row.workload)).length,
    commitCount: unique(rows.map((row) => row.commit)).length,
    minTimestamp: Math.min(...timestamps),
    maxTimestamp: Math.max(...timestamps),
  };
}

function metric(value, label) {
  return `
    <div class="metric">
      <strong>${escapeHtml(value)}</strong>
      <span>${escapeHtml(label)}</span>
    </div>
  `;
}

function claimPosture(rustiaRow, luauRow) {
  if (!rustiaRow) {
    return { ready: false, label: "Blocked", text: "missing rustia baseline" };
  }
  if (!luauRow || !Number.isFinite(luauRow.speed_vs_rustia)) {
    return { ready: false, label: "Blocked", text: "missing mlua-luau baseline" };
  }
  if (luauRow.speed_vs_rustia > 1.05) {
    return {
      ready: false,
      label: "Not ready",
      text: `rustia trails mlua-luau by ${formatSpeed(luauRow.speed_vs_rustia)}`,
    };
  }
  if (luauRow.speed_vs_rustia >= 0.95) {
    return {
      ready: true,
      label: "Eligible",
      text: "rustia is near mlua-luau for this workload",
    };
  }
  return {
    ready: true,
    label: "Eligible",
    text: `rustia leads mlua-luau by ${formatSpeed(1 / luauRow.speed_vs_rustia)}`,
  };
}

function summarizeClaimRows(rows) {
  const interpreterRows = rows.filter((row) => (row.execution_mode ?? executionMode(row.engine)) === "interpreter");
  return [...groupBy(interpreterRows, (row) => `${row.target}\0${row.workload}`).entries()]
    .map(([key, values]) => {
      const [target, workload] = key.split("\0");
      const sorted = values
        .slice()
        .sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min || left.engine.localeCompare(right.engine));
      const rustiaRow = values.find((row) => row.engine === "rustia") ?? null;
      const luauRow = values.find((row) => row.engine === "mlua-luau") ?? null;
      return {
        target,
        workload,
        unit: rustiaRow?.unit ?? sorted[0]?.unit ?? "",
        rustiaRow,
        luauRow,
        fastest: sorted[0] ?? null,
        rustiaRank: rustiaRow ? sorted.findIndex((row) => row.engine === "rustia") + 1 : null,
        engineCount: sorted.length,
        posture: claimPosture(rustiaRow, luauRow),
      };
    })
    .sort((left, right) =>
      left.target.localeCompare(right.target) ||
      left.workload.localeCompare(right.workload)
    );
}

function claimGateSection(claimRows) {
  const readyCount = claimRows.filter((row) => row.posture.ready).length;
  const headline = readyCount === claimRows.length
    ? "All current workload claims are eligible, but keep wording per-workload."
    : `${formatInteger(claimRows.length - readyCount)} current workload claims are not public-ready against mlua-luau.`;
  return `
    <section class="panel" id="claim-gate">
      <div class="section-head">
        <div>
          <h2>Claim Gate</h2>
          <p>${formatInteger(readyCount)} / ${formatInteger(claimRows.length)} eligible · ${escapeHtml(headline)}</p>
        </div>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Target / workload</th>
              <th>rustia min</th>
              <th>mlua-luau vs rustia</th>
              <th>rustia rank</th>
              <th>Fastest</th>
              <th>Posture</th>
            </tr>
          </thead>
          <tbody>
            ${claimRows.map(claimGateTableRow).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function claimGateTableRow(row) {
  const rustiaMin = row.rustiaRow ? `${formatNumber(row.rustiaRow.ns_per_unit_min)} ns` : "n/a";
  const luauSpeed = row.luauRow ? formatSpeed(row.luauRow.speed_vs_rustia) : "n/a";
  const rank = row.rustiaRank == null
    ? "n/a"
    : `${formatInteger(row.rustiaRank)} / ${formatInteger(row.engineCount)}`;
  const fastest = row.fastest
    ? `${implementationLabel(row.fastest.engine)} · ${languageLabel(row.fastest.dialect)} · ${formatNumber(row.fastest.ns_per_unit_min)} ns`
    : "n/a";
  const postureClass = row.posture.ready ? "claim-ready" : "claim-blocked";
  return `
    <tr>
      <td>${claimWorkloadLabel(row)}</td>
      <td>${escapeHtml(rustiaMin)}</td>
      <td>${escapeHtml(luauSpeed)}</td>
      <td>${escapeHtml(rank)}</td>
      <td>${escapeHtml(fastest)}</td>
      <td>
        <span class="claim-posture">
          <span class="claim-pill ${postureClass}">${escapeHtml(row.posture.label)}</span>
          <small>${escapeHtml(row.posture.text)}</small>
        </span>
      </td>
    </tr>
  `;
}

function claimWorkloadLabel(row) {
  return `<span class="target-runtime"><strong>${escapeHtml(row.target)}</strong><span>${escapeHtml(row.workload)} · ${escapeHtml(row.unit)}</span></span>`;
}

function historySection(history, cohorts, currentRows, wasmRows) {
  const allBounds = cohorts.flatMap((cohort) => [cohort.lower, cohort.upper, cohort.delta, 0]);
  const axisMin = Math.min(-10, Math.floor((Math.min(...allBounds) - 2) / 10) * 10);
  const axisMax = Math.max(10, Math.ceil((Math.max(...allBounds) + 2) / 10) * 10);
  const axisPosition = (value) => ((value - axisMin) / (axisMax - axisMin)) * 100;
  const zeroPosition = axisPosition(0);
  const ticks = [axisMin, axisMin + (axisMax - axisMin) / 4, axisMin + (axisMax - axisMin) / 2, 0, axisMax]
    .filter((value, index, values) => values.findIndex((candidate) => Math.abs(candidate - value) < 0.001) === index)
    .sort((left, right) => left - right);
  const currentCommit = unique(currentRows.map((row) => row.commit)).join(", ");
  const latestTimestamp = Math.max(...currentRows.map((row) => row.timestamp_unix));
  return `
    <section class="panel" id="history">
      <div class="section-head">
        <div>
          <h2>Benchmark History</h2>
          <p>Controlled deltas stay separate from the full language-matrix baseline.</p>
        </div>
      </div>
      <div class="cohort-chart" role="img" aria-label="Accepted controlled benchmark cohort deltas; lower percentages are faster">
        <div class="cohort-axis" style="--zero: ${zeroPosition.toFixed(3)}%" aria-hidden="true">
          ${ticks.map((tick) => `<span style="left:${axisPosition(tick).toFixed(3)}%">${tick > 0 ? "+" : ""}${formatNumber(tick)}%</span>`).join("")}
        </div>
        ${cohorts.map((cohort) => cohortPlotRow(cohort, axisPosition, zeroPosition)).join("")}
      </div>
      <div class="table-wrap cohort-table-wrap">
        <table class="cohort-table">
          <thead><tr><th>Accepted cohort</th><th>Coverage</th><th>Comparable result</th><th>Status</th></tr></thead>
          <tbody>
            ${cohorts.map(cohortTableRow).join("")}
            <tr>
              <td><span class="implementation-language"><strong>Language matrix</strong><span>${escapeHtml(currentCommit.slice(0, 7))} · ${formatTime(latestTimestamp)}</span></span></td>
              <td>${formatInteger(currentRows.length)} native · ${formatInteger(wasmRows.length)} Wasm rows</td>
              <td>Implementations within one exact target</td>
              <td><span class="cohort-status">current baseline</span></td>
            </tr>
          </tbody>
        </table>
      </div>
      <p class="cohort-note">Lower is faster. Confidence intervals appear only for same-protocol candidate/baseline cohorts; the language matrix remains a baseline until another complete cohort exists. Source <code>${escapeHtml(relative(historyPath))}</code>.</p>
    </section>
  `;
}

function cohortPlotRow(cohort, axisPosition, zeroPosition) {
  const low = axisPosition(cohort.lower);
  const high = axisPosition(cohort.upper);
  const mid = axisPosition(cohort.delta);
  const labelSide = mid < 18 ? "cohort-label-right" : "cohort-label-left";
  return `
    <div class="cohort-plot-row">
      <div class="cohort-plot-name"><strong>${escapeHtml(workloadLabel(cohort.workload))}</strong><span>${escapeHtml(cohort.candidateCommit.slice(0, 7))} · ${escapeHtml(formatIsoDate(cohort.createdAt))}</span></div>
      <div class="cohort-track" style="--zero: ${zeroPosition.toFixed(3)}%">
        <span class="cohort-interval" style="left:${low.toFixed(3)}%;width:${Math.max(0.2, high - low).toFixed(3)}%"></span>
        <span class="cohort-point" style="left:${mid.toFixed(3)}%"></span>
        <span class="cohort-point-label ${labelSide}" style="left:${mid.toFixed(3)}%">${cohort.delta > 0 ? "+" : ""}${cohort.delta.toFixed(2)}%</span>
      </div>
    </div>
  `;
}

function cohortTableRow(cohort) {
  return `
    <tr>
      <td><span class="implementation-language"><strong>${escapeHtml(workloadLabel(cohort.workload))}</strong><span>${escapeHtml(cohort.candidateCommit.slice(0, 7))} · ${escapeHtml(formatIsoDate(cohort.createdAt))}</span></span></td>
      <td>${formatInteger(cohort.legCount)} legs · ${formatInteger(cohort.sampleCount)} samples</td>
      <td>${cohort.delta.toFixed(2)}% · 95% CI ${cohort.lower.toFixed(2)}% to ${cohort.upper.toFixed(2)}%</td>
      <td><span class="cohort-status">published</span></td>
    </tr>
  `;
}

function workloadLabel(workload) {
  const labels = {
    dense_tables: "Dense tables",
    sparse_tables: "Sparse tables",
    closures_upvalues: "Closures",
    generic_iteration: "Iteration",
    stdlib_math_table: "Math / table",
    oop_metamethods: "OOP metamethods",
  };
  return labels[workload] ?? languageLabel(workload);
}

function formatIsoDate(value) {
  return new Date(value).toISOString().slice(0, 10);
}

function historyTargetSection(group) {
  const workloads = [...groupBy(group.values, (row) => row.workload)]
    .map(([workload, rows]) => ({ workload, rows }))
    .sort((left, right) => left.workload.localeCompare(right.workload));
  return `
    <section class="history-target">
      <div class="history-target-head">
        <h3>${escapeHtml(group.target)}</h3>
        <p>${formatInteger(group.values.length)} records across ${formatInteger(workloads.length)} workloads</p>
      </div>
      ${workloads.map((entry) => historyWorkloadSection(group.target, entry.workload, entry.rows)).join("")}
    </section>
  `;
}

function historyWorkloadSection(target, workload, rows) {
  const latestRows = latestBy(rows, (row) => row.engine)
    .sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min || left.engine.localeCompare(right.engine));
  const unit = latestRows[0]?.unit ?? rows[0]?.unit ?? "";
  return `
    <section class="history-workload" id="${escapeHtml(slug(`history-${target}-${workload}`))}">
      <div class="history-workload-head">
        <div>
          <h4>${escapeHtml(workload)}</h4>
          <p>${escapeHtml(unit)} · ${formatInteger(rows.length)} records · ${formatInteger(latestRows.length)} engines</p>
        </div>
      </div>
      ${historyChart(rows)}
      <div class="table-wrap history-table">
        <table>
          <thead>
            <tr>
              <th>Engine</th>
              <th>Latest min ns/unit</th>
              <th>Median</th>
              <th>Trend vs previous</th>
              <th>Commit</th>
              <th>Measured</th>
            </tr>
          </thead>
          <tbody>
            ${latestRows.map((row) => historyLatestRow(row, rows)).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function historyLatestRow(row, rows) {
  const previous = previousHistoryRecord(row, rows);
  const trend = previous ? (row.ns_per_unit_min / previous.ns_per_unit_min - 1) * 100 : null;
  return `
    <tr>
      <td>${historyEngineLabel(row.engine)}</td>
      <td>${formatNumber(row.ns_per_unit_min)}</td>
      <td>${formatNumber(row.ns_per_unit_median)}</td>
      <td>${historyTrend(trend)}</td>
      <td>${escapeHtml(row.commit)}</td>
      <td>${formatTime(row.timestamp_unix)}</td>
    </tr>
  `;
}

function previousHistoryRecord(row, rows) {
  return rows
    .filter((record) =>
      record.engine === row.engine &&
      record.target === row.target &&
      record.workload === row.workload &&
      record.timestamp_unix < row.timestamp_unix
    )
    .sort(compareRunTime)
    .at(-1) ?? null;
}

function historyTrend(value) {
  if (!Number.isFinite(value)) {
    return '<span class="trend trend-flat">n/a</span>';
  }
  const className = value <= 0 ? "trend trend-good" : "trend trend-bad";
  return `<span class="${className}">${value > 0 ? "+" : ""}${value.toFixed(1)}%</span>`;
}

function historyChart(rows) {
  const width = 920;
  const height = 260;
  const pad = { left: 62, right: 20, top: 20, bottom: 42 };
  const values = rows.map((row) => row.ns_per_unit_min).filter((value) => Number.isFinite(value) && value > 0);
  const minValue = Math.min(...values);
  const maxValue = Math.max(...values);
  const minY = Math.log10(minValue * 0.85);
  const maxY = Math.log10(maxValue * 1.15);
  const minX = Math.min(...rows.map((row) => row.timestamp_unix));
  const maxX = Math.max(...rows.map((row) => row.timestamp_unix));
  const plotWidth = width - pad.left - pad.right;
  const plotHeight = height - pad.top - pad.bottom;
  const xScale = (timestamp) => maxX === minX
    ? pad.left + plotWidth / 2
    : pad.left + ((timestamp - minX) / (maxX - minX)) * plotWidth;
  const yScale = (value) => {
    const span = maxY - minY || 1;
    return pad.top + (1 - (Math.log10(value) - minY) / span) * plotHeight;
  };
  const series = [...groupBy(rows, (row) => row.engine)]
    .map(([engine, engineRows], index) => ({
      engine,
      color: engineColor(engine, index),
      rows: engineRows.slice().sort(compareRunTime),
    }))
    .sort((left, right) => left.engine.localeCompare(right.engine));
  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((tick) => {
    const y = pad.top + tick * plotHeight;
    const value = 10 ** (maxY - tick * (maxY - minY));
    return `
      <line x1="${pad.left}" y1="${y.toFixed(2)}" x2="${width - pad.right}" y2="${y.toFixed(2)}" class="history-grid-line" />
      <text x="8" y="${(y + 4).toFixed(2)}" class="history-axis-label">${escapeHtml(formatNumber(value))}</text>
    `;
  }).join("");
  return `
    <div class="history-chart">
      <svg viewBox="0 0 ${width} ${height}" role="img" aria-label="${escapeHtml(rows[0].workload)} timing history">
        <rect x="0" y="0" width="${width}" height="${height}" class="history-chart-bg" />
        ${yTicks}
        <line x1="${pad.left}" y1="${pad.top}" x2="${pad.left}" y2="${height - pad.bottom}" class="history-axis" />
        <line x1="${pad.left}" y1="${height - pad.bottom}" x2="${width - pad.right}" y2="${height - pad.bottom}" class="history-axis" />
        <text x="${pad.left}" y="${height - 10}" class="history-axis-label">${escapeHtml(formatTime(minX))}</text>
        <text x="${width - pad.right}" y="${height - 10}" text-anchor="end" class="history-axis-label">${escapeHtml(formatTime(maxX))}</text>
        ${series.map((item) => historySeriesSvg(item, xScale, yScale)).join("")}
      </svg>
      <div class="history-legend">
        ${series.map((item) => `<span>${colorDotMarkup(item.color)}${escapeHtml(item.engine)}</span>`).join("")}
      </div>
    </div>
  `;
}

function historySeriesSvg(series, xScale, yScale) {
  const points = series.rows
    .map((row) => `${xScale(row.timestamp_unix).toFixed(2)},${yScale(row.ns_per_unit_min).toFixed(2)}`)
    .join(" ");
  const circles = series.rows.map((row) => `
    <circle cx="${xScale(row.timestamp_unix).toFixed(2)}" cy="${yScale(row.ns_per_unit_min).toFixed(2)}" r="3.2" fill="${series.color}">
      <title>${escapeHtml(`${series.engine} ${row.commit}: ${formatNumber(row.ns_per_unit_min)} ns/unit`)}</title>
    </circle>
  `).join("");
  const line = series.rows.length > 1
    ? `<polyline points="${points}" fill="none" stroke="${series.color}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round" />`
    : "";
  return `<g>${line}${circles}</g>`;
}

function historyEngineLabel(engine) {
  return `<span class="history-engine">${colorDotMarkup(engineColor(engine))}<strong>${escapeHtml(engine)}</strong></span>`;
}

function colorDotMarkup(color) {
  return `<span class="swatch" style="color: ${escapeHtml(color)}"></span>`;
}

function engineColor(engine, fallbackIndex = 0) {
  const known = {
    rustia: "#2563eb",
    koto: "#0f766e",
    luaur: "#0891b2",
    "mlua-luau": "#b45309",
    "mlua-luau-jit": "#ea580c",
    "mlua-lua54": "#7c3aed",
    rhai: "#b91c1c",
    boa: "#047857",
    rquickjs: "#be185d",
    roto: "#475569",
    ruau: "#9333ea",
    wasmi: "#be185d",
    wasmtime: "#111827",
  };
  const fallback = ["#2563eb", "#0f766e", "#b45309", "#b91c1c", "#7c3aed", "#047857", "#be185d", "#475569"];
  return known[engine] ?? fallback[fallbackIndex % fallback.length];
}

function languagesByTargetSection(targetRows, {
  id = "languages-by-target",
  title = "Interpreter-only Languages by Target",
  description = "exact targets complete · ranked headline and per-workload detail use the same five common workloads; accelerated modes are excluded",
} = {}) {
  const completeCount = targetRows.filter((row) => row.complete).length;
  return `
    <section class="panel metric-panel" id="${escapeHtml(id)}">
      <div class="section-head">
        <div>
          <h2>${escapeHtml(title)}</h2>
          <p>${formatInteger(completeCount)} / ${formatInteger(targetRows.length)} ${escapeHtml(description)}.</p>
        </div>
        <div class="metric-toggle" role="group" aria-label="Benchmark value display">
          <button type="button" data-metric="relative" aria-pressed="true">vs rustia</button>
          <button type="button" data-metric="absolute" aria-pressed="false">absolute</button>
        </div>
      </div>
      <div class="language-targets">
        ${targetRows.map(languageTargetSummary).join("")}
      </div>
    </section>
  `;
}

function linuxComparisonSection(views) {
  const allRows = views.all ?? [];
  const completeCount = allRows.filter((row) => row.complete).length;
  const renderView = (name, rows, hidden = false) => `
    <div class="comparison-mode-view" data-mode-view="${escapeHtml(name)}"${hidden ? " hidden" : ""}>
      <div class="language-targets">
        ${rows.map(languageTargetSummary).join("")}
      </div>
    </div>
  `;
  return `
    <section class="panel metric-panel" id="linux-comparison">
      <div class="section-head">
        <div>
          <h2>Linux Language Comparison</h2>
          <p>${formatInteger(completeCount)} / ${formatInteger(allRows.length)} exact Linux targets complete · switch between every measured mode, interpreters only, or verified JIT/native modes plus the rustia baseline.</p>
        </div>
        <div class="comparison-controls">
          <div class="metric-toggle mode-toggle" role="group" aria-label="Execution mode filter">
            <button type="button" data-mode-filter="all" aria-pressed="true">All</button>
            <button type="button" data-mode-filter="interpreter" aria-pressed="false">Interpreter</button>
            <button type="button" data-mode-filter="accelerated" aria-pressed="false">JIT/native</button>
          </div>
          <div class="metric-toggle" role="group" aria-label="Benchmark value display">
            <button type="button" data-metric="relative" aria-pressed="true">vs rustia</button>
            <button type="button" data-metric="absolute" aria-pressed="false">absolute</button>
          </div>
        </div>
      </div>
      ${renderView("all", views.all)}
      ${renderView("interpreter", views.interpreter, true)}
      ${renderView("accelerated", views.accelerated, true)}
    </section>
  `;
}

function coldStartSection(rows) {
  return `
    <section class="panel" id="cold-start">
      <div class="section-head">
        <div>
          <h2>Compile and First-call Cost</h2>
          <p>Geometric means across the five common workloads. Preparation and the first invocation are measured on a fresh engine instance; steady-state timings use a separate warmed instance.</p>
        </div>
      </div>
      <div class="table-wrap">
        <table>
          <thead><tr><th>Implementation · language</th><th>Mode</th><th>Prepare</th><th>First call</th><th>Cold total</th><th>Coverage</th><th>Native proof</th></tr></thead>
          <tbody>
            ${rows.map((row) => `
              <tr>
                <td>${implementationLanguage(row.engine, languageLabel(row.dialect))}</td>
                <td>${modeBadge(row.mode)}</td>
                <td>${formatDurationNs(row.prepare)}</td>
                <td>${formatDurationNs(row.firstCall)}</td>
                <td><strong>${formatDurationNs(row.total)}</strong></td>
                <td>${formatInteger(row.coverage)}/${formatInteger(REQUIRED_COMMON_WORKLOADS.length)}</td>
                <td>${nativeProof(row)}</td>
              </tr>
            `).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function formatDurationNs(value) {
  if (!Number.isFinite(value)) return "n/a";
  if (value >= 1e6) return `${formatNumber(value / 1e6)} ms`;
  if (value >= 1e3) return `${formatNumber(value / 1e3)} µs`;
  return `${formatNumber(value)} ns`;
}

function modeBadge(mode) {
  return `<span class="mode-badge mode-${escapeHtml(mode)}">${escapeHtml(mode)}</span>`;
}

function nativeProof(row) {
  if (row.mode === "interpreter") return '<span class="subtle">not applicable</span>';
  if (!row.nativeVerified) return '<span class="claim-pill claim-blocked">unverified</span>';
  const ratio = Number.isFinite(row.nativeVerificationRatio) ? ` · ${formatSpeed(row.nativeVerificationRatio)} A/B` : "";
  return `<span class="claim-pill claim-ready">verified</span><span class="subtle"> ${escapeHtml(row.nativeVerification)}${escapeHtml(ratio)}</span>`;
}

function languageTargetSummary(targetRow) {
  const runtimeDetail = targetRow.runtimes.length === 0
    ? "native"
    : `wasm · ${targetRow.runtimes.join(", ")}`;
  const implementationLabel = targetRow.implementationRows.length === 1 ? "implementation" : "implementations";
  const revisionClass = targetRow.commits.length === 1 ? "summary-revision-single" : "summary-revision-mixed";
  return `
    <section class="language-target-summary" data-target="${escapeHtml(targetRow.target)}">
      <div class="language-target-head">
        <div>
          <h3>${escapeHtml(targetRow.target)}</h3>
          <p>${escapeHtml(runtimeDetail)} · exact target · ${formatInteger(targetRow.implementationRows.length)} ${implementationLabel}</p>
          <p>Measured ${formatTime(targetRow.minTimestamp)} to ${formatTime(targetRow.maxTimestamp)} · commits ${targetRow.commits.map((commit) => `<code>${escapeHtml(commit)}</code>`).join(", ")}</p>
        </div>
        <div class="summary-statuses">
          <span class="summary-pill ${targetRow.complete ? "summary-complete" : "summary-partial"}">${escapeHtml(targetRow.status)}</span>
          <span class="summary-pill ${revisionClass}">${escapeHtml(targetRow.revisionStatus)}</span>
        </div>
      </div>
      <div class="ranked-headline">
        <div class="rank-axis-caption metric-caption" data-relative="geometric mean speed vs rustia · higher is faster" data-absolute="geometric mean ns/unit · lower is faster">geometric mean speed vs rustia · higher is faster</div>
        <div class="ranked-bars" role="list" aria-label="Best-first implementation ranking for ${escapeHtml(targetRow.target)}">
          ${targetRow.implementationRows.map((row) => languageRankRow(row, targetRow)).join("")}
        </div>
      </div>
      <div class="heatmap-detail">
        <div class="heatmap-head">
          <strong>Workload detail</strong>
          <span class="metric-caption" data-relative="speed vs rustia · higher is faster" data-absolute="minimum ns/unit · lower is faster">speed vs rustia · higher is faster</span>
        </div>
        ${languageHeatmap(targetRow)}
        <div class="heat-legend metric-caption" data-relative="Color: red below 0.75× · neutral near rustia · stronger green above 1.25×" data-absolute="Color: strongest for the fastest implementation in each workload">Color: red below 0.75× · neutral near rustia · stronger green above 1.25×</div>
      </div>
    </section>
  `;
}

function languageRankRow(row, targetRow) {
  const rank = row.rank == null ? "–" : formatInteger(row.rank);
  const width = rankBarWidth(row, targetRow.implementationRows);
  const wins = row.bestCount == null ? "n/a" : `${formatInteger(row.bestCount)} wins`;
  const relative = formatSpeed(row.geoSpeed);
  const absolute = row.geoTime == null ? "n/a" : `${formatNumber(row.geoTime)} ns`;
  return `
    <div class="language-rank-row" role="listitem" data-language="${escapeHtml(row.dialect)}" data-implementation="${escapeHtml(row.engine)}">
      <span class="rank-number">${rank}</span>
      ${implementationLanguage(row.engine, row.language, row.mode)}
      <div class="rank-track" aria-hidden="true"><span class="rank-fill metric-bar" data-relative-width="${width.toFixed(3)}" data-absolute-width="${width.toFixed(3)}" style="width:${width.toFixed(3)}%"></span></div>
      <strong class="rank-value metric-value" data-relative="${escapeHtml(relative)}" data-absolute="${escapeHtml(absolute)}">${escapeHtml(relative)}</strong>
      <span class="rank-meta">${escapeHtml(wins)} · ${formatInteger(row.coverage)}/${formatInteger(REQUIRED_COMMON_WORKLOADS.length)}</span>
    </div>
  `;
}

function rankBarWidth(row, rows) {
  if (!Number.isFinite(row.geoTime) || row.geoTime <= 0) return 0;
  const values = rows.map((candidate) => candidate.geoTime).filter((value) => Number.isFinite(value) && value > 0);
  const low = Math.min(...values);
  const high = Math.max(...values);
  if (low === high) return 100;
  return 14 + ((Math.log(high) - Math.log(row.geoTime)) / (Math.log(high) - Math.log(low))) * 86;
}

function implementationLanguage(engine, language, mode = null) {
  if (mode == null) {
    return `<span class="implementation-language"><strong>${escapeHtml(implementationLabel(engine))}</strong><span>${escapeHtml(language)}</span></span>`;
  }
  const modeLabels = {
    interpreter: "Interpreter",
    jit: "JIT",
    native: "Native",
    "wasm-runtime": "Wasm runtime",
  };
  return `<span class="implementation-language comparison-label"><strong>${escapeHtml(implementationLabel(engine))}</strong><span>${escapeHtml(` / ${language} / ${modeLabels[mode] ?? mode}`)}</span></span>`;
}

function languageHeatmap(targetRow) {
  const fastest = new Map(REQUIRED_COMMON_WORKLOADS.map((workload) => {
    const values = targetRow.implementationRows
      .map((row) => row.measuredRows.find((measurement) => measurement.workload === workload)?.ns_per_unit_min)
      .filter((value) => Number.isFinite(value) && value > 0);
    return [workload, values.length === 0 ? null : Math.min(...values)];
  }));
  return `
    <div class="table-wrap">
      <table class="language-heatmap">
        <thead><tr><th>Implementation · language</th>${REQUIRED_COMMON_WORKLOADS.map((workload) => `<th>${escapeHtml(workloadLabel(workload))}</th>`).join("")}</tr></thead>
        <tbody>
          ${targetRow.implementationRows.map((row) => `
            <tr data-language="${escapeHtml(row.dialect)}" data-implementation="${escapeHtml(row.engine)}">
              <th>${implementationLanguage(row.engine, row.language, row.mode)}</th>
              ${REQUIRED_COMMON_WORKLOADS.map((workload) => heatmapCell(row, workload, fastest.get(workload))).join("")}
            </tr>
          `).join("")}
        </tbody>
      </table>
    </div>
  `;
}

function heatmapCell(row, workload, fastest) {
  const measurement = row.measuredRows.find((candidate) => candidate.workload === workload);
  if (!measurement) return '<td class="heat-cell heat-missing">–</td>';
  const relativeClass = relativeHeatClass(measurement.speed_vs_rustia);
  const absoluteScore = fastest / measurement.ns_per_unit_min;
  const absoluteClass = absoluteHeatClass(absoluteScore);
  const baselineClass = row.engine === "rustia" ? " heat-baseline" : "";
  return `<td class="heat-cell ${relativeClass}${baselineClass}" data-heat-relative="${relativeClass}" data-heat-absolute="${absoluteClass}" data-relative="${escapeHtml(formatSpeed(measurement.speed_vs_rustia))}" data-absolute="${escapeHtml(`${formatNumber(measurement.ns_per_unit_min)} ns`)}">${escapeHtml(formatSpeed(measurement.speed_vs_rustia))}</td>`;
}

function relativeHeatClass(value) {
  if (value >= 25) return "heat-5";
  if (value >= 10) return "heat-4";
  if (value >= 3) return "heat-3";
  if (value >= 1.25) return "heat-2";
  if (value >= 0.75) return "heat-1";
  return "heat-0";
}

function absoluteHeatClass(value) {
  if (value >= 0.9) return "heat-5";
  if (value >= 0.5) return "heat-4";
  if (value >= 0.25) return "heat-3";
  if (value >= 0.1) return "heat-2";
  if (value >= 0.04) return "heat-1";
  return "heat-0";
}

function workloadSection(target, workload, rows) {
  const sorted = rows.slice().sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min || left.engine.localeCompare(right.engine));
  const fastest = sorted[0].ns_per_unit_min;
  const slowest = sorted[sorted.length - 1].ns_per_unit_min;
  const maxSpeed = Math.max(...sorted.map((row) => row.speed_vs_rustia ?? 0), 1);
  return `
    <section class="workload" id="${escapeHtml(slug(`${target}-${workload}`))}">
      <div class="workload-head">
        <div>
          <h3>${escapeHtml(workload)}</h3>
          <p>${escapeHtml(sorted[0].unit)} · fastest ${formatNumber(fastest)} ns/unit · slowest ${formatNumber(slowest)} ns/unit</p>
        </div>
      </div>
      <div class="bars">
        ${sorted.map((row) => resultBar(row, fastest, maxSpeed)).join("")}
      </div>
    </section>
  `;
}

function resultBar(row, fastest, maxSpeed) {
  const speedWidth = row.speed_vs_rustia ? Math.max(2, (row.speed_vs_rustia / maxSpeed) * 100) : 0;
  const timeRatio = fastest / row.ns_per_unit_min;
  const timeWidth = Math.max(2, Math.min(100, timeRatio * 100));
  return `
    <article class="result-row">
      <div class="result-label">
        <strong>${escapeHtml(implementationLabel(row.engine))}</strong>
        <span>${escapeHtml(languageLabel(row.dialect))} · ${escapeHtml(row.commit)} · ${formatTime(row.timestamp_unix)}</span>
      </div>
      <div class="result-values">
        <span>${formatNumber(row.ns_per_unit_min)} ns min</span>
        <span>${formatSpeed(row.speed_vs_rustia)} vs rustia</span>
      </div>
      <div class="bars-pair">
        <div class="mini-bar time" title="Relative to fastest row in this workload"><span style="width: ${timeWidth.toFixed(3)}%"></span></div>
        <div class="mini-bar speed" title="Speed relative to rustia"><span style="width: ${speedWidth.toFixed(3)}%"></span></div>
      </div>
    </article>
  `;
}

function targetSection(group) {
  const workloads = [...groupBy(group.values, (row) => row.workload)]
    .map(([workload, rows]) => ({ workload, rows }))
    .sort((left, right) => left.workload.localeCompare(right.workload));
  return `
    <section class="target" id="${escapeHtml(slug(group.target))}">
      <div class="target-head">
        <div>
          <h2>${escapeHtml(group.target)}</h2>
          <p>${formatInteger(group.values.length)} rows across ${formatInteger(workloads.length)} workloads</p>
        </div>
      </div>
      ${workloads.map((entry) => workloadSection(group.target, entry.workload, entry.rows)).join("")}
    </section>
  `;
}

function wasmTargetRows(data) {
  const rows = [];
  for (const run of data.runs) {
    for (const row of run.rows) {
      rows.push({
        target: run.target,
        runtime: runtimeLabel(run.runtime),
        adapter: run.adapter.id,
        dialect: run.adapter.dialect,
        workload: row.workload,
        unit: row.unit,
        ns_per_unit_min: row.ns_per_unit_min,
        ns_per_unit_median: row.ns_per_unit_median,
        commit: run.commit,
        timestamp_unix: run.timestamp_unix,
      });
    }
  }
  const rustiaBaselines = new Map(
    rows
      .filter((row) => row.adapter === "rustia")
      .map((row) => [`${row.target}\0${row.runtime}\0${row.workload}`, row.ns_per_unit_min]),
  );
  return rows
    .map((row) => ({
      ...row,
      speed_vs_rustia: rustiaBaselines.get(`${row.target}\0${row.runtime}\0${row.workload}`) == null
        ? null
        : rustiaBaselines.get(`${row.target}\0${row.runtime}\0${row.workload}`) / row.ns_per_unit_min,
    }))
    .sort((left, right) =>
      left.target.localeCompare(right.target) ||
      left.runtime.localeCompare(right.runtime) ||
      left.workload.localeCompare(right.workload) ||
      left.adapter.localeCompare(right.adapter)
    );
}

function wasmDispatchRows(data) {
  const rows = [];
  for (const run of data.runs) {
    for (const row of run.rows) {
      rows.push({
        target: run.target,
        runtime: runtimeLabel(run.runtime),
        style: row.style,
        feature: row.feature,
        ns_per_dispatch_min: row.ns_per_dispatch_min,
        ns_per_dispatch_median: row.ns_per_dispatch_median,
        commit: run.commit,
      });
    }
  }
  const baselines = new Map(
    rows
      .filter((row) => row.style === "br_table")
      .map((row) => [row.runtime, row.ns_per_dispatch_min]),
  );
  return rows
    .map((row) => ({
      ...row,
      slower_vs_br_table: baselines.get(row.runtime) == null
        ? null
        : row.ns_per_dispatch_min / baselines.get(row.runtime),
    }))
    .sort((left, right) =>
      left.target.localeCompare(right.target) ||
      left.runtime.localeCompare(right.runtime) ||
      left.style.localeCompare(right.style)
    );
}

function wasmTargetSection(data) {
  const rows = wasmTargetRows(data);
  const targets = unique(rows.map((row) => row.target));
  const runtimes = unique(rows.map((row) => row.runtime));
  const adapters = unique(rows.map((row) => row.adapter));
  return `
    <section class="panel" id="wasm-target">
      <div class="section-head">
        <div>
          <h2>Wasm Target Bench</h2>
          <p>${formatInteger(rows.length)} rows · ${formatInteger(adapters.length)} adapters · ${formatInteger(targets.length)} targets · ${formatInteger(runtimes.length)} runtimes · source <code>${escapeHtml(relative(wasmResultsPath))}</code></p>
        </div>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Target / runtime</th>
              <th>Adapter</th>
              <th>Workload</th>
              <th>Min ns/unit</th>
              <th>Median</th>
              <th>vs rustia</th>
              <th>Commit</th>
            </tr>
          </thead>
          <tbody>
            ${rows.map((row) => `
              <tr>
                <td>${targetRuntimeLabel(row)}</td>
                <td><strong>${escapeHtml(implementationLabel(row.adapter))}</strong> <span class="subtle">${escapeHtml(languageLabel(row.dialect))}</span></td>
                <td>${escapeHtml(row.workload)}</td>
                <td>${formatNumber(row.ns_per_unit_min)}</td>
                <td>${formatNumber(row.ns_per_unit_median)}</td>
                <td>${formatSpeed(row.speed_vs_rustia)}</td>
                <td>${escapeHtml(row.commit)}</td>
              </tr>
            `).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function wasmDispatchSection(data) {
  const rows = wasmDispatchRows(data);
  const targets = unique(rows.map((row) => row.target));
  const runtimes = unique(rows.map((row) => row.runtime));
  return `
    <section class="panel" id="wasm-dispatch">
      <div class="section-head">
        <div>
          <h2>Wasm Dispatch Bench</h2>
          <p>${formatInteger(rows.length)} rows · ${formatInteger(targets.length)} targets · ${formatInteger(runtimes.length)} runtimes · source <code>${escapeHtml(relative(wasmDispatchPath))}</code></p>
        </div>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Target / runtime</th>
              <th>Style</th>
              <th>Min ns/dispatch</th>
              <th>Median</th>
              <th>vs br_table</th>
              <th>Commit</th>
            </tr>
          </thead>
          <tbody>
            ${rows.map((row) => `
              <tr>
                <td>${targetRuntimeLabel(row)}</td>
                <td><strong>${escapeHtml(row.style)}</strong> <span class="subtle">${escapeHtml(row.feature)}</span></td>
                <td>${formatNumber(row.ns_per_dispatch_min)}</td>
                <td>${formatNumber(row.ns_per_dispatch_median)}</td>
                <td>${formatSpeed(row.slower_vs_br_table)}</td>
                <td>${escapeHtml(row.commit)}</td>
              </tr>
            `).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function targetRuntimeLabel(row) {
  return `<span class="target-runtime"><strong>${escapeHtml(row.target)}</strong><span>${escapeHtml(row.runtime)}</span></span>`;
}

function render(data, wasmResults, wasmDispatchResults, historyRecords, cohorts) {
  const wasmRows = wasmTargetRows(wasmResults);
  const summary = summarize(data, wasmRows);
  const history = summarizeHistory(historyRecords);
  const claimRows = summarizeClaimRows(summary.rows);
  const claimReadyCount = claimRows.filter((row) => row.posture.ready).length;
  const dispatchRows = wasmDispatchRows(wasmDispatchResults);
  const latestCommit = summary.rows
    .slice()
    .sort((left, right) => right.timestamp_unix - left.timestamp_unix || right.commit.localeCompare(left.commit))[0].commit;

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>rustia Benchmark Results and History</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f5f7fb;
      --panel: #ffffff;
      --panel-soft: #eef3f6;
      --text: #17202a;
      --muted: #5d6875;
      --line: #d8e0e7;
      --accent: #0f766e;
      --accent-2: #2563eb;
      --warn: #b45309;
      --shadow: 0 16px 34px rgba(30, 45, 62, 0.08);
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font: 15px/1.5 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    main {
      width: min(1180px, calc(100% - 32px));
      margin: 0 auto;
      padding: 30px 0 54px;
    }

    h1, h2, h3, p { margin-top: 0; }

    .hero {
      display: grid;
      grid-template-columns: minmax(0, 1.4fr) minmax(260px, 0.8fr);
      gap: 18px;
      margin-bottom: 18px;
    }

    .hero-panel,
    .metric,
    .panel,
    .target {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      box-shadow: var(--shadow);
    }

    .hero-panel {
      padding: 28px;
    }

    .eyebrow {
      margin-bottom: 8px;
      color: var(--accent);
      font-size: 12px;
      font-weight: 800;
      letter-spacing: 0;
      text-transform: uppercase;
    }

    h1 {
      margin-bottom: 12px;
      font-size: 46px;
      line-height: 1.05;
      letter-spacing: 0;
    }

    .lead {
      max-width: 760px;
      margin-bottom: 18px;
      color: var(--muted);
      font-size: 16px;
    }

    .source-line {
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin: 0;
      color: var(--muted);
      font-size: 13px;
    }

    code {
      padding: 2px 6px;
      border-radius: 6px;
      background: var(--panel-soft);
      color: var(--text);
    }

    .metrics {
      display: grid;
      gap: 12px;
    }

    .metric {
      padding: 16px;
    }

    .metric strong {
      display: block;
      margin-bottom: 2px;
      font-size: 25px;
      line-height: 1.1;
    }

    .metric span,
    .section-head p,
    .target-head p,
    .history-target-head p,
    .history-workload-head p,
    .workload-head p,
    .result-label span {
      color: var(--muted);
      font-size: 13px;
    }

    .subtle {
      color: var(--muted);
      font-size: 12px;
      font-weight: 600;
    }

    .language-targets {
      display: grid;
    }

    .language-target-summary {
      border-bottom: 1px solid var(--line);
    }

    .language-target-summary:last-child {
      border-bottom: 0;
    }

    .language-target-head {
      display: flex;
      justify-content: space-between;
      gap: 16px;
      align-items: flex-start;
      padding: 16px 20px;
      border-bottom: 1px solid var(--line);
      background: #fff;
    }

    .language-target-head h3 {
      margin-bottom: 2px;
      font-size: 19px;
    }

    .language-target-head p {
      margin-bottom: 2px;
      color: var(--muted);
      font-size: 12px;
    }

    .language-target-head p:last-child {
      margin-bottom: 0;
    }

    .summary-statuses {
      display: flex;
      flex-wrap: wrap;
      justify-content: flex-end;
      gap: 6px;
    }

    .summary-pill {
      display: inline-flex;
      padding: 3px 8px;
      border-radius: 999px;
      font-size: 12px;
      font-weight: 800;
      white-space: nowrap;
    }

    .summary-complete,
    .summary-revision-single {
      background: #dff7ed;
      color: #047857;
    }

    .summary-partial,
    .summary-revision-mixed {
      background: #fff1d6;
      color: #92400e;
    }

    .metric-toggle {
      display: inline-flex;
      flex: 0 0 auto;
      padding: 3px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: var(--panel);
    }

    .metric-toggle button {
      padding: 6px 11px;
      border: 0;
      border-radius: 999px;
      background: transparent;
      color: var(--muted);
      font: inherit;
      font-size: 13px;
      font-weight: 700;
      cursor: pointer;
    }

    .metric-toggle button[aria-pressed="true"] {
      background: var(--text);
      color: var(--panel);
    }

    .comparison-controls {
      display: flex;
      flex: 0 0 auto;
      flex-wrap: wrap;
      justify-content: flex-end;
      gap: 8px;
    }

    .ranked-headline,
    .heatmap-detail {
      padding: 18px 20px;
    }

    .heatmap-detail {
      border-top: 1px solid var(--line);
      background: var(--panel-soft);
    }

    .rank-axis-caption,
    .heatmap-head,
    .heat-legend {
      color: var(--muted);
      font-size: 12px;
    }

    .rank-axis-caption {
      margin: 0 0 8px min(290px, 35%);
      text-align: right;
    }

    .ranked-bars {
      display: grid;
      gap: 4px;
    }

    .language-rank-row {
      display: grid;
      grid-template-columns: 28px minmax(180px, 240px) minmax(140px, 1fr) 90px 96px;
      gap: 12px;
      align-items: center;
      min-height: 38px;
    }

    .rank-number,
    .rank-meta,
    .implementation-language span,
    .metric-caption {
      color: var(--muted);
      font-size: 12px;
    }

    .rank-number,
    .rank-value,
    .heat-cell {
      font-variant-numeric: tabular-nums;
    }

    .rank-track {
      height: 11px;
      overflow: hidden;
      border-radius: 999px;
      background: #e2e8ef;
    }

    .rank-fill {
      display: block;
      height: 100%;
      border-radius: inherit;
      background: linear-gradient(90deg, var(--accent), var(--accent-2));
      transition: width 180ms ease;
    }

    .rank-value {
      text-align: right;
    }

    .rank-meta {
      text-align: right;
      white-space: nowrap;
    }

    .implementation-language {
      display: grid;
      gap: 1px;
      min-width: 0;
    }

    .implementation-language.comparison-label {
      display: flex;
      gap: 0;
      align-items: baseline;
      white-space: nowrap;
    }

    .implementation-language strong,
    .implementation-language span {
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .mode-badge {
      display: inline-flex;
      width: fit-content;
      padding: 2px 7px;
      border-radius: 999px;
      background: #e2e8f0;
      color: #334155;
      font-size: 10px;
      font-weight: 800;
      letter-spacing: 0.03em;
      text-transform: uppercase;
      white-space: nowrap;
    }

    .mode-jit,
    .mode-native,
    .mode-wasm-runtime {
      background: #ffedd5;
      color: #9a3412;
    }

    .heatmap-head {
      display: flex;
      justify-content: space-between;
      gap: 12px;
      align-items: baseline;
      margin-bottom: 8px;
    }

    .heatmap-head strong {
      color: var(--text);
      font-size: 14px;
    }

    .language-heatmap {
      min-width: 790px;
      table-layout: fixed;
      background: var(--panel);
    }

    .language-heatmap th:first-child {
      width: 190px;
    }

    .language-heatmap th:not(:first-child),
    .language-heatmap td {
      text-align: center;
    }

    .language-heatmap tbody th {
      color: var(--text);
      font-size: 14px;
      text-transform: none;
    }

    .heat-cell {
      transition: background 180ms ease;
    }

    .heat-0 { background: #fee2e2; }
    .heat-1 { background: #f1f5f9; }
    .heat-2 { background: #dff7ed; }
    .heat-3 { background: #b7ead7; }
    .heat-4 { background: #86d8bf; }
    .heat-5 { background: #58c3a3; }

    .heat-baseline {
      box-shadow: inset 0 0 0 1px var(--text);
    }

    .heat-missing {
      color: var(--muted);
      background: var(--panel-soft);
    }

    .heat-legend {
      margin-top: 8px;
    }

    .cohort-chart {
      padding: 20px;
    }

    .cohort-axis {
      position: relative;
      height: 28px;
      margin-left: 170px;
      border-bottom: 1px solid var(--line);
    }

    .cohort-axis::after,
    .cohort-track::after {
      content: "";
      position: absolute;
      top: 0;
      bottom: 0;
      left: var(--zero);
      width: 1px;
      background: var(--text);
      opacity: .6;
    }

    .cohort-axis span {
      position: absolute;
      bottom: 5px;
      transform: translateX(-50%);
      color: var(--muted);
      font-size: 11px;
    }

    .cohort-plot-row {
      display: grid;
      grid-template-columns: 158px minmax(0, 1fr);
      gap: 12px;
      align-items: center;
      min-height: 54px;
    }

    .cohort-plot-name strong,
    .cohort-plot-name span {
      display: block;
    }

    .cohort-plot-name span,
    .cohort-point-label {
      color: var(--muted);
      font-size: 12px;
    }

    .cohort-track {
      position: relative;
      height: 28px;
      background: var(--panel-soft);
    }

    .cohort-interval {
      position: absolute;
      top: 13px;
      height: 3px;
      background: var(--accent-2);
    }

    .cohort-point {
      position: absolute;
      top: 9px;
      width: 11px;
      height: 11px;
      transform: translateX(-50%);
      border: 2px solid var(--panel);
      border-radius: 999px;
      background: var(--accent-2);
    }

    .cohort-point-label {
      position: absolute;
      top: -2px;
      white-space: nowrap;
    }

    .cohort-label-left { transform: translateX(calc(-100% - 9px)); }
    .cohort-label-right { transform: translateX(9px); }

    .cohort-table {
      min-width: 760px;
    }

    .cohort-table-wrap {
      border-top: 1px solid var(--line);
    }

    .cohort-status {
      display: inline-flex;
      align-items: center;
      gap: 6px;
      font-weight: 700;
    }

    .cohort-status::before {
      content: "";
      width: 8px;
      height: 8px;
      border-radius: 999px;
      background: var(--accent);
    }

    .cohort-note {
      margin: 0;
      padding: 12px 20px 16px;
      border-top: 1px solid var(--line);
      color: var(--muted);
      font-size: 12px;
    }

    .target-runtime {
      display: grid;
      gap: 2px;
      min-width: 0;
    }

    .target-runtime strong,
    .target-runtime span {
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .target-runtime span {
      color: var(--muted);
      font-size: 12px;
      font-weight: 600;
    }

    .claim-posture {
      display: grid;
      gap: 4px;
      min-width: 220px;
      white-space: normal;
    }

    .claim-posture small {
      color: var(--muted);
      font-size: 12px;
      font-weight: 600;
      line-height: 1.35;
    }

    .claim-pill {
      display: inline-flex;
      width: fit-content;
      align-items: center;
      padding: 3px 8px;
      border-radius: 999px;
      font-size: 12px;
      font-weight: 800;
    }

    .claim-ready {
      background: #dff7ed;
      color: #047857;
    }

    .claim-blocked {
      background: #fee2e2;
      color: #b91c1c;
    }

    .swatch {
      display: inline-block;
      width: 10px;
      height: 10px;
      margin-right: 8px;
      border-radius: 999px;
      background: currentColor;
      vertical-align: -1px;
    }

    .nav {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin: 0 0 18px;
    }

    .nav a {
      padding: 7px 10px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: var(--panel);
      color: var(--text);
      font-size: 13px;
      font-weight: 700;
      text-decoration: none;
    }

    .nav a:hover {
      border-color: var(--accent);
      color: var(--accent);
    }

    .panel,
    .target {
      margin-bottom: 18px;
      overflow: hidden;
    }

    .section-head,
    .target-head,
    .history-target-head,
    .history-workload-head,
    .workload-head {
      display: flex;
      justify-content: space-between;
      gap: 16px;
      align-items: flex-start;
      padding: 18px 20px;
      border-bottom: 1px solid var(--line);
      background: var(--panel-soft);
    }

    .section-head h2,
    .target-head h2,
    .history-target-head h3,
    .history-workload-head h4,
    .workload-head h3 {
      margin-bottom: 2px;
      font-size: 22px;
    }

    .history-body {
      display: grid;
      min-width: 0;
    }

    .history-target {
      min-width: 0;
      border-bottom: 1px solid var(--line);
    }

    .history-target:last-child {
      border-bottom: 0;
    }

    .history-workload {
      min-width: 0;
      border-top: 1px solid var(--line);
    }

    .history-workload-head {
      background: #fff;
    }

    .history-workload-head h4 {
      font-size: 18px;
    }

    .history-chart {
      padding: 16px 20px 8px;
    }

    .history-chart svg {
      display: block;
      width: 100%;
      height: auto;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: #fff;
    }

    .history-chart-bg {
      fill: #fff;
    }

    .history-axis,
    .history-grid-line {
      stroke: var(--line);
      stroke-width: 1;
    }

    .history-grid-line {
      stroke: #edf1f5;
    }

    .history-axis-label {
      fill: var(--muted);
      font-size: 11px;
      font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    .history-legend {
      display: flex;
      flex-wrap: wrap;
      gap: 10px 16px;
      padding-top: 10px;
      color: var(--muted);
      font-size: 12px;
      font-weight: 700;
    }

    .history-table table {
      min-width: 860px;
    }

    .history-engine {
      display: inline-flex;
      align-items: center;
      gap: 0;
    }

    .trend {
      font-weight: 800;
    }

    .trend-good {
      color: #047857;
    }

    .trend-bad {
      color: #b91c1c;
    }

    .trend-flat {
      color: var(--muted);
    }

    .table-wrap {
      overflow-x: auto;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      min-width: 760px;
    }

    th,
    td {
      padding: 12px 14px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: middle;
      white-space: nowrap;
    }

    th {
      color: var(--muted);
      font-size: 12px;
      text-transform: uppercase;
    }

    tr:last-child td {
      border-bottom: 0;
    }

    .bar-cell {
      display: grid;
      grid-template-columns: 82px minmax(120px, 1fr);
      gap: 12px;
      align-items: center;
    }

    .bar,
    .mini-bar {
      overflow: hidden;
      border-radius: 999px;
      background: #e2e8ef;
    }

    .bar {
      height: 10px;
    }

    .bar span,
    .mini-bar span {
      display: block;
      height: 100%;
      border-radius: inherit;
      background: linear-gradient(90deg, var(--accent), var(--accent-2));
    }

    .workload {
      border-bottom: 1px solid var(--line);
    }

    .workload:last-child {
      border-bottom: 0;
    }

    .workload-head {
      background: #fff;
    }

    .bars {
      display: grid;
    }

    .result-row {
      display: grid;
      grid-template-columns: minmax(180px, 0.9fr) minmax(160px, 0.7fr) minmax(220px, 1.4fr);
      gap: 16px;
      align-items: center;
      padding: 13px 20px;
      border-top: 1px solid var(--line);
    }

    .result-label {
      min-width: 0;
    }

    .result-label strong,
    .result-label span {
      display: block;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .result-values {
      display: grid;
      gap: 2px;
      color: var(--text);
      font-variant-numeric: tabular-nums;
    }

    .result-values span:last-child {
      color: var(--muted);
      font-size: 13px;
    }

    .bars-pair {
      display: grid;
      gap: 6px;
    }

    .mini-bar {
      height: 8px;
    }

    .mini-bar.time span {
      background: linear-gradient(90deg, var(--warn), var(--accent));
    }

    .footnote {
      color: var(--muted);
      font-size: 13px;
    }

    @media (max-width: 780px) {
      main {
        width: min(100% - 20px, 1180px);
        padding-top: 18px;
      }

      .hero {
        grid-template-columns: 1fr;
      }

      h1 {
        font-size: 34px;
      }

      .result-row {
        grid-template-columns: 1fr;
        gap: 8px;
      }

      .language-target-head {
        flex-direction: column;
      }

      #linux-comparison > .section-head {
        flex-direction: column;
      }

      .comparison-controls {
        justify-content: flex-start;
      }

      .language-rank-row {
        grid-template-columns: 24px minmax(120px, 1fr) 68px;
        gap: 8px;
        padding: 4px 0;
      }

      .language-rank-row .rank-track {
        grid-column: 2 / 4;
        grid-row: 2;
      }

      .rank-meta {
        display: none;
      }

      .rank-axis-caption {
        margin-left: 0;
      }

      .cohort-axis {
        margin-left: 0;
      }

      .cohort-plot-row {
        grid-template-columns: 1fr;
        gap: 4px;
        margin-top: 12px;
      }

      .summary-statuses {
        justify-content: flex-start;
      }
    }
  </style>
</head>
<body>
  <main>
    <section class="hero">
      <div class="hero-panel">
        <p class="eyebrow">script-bench current snapshot</p>
        <h1>rustia Benchmark Results and History</h1>
        <p class="lead">Committed Linux results with a best-first language headline, workload-level detail, and protocol-aware benchmark history.</p>
        <p class="source-line">
          <span>Source <code>${escapeHtml(relative(sourcePath))}</code></span>
          <span>History <code>${escapeHtml(relative(historyPath))}</code></span>
          <span>Generated by <code>${escapeHtml(relative(__filename))}</code></span>
        </p>
      </div>
      <div class="metrics">
        ${metric(formatInteger(data.metadata.row_count), "current result rows")}
        ${metric(formatInteger(history.rows.length), "history records")}
        ${metric(formatInteger(data.metadata.engines.length), "engines")}
        ${metric(formatInteger(data.metadata.workloads.length), "workloads")}
        ${metric(`${formatInteger(claimReadyCount)} / ${formatInteger(claimRows.length)}`, "claims ready")}
        ${metric(formatInteger(wasmRows.length), "wasm target rows")}
        ${metric(formatInteger(dispatchRows.length), "wasm dispatch rows")}
        ${metric(escapeHtml(latestCommit), "latest commit in snapshot")}
        ${metric(`${formatTime(summary.minTimestamp)} to ${formatTime(summary.maxTimestamp)}`, "measurement window")}
      </div>
    </section>

    <nav class="nav" aria-label="Targets">
      <a href="#linux-comparison">Linux Comparison</a>
      <a href="#cold-start">Cold Start</a>
      <a href="#claim-gate">Claim Gate</a>
      <a href="#history">History</a>
      <a href="#wasm-target">Wasm Target</a>
      <a href="#wasm-dispatch">Wasm Dispatch</a>
      ${summary.targetRows.map((group) => `<a href="#${escapeHtml(slug(group.target))}">${escapeHtml(group.target)}</a>`).join("")}
    </nav>

    ${linuxComparisonSection(summary.linuxComparisonViews)}
    ${coldStartSection(summary.coldStartRows)}
    ${claimGateSection(claimRows)}
    ${historySection(history, cohorts, summary.rows, wasmRows)}
    ${wasmTargetSection(wasmResults)}
    ${wasmDispatchSection(wasmDispatchResults)}
    ${summary.targetRows.map(targetSection).join("")}

    <p class="footnote">Schema <code>${escapeHtml(data.schema)}</code>. Wasm sources <code>${escapeHtml(relative(wasmResultsPath))}</code> and <code>${escapeHtml(relative(wasmDispatchPath))}</code>. History <code>${escapeHtml(relative(historyPath))}</code> (${escapeHtml(data.metadata.history_schema)}). The report is deterministic, contains no external assets, and uses inline presentation controls. Refresh data with the benchmark harness before rendering when current history changes.</p>
  </main>
  <script>
    (() => {
      const heatClasses = ["heat-0", "heat-1", "heat-2", "heat-3", "heat-4", "heat-5"];
      const setMetric = (root, metric) => {
        root.dataset.metric = metric;
        root.querySelectorAll("[data-metric]").forEach((button) => {
          button.setAttribute("aria-pressed", String(button.dataset.metric === metric));
        });
        root.querySelectorAll(".metric-value, .metric-caption").forEach((element) => {
          element.textContent = element.dataset[metric];
        });
        root.querySelectorAll(".metric-bar").forEach((bar) => {
          bar.style.width = bar.dataset[metric + "Width"] + "%";
        });
        root.querySelectorAll(".heat-cell[data-heat-relative]").forEach((cell) => {
          cell.textContent = cell.dataset[metric];
          cell.classList.remove(...heatClasses);
          cell.classList.add(cell.dataset[metric === "relative" ? "heatRelative" : "heatAbsolute"]);
        });
      };
      document.querySelectorAll(".metric-panel").forEach((root) => {
        root.querySelectorAll("[data-metric]").forEach((button) => {
          button.addEventListener("click", () => setMetric(root, button.dataset.metric));
        });
        setMetric(root, "relative");
      });
      document.querySelectorAll("[data-mode-filter]").forEach((button) => {
        button.addEventListener("click", () => {
          const root = button.closest(".metric-panel");
          const mode = button.dataset.modeFilter;
          root.querySelectorAll("[data-mode-filter]").forEach((candidate) => {
            candidate.setAttribute("aria-pressed", String(candidate.dataset.modeFilter === mode));
          });
          root.querySelectorAll("[data-mode-view]").forEach((view) => {
            view.hidden = view.dataset.modeView !== mode;
          });
        });
      });
    })();
  </script>
</body>
</html>
`;
}

function main() {
  const data = readData();
  const historyRecords = readHistory();
  const wasmResults = readWasmResults();
  const wasmDispatchResults = readWasmDispatchResults();
  const cohorts = readCohortEvidence();
  const html = render(data, wasmResults, wasmDispatchResults, historyRecords, cohorts).replace(/[ \t]+$/gm, "").replace(/\n+$/, "");
  const output = `${html}\n`;
  if (checkOnly) {
    let current;
    try {
      current = fs.readFileSync(outputPath, "utf8");
    } catch (error) {
      fail(`could not read ${relative(outputPath)}: ${error.message}`);
    }
    if (current !== output) {
      fail(`${relative(outputPath)} is stale; run node bench/render-current-results.js`);
    }
    console.log(`${relative(outputPath)} is current`);
  } else {
    fs.writeFileSync(outputPath, output);
    console.log(`wrote ${relative(outputPath)} from ${relative(sourcePath)} and ${relative(historyPath)}`);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  REQUIRED_COMMON_WORKLOADS,
  bestAvailableRows,
  coldStartSection,
  implementationLabel,
  languageLabel,
  linuxComparisonSection,
  languagesByTargetSection,
  summarizeLanguageTargets,
  wasmTargetRows,
};
