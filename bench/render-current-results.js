#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = path.join(__dirname, "current-results.json");
const historyPath = path.join(__dirname, "history", "runs.jsonl");
const historyBackfillDirectory = path.join(__dirname, "history", "backfills");
const wasmResultsPath = path.join(__dirname, "wasm", "current-results.json");
const wasmDispatchPath = path.join(__dirname, "wasm", "dispatch-current-results.json");
const cohortEvidencePath = path.join(__dirname, "history", "evidence");
const memoryResultsPath = path.join(__dirname, "memory", "current-results.json");
const memoryHistoryPath = path.join(__dirname, "memory", "history", "runs.jsonl");
const operationsStatusPath = path.join(__dirname, "operations-status.json");
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
const MEMORY_HISTORY_WORKLOADS = Object.freeze([
  ...REQUIRED_COMMON_WORKLOADS,
  "memory_burst",
  "memory_churn",
  "todo_history",
  "todo_steady",
]);

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

function readOperationsStatus(data) {
  let status;
  try {
    status = JSON.parse(fs.readFileSync(operationsStatusPath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(operationsStatusPath)}: ${error.message}`);
  }
  if (status.schema !== "rustia.bench.operations-status.v1" || status.schema_version !== 1) {
    fail(`${relative(operationsStatusPath)} must use rustia.bench.operations-status.v1`);
  }
  if (status.runner?.environment !== "self-hosted-shared" || status.runner?.timing_authority !== false) {
    fail("benchmark operations must keep shared self-hosted runners non-authoritative for timing");
  }
  if (status.measurement?.mode !== "operator-started-qualified-host" ||
      status.measurement?.scheduled_billing_allowed !== false) {
    fail("benchmark operations must require explicit qualified-host measurement without scheduled billing");
  }
  const states = new Set(["current", "pending", "stale", "paused"]);
  for (const lane of ["publication", "progress", "landscape"]) {
    if (!states.has(status[lane]?.state)) fail(`${lane}.state is invalid`);
    if (typeof status[lane]?.next_action !== "string" || status[lane].next_action.trim() === "") {
      fail(`${lane}.next_action must be a non-empty string`);
    }
  }
  const measuredCommit = data.metadata.commits.length === 1 ? data.metadata.commits[0] : null;
  if (status.progress.measured_commit !== measuredCommit || status.landscape.measured_commit !== measuredCommit) {
    fail("benchmark operations measured commits must match the current result cohort");
  }
  if (!/^[0-9a-f]{40}$/.test(status.progress.latest_relevant_commit)) {
    fail("progress.latest_relevant_commit must be an exact commit");
  }
  if (!Number.isSafeInteger(status.progress.pending_relevant_commit_count) ||
      status.progress.pending_relevant_commit_count < 0) {
    fail("progress.pending_relevant_commit_count must be a non-negative integer");
  }
  if ((status.progress.state === "current") !== (status.progress.pending_relevant_commit_count === 0)) {
    fail("progress state and pending relevant commit count disagree");
  }
  return status;
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

function readHistoryBackfills() {
  if (!fs.existsSync(historyBackfillDirectory)) {
    return [];
  }
  const files = fs
    .readdirSync(historyBackfillDirectory)
    .filter((file) => file.endsWith(".json"))
    .sort();
  return files.map((file) => {
    const absolute = path.join(historyBackfillDirectory, file);
    let value;
    try {
      value = JSON.parse(fs.readFileSync(absolute, "utf8"));
    } catch (error) {
      fail(`could not parse ${relative(absolute)}: ${error.message}`);
    }
    if (
      value == null ||
      typeof value !== "object" ||
      Array.isArray(value) ||
      value.schema !== "rustia.bench.history-backfill.v1" ||
      !Array.isArray(value.targets) ||
      !Array.isArray(value.rows)
    ) {
      fail(`${relative(absolute)} is not a fixed-harness history backfill`);
    }
    return value;
  });
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

function readMemoryResults() {
  const data = readJson(memoryResultsPath);
  if (data.schema_version !== 1 || data.schema !== "rustia.bench.memory-current-results.v1") {
    fail("memory results must use rustia.bench.memory-current-results.v1");
  }
  if (!["pending", "ready"].includes(data.status)) {
    fail("memory results status must be pending or ready");
  }
  if (!Array.isArray(data.rows) || !Array.isArray(data.detector_coverage)) {
    fail("memory results rows and detector_coverage must be arrays");
  }
  if (data.status === "pending" && data.rows.length !== 0) {
    fail("pending memory results must not contain rows");
  }
  if (data.status === "ready" && data.rows.length === 0) {
    fail("ready memory results must contain rows");
  }
  data.rows.forEach((row, index) => {
    const label = `memory results row ${index + 1}`;
    for (const key of ["commit", "target", "engine", "dialect", "execution_mode", "workload", "classification"]) {
      if (typeof row[key] !== "string" || row[key] === "") fail(`${label}.${key} must be a string`);
    }
    if (row.metrics == null || typeof row.metrics !== "object" || Array.isArray(row.metrics)) {
      fail(`${label}.metrics must be an object`);
    }
    for (const metric of [
      "spawned_anon_kib",
      "prepared_delta_anon_kib",
      "peak_delta_kib",
      "quiescent_retained_delta_anon_kib",
      "tail_growth_kib_per_epoch",
    ]) {
      if (!Number.isFinite(Number(row.metrics[metric]?.median))) {
        fail(`${label}.metrics.${metric}.median must be finite`);
      }
    }
  });
  return data;
}

function readMemoryHistory() {
  let text;
  try {
    text = fs.readFileSync(memoryHistoryPath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(memoryHistoryPath)}: ${error.message}`);
  }
  const records = [];
  for (const [index, rawLine] of text.split(/\r?\n/).entries()) {
    const line = rawLine.trim();
    if (line === "") continue;
    let record;
    try {
      record = JSON.parse(line);
    } catch (error) {
      fail(`${relative(memoryHistoryPath)} line ${index + 1} is not JSON: ${error.message}`);
    }
    const label = `${relative(memoryHistoryPath)} line ${index + 1}`;
    if (record.schema_version !== 1 || record.schema !== "rustia.bench.memory-cohort.v1") {
      fail(`${label} must use rustia.bench.memory-cohort.v1`);
    }
    if (typeof record.commit !== "string" || record.commit === "") {
      fail(`${label}.commit must be a string`);
    }
    if (!Number.isFinite(Date.parse(record.created_at))) {
      fail(`${label}.created_at must be an ISO timestamp`);
    }
    if (!Array.isArray(record.rows) || record.rows.length === 0) {
      fail(`${label}.rows must be a non-empty array`);
    }
    records.push(record);
  }
  if (records.length === 0) {
    fail(`${relative(memoryHistoryPath)} must contain at least one memory cohort`);
  }
  return records.sort(
    (left, right) => Date.parse(left.created_at) - Date.parse(right.created_at),
  );
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
    baselineCommit: data.source?.baseline_commit,
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

function historySection(history, backfills, currentRows) {
  const reference = mluaJitHistoryReference(currentRows);
  const reconstructed = rustiaBackfillEras(backfills, reference);
  const recorded = rustiaHistoryEras(history.rows, reference);
  const visibleRecorded = reconstructed.length > 0
    ? recorded.filter((era) => era.title === "Current dedicated Linux history")
    : recorded;
  const eras = [...reconstructed, ...visibleRecorded];
  const snapshotCount = eras.reduce((total, era) => total + era.snapshotCount, 0);
  const summary = reconstructed.length > 0
    ? `${formatInteger(reconstructed.reduce((total, era) => total + era.snapshotCount, 0))} historical Rustia revisions reconstructed with a single fixed harness, followed by the directly recorded dedicated-VM series.`
    : `${formatInteger(snapshotCount)} recorded Linux matrix runs across two eras. Each chart is normalized to its own first run, so different hosts and protocols are never blended.`;
  const explanation = reconstructed.length > 0
    ? `The Rustia score is the geometric mean across all five workloads. The 1× reference is <strong>mlua / Luau / JIT</strong> from the current dedicated-VM matrix (<code>${escapeHtml(reference.commit.slice(0, 7))}</code>); the reconstructed Rustia measurements used a separate VM of the same class.`
    : `The Rustia score is the geometric mean across all five workloads. The 1× reference is <strong>mlua / Luau / JIT</strong> from the current dedicated-VM matrix (<code>${escapeHtml(reference.commit.slice(0, 7))}</code>).`;
  return `
    <section class="panel history-axis-panel" id="history">
      <div class="section-head">
        <div>
          <h2>rustia Performance History</h2>
          <p>${summary}</p>
        </div>
        <div class="metric-toggle" role="group" aria-label="History x-axis">
          <button type="button" data-history-axis="date" aria-pressed="true">Date</button>
          <button type="button" data-history-axis="commit" aria-pressed="false">Commit</button>
        </div>
      </div>
      <p class="history-answer"><strong>Higher is faster; mlua/JIT is 1×.</strong> ${explanation}</p>
      <p class="cohort-note">The two Rustia lines share the same workloads and score, but remain separate because reconstruction and directly recorded matrix runs use different measurement protocols.</p>
      <div data-history-axis-view="date">${rustiaHistoryChart(eras, reference, "date")}</div>
      <div data-history-axis-view="commit" hidden>${rustiaHistoryChart(eras, reference, "commit")}</div>
      ${rustiaOverallHistoryTable(eras)}
      <p class="cohort-note">${reconstructed.length > 0 ? `Reconstruction measurement time is retained honestly in <code>${escapeHtml(relative(historyBackfillDirectory))}</code>; the chart's x-axis uses each source commit's original date.` : `Restored from published Linux matrix snapshots that were previously overwritten in <code>${escapeHtml(relative(historyPath))}</code>. Future accepted matrix runs now append to this ledger.`}</p>
    </section>
  `;
}

function mluaJitHistoryReference(rows) {
  const referenceRows = rows.filter(
    (row) =>
      row.target === "linux-x86_64" &&
      row.engine === "mlua-luau-jit" &&
      REQUIRED_COMMON_WORKLOAD_SET.has(row.workload),
  );
  const byWorkload = new Map(referenceRows.map((row) => [row.workload, row]));
  for (const workload of REQUIRED_COMMON_WORKLOADS) {
    if (!byWorkload.has(workload)) {
      fail(`history reference has no mlua-luau-jit linux-x86_64 result for ${workload}`);
    }
  }
  return {
    commit: referenceRows[0].commit,
    byWorkload,
  };
}

function rustiaBackfillEras(backfills, reference) {
  return backfills.map((backfill) => {
    const targets = new Map(
      backfill.targets.map((target, index) => [
        target.commit,
        { committedAt: target.committed_at_unix, index },
      ]),
    );
    const rows = backfill.rows
      .filter(
        (row) =>
          row.target === "linux-x86_64" &&
          row.engine === "rustia" &&
          REQUIRED_COMMON_WORKLOAD_SET.has(row.workload) &&
          targets.has(row.commit),
      )
      .map((row) => ({
        ...row,
        timestamp_unix: targets.get(row.commit).committedAt,
        backfill_order: targets.get(row.commit).index,
      }));
    return rustiaHistoryEra(
      "Fixed-harness reconstructed history",
      `Harness ${backfill.harness_commit.slice(0, 7)} · measured together on one dedicated Linux VM`,
      rows,
      reference,
    );
  });
}

function rustiaHistoryEras(rows, reference) {
  const rustiaRows = rows
    .filter((row) => row.target === "linux-x86_64" && row.engine === "rustia" && REQUIRED_COMMON_WORKLOAD_SET.has(row.workload))
    .slice()
    .sort(compareRunTime);
  const timestamps = unique(rustiaRows.map((row) => String(row.timestamp_unix))).map(Number).sort((left, right) => left - right);
  const gaps = [];
  for (let index = 1; index < timestamps.length; index += 1) {
    gaps.push({ gap: timestamps[index] - timestamps[index - 1], splitAt: timestamps[index] });
  }
  gaps.sort((left, right) => right.gap - left.gap);
  for (const { gap, splitAt } of gaps) {
    if (gap < 24 * 60 * 60) break;
    const current = rustiaRows.filter((row) => row.timestamp_unix >= splitAt);
    // A single-workload acceptance append (for example a dense-only fixed
    // run) must not become its own era: every displayed era needs full
    // required-workload coverage, so fall through to the next widest gap.
    if (!REQUIRED_COMMON_WORKLOADS.every((workload) => current.some((row) => row.workload === workload))) {
      continue;
    }
    return [
      rustiaHistoryEra("Earlier Linux history", "Earlier host/protocol era; kept on its own scale", rustiaRows.filter((row) => row.timestamp_unix < splitAt), reference),
      rustiaHistoryEra("Current dedicated Linux history", "Controlled matrix era; this is the chart to use for current progress", current, reference),
    ];
  }
  return [rustiaHistoryEra("Comparable Linux history", "All recorded Linux runs", rustiaRows, reference)];
}

function rustiaHistoryEra(title, description, rows, reference) {
  const workloads = rustiaHistoryWorkloads(rows);
  const timestamps = workloads.flatMap((workload) => workload.rows.map((row) => row.timestamp_unix));
  return {
    title,
    description,
    workloads,
    overall: rustiaOverallHistory(workloads, reference),
    reference,
    snapshotCount: Math.max(...workloads.map((workload) => workload.rows.length)),
    minTimestamp: Math.min(...timestamps),
    maxTimestamp: Math.max(...timestamps),
  };
}

function rustiaHistoryWorkloads(rows) {
  return REQUIRED_COMMON_WORKLOADS.map((workload) => {
    const workloadRows = rows
      .filter((row) => row.target === "linux-x86_64" && row.engine === "rustia" && row.workload === workload)
      .slice()
      .sort(compareRunTime);
    if (workloadRows.length === 0) fail(`history has no rustia linux-x86_64 records for ${workload}`);
    const baseline = workloadRows[0].ns_per_unit_min;
    return {
      workload,
      rows: workloadRows.map((row) => ({ ...row, speed: baseline / row.ns_per_unit_min })),
    };
  });
}

function rustiaOverallHistory(workloads, reference) {
  const rows = workloads
    .flatMap((workload) => workload.rows)
    .slice()
    .sort(compareRunTime);
  const snapshots = [];
  let active = null;
  for (const row of rows) {
    const snapshotKey = Number.isInteger(row.backfill_order) ? `backfill:${row.backfill_order}` : null;
    const needsSnapshot =
      active == null ||
      (snapshotKey != null && active.key !== snapshotKey) ||
      (snapshotKey == null && (
        active.key != null ||
        row.timestamp_unix - active.timestamp_unix > 120 ||
        active.ratios.has(row.workload)
      ));
    if (needsSnapshot) {
      active = {
        key: snapshotKey,
        commit: row.commit,
        timestamp_unix: row.timestamp_unix,
        ratios: new Map(),
      };
      snapshots.push(active);
    }
    active.ratios.set(
      row.workload,
      reference.byWorkload.get(row.workload).ns_per_unit_min / row.ns_per_unit_min,
    );
  }
  const points = snapshots
    .flatMap((snapshot) => {
      const ratios = REQUIRED_COMMON_WORKLOADS.map((workload) => snapshot.ratios.get(workload));
      if (ratios.some((ratio) => !Number.isFinite(ratio) || ratio <= 0)) {
        return [];
      }
      return [{
        commit: snapshot.commit,
        timestamp_unix: snapshot.timestamp_unix,
        score: geometricMean(ratios),
      }];
    })
    .sort(compareRunTime);
  if (points.length === 0) {
    fail("history has no complete five-workload snapshots");
  }
  const baseline = points[0].score;
  return points.map((point) => ({
    ...point,
    gain: point.score / baseline,
  }));
}

function rustiaHistoryCommitOrder(eras) {
  const commits = [];
  for (const era of eras) {
    const sequence = [...new Set(era.overall.map((point) => point.commit))];
    for (let index = 0; index < sequence.length; index += 1) {
      const commit = sequence[index];
      if (commits.includes(commit)) continue;
      const next = sequence.slice(index + 1).find((candidate) => commits.includes(candidate));
      if (next == null) {
        commits.push(commit);
      } else {
        commits.splice(commits.indexOf(next), 0, commit);
      }
    }
  }
  return commits;
}

function rustiaHistoryChart(eras, reference, axisMode) {
  const width = 920;
  const height = 300;
  const pad = { left: 54, right: 20, top: 22, bottom: 42 };
  const allPoints = eras.flatMap((era) => era.overall);
  const commitOrder = rustiaHistoryCommitOrder(eras);
  const commitIndex = new Map(commitOrder.map((commit, index) => [commit, index]));
  const minX = axisMode === "commit" ? 0 : Math.min(...allPoints.map((point) => point.timestamp_unix));
  const maxX = axisMode === "commit" ? Math.max(1, commitOrder.length - 1) : Math.max(...allPoints.map((point) => point.timestamp_unix));
  const maxScore = Math.max(1, ...allPoints.map((point) => point.score));
  const maxY = Math.max(1.05, Math.ceil(maxScore * 1.08 * 4) / 4);
  const plotWidth = width - pad.left - pad.right;
  const plotHeight = height - pad.top - pad.bottom;
  const xValue = (point) => axisMode === "commit" ? commitIndex.get(point.commit) : point.timestamp_unix;
  const xScale = (point) => maxX === minX
    ? pad.left + plotWidth / 2
    : pad.left + ((xValue(point) - minX) / (maxX - minX)) * plotWidth;
  const yScale = (score) => pad.top + (1 - score / maxY) * plotHeight;
  const ticks = [...new Set([0, 0.25, 0.5, 0.75, 1, maxY])]
    .filter((tick) => tick <= maxY)
    .sort((left, right) => left - right);
  const referenceColor = "#ea580c";
  const styles = [
    { color: "#2563eb", label: "Reconstructed", dash: "" },
    { color: "#0f766e", label: "Recorded dedicated", dash: "5 4" },
  ];
  const firstLabel = axisMode === "commit" ? commitOrder[0].slice(0, 7) : formatIsoDate(minX);
  const lastLabel = axisMode === "commit" ? commitOrder.at(-1).slice(0, 7) : formatIsoDate(maxX);
  return `
    <div class="history-chart">
      <svg viewBox="0 0 ${width} ${height}" role="img" aria-label="rustia overall Linux performance history by ${escapeHtml(axisMode)} versus mlua JIT, higher is faster">
        <rect x="0" y="0" width="${width}" height="${height}" class="history-chart-bg" />
        ${ticks.map((tick) => `<line x1="${pad.left}" y1="${yScale(tick).toFixed(2)}" x2="${width - pad.right}" y2="${yScale(tick).toFixed(2)}" class="history-grid-line" /><text x="8" y="${(yScale(tick) + 4).toFixed(2)}" class="history-axis-label">${formatSpeed(tick)}</text>`).join("")}
        <line x1="${pad.left}" y1="${pad.top}" x2="${pad.left}" y2="${height - pad.bottom}" class="history-axis" />
        <line x1="${pad.left}" y1="${height - pad.bottom}" x2="${width - pad.right}" y2="${height - pad.bottom}" class="history-axis" />
        <text x="${pad.left}" y="${height - 10}" class="history-axis-label">${escapeHtml(firstLabel)}</text>
        <text x="${width - pad.right}" y="${height - 10}" text-anchor="end" class="history-axis-label">${escapeHtml(lastLabel)}</text>
        <line x1="${pad.left}" y1="${yScale(1).toFixed(2)}" x2="${width - pad.right}" y2="${yScale(1).toFixed(2)}" stroke="${referenceColor}" stroke-width="1.4" stroke-dasharray="7 6"><title>${escapeHtml(`mlua / Luau / JIT · current matrix ${reference.commit.slice(0, 7)} · 1×`)}</title></line>
        ${eras.map((era, index) => {
          const style = styles[index] ?? { color: engineColor("rustia", index), label: era.title, dash: "" };
          const points = era.overall.map((point) => `${xScale(point).toFixed(2)},${yScale(point.score).toFixed(2)}`).join(" ");
          const dash = style.dash === "" ? "" : ` stroke-dasharray="${style.dash}"`;
          return `<g><polyline points="${points}" fill="none" stroke="${style.color}" stroke-width="1.6"${dash} stroke-linecap="round" stroke-linejoin="round" />${era.overall.map((point) => `<circle cx="${xScale(point).toFixed(2)}" cy="${yScale(point.score).toFixed(2)}" r="2.2" fill="${style.color}"><title>${escapeHtml(`${style.label} · ${point.commit.slice(0, 7)} · ${formatSpeed(point.score)} mlua/JIT · ${formatSpeed(point.gain)} vs first in era`)}</title></circle>`).join("")}</g>`;
        }).join("")}
      </svg>
      <div class="history-legend">${eras.map((era, index) => {
        const style = styles[index] ?? { color: engineColor("rustia", index), label: era.title };
        return `<span>${colorDotMarkup(style.color)}rustia · ${escapeHtml(style.label)}</span>`;
      }).join("")}<span>${colorDotMarkup(referenceColor)}<strong>mlua</strong> / Luau / JIT · 1×</span></div>
    </div>
  `;
}

function rustiaOverallHistoryTable(eras) {
  const rows = eras.map((era) => {
    const first = era.overall[0];
    const latest = era.overall.at(-1);
    const percent = (latest.gain - 1) * 100;
    const gap = 1 / latest.score;
    return `<tr><td><strong>${escapeHtml(era.title === "Fixed-harness reconstructed history" ? "Reconstructed" : "Recorded dedicated")}</strong><br><small>${formatInteger(era.snapshotCount)} complete scores · geometric mean of 5 workloads</small></td><td>${formatSpeed(first.score)} of mlua/JIT · <code>${escapeHtml(first.commit.slice(0, 7))}</code></td><td>${formatSpeed(latest.score)} of mlua/JIT · <code>${escapeHtml(latest.commit.slice(0, 7))}</code></td><td><strong>${formatSpeed(latest.gain)}</strong> · ${percent >= 0 ? "+" : ""}${formatNumber(percent)}%</td><td>mlua/JIT is <strong>${formatSpeed(gap)}</strong> faster</td></tr>`;
  }).join("");
  return `<div class="table-wrap history-table"><table><thead><tr><th>Series</th><th>First run</th><th>Latest run</th><th>Improvement</th><th>Latest gap</th></tr></thead><tbody>${rows}</tbody></table></div>`;
}

function memoryHistoryModel(records) {
  const completeRows = (record, engine) => {
    const byWorkload = new Map(
      record.rows
        .filter((row) => row.target === "linux-x86_64" && row.engine === engine)
        .map((row) => [row.workload, row]),
    );
    const rows = MEMORY_HISTORY_WORKLOADS.map((workload) => byWorkload.get(workload));
    return rows.every(Boolean) ? rows : null;
  };
  const referenceRecord = records.find((record) => completeRows(record, "mlua-luau-jit") != null);
  if (referenceRecord == null) {
    fail("memory history has no complete mlua-luau-jit reference cohort");
  }
  const referenceRows = completeRows(referenceRecord, "mlua-luau-jit");
  const metricKeys = {
    peak: "peak_delta_kib",
    retained: "quiescent_retained_delta_anon_kib",
  };
  const points = records.flatMap((record) => {
    const rustiaRows = completeRows(record, "rustia");
    if (rustiaRows == null) return [];
    const scores = Object.fromEntries(
      Object.entries(metricKeys).map(([metric, key]) => {
        const ratios = rustiaRows.map((row, index) => {
          const rustia = Number(row.metrics?.[key]?.median);
          const reference = Number(referenceRows[index].metrics?.[key]?.median);
          if (!Number.isFinite(rustia) || rustia < 0 || !Number.isFinite(reference) || reference < 0) {
            fail(`memory history ${record.cohort_id ?? record.commit} has invalid ${key}`);
          }
          return Math.max(1, reference) / Math.max(1, rustia);
        });
        return [metric, geometricMean(ratios)];
      }),
    );
    return [{
      commit: record.commit,
      cohort: record.cohort_id,
      timestamp_unix: Date.parse(record.created_at) / 1000,
      ...scores,
    }];
  });
  if (points.length === 0) {
    fail("memory history has no complete Rustia cohorts");
  }
  return {
    points,
    reference: {
      commit: referenceRecord.commit,
      cohort: referenceRecord.cohort_id,
    },
  };
}

function memoryHistorySection(records) {
  const model = memoryHistoryModel(records);
  const latest = model.points.at(-1);
  return `
    <section class="panel history-axis-panel memory-history-panel" id="memory-history">
      <div class="section-head">
        <div>
          <h2>rustia Memory History</h2>
          <p>${formatInteger(model.points.length)} complete dedicated-Linux Rustia cohorts across ${formatInteger(MEMORY_HISTORY_WORKLOADS.length)} non-control workloads.</p>
        </div>
        <div class="comparison-controls">
          <div class="metric-toggle" role="group" aria-label="Memory history metric">
            <button type="button" data-memory-history-metric="peak" aria-pressed="true">Peak</button>
            <button type="button" data-memory-history-metric="retained" aria-pressed="false">Retained</button>
          </div>
          <div class="metric-toggle" role="group" aria-label="Memory history x-axis">
            <button type="button" data-history-axis="date" aria-pressed="true">Date</button>
            <button type="button" data-history-axis="commit" aria-pressed="false">Commit</button>
          </div>
        </div>
      </div>
      <p class="history-answer"><strong>Higher is better; mlua/JIT is 1×.</strong> Each score is the geometric mean of mlua/Luau/JIT memory divided by Rustia memory per workload. Values use a 1 KiB measurement floor; intentional-retention controls are excluded.</p>
      <div data-history-axis-view="date">
        <div data-memory-history-metric-view="peak">${memoryHistoryChart(model, "peak", "date")}</div>
        <div data-memory-history-metric-view="retained" hidden>${memoryHistoryChart(model, "retained", "date")}</div>
      </div>
      <div data-history-axis-view="commit" hidden>
        <div data-memory-history-metric-view="peak">${memoryHistoryChart(model, "peak", "commit")}</div>
        <div data-memory-history-metric-view="retained" hidden>${memoryHistoryChart(model, "retained", "commit")}</div>
      </div>
      ${memoryHistoryTable(model)}
      <p class="cohort-note">The mlua/JIT reference is fixed from cross-engine cohort <code>${escapeHtml(model.reference.cohort)}</code> at <code>${escapeHtml(model.reference.commit.slice(0, 7))}</code>. Rustia points are separate fresh-VM cohorts; the latest is <code>${escapeHtml(latest.commit.slice(0, 7))}</code>.</p>
    </section>
  `;
}

function memoryHistoryChart(model, metric, axisMode) {
  const width = 920;
  const height = 300;
  const pad = { left: 54, right: 20, top: 22, bottom: 42 };
  const points = model.points;
  const minX = axisMode === "commit" ? 0 : Math.min(...points.map((point) => point.timestamp_unix));
  const maxX = axisMode === "commit"
    ? Math.max(1, points.length - 1)
    : Math.max(...points.map((point) => point.timestamp_unix));
  const maxScore = Math.max(1, ...points.map((point) => point[metric]));
  const maxY = Math.max(1.25, Math.ceil(maxScore * 1.08 * 4) / 4);
  const tickStep = maxY <= 2 ? 0.25 : maxY <= 5 ? 1 : Math.ceil(maxY / 5);
  const ticks = [];
  for (let tick = 0; tick <= maxY + Number.EPSILON; tick += tickStep) {
    ticks.push(Number(tick.toFixed(6)));
  }
  if (!ticks.includes(1)) ticks.push(1);
  if (!ticks.includes(maxY)) ticks.push(maxY);
  ticks.sort((left, right) => left - right);
  const plotWidth = width - pad.left - pad.right;
  const plotHeight = height - pad.top - pad.bottom;
  const xValue = (point, index) => axisMode === "commit" ? index : point.timestamp_unix;
  const xScale = (point, index) => maxX === minX
    ? pad.left + plotWidth / 2
    : pad.left + ((xValue(point, index) - minX) / (maxX - minX)) * plotWidth;
  const yScale = (score) => pad.top + (1 - score / maxY) * plotHeight;
  const firstLabel = axisMode === "commit"
    ? points[0].commit.slice(0, 7)
    : formatIsoDate(minX);
  const lastLabel = axisMode === "commit"
    ? points.at(-1).commit.slice(0, 7)
    : formatIsoDate(maxX);
  const rustiaColor = "#0f766e";
  const referenceColor = "#ea580c";
  const metricLabel = metric === "peak" ? "peak" : "retained";
  const polyline = points
    .map((point, index) => `${xScale(point, index).toFixed(2)},${yScale(point[metric]).toFixed(2)}`)
    .join(" ");
  return `
    <div class="history-chart">
      <svg viewBox="0 0 ${width} ${height}" role="img" aria-label="rustia ${escapeHtml(metricLabel)} memory efficiency history by ${escapeHtml(axisMode)} versus mlua JIT, higher is better">
        <rect x="0" y="0" width="${width}" height="${height}" class="history-chart-bg" />
        ${ticks.map((tick) => `<line x1="${pad.left}" y1="${yScale(tick).toFixed(2)}" x2="${width - pad.right}" y2="${yScale(tick).toFixed(2)}" class="history-grid-line" /><text x="8" y="${(yScale(tick) + 4).toFixed(2)}" class="history-axis-label">${formatSpeed(tick)}</text>`).join("")}
        <line x1="${pad.left}" y1="${pad.top}" x2="${pad.left}" y2="${height - pad.bottom}" class="history-axis" />
        <line x1="${pad.left}" y1="${height - pad.bottom}" x2="${width - pad.right}" y2="${height - pad.bottom}" class="history-axis" />
        <text x="${pad.left}" y="${height - 10}" class="history-axis-label">${escapeHtml(firstLabel)}</text>
        <text x="${width - pad.right}" y="${height - 10}" text-anchor="end" class="history-axis-label">${escapeHtml(lastLabel)}</text>
        <line x1="${pad.left}" y1="${yScale(1).toFixed(2)}" x2="${width - pad.right}" y2="${yScale(1).toFixed(2)}" stroke="${referenceColor}" stroke-width="1.4" stroke-dasharray="7 6"><title>${escapeHtml(`mlua / Luau / JIT · ${metricLabel} reference · 1×`)}</title></line>
        <polyline points="${polyline}" fill="none" stroke="${rustiaColor}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
        ${points.map((point, index) => `<circle cx="${xScale(point, index).toFixed(2)}" cy="${yScale(point[metric]).toFixed(2)}" r="2.2" fill="${rustiaColor}"><title>${escapeHtml(`rustia · ${point.commit.slice(0, 7)} · ${formatSpeed(point[metric])} mlua/JIT ${metricLabel} efficiency`)}</title></circle>`).join("")}
      </svg>
      <div class="history-legend"><span>${colorDotMarkup(rustiaColor)}<strong>rustia</strong> · ${escapeHtml(metricLabel)} memory efficiency</span><span>${colorDotMarkup(referenceColor)}<strong>mlua</strong> / Luau / JIT · 1×</span></div>
    </div>
  `;
}

function memoryHistoryTable(model) {
  const labels = {
    peak: "Peak allocation",
    retained: "Retained after quiescence",
  };
  const rows = Object.entries(labels).map(([metric, label]) => {
    const first = model.points[0][metric];
    const latest = model.points.at(-1)[metric];
    const improvement = latest / first;
    const relativePercent = (1 / latest - 1) * 100;
    const comparison = Math.abs(relativePercent) < 0.05
      ? "matches mlua/JIT"
      : `Rustia uses <strong>${formatNumber(Math.abs(relativePercent))}% ${relativePercent < 0 ? "less" : "more"}</strong>`;
    return `<tr><td><strong>${escapeHtml(label)}</strong></td><td>${formatSpeed(first)} of mlua/JIT · <code>${escapeHtml(model.points[0].commit.slice(0, 7))}</code></td><td>${formatSpeed(latest)} of mlua/JIT · <code>${escapeHtml(model.points.at(-1).commit.slice(0, 7))}</code></td><td><strong>${formatSpeed(improvement)}</strong></td><td>${comparison}</td></tr>`;
  }).join("");
  return `<div class="table-wrap history-table"><table><thead><tr><th>Metric</th><th>First run</th><th>Latest run</th><th>Efficiency gain</th><th>Latest comparison</th></tr></thead><tbody>${rows}</tbody></table></div>`;
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
  const date = typeof value === "number" ? new Date(value * 1000) : new Date(value);
  return date.toISOString().slice(0, 10);
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

function memoryResultsSection(data) {
  if (data.status === "pending") {
    return `
      <section class="panel" id="memory">
        <div class="section-head">
          <div>
            <h2>Memory</h2>
            <p>Fresh-process cgroup peaks, smaps phase deltas, and epoch growth across engines.</p>
          </div>
        </div>
        <p class="pending-note"><strong>Pre-reclamation baseline pending.</strong> The supervisor and detector-validation workloads are being implemented; this panel will remain empty rather than showing local or Rustia-only measurements.</p>
      </section>
    `;
  }
  const rows = [...data.rows].sort(
    (left, right) =>
      left.workload.localeCompare(right.workload) ||
      Number(left.metrics.peak_delta_kib.median) - Number(right.metrics.peak_delta_kib.median) ||
      left.engine.localeCompare(right.engine),
  );
  const coverage = new Map(data.detector_coverage.map((entry) => [entry.engine, entry.detector_validated === true]));
  return `
    <section class="panel" id="memory">
      <div class="section-head">
        <div>
          <h2>Linux Memory Comparison</h2>
          <p>Best-first charts use fresh process+cgroup samples. Peak catches short spikes; retained uses anonymous memory after quiescence.</p>
        </div>
      </div>
      <div class="memory-cohorts">
        ${memoryComparisonCharts(rows)}
      </div>
      <div class="table-wrap">
        <table>
          <thead><tr><th>Implementation · language</th><th>Workload</th><th>Mode</th><th>Idle anon</th><th>Prepared Δ</th><th>Peak Δ</th><th>Retained Δ</th><th>Tail growth</th><th>Signal</th></tr></thead>
          <tbody>
            ${rows.map((row) => `
              <tr>
                <td>${implementationLanguage(row.engine, languageLabel(row.dialect))}</td>
                <td><strong>${escapeHtml(workloadLabel(row.workload))}</strong></td>
                <td>${modeBadge(row.execution_mode)}</td>
                <td>${formatMemoryKib(row.metrics.spawned_anon_kib.median)}</td>
                <td>${formatSignedMemoryKib(row.metrics.prepared_delta_anon_kib.median)}</td>
                <td><strong>${formatMemoryKib(row.metrics.peak_delta_kib.median)}</strong></td>
                <td>${formatSignedMemoryKib(row.metrics.quiescent_retained_delta_anon_kib.median)}</td>
                <td>${formatSignedMemoryKib(row.metrics.tail_growth_kib_per_epoch.median)}/epoch</td>
                <td>${memoryClassification(row.classification, coverage.get(row.engine) === true)}</td>
              </tr>
            `).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function memoryComparisonCharts(rows) {
  const cohorts = [
    {
      title: "Shared benchmark workloads",
      description: "the same five workloads used by the Linux speed comparison",
      workloads: REQUIRED_COMMON_WORKLOADS,
    },
    {
      title: "Allocation stress",
      description: "bounded burst and churn probes; the intentional-retention control is excluded",
      workloads: ["memory_burst", "memory_churn"],
    },
    {
      title: "Stateful todo application",
      description: "bounded steady-state CRUD and fixed-size history; the intentional-retention control is excluded",
      workloads: ["todo_steady", "todo_history"],
    },
  ];
  return cohorts.map((cohort) => memoryCohortCharts(rows, cohort)).join("");
}

function memoryCohortCharts(rows, cohort) {
  const implementations = [...groupBy(
    rows.filter((row) => cohort.workloads.includes(row.workload)),
    (row) => row.engine,
  )]
    .map(([engine, values]) => {
      const measurements = cohort.workloads
        .map((workload) => values.find((row) => row.workload === workload))
        .filter(Boolean);
      if (measurements.length !== cohort.workloads.length) return null;
      return {
        engine,
        language: languageLabel(measurements[0].dialect),
        mode: measurements[0].execution_mode,
        coverage: measurements.length,
        peak: geometricMean(
          measurements.map((row) => Number(row.metrics.peak_delta_kib.median)),
        ),
        retained: measurements.reduce(
          (sum, row) => sum + Number(row.metrics.quiescent_retained_delta_anon_kib.median),
          0,
        ) / measurements.length,
      };
    })
    .filter(Boolean);
  return `
    <section class="memory-cohort-summary">
      <div class="language-target-head">
        <div>
          <h3>${escapeHtml(cohort.title)}</h3>
          <p>${escapeHtml(cohort.description)} · ${formatInteger(implementations.length)} complete implementations</p>
        </div>
      </div>
      <div class="memory-chart-grid">
        ${memoryMetricChart(
          implementations,
          "peak",
          "Peak allocation",
          "geometric mean peak Δ · lower is better",
        )}
        ${memoryMetricChart(
          implementations,
          "retained",
          "Retained after quiescence",
          "arithmetic mean retained Δ · lower is better",
        )}
      </div>
    </section>
  `;
}

function memoryMetricChart(rows, metric, title, caption) {
  const ranked = [...rows].sort(
    (left, right) =>
      left[metric] - right[metric] ||
      left.language.localeCompare(right.language) ||
      left.engine.localeCompare(right.engine),
  );
  const values = ranked.map((row) => row[metric]);
  return `
    <div class="memory-ranked-headline">
      <div class="memory-chart-head">
        <strong>${escapeHtml(title)}</strong>
        <span>${escapeHtml(caption)}</span>
      </div>
      <div class="ranked-bars" role="list" aria-label="${escapeHtml(`${title}, ${caption}`)}">
        ${ranked.map((row, index) => `
          <div class="memory-rank-row" role="listitem">
            <span class="rank-number">${formatInteger(index + 1)}</span>
            ${implementationLanguage(row.engine, row.language, row.mode)}
            <div class="rank-track" aria-hidden="true"><span class="rank-fill" style="width:${memoryRankBarWidth(row[metric], values).toFixed(3)}%"></span></div>
            <strong class="rank-value">${escapeHtml(formatMemoryKib(row[metric]))}</strong>
          </div>
        `).join("")}
      </div>
    </div>
  `;
}

function memoryRankBarWidth(value, values) {
  if (!Number.isFinite(value) || value < 0) return 0;
  const finite = values.filter((candidate) => Number.isFinite(candidate) && candidate >= 0);
  if (finite.length === 0) return 0;
  const low = Math.log1p(Math.min(...finite));
  const high = Math.log1p(Math.max(...finite));
  if (low === high) return 100;
  return 14 + ((high - Math.log1p(value)) / (high - low)) * 86;
}

function formatMemoryKib(value) {
  value = Number(value);
  if (!Number.isFinite(value)) return "n/a";
  if (Math.abs(value) >= 1024) return `${formatNumber(value / 1024)} MiB`;
  return `${formatNumber(value)} KiB`;
}

function formatSignedMemoryKib(value) {
  value = Number(value);
  if (!Number.isFinite(value)) return "n/a";
  return `${value > 0 ? "+" : ""}${formatMemoryKib(value)}`;
}

function memoryClassification(classification, detectorValidated) {
  const labels = {
    expected_retention: "expected retention",
    growth_signal: "growth signal",
    stable_in_tested_window: "stable",
    inconclusive: "inconclusive",
    detector_failed: "detector failed",
  };
  const ready = classification === "stable_in_tested_window" || classification === "expected_retention";
  const detector = detectorValidated ? "" : ' <span class="subtle">control unverified</span>';
  return `<span class="claim-pill ${ready ? "claim-ready" : "claim-blocked"}">${escapeHtml(labels[classification] ?? classification)}</span>${detector}`;
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
  return `<span class="implementation-language comparison-label"><strong>${escapeHtml(implementationLabel(engine))}</strong><span>${escapeHtml(`${language} / ${modeLabels[mode] ?? mode}`)}</span></span>`;
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

function operationsStateLabel(state) {
  return {
    current: "Current",
    pending: "Pending",
    stale: "Stale",
    paused: "Paused",
  }[state] ?? state;
}

function operationsStatusSection(status) {
  const progress = status.progress;
  const landscape = status.landscape;
  const publication = status.publication;
  const pendingLabel = progress.pending_relevant_commit_count === 1 ? "commit" : "commits";
  return `
    <section class="panel operations-panel" id="operations-status">
      <div class="operations-banner state-${escapeHtml(publication.state)}">
        <div>
          <p class="eyebrow">Benchmark operations</p>
          <h2>Automatic publication is ${escapeHtml(publication.state)}</h2>
          <p>${escapeHtml(publication.reason)}</p>
        </div>
        <span class="operations-state state-${escapeHtml(publication.state)}">${escapeHtml(operationsStateLabel(publication.state))}</span>
      </div>
      <div class="operations-grid">
        <article class="operations-card">
          <div class="operations-card-head">
            <h3>Rustia progress</h3>
            <span class="operations-state state-${escapeHtml(progress.state)}">${escapeHtml(operationsStateLabel(progress.state))}</span>
          </div>
          <p>Measured ${formatTime(progress.measured_at_unix)} at <code>${escapeHtml(progress.measured_commit.slice(0, 7))}</code>.</p>
          <p><strong>${formatInteger(progress.pending_relevant_commit_count)} relevant ${pendingLabel} waiting</strong>; newest <code>${escapeHtml(progress.latest_relevant_commit.slice(0, 7))}</code>.</p>
          <p class="operations-next">Next: ${escapeHtml(progress.next_action)}</p>
        </article>
        <article class="operations-card">
          <div class="operations-card-head">
            <h3>Current engine landscape</h3>
            <span class="operations-state state-${escapeHtml(landscape.state)}">${escapeHtml(operationsStateLabel(landscape.state))}</span>
          </div>
          <p>Last full cohort measured ${formatTime(landscape.measured_at_unix)} at <code>${escapeHtml(landscape.measured_commit.slice(0, 7))}</code>.</p>
          <p>${escapeHtml(landscape.reason)}</p>
          <p class="operations-next">Next: ${escapeHtml(landscape.next_action)}</p>
        </article>
        <article class="operations-card">
          <div class="operations-card-head">
            <h3>Runner and publication policy</h3>
            <span class="operations-state state-${escapeHtml(publication.state)}">${escapeHtml(operationsStateLabel(publication.state))}</span>
          </div>
          <p>Shared self-hosted runners may build, test correctness, validate evidence, detect updates, and publish accepted artifacts.</p>
          <p><strong>They are not timing authorities.</strong> Fresh measurements require an explicit qualified-host lifecycle; scheduled billing is disabled.</p>
          <p class="operations-next">Next: ${escapeHtml(publication.next_action)}</p>
        </article>
      </div>
    </section>
  `;
}

function render(data, operationsStatus, wasmResults, wasmDispatchResults, memoryResults, memoryHistory, historyRecords, cohorts, historyBackfills = []) {
  const wasmRows = wasmTargetRows(wasmResults);
  const summary = summarize(data, wasmRows);
  const history = summarizeHistory(historyRecords);
  const claimRows = summarizeClaimRows(summary.rows);
  const dispatchRows = wasmDispatchRows(wasmDispatchResults);

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
      margin-bottom: 18px;
    }

    .hero-panel,
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

    .operations-banner {
      display: flex;
      justify-content: space-between;
      gap: 18px;
      align-items: flex-start;
      padding: 20px;
      border-bottom: 1px solid var(--line);
      background: #fff7ed;
    }

    .operations-banner h2 {
      margin-bottom: 6px;
      font-size: 25px;
    }

    .operations-banner p:last-child {
      max-width: 780px;
      margin-bottom: 0;
      color: var(--muted);
    }

    .operations-grid {
      display: grid;
      grid-template-columns: repeat(3, minmax(0, 1fr));
    }

    .operations-card {
      min-width: 0;
      padding: 18px 20px;
      border-right: 1px solid var(--line);
    }

    .operations-card:last-child {
      border-right: 0;
    }

    .operations-card-head {
      display: flex;
      justify-content: space-between;
      gap: 10px;
      align-items: flex-start;
      margin-bottom: 12px;
    }

    .operations-card h3 {
      margin-bottom: 0;
      font-size: 17px;
    }

    .operations-card p {
      margin-bottom: 9px;
      color: var(--muted);
      font-size: 13px;
    }

    .operations-card .operations-next {
      margin-bottom: 0;
      color: var(--text);
    }

    .operations-state {
      display: inline-flex;
      flex: 0 0 auto;
      padding: 4px 9px;
      border-radius: 999px;
      font-size: 12px;
      font-weight: 800;
      line-height: 1.2;
    }

    .state-current {
      background: #dff7ed;
      color: #047857;
    }

    .state-pending,
    .state-stale,
    .state-paused {
      background: #fff1d6;
      color: #92400e;
    }

    code {
      padding: 2px 6px;
      border-radius: 6px;
      background: var(--panel-soft);
      color: var(--text);
    }

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

    .pending-note {
      margin: 0;
      padding: 18px 20px;
      border-top: 1px solid var(--line);
      color: var(--muted);
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

    .memory-cohorts {
      display: grid;
      gap: 18px;
      padding: 0 20px 20px;
    }

    .memory-cohort-summary {
      overflow: hidden;
      border: 1px solid var(--line);
      border-radius: 12px;
      background: var(--panel);
    }

    .memory-chart-grid {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      border-top: 1px solid var(--line);
    }

    .memory-ranked-headline {
      min-width: 0;
      padding: 16px 18px 18px;
    }

    .memory-ranked-headline + .memory-ranked-headline {
      border-left: 1px solid var(--line);
    }

    .memory-chart-head {
      display: grid;
      gap: 2px;
      margin-bottom: 10px;
    }

    .memory-chart-head strong {
      font-size: 14px;
    }

    .memory-chart-head span {
      color: var(--muted);
      font-size: 12px;
    }

    .memory-rank-row {
      display: grid;
      grid-template-columns: 24px minmax(135px, 180px) minmax(80px, 1fr) 78px;
      gap: 9px;
      align-items: center;
      min-height: 36px;
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
      gap: 0.35em;
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

    .history-answer {
      margin: 0;
      padding: 16px 20px;
      border-bottom: 1px solid var(--line);
      background: #fff;
      color: var(--muted);
    }

    .history-answer strong {
      color: var(--text);
    }

    .history-era + .history-era {
      border-top: 1px solid var(--line);
    }

    .history-era-head {
      padding: 16px 20px;
      background: var(--panel-soft);
    }

    .history-era-head h3 {
      margin-bottom: 2px;
      font-size: 18px;
    }

    .history-era-head p {
      margin: 0;
      color: var(--muted);
      font-size: 13px;
    }

    .cohort-chart-heading {
      margin: 0 0 12px 170px;
      color: var(--muted);
      font-size: 12px;
      font-weight: 700;
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
      .operations-grid {
        grid-template-columns: 1fr;
      }

      .operations-card {
        border-right: 0;
        border-bottom: 1px solid var(--line);
      }

      .operations-card:last-child {
        border-bottom: 0;
      }

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

      .memory-chart-grid {
        grid-template-columns: 1fr;
      }

      .memory-ranked-headline + .memory-ranked-headline {
        border-top: 1px solid var(--line);
        border-left: 0;
      }

      .memory-rank-row {
        grid-template-columns: 22px minmax(110px, 150px) minmax(60px, 1fr) 72px;
        gap: 7px;
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

      .cohort-chart-heading {
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
    </section>

    <nav class="nav" aria-label="Targets">
      <a href="#operations-status">Operations Status</a>
      <a href="#linux-comparison">Linux Comparison</a>
      <a href="#memory">Memory</a>
      <a href="#memory-history">Memory History</a>
      <a href="#cold-start">Cold Start</a>
      <a href="#claim-gate">Claim Gate</a>
      <a href="#history">History</a>
    </nav>

    ${operationsStatusSection(operationsStatus)}
    ${linuxComparisonSection(summary.linuxComparisonViews)}
    ${memoryResultsSection(memoryResults)}
    ${memoryHistorySection(memoryHistory)}
    ${coldStartSection(summary.coldStartRows)}
    ${claimGateSection(claimRows)}
      ${historySection(history, historyBackfills, data.rows)}

    <p class="footnote">Schema <code>${escapeHtml(data.schema)}</code>. Operations <code>${escapeHtml(relative(operationsStatusPath))}</code>. Memory <code>${escapeHtml(relative(memoryResultsPath))}</code> with history <code>${escapeHtml(relative(memoryHistoryPath))}</code>. Wasm sources <code>${escapeHtml(relative(wasmResultsPath))}</code> and <code>${escapeHtml(relative(wasmDispatchPath))}</code>. Performance history <code>${escapeHtml(relative(historyPath))}</code> (${escapeHtml(data.metadata.history_schema)}). The report is deterministic, contains no external assets, and uses inline presentation controls. Refresh data with the benchmark harness before rendering when current history changes.</p>
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
      document.querySelectorAll(".history-axis-panel").forEach((root) => {
        const setHistoryAxis = (axis) => {
          root.querySelectorAll("[data-history-axis]").forEach((button) => {
            button.setAttribute("aria-pressed", String(button.dataset.historyAxis === axis));
          });
          root.querySelectorAll("[data-history-axis-view]").forEach((view) => {
            view.hidden = view.dataset.historyAxisView !== axis;
          });
        };
        root.querySelectorAll("[data-history-axis]").forEach((button) => {
          button.addEventListener("click", () => setHistoryAxis(button.dataset.historyAxis));
        });
        setHistoryAxis("date");
      });
      document.querySelectorAll(".memory-history-panel").forEach((root) => {
        const setMemoryHistoryMetric = (metric) => {
          root.querySelectorAll("[data-memory-history-metric]").forEach((button) => {
            button.setAttribute("aria-pressed", String(button.dataset.memoryHistoryMetric === metric));
          });
          root.querySelectorAll("[data-memory-history-metric-view]").forEach((view) => {
            view.hidden = view.dataset.memoryHistoryMetricView !== metric;
          });
        };
        root.querySelectorAll("[data-memory-history-metric]").forEach((button) => {
          button.addEventListener("click", () => setMemoryHistoryMetric(button.dataset.memoryHistoryMetric));
        });
        setMemoryHistoryMetric("peak");
      });
    })();
  </script>
</body>
</html>
`;
}

function main() {
  const data = readData();
  const operationsStatus = readOperationsStatus(data);
  const historyRecords = readHistory();
  const historyBackfills = readHistoryBackfills();
  const wasmResults = readWasmResults();
  const wasmDispatchResults = readWasmDispatchResults();
  const memoryResults = readMemoryResults();
  const memoryHistory = readMemoryHistory();
  const cohorts = readCohortEvidence();
  const html = render(data, operationsStatus, wasmResults, wasmDispatchResults, memoryResults, memoryHistory, historyRecords, cohorts, historyBackfills).replace(/[ \t]+$/gm, "").replace(/\n+$/, "");
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
  memoryComparisonCharts,
  memoryHistoryModel,
  memoryHistorySection,
  operationsStatusSection,
  summarizeLanguageTargets,
  wasmTargetRows,
};
