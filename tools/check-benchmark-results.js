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
// Cohorts whose rows must retain the extended harness provenance (prepare /
// first-call / cold-start timings and verified mlua-luau native-JIT identity).
const extendedProvenanceCohorts = new Set([
  "34f6deff4a42501b6d66d257249eecfcbca1976a",
  "83bc2f370ee4149b6fa6e11eca951b41b4cccbd2",
  "bbe0c5c30ff0eaedba519d37638f4b91644e94a7",
  "f412e2dadd4568b4ba35158090978c79413fd0f7",
]);
const expectedEngineCohorts = new Map([
  [
    "8f0e7f474f2b895be73aecec2a70a215e8467322",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["mlua-luau-jit", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "7f416028e77e7a05b97c1702c1c0b6f11529e970",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["mlua-luau-jit", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "f412e2dadd4568b4ba35158090978c79413fd0f7",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["mlua-luau-jit", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "bbe0c5c30ff0eaedba519d37638f4b91644e94a7",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["mlua-luau-jit", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "34f6deff4a42501b6d66d257249eecfcbca1976a",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["mlua-luau-jit", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "83bc2f370ee4149b6fa6e11eca951b41b4cccbd2",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["mlua-luau-jit", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "b1cd2841fb8fc2d916047391a1d757d53a1303cd",
    new Map([
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
    ]),
  ],
  [
    "e53cbcffc08a5924f16c7b6114f3f7c9dae92162",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "fa8c8571d60659c4f637940acbb84eb591b1e039",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "3732b80d5200edf053e213ef931b838c003968f3",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "d4ec275e19bad0efcf8e96db4030e7380bd93a9b",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
  [
    "a5629199643847c0da78792644ad3656887bee21",
    new Map([
      ["rustia", "luau"],
      ["boa", "javascript"],
      ["koto", "koto"],
      ["luaur", "luau"],
      ["mlua-luau", "luau"],
      ["rhai", "rhai"],
      ["roto", "roto"],
      ["ruau", "luau"],
      ["rquickjs", "javascript"],
      ["wasmi", "wasm"],
      ["wasmtime", "wasm"],
      ["mlua-lua54", "lua54"],
    ]),
  ],
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
  let previousTimestamp = 0;
  for (const [index, row] of rows.entries()) {
    const label = `${relative(historyPath)} line ${index + 1}`;
    if (row == null || typeof row !== "object" || Array.isArray(row)) fail(`${label} must be an object`);
    if (row.target !== dedicatedLinuxTarget) {
      fail(`${label}.target must be ${JSON.stringify(dedicatedLinuxTarget)}, got ${JSON.stringify(row.target)}`);
    }
    if (!Number.isSafeInteger(row.timestamp_unix) || row.timestamp_unix <= 0) {
      fail(`${label}.timestamp_unix must be a positive safe integer`);
    }
    if (row.timestamp_unix < previousTimestamp) {
      fail(`${label}.timestamp_unix must not precede the prior history row`);
    }
    previousTimestamp = row.timestamp_unix;
  }

  const cohortCommit = rows.at(-1)?.commit;
  const expectedEngines = expectedEngineCohorts.get(cohortCommit);
  if (expectedEngines == null) {
    fail(`${relative(historyPath)} ends with an unknown frozen matrix cohort: ${JSON.stringify(cohortCommit)}`);
  }
  const cohortRows = rows.filter((row) => row.commit === cohortCommit);
  const expectedRowCount = expectedEngines.size * expectedWorkloads.length;
  if (cohortRows.length !== expectedRowCount) {
    fail(
      `latest dedicated-Linux cohort must contain exactly ${expectedRowCount} rows; ` +
      `got ${cohortRows.length}`,
    );
  }

  const commits = new Set();
  const slots = new Set();
  const timestamps = [];
  let cohortPreviousTimestamp = 0;
  for (const [index, row] of cohortRows.entries()) {
    const label = `latest dedicated-Linux cohort row ${index + 1}`;
    const expectedDialect = expectedEngines.get(row.engine);
    if (expectedDialect == null) {
      fail(`${label}.engine is not in the frozen ${expectedEngines.size}-engine matrix: ${JSON.stringify(row.engine)}`);
    }
    if (row.dialect !== expectedDialect) {
      fail(`${label}.dialect must be ${JSON.stringify(expectedDialect)} for ${row.engine}`);
    }
    if (extendedProvenanceCohorts.has(cohortCommit)) {
      for (const field of [
        "prepare_ns_min", "prepare_ns_median", "first_call_ns_min", "first_call_ns_median",
        "cold_start_ns_min", "cold_start_ns_median",
      ]) {
        if (!Number.isFinite(row[field]) || row[field] <= 0) fail(`${label}.${field} must be positive and finite`);
      }
      if (row.engine === "mlua-luau-jit" &&
          (row.execution_mode !== "jit" || row.mode_family !== "mlua-luau" ||
           row.native_verified !== true || row.native_verification !== "ab-native-sentinel" ||
           !Number.isFinite(row.native_verification_ratio) || row.native_verification_ratio < 1.5)) {
        fail(`${label} must retain verified mlua-luau native-JIT provenance`);
      }
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
    if (row.timestamp_unix < cohortPreviousTimestamp) {
      fail(`${label}.timestamp_unix must not precede the prior matrix row`);
    }
    cohortPreviousTimestamp = row.timestamp_unix;
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
