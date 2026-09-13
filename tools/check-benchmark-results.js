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
const operationsStatusPath = path.join(root, "bench", "operations-status.json");
const memoryResultsPath = path.join(root, "bench", "memory", "current-results.json");
const memoryHistoryPath = path.join(root, "bench", "memory", "history", "runs.jsonl");
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
    "f4807587780aec5829b297c037d5dd03ca40d251",
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

// Thrown, not exited: the self-test below hands the validators tables they
// must refuse, and it needs to see the refusal to assert on it. The one
// catch at the bottom of the file turns any refusal into the exit status.
function fail(message) {
  throw new Error(message);
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

// The one machine the operations policy qualifies as a timing authority. Read
// rather than restated, so this check and the published dashboard can never
// disagree about which machine is allowed to produce a number.
function authoritativeHost() {
  const status = JSON.parse(read(operationsStatusPath));
  const classes = Object.entries(status.runner_classes ?? {}).filter(
    ([, runner]) => runner?.timing_authority === true,
  );
  if (classes.length !== 1) {
    fail(
      `${relative(operationsStatusPath)} must declare exactly one timing-authoritative runner class, ` +
      `got ${classes.length}`,
    );
  }
  const [name, runner] = classes[0];
  if (typeof runner.host !== "string" || runner.host === "") {
    fail(`${relative(operationsStatusPath)} runner class ${name} is a timing authority but names no host`);
  }
  return runner.host;
}

// Provenance, per row and for the table as a whole.
//
// bench/current-results.json and bench/memory/current-results.json are per-slot
// merges -- the latest row per (target, engine, workload) across every cohort
// in their histories. A run that re-measures only some slots therefore leaves
// the rest standing, and before rows named their machine that is exactly what
// happened: a table announced one machine in its metadata while most of its
// rows had been measured on a rented cloud host six weeks earlier, and only
// the per-row commit revealed it. A comparison between two machines is not a
// comparison between two engines, so each table must come from one machine,
// and that machine must be the one the operations policy qualifies to produce
// timings. The checks below are pure so the self-test can hand them tables
// that must be refused.
function requireLabelledRows(rows, label) {
  for (const [index, row] of rows.entries()) {
    if (typeof row?.host !== "string" || row.host === "") {
      fail(
        `${label} row ${index + 1} does not name the machine it was measured on; ` +
        "a row that cannot say where it came from can outlive the machine that produced it",
      );
    }
  }
}

function requireSingleHostTable(table, label, expectedHost) {
  const rows = Array.isArray(table?.rows) ? table.rows : fail(`${label}.rows must be an array`);
  if (rows.length === 0) fail(`${label}.rows must not be empty`);
  requireLabelledRows(rows, label);
  const hosts = [...new Set(rows.map((row) => row.host))].sort();
  if (hosts.length !== 1) {
    const byHost = hosts.map(
      (host) => `${host}: ${[...new Set(rows.filter((row) => row.host === host).map((row) => row.engine))].sort().join(", ")}`,
    );
    fail(
      `${label} publishes rows measured on ${hosts.length} machines (${byHost.join("; ")}); ` +
      "a published table is one machine, so re-measure the stale slots or retire their rows",
    );
  }
  if (hosts[0] !== expectedHost) {
    fail(
      `${label} was measured on ${JSON.stringify(hosts[0])}, but ` +
      `${relative(operationsStatusPath)} qualifies ${JSON.stringify(expectedHost)} as the timing authority`,
    );
  }
  // The metadata is what a reader sees first, so it may not disagree with the
  // rows. Either spelling is checked when present: the harness and the merge
  // write `hosts`, a memory cohort publication writes `host`.
  const metadata = table.metadata ?? {};
  if (metadata.hosts !== undefined && JSON.stringify(metadata.hosts) !== JSON.stringify(hosts)) {
    fail(
      `${label}.metadata.hosts is ${JSON.stringify(metadata.hosts)}, ` +
      `but its rows were measured on ${JSON.stringify(hosts)}`,
    );
  }
  if (metadata.host !== undefined && metadata.host !== hosts[0]) {
    fail(
      `${label}.metadata.host is ${JSON.stringify(metadata.host)}, ` +
      `but its rows were measured on ${JSON.stringify(hosts[0])}`,
    );
  }
  return hosts[0];
}

function readMemoryHistoryCohorts() {
  const text = read(memoryHistoryPath);
  return text
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        fail(`${relative(memoryHistoryPath)} line ${index + 1} is not JSON: ${error.message}`);
      }
    });
}

