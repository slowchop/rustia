#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const siteDir = path.join(root, "bench", "site");
const indexPath = path.join(siteDir, "index.html");
const appPath = path.join(siteDir, "app.js");
const stylePath = path.join(siteDir, "style.css");
const historyPath = path.join(root, "bench", "history", "runs.jsonl");
const currentResultsPath = path.join(root, "bench", "current-results.json");
const wasmGatePath = path.join(root, "bench", "wasm-gate.json");
const wasmResultsPath = path.join(root, "bench", "wasm", "current-results.json");

function fail(message) {
  console.error(`check-benchmark-site: ${message}`);
  process.exit(1);
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function relative(filePath) {
  return slash(path.relative(root, filePath));
}

function read(filePath) {
  try {
    return fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(filePath)}: ${error.message}`);
  }
}

function parseJson(filePath) {
  try {
    return JSON.parse(read(filePath));
  } catch (error) {
    fail(`could not parse ${relative(filePath)}: ${error.message}`);
  }
}

function requireIncludes(text, needle, filePath, description) {
  if (!text.includes(needle)) {
    fail(`${relative(filePath)} must include ${description}: ${JSON.stringify(needle)}`);
  }
}

function assertEqual(actual, expected, description) {
  if (actual !== expected) {
    fail(`${description}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertFinitePositive(value, description) {
  if (!Number.isFinite(value) || value <= 0) {
    fail(`${description} must be finite and positive, got ${JSON.stringify(value)}`);
  }
}

function noExternalAssets(text, filePath) {
  const external = text.match(/\b(?:href|src)=["']https?:\/\//i) || text.match(/@import\s+url\(["']?https?:\/\//i);
  if (external) {
    fail(`${relative(filePath)} must stay dependency-free; found ${JSON.stringify(external[0])}`);
  }
}

function htmlIds(html) {
  return new Set(
    [...html.matchAll(/\bid=(["'])([^"']+)\1/g)].map((match) => match[2]),
  );
}

function queriedIds(js) {
  return [...js.matchAll(/querySelector\(["']#([^"']+)["']\)/g)]
    .map((match) => match[1])
    .sort();
}

function parseHistory(text) {
  const records = [];
  const lines = text.split(/\r?\n/);
  for (const [index, rawLine] of lines.entries()) {
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
    for (const field of ["commit", "target", "engine", "dialect", "workload", "unit", "checksum"]) {
      if (typeof record[field] !== "string" || record[field] === "") {
        fail(`${relative(historyPath)} line ${index + 1} field ${field} must be a non-empty string`);
      }
    }
    for (const field of ["timestamp_unix", "n", "reps", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
      assertFinitePositive(Number(record[field]), `${relative(historyPath)} line ${index + 1} field ${field}`);
    }
    records.push({
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
    });
  }
  if (records.length === 0) {
    fail(`${relative(historyPath)} must contain at least one benchmark record`);
  }
  return records;
}

function compareRunTime(left, right) {
  return (
    left.timestamp_unix - right.timestamp_unix ||
    left.commit.localeCompare(right.commit) ||
    left.engine.localeCompare(right.engine) ||
    left.workload.localeCompare(right.workload)
  );
}

function latestRows(records) {
  const latest = new Map();
  for (const record of records) {
    const key = `${record.target}\0${record.engine}\0${record.workload}`;
    const existing = latest.get(key);
    if (!existing || compareRunTime(existing, record) < 0) {
      latest.set(key, record);
    }
  }
  return [...latest.values()].sort((left, right) =>
    left.target.localeCompare(right.target) ||
    left.engine.localeCompare(right.engine) ||
    left.workload.localeCompare(right.workload)
  );
}

function rowKey(row) {
  return `${row.target}\0${row.engine}\0${row.workload}`;
}

function checkCurrentResultsAgreement(historyRecords, currentResults) {
  const rows = latestRows(historyRecords);
  assertEqual(rows.length, currentResults.metadata.row_count, "latest history row count");
  assertEqual(currentResults.rows.length, currentResults.metadata.row_count, "current-results row count");

  const currentByKey = new Map(currentResults.rows.map((row) => [rowKey(row), row]));
  for (const row of rows) {
    const current = currentByKey.get(rowKey(row));
    if (!current) {
      fail(`bench/current-results.json is missing latest history row ${row.target}/${row.engine}/${row.workload}`);
    }
    for (const field of ["timestamp_unix", "commit", "dialect", "unit", "checksum"]) {
      assertEqual(current[field], row[field], `${row.target}/${row.engine}/${row.workload} ${field}`);
    }
    assertEqual(current.n, row.n, `${row.target}/${row.engine}/${row.workload} n`);
    assertEqual(current.ns_per_unit_min, row.ns_per_unit_min, `${row.target}/${row.engine}/${row.workload} min ns`);
    assertEqual(current.ns_per_unit_median, row.ns_per_unit_median, `${row.target}/${row.engine}/${row.workload} median ns`);
    assertEqual(current.units_per_sec, row.units_per_sec, `${row.target}/${row.engine}/${row.workload} units/sec`);
  }
}

function isObject(value) {
  return value != null && typeof value === "object" && !Array.isArray(value);
}

function isCompleteStatus(status) {
  return ["complete", "measured", "closed"].includes(status);
}

function checkWasmGate(gate) {
  assertEqual(gate.schema_version, 1, "wasm gate schema_version");
  assertEqual(gate.schema, "rustia.bench.wasm-gate.v1", "wasm gate schema");
  assertEqual(gate.source, "script-bench-harness", "wasm gate source");
  if (typeof gate.updated !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(gate.updated)) {
    fail(`${relative(wasmGatePath)} updated must be an ISO date`);
  }
  for (const field of ["scope", "status", "summary"]) {
    if (typeof gate[field] !== "string" || gate[field].trim() === "") {
      fail(`${relative(wasmGatePath)} ${field} must be a non-empty string`);
    }
  }
  if (!Array.isArray(gate.requirements) || gate.requirements.length === 0) {
    fail(`${relative(wasmGatePath)} requirements must be a non-empty array`);
  }

  const requiredIds = new Set([
    "v8-production-rustia-wasm",
    "javascriptcore-production-rustia-wasm",
    "spidermonkey-production-rustia-wasm",
    "call-ref-vs-br-table",
  ]);
  const completeRequired = [];
  const seen = new Set();
  for (const [index, requirement] of gate.requirements.entries()) {
    const label = `${relative(wasmGatePath)} requirements[${index}]`;
    if (!isObject(requirement)) {
      fail(`${label} must be an object`);
    }
    for (const field of ["id", "label", "engine", "runtime", "status", "next"]) {
      if (typeof requirement[field] !== "string" || requirement[field].trim() === "") {
        fail(`${label}.${field} must be a non-empty string`);
      }
    }
    if (requirement.required !== true) {
      fail(`${label}.required must be true for the current pre-lock gate`);
    }
    seen.add(requirement.id);
    if (isCompleteStatus(requirement.status)) {
      completeRequired.push(requirement.id);
    }
    if (!Array.isArray(requirement.evidence)) {
      fail(`${label}.evidence must be an array`);
    }
    for (const [evidenceIndex, evidence] of requirement.evidence.entries()) {
      const evidenceLabel = `${label}.evidence[${evidenceIndex}]`;
      if (!isObject(evidence)) {
        fail(`${evidenceLabel} must be an object`);
      }
      for (const field of ["kind", "summary"]) {
        if (typeof evidence[field] !== "string" || evidence[field].trim() === "") {
          fail(`${evidenceLabel}.${field} must be a non-empty string`);
        }
      }
      if (evidence.path != null && evidence.path !== "") {
        const evidencePath = path.join(root, evidence.path);
        if (!fs.existsSync(evidencePath)) {
          fail(`${evidenceLabel}.path does not exist: ${evidence.path}`);
        }
      }
    }
  }

  for (const id of requiredIds) {
    if (!seen.has(id)) {
      fail(`${relative(wasmGatePath)} is missing requirement ${id}`);
    }
  }
  const expectedComplete = completeRequired.length === gate.requirements.length;
  assertEqual(gate.gate_complete, expectedComplete, "wasm gate gate_complete");
}

function checkWasmResults(results) {
  assertEqual(results.schema_version, 1, "wasm results schema_version");
  assertEqual(results.schema, "rustia.bench.wasm-gate-current-results.v1", "wasm results schema");
  if (!Array.isArray(results.runs) || results.runs.length === 0) {
    fail(`${relative(wasmResultsPath)} runs must be a non-empty array`);
  }
  const adaptersByTarget = new Map();
  for (const [index, run] of results.runs.entries()) {
    const label = `${relative(wasmResultsPath)} runs[${index}]`;
    if (!isObject(run.runtime) || !isObject(run.adapter)) {
      fail(`${label} must include runtime and adapter metadata`);
    }
    for (const field of ["target", "commit"]) {
      if (typeof run[field] !== "string" || run[field].trim() === "") {
        fail(`${label}.${field} must be a non-empty string`);
      }
    }
    for (const field of ["name", "family"]) {
      if (typeof run.runtime[field] !== "string" || run.runtime[field].trim() === "") {
        fail(`${label}.runtime.${field} must be a non-empty string`);
      }
    }
    for (const field of ["id", "dialect"]) {
      if (typeof run.adapter[field] !== "string" || run.adapter[field].trim() === "") {
        fail(`${label}.adapter.${field} must be a non-empty string`);
      }
    }
    if (!["rustia", "koto"].includes(run.adapter.id)) {
      fail(`${label}.adapter.id must be rustia or koto for the wasm target bench`);
    }
    const adapters = adaptersByTarget.get(run.target) ?? new Set();
    adapters.add(run.adapter.id);
    adaptersByTarget.set(run.target, adapters);
    if (!Array.isArray(run.rows) || run.rows.length !== 5) {
      fail(`${label}.rows must contain five default workload rows`);
    }
    for (const [rowIndex, row] of run.rows.entries()) {
      const rowLabel = `${label}.rows[${rowIndex}]`;
      for (const field of ["engine", "dialect", "workload", "unit", "checksum"]) {
        if (typeof row[field] !== "string" || row[field].trim() === "") {
          fail(`${rowLabel}.${field} must be a non-empty string`);
        }
      }
      for (const field of ["n", "reps", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
        assertFinitePositive(Number(row[field]), `${rowLabel}.${field}`);
      }
    }
  }
  for (const [target, adapters] of adaptersByTarget) {
    for (const adapter of ["rustia", "koto"]) {
      if (!adapters.has(adapter)) {
        fail(`${relative(wasmResultsPath)} target ${target} is missing ${adapter}`);
      }
    }
  }
}

const html = read(indexPath);
const js = read(appPath);
const css = read(stylePath);
const historyRecords = parseHistory(read(historyPath));
const currentResults = parseJson(currentResultsPath);
const wasmGate = parseJson(wasmGatePath);
const wasmResults = parseJson(wasmResultsPath);

requireIncludes(html, '<link rel="stylesheet" href="./style.css">', indexPath, "local stylesheet link");
requireIncludes(html, '<script src="./app.js" defer></script>', indexPath, "local app script");
requireIncludes(html, 'href="../history/runs.jsonl"', indexPath, "history download link");
requireIncludes(js, 'fetch("../history/runs.jsonl"', appPath, "relative history fetch");
requireIncludes(js, 'fetch("../wasm-gate.json"', appPath, "relative wasm gate fetch");
requireIncludes(js, 'fetch("../wasm/current-results.json"', appPath, "relative wasm results fetch");
requireIncludes(js, "function parseHistory(text)", appPath, "JSONL parser");
requireIncludes(js, "function drawChart()", appPath, "canvas chart renderer");
requireIncludes(js, "function renderWasmGate()", appPath, "wasm gate renderer");
requireIncludes(js, "function renderWasmResults()", appPath, "wasm results renderer");
noExternalAssets(html, indexPath);
noExternalAssets(css, stylePath);
noExternalAssets(js, appPath);

const ids = htmlIds(html);
for (const id of queriedIds(js)) {
  if (!ids.has(id)) {
    fail(`${relative(appPath)} queries #${id}, but ${relative(indexPath)} does not define it`);
  }
}

checkCurrentResultsAgreement(historyRecords, currentResults);
checkWasmGate(wasmGate);
checkWasmResults(wasmResults);

console.log(
  `benchmark site contract ok: ${historyRecords.length} history records, ${currentResults.metadata.row_count} current rows, ${wasmGate.requirements.length} wasm gate requirements, ${wasmResults.runs.length} wasm adapter runs`,
);
