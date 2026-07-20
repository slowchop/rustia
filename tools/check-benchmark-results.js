#!/usr/bin/env node
"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const historyPath = path.join(root, "bench", "history", "runs.jsonl");
const expectedResults = path.join(root, "bench", "RESULTS.md");
const expectedJson = path.join(root, "bench", "current-results.json");
const expectedClaims = path.join(root, "bench", "CLAIMS.md");
const dedicatedLinuxTarget = "linux-x86_64";
const expectedEngines = new Map([
  ["rustia", "luau"],
  ["boa", "javascript"],
  ["koto", "koto"],
  ["mlua-luau", "luau"],
  ["rhai", "rhai"],
  ["roto", "roto"],
  ["rquickjs", "javascript"],
  ["wasmi", "wasm"],
  ["wasmtime", "wasm"],
  ["mlua-lua54", "lua54"],
]);
const expectedWorkloads = [
  "dense_tables",
  "sparse_tables",
  "closures_upvalues",
  "generic_iteration",
  "stdlib_math_table",
];
const fullCommit = /^[0-9a-f]{40}$/;
const maximumMatrixCohortSeconds = 55 * 60;

function fail(message) {
  console.error(`check-benchmark-results: ${message}`);
  process.exit(1);
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function relative(filePath) {
  return slash(path.relative(root, filePath));
}

function runCargo(args) {
  try {
    childProcess.execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`cargo ${args.join(" ")} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function runNode(args) {
  try {
    childProcess.execFileSync(process.execPath, args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    const command = args
      .map((arg) => path.isAbsolute(arg) ? relative(arg) : arg)
      .join(" ");
    fail(`node ${command} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function read(filePath) {
  try {
    return fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(filePath)}: ${error.message}`);
  }
}

function compare(expectedPath, actualPath, refreshCommand) {
  const expected = read(expectedPath);
  const actual = read(actualPath);
  if (expected !== actual) {
    fail(`${relative(expectedPath)} is stale; run ${refreshCommand}`);
  }
}

function readHistory() {
  const text = read(historyPath);
  if (text.length === 0 || !text.endsWith("\n")) {
    fail(`${relative(historyPath)} must be non-empty and newline terminated`);
  }
  const lines = text.slice(0, -1).split("\n");
  if (lines.some((line) => line.trim() === "")) {
    fail(`${relative(historyPath)} must not contain blank rows`);
  }
  return lines.map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      fail(`${relative(historyPath)} line ${index + 1} is not JSON: ${error.message}`);
    }
  });
}

function validateDedicatedLinuxMatrix() {
  const rows = readHistory();
  const expectedRowCount = expectedEngines.size * expectedWorkloads.length;
  if (rows.length !== expectedRowCount) {
    fail(
      `${relative(historyPath)} must contain exactly ${expectedRowCount} active dedicated-Linux rows; ` +
      `got ${rows.length}`,
    );
  }

  const commits = new Set();
  const slots = new Set();
  const timestamps = [];
  let previousTimestamp = 0;
  for (const [index, row] of rows.entries()) {
    const label = `${relative(historyPath)} line ${index + 1}`;
    if (row == null || typeof row !== "object" || Array.isArray(row)) {
      fail(`${label} must be an object`);
    }
    if (row.target !== dedicatedLinuxTarget) {
      fail(`${label}.target must be ${JSON.stringify(dedicatedLinuxTarget)}, got ${JSON.stringify(row.target)}`);
    }
    const expectedDialect = expectedEngines.get(row.engine);
    if (expectedDialect == null) {
      fail(`${label}.engine is not in the frozen 10-engine matrix: ${JSON.stringify(row.engine)}`);
    }
    if (row.dialect !== expectedDialect) {
      fail(`${label}.dialect must be ${JSON.stringify(expectedDialect)} for ${row.engine}`);
    }
    if (!expectedWorkloads.includes(row.workload)) {
      fail(`${label}.workload is not in the frozen five-workload matrix: ${JSON.stringify(row.workload)}`);
    }
    if (typeof row.commit !== "string" || !fullCommit.test(row.commit)) {
      fail(`${label}.commit must be a full 40-character lowercase Git commit`);
    }
    commits.add(row.commit);
    if (!Number.isSafeInteger(row.timestamp_unix) || row.timestamp_unix <= 0) {
      fail(`${label}.timestamp_unix must be a positive safe integer`);
    }
    if (row.timestamp_unix < previousTimestamp) {
      fail(`${label}.timestamp_unix must not precede the prior matrix row`);
    }
    previousTimestamp = row.timestamp_unix;
    timestamps.push(row.timestamp_unix);

    const slot = `${row.engine}\0${row.workload}`;
    if (slots.has(slot)) {
      fail(`${relative(historyPath)} duplicates ${row.engine}/${row.workload}`);
    }
    slots.add(slot);
  }

  if (commits.size !== 1) {
    fail(`${relative(historyPath)} must contain one benchmark commit cohort, got ${commits.size}`);
  }
  const cohortSeconds = timestamps.at(-1) - timestamps[0];
  if (cohortSeconds > maximumMatrixCohortSeconds) {
    fail(
      `${relative(historyPath)} timestamps span ${cohortSeconds}s; ` +
      `the dedicated matrix cohort must fit within ${maximumMatrixCohortSeconds}s`,
    );
  }
  for (const engine of expectedEngines.keys()) {
    for (const workload of expectedWorkloads) {
      if (!slots.has(`${engine}\0${workload}`)) {
        fail(`${relative(historyPath)} is missing ${engine}/${workload}`);
      }
    }
  }
}

validateDedicatedLinuxMatrix();

const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-bench-results-"));
try {
  const actualResults = path.join(tempDir, "RESULTS.md");
  const actualJson = path.join(tempDir, "current-results.json");
  const actualClaims = path.join(tempDir, "CLAIMS.md");
  runCargo([
    "run",
    "-p",
    "script-bench-harness",
    "--",
    "--refresh-results",
    "--results",
    actualResults,
    "--latest-json",
    actualJson,
  ]);
  compare(expectedResults, actualResults, "just gen-current-results");
  compare(expectedJson, actualJson, "just gen-current-results");
  runNode([
    path.join(root, "bench", "render-claims.js"),
    "--source",
    actualJson,
    "--output",
    actualClaims,
  ]);
  compare(expectedClaims, actualClaims, "just gen-current-results");
  console.log("dedicated Linux benchmark matrix, markdown, JSON, and claim results are current");
} finally {
  fs.rmSync(tempDir, { recursive: true, force: true });
}
