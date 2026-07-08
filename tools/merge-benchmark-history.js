#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = resolveArg("--source");
const destinationPath = resolveArg("--destination");

function fail(message) {
  console.error(`merge-benchmark-history: ${message}`);
  process.exit(1);
}

function argValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) {
    return null;
  }
  const value = process.argv[index + 1];
  if (value == null || value.startsWith("--")) {
    fail(`${name} requires a path`);
  }
  return value;
}

function resolveArg(name) {
  const value = argValue(name);
  if (value == null) {
    fail(`${name} is required`);
  }
  return path.isAbsolute(value) ? value : path.resolve(root, value);
}

function readRows(filePath) {
  const text = fs.existsSync(filePath) ? fs.readFileSync(filePath, "utf8") : "";
  return text
    .split(/\n/)
    .filter((line) => line.trim() !== "")
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        fail(`${filePath} line ${index + 1}: ${error.message}`);
      }
    });
}

function rowKey(row) {
  return [
    row.timestamp_unix,
    row.commit,
    row.target,
    row.engine,
    row.dialect,
    row.workload,
    row.n,
    row.reps,
    row.ns_per_unit_min,
    row.ns_per_unit_median,
    row.checksum,
  ].join("\0");
}

const destinationRows = readRows(destinationPath);
const sourceRows = readRows(sourcePath);
const seen = new Set(destinationRows.map(rowKey));
const latestTimestamp = destinationRows.reduce(
  (max, row) => Math.max(max, Number(row.timestamp_unix) || 0),
  0,
);

const additions = [];
for (const row of sourceRows) {
  const timestamp = Number(row.timestamp_unix) || 0;
  const key = rowKey(row);
  if (timestamp < latestTimestamp || seen.has(key)) {
    continue;
  }
  seen.add(key);
  additions.push(row);
}

additions.sort((lhs, rhs) => {
  const timestamp = lhs.timestamp_unix - rhs.timestamp_unix;
  if (timestamp !== 0) {
    return timestamp;
  }
  return sourceRows.indexOf(lhs) - sourceRows.indexOf(rhs);
});

if (additions.length === 0) {
  console.log("merge-benchmark-history: no new rows");
  process.exit(0);
}

fs.mkdirSync(path.dirname(destinationPath), { recursive: true });
const existing = fs.existsSync(destinationPath) ? fs.readFileSync(destinationPath, "utf8") : "";
const prefix = existing.length === 0 || existing.endsWith("\n") ? existing : `${existing}\n`;
const appended = additions.map((row) => JSON.stringify(row)).join("\n");
fs.writeFileSync(destinationPath, `${prefix}${appended}\n`);

const commits = [...new Set(additions.map((row) => row.commit))].join(", ");
console.log(`merge-benchmark-history: appended ${additions.length} rows for ${commits}`);
