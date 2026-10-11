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
    "343c2615e6f2a38b918258c86c5c7dac0f2ca7fe",
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
    "c13940bd14fcb1076588aecb524e3b12229cb8ea",
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
    "530c15d1bcd474f7baf8ca6870f502c4900f4900",
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
    "a5cd40cbd8d752ce40ee4304b8ca20f39b156f94",
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
    "7826508f767013e14c0030cbc2f741c5471acd47",
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
    "f49646ff11b0028c2a003ae12a48fe509525d903",
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

// Every .js script under tools/ and bench/ that defines HOST_LABEL, as
// repo-relative paths. Dependency and build directories are skipped.
function scriptsDefiningHostLabel(base) {
  const found = [];
  const walk = (directory) => {
    for (const entry of fs.readdirSync(path.join(base, directory), { withFileTypes: true })) {
      if (entry.name.startsWith(".") || entry.name === "node_modules" || entry.name === "target") continue;
      const child = path.posix.join(directory, entry.name);
      if (entry.isDirectory()) walk(child);
      else if (entry.name.endsWith(".js") && /^const HOST_LABEL\s*=/m.test(read(path.join(base, child)))) found.push(child);
    }
  };
  ["tools", "bench"].forEach(walk);
  return found.sort();
}

// Refuses a listed script that does not carry `labelLiteral` as a line of its
// own, or a script under `base` that defines HOST_LABEL without being listed.
function requireHostLabelParity(base, labelFiles, labelLiteral) {
  for (const file of labelFiles) {
    if (!read(path.join(base, file)).split("\n").includes(labelLiteral)) {
      fail(`${file} does not carry the label character set ${labelLiteral} as a line of its own; every emitter and validator must agree`);
    }
  }
  // The list is only as good as its completeness: a script that defines the
  // character set without being listed drifts unnoticed, which is how
  // tools/check-dedicated-config.js went unchecked when it gained one.
  for (const file of scriptsDefiningHostLabel(base)) {
    if (!labelFiles.includes(file)) {
      fail(`${file} defines HOST_LABEL but is not in the label parity list in tools/check-benchmark-results.js`);
    }
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
// bench/current-results.json and bench/memory/current-results.json are merges
// across every cohort in their histories: the timing table keeps the latest row
// per (target, engine, workload), and the memory table takes each engine's rows
// from the newest cohort that measured it. A run that re-measures only some
// slots or engines therefore leaves the rest standing, and before rows named
// their machine that is exactly what happened: a table announced one machine
// in its metadata while most of its rows had been measured on a rented cloud
// host six weeks earlier, and only the per-row commit revealed it. A
// comparison between two machines is not a comparison between two engines, so
// each table must come from one machine, and that machine must be the one the
// operations policy qualifies to produce timings. The checks below are pure so
// the self-test can hand them tables that must be refused.
// A label, never an address: no dots and no colons, so it can be neither a
// fully qualified name nor an IPv4 or IPv6 address, whatever an operator types.
// Every emitter and validator carries this same literal; selfTest() asserts it.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

function requireLabelledRows(rows, label) {
  for (const [index, row] of rows.entries()) {
    if (typeof row?.host !== "string" || row.host === "") {
      fail(
        `${label} row ${index + 1} does not name the machine it was measured on; ` +
        "a row that cannot say where it came from can outlive the machine that produced it",
      );
    }
    if (!HOST_LABEL.test(row.host)) {
      fail(
        `${label} row ${index + 1} names its machine ${JSON.stringify(row.host)}, which is not a label: ` +
        "a label may contain only [A-Za-z0-9_-], never an address",
      );
    }
  }
}

// Every cohort in the memory history is one run on one machine, and the
// cohort the page draws -- the latest, the machine the memory history chart
// is keyed on -- is the timing authority's. Older cohorts may name another
// machine: they are history, kept undrawn and never mixed.
function requireMemoryHistory(cohorts, label, expectedHost) {
  let latestHost = null;
  for (const [index, cohort] of cohorts.entries()) {
    const line = `${label} line ${index + 1}`;
    if (!Array.isArray(cohort.rows) || cohort.rows.length === 0) fail(`${line}.rows must be a non-empty array`);
    requireLabelledRows(cohort.rows, line);
    const hosts = [...new Set(cohort.rows.map((row) => row.host))].sort();
    if (hosts.length !== 1) {
      fail(`${line} was measured on ${hosts.length} machines (${hosts.join(", ")}); a cohort is one run on one machine`);
    }
    if (cohort.host !== undefined && cohort.host !== hosts[0]) {
      fail(`${line}.host is ${JSON.stringify(cohort.host)}, but its rows were measured on ${JSON.stringify(hosts[0])}`);
    }
    latestHost = hosts[0];
  }
  if (cohorts.length > 0 && latestHost !== expectedHost) {
    fail(
      `${label} line ${cohorts.length} is the latest cohort, the one the page draws, and was measured on ` +
      `${JSON.stringify(latestHost)}, but ${relative(operationsStatusPath)} qualifies ${JSON.stringify(expectedHost)} as the timing authority`,
    );
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
  requireMemoryHistory(readMemoryHistoryCohorts(), relative(memoryHistoryPath), expectedHost);
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
  for (const address of ["bench.example.com", "10.0.0.5", "fe80::1", "user@box", "a".repeat(129)]) {
    refuses(/row 1 names its machine .* which is not a label/, () => requireLabelledRows([row("rustia", address)], "fixture"));
  }

  // The memory history: every cohort one machine, the latest the authority's,
  // older cohorts free to name the machine they were measured on.
  const cohort = (host, rows = [row("rustia", host), row("mlua-luau-jit", host)]) => ({ rows });
  requireMemoryHistory([], "fixture", "ci-bench");
  requireMemoryHistory([cohort("elsewhere"), cohort("ci-bench")], "fixture", "ci-bench");
  refuses(/line 2 is the latest cohort, the one the page draws, and was measured on "elsewhere"/, () =>
    requireMemoryHistory([cohort("ci-bench"), cohort("elsewhere")], "fixture", "ci-bench"));
  refuses(/line 1 was measured on 2 machines \(ci-bench, elsewhere\)/, () =>
    requireMemoryHistory([cohort("ci-bench", [row("rustia", "ci-bench"), row("koto", "elsewhere")])], "fixture", "ci-bench"));
  refuses(/line 1\.host is "elsewhere", but its rows were measured on "ci-bench"/, () =>
    requireMemoryHistory([{ ...cohort("ci-bench"), host: "elsewhere" }], "fixture", "ci-bench"));
  refuses(/line 1\.rows must be a non-empty array/, () => requireMemoryHistory([{ rows: [] }], "fixture", "ci-bench"));

  // The label character set is one literal carried by every file that emits
  // or validates a label; they cannot share a module (the on-host runners are
  // copied to the machine one file at a time), so their agreement is asserted
  // here by reading the files, and the harness's Rust arm alongside.
  const labelLiteral = "const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;";
  const labelFiles = [
    "tools/check-benchmark-results.js",
    "tools/render-benchmark-operations-status.js",
    "bench/render-current-results.js",
    "bench/analyze-memory-samples.js",
    "bench/run-memory-cohort.js",
    "bench/publish-memory-analysis.js",
    "tools/merge-rustia-ci-memory-data.js",
    "tools/check-dedicated-config.js",
    "tools/run-linode-benchmark.js",
    "tools/run-linux-matrix-benchmark.js",
    "tools/run-linux-memory-benchmark.js",
    "tools/run-linux-history-backfill.js",
    "tools/record-host-bridge.js",
  ];
  requireHostLabelParity(root, labelFiles, labelLiteral);
  const fixtureRoot = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-host-label-"));
  const writeFixture = (file, text) => {
    fs.mkdirSync(path.dirname(path.join(fixtureRoot, file)), { recursive: true });
    fs.writeFileSync(path.join(fixtureRoot, file), text);
  };
  try {
    for (const [file, text] of [
      ["tools/emits.js", `${labelLiteral}\n`],
      ["bench/nested/validates.js", `"use strict";\nconst HOST_LABEL=/x/;\n`],
      ["tools/mentions.js", `const HOST_LABEL_LINE = ${JSON.stringify(labelLiteral)};\n`],
      ["tools/node_modules/vendored.js", `${labelLiteral}\n`],
      ["bench/notes.md", `${labelLiteral}\n`],
    ]) {
      writeFixture(file, text);
    }
    const found = scriptsDefiningHostLabel(fixtureRoot);
    if (JSON.stringify(found) !== JSON.stringify(["bench/nested/validates.js", "tools/emits.js"])) {
      fail(`scriptsDefiningHostLabel found ${JSON.stringify(found)} in its fixture`);
    }
    // Both refusals, each for its own reason. A copy of the literal that is
    // indented, annotated or commented out is not the line, so it could sit
    // beside a validator that says something else.
    const bothListed = ["tools/emits.js", "bench/nested/validates.js"];
    writeFixture("bench/nested/validates.js", `"use strict";\n${labelLiteral}\n`);
    requireHostLabelParity(fixtureRoot, bothListed, labelLiteral);
    for (const variant of [`  ${labelLiteral}`, `${labelLiteral} // same`, `// ${labelLiteral}`]) {
      writeFixture("tools/emits.js", `${variant}\n`);
      refuses(/^tools\/emits\.js does not carry the label character set .* as a line of its own/, () =>
        requireHostLabelParity(fixtureRoot, bothListed, labelLiteral));
    }
    writeFixture("tools/emits.js", `${labelLiteral}\n`);
    refuses(/^bench\/nested\/validates\.js defines HOST_LABEL but is not in the label parity list/, () =>
      requireHostLabelParity(fixtureRoot, ["tools/emits.js"], labelLiteral));
  } finally {
    fs.rmSync(fixtureRoot, { recursive: true, force: true });
  }
  const harness = read(path.join(root, "bench", "src", "main.rs"));
  for (const fragment of ["c.is_ascii_alphanumeric() || c == '-' || c == '_'", "value.len() > 128"]) {
    if (!harness.includes(fragment)) {
      fail(`bench/src/main.rs --host does not carry the label character set (${fragment}); every emitter and validator must agree`);
    }
  }
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
