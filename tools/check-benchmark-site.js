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

const html = read(indexPath);
const js = read(appPath);
const css = read(stylePath);
const historyRecords = parseHistory(read(historyPath));
const currentResults = parseJson(currentResultsPath);

requireIncludes(html, '<link rel="stylesheet" href="./style.css">', indexPath, "local stylesheet link");
requireIncludes(html, '<script src="./app.js" defer></script>', indexPath, "local app script");
requireIncludes(html, 'href="../history/runs.jsonl"', indexPath, "history download link");
requireIncludes(js, 'fetch("../history/runs.jsonl"', appPath, "relative history fetch");
requireIncludes(js, "function parseHistory(text)", appPath, "JSONL parser");
requireIncludes(js, "function drawChart()", appPath, "canvas chart renderer");
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

console.log(
  `benchmark site contract ok: ${historyRecords.length} history records, ${currentResults.metadata.row_count} current rows`,
);