function validateMeasurementHosts() {
  const expectedHost = authoritativeHost();
  requireLabelledRows(readHistory(), relative(historyPath));
  requireSingleHostTable(JSON.parse(read(expectedJson)), relative(expectedJson), expectedHost);

  // The memory table is published beside the timing table and its ratios are
  // read the same way, so it answers to the same machine. A pending table has
  // no rows to place anywhere and is left alone; a cohort in the memory history
  // is one run on one machine, so each is checked as a table of its own.
  const memory = JSON.parse(read(memoryResultsPath));
  if (memory.status === "ready") {
    requireSingleHostTable(memory, relative(memoryResultsPath), expectedHost);
  }
  for (const [index, cohort] of readMemoryHistoryCohorts().entries()) {
    const label = `${relative(memoryHistoryPath)} line ${index + 1}`;
    if (!Array.isArray(cohort.rows) || cohort.rows.length === 0) fail(`${label}.rows must be a non-empty array`);
    requireLabelledRows(cohort.rows, label);
    const hosts = [...new Set(cohort.rows.map((row) => row.host))].sort();
    if (hosts.length !== 1) {
      fail(`${label} was measured on ${hosts.length} machines (${hosts.join(", ")}); a cohort is one run on one machine`);
    }
    if (cohort.host !== undefined && cohort.host !== hosts[0]) {
      fail(`${label}.host is ${JSON.stringify(cohort.host)}, but its rows were measured on ${JSON.stringify(hosts[0])}`);
    }
  }
}

// The refusals above are the whole point of this file, so they are exercised
// on fixtures before the committed data is checked: a validator that accepts
// everything would otherwise pass this script without anyone noticing.
function selfTest() {
  const row = (engine, host) => ({ target: dedicatedLinuxTarget, engine, workload: "dense_tables", host });
  const refuses = (pattern, thunk) => {
    let message = null;
    try {
      thunk();
    } catch (error) {
      message = error.message;
    }
    if (message == null || !pattern.test(message)) {
      fail(`self-test expected a refusal matching ${pattern}, got ${JSON.stringify(message)}`);
    }
  };
  const table = (rows, metadata = {}) => ({ metadata, rows });

  if (requireSingleHostTable(table([row("rustia", "ci-bench"), row("koto", "ci-bench")], { hosts: ["ci-bench"] }), "fixture", "ci-bench") !== "ci-bench") {
    fail("self-test did not accept a single-host table");
  }
  refuses(/2 machines \(ci-bench: rustia; elsewhere: koto\)/, () =>
    requireSingleHostTable(table([row("rustia", "ci-bench"), row("koto", "elsewhere")]), "fixture", "ci-bench"));
  refuses(/fixture row 2 does not name the machine/, () =>
    requireSingleHostTable(table([row("rustia", "ci-bench"), row("koto", undefined)]), "fixture", "ci-bench"));
  refuses(/measured on "elsewhere", but .* qualifies "ci-bench"/, () =>
    requireSingleHostTable(table([row("rustia", "elsewhere")]), "fixture", "ci-bench"));
  refuses(/metadata\.hosts is \["elsewhere"\]/, () =>
    requireSingleHostTable(table([row("rustia", "ci-bench")], { hosts: ["elsewhere"] }), "fixture", "ci-bench"));
  refuses(/metadata\.host is "elsewhere"/, () =>
    requireSingleHostTable(table([row("rustia", "ci-bench")], { host: "elsewhere" }), "fixture", "ci-bench"));
  refuses(/rows must not be empty/, () => requireSingleHostTable(table([]), "fixture", "ci-bench"));
  refuses(/row 2 does not name the machine/, () =>
    requireLabelledRows([row("rustia", "ci-bench"), row("koto", "")], "fixture"));
}

function main() {
  selfTest();
  validateDedicatedLinuxMatrix();
  validateMeasurementHosts();

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
}

try {
  main();
} catch (error) {
  console.error(`check-benchmark-results: ${error.message}`);
  process.exit(1);
}
