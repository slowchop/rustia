#!/usr/bin/env node

"use strict";

// Freezes the cross-engine memory protocol for the owned host.
//
// The rustia-only generator, tools/create-rustia-ci-memory-config.js, is the
// routine per-commit cohort: 11 Rustia rows. This one measures the same
// protocol over thirteen engines, 110 rows, so that the memory comparison and
// the mlua/Luau/JIT reference the dashboard ratios against come from the same
// machine as the Rustia rows they are compared with. Everything but the engine
// set, the cohort id and the publication shape is inherited from the
// rustia-only freeze, so the two cohort kinds differ in nothing else: same
// samples, same bootstrap, same thresholds, same host block, same pinned cores.
//
// The engine groups are the protocol the rented lane measured as cohort
// pre-reclamation-18b58d8. They are not read from that frozen file; the file
// is history, and this is the one place the live protocol is written down.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const rustiaOnly = require("./create-rustia-ci-memory-config.js");

const ROOT = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_OUTPUT = /^bench\/cloud\/dedicated-cross-engine-memory-[0-9a-f]{7,40}\.json$/;

// The thirteen engines the throughput matrix measures; the memory comparison
// covers the same set so that the two dashboards name the same competitors.
const ENGINES = Object.freeze([
  "rustia",
  "boa",
  "koto",
  "luaur",
  "mlua-luau",
  "mlua-luau-jit",
  "rhai",
  "roto",
  "ruau",
  "rquickjs",
  "wasmi",
  "wasmtime",
  "mlua-lua54",
]);

// The nine workloads the memory history chart requires of its reference
// engine and of Rustia; the two retention controls are measured beside them.
const HISTORY_WORKLOADS = Object.freeze([
  "closures_upvalues",
  "dense_tables",
  "generic_iteration",
  "sparse_tables",
  "stdlib_math_table",
  "memory_burst",
  "memory_churn",
  "todo_history",
  "todo_steady",
]);

const COMMON_WORKLOADS = Object.freeze([
  { id: "dense_tables", n: 128, epochs: 16 },
  { id: "sparse_tables", n: 128, epochs: 16 },
  { id: "closures_upvalues", n: 128, epochs: 16 },
  { id: "generic_iteration", n: 128, epochs: 16 },
  { id: "stdlib_math_table", n: 128, epochs: 16 },
]);
const STRESS_WORKLOADS = Object.freeze([
  { id: "memory_burst", n: 1, epochs: 16 },
  { id: "memory_churn", n: 1, epochs: 40 },
  { id: "memory_retained_control", n: 1, epochs: 40 },
]);
const TODO_WORKLOADS = Object.freeze([
  { id: "todo_steady", n: 128, epochs: 40 },
  { id: "todo_history", n: 128, epochs: 40 },
  { id: "todo_retained_control", n: 128, epochs: 40 },
]);

// mlua-lua54 links a second `lua` and lives in its own binary; every other
// engine is in the all-engines build. The stress and todo workloads are
// measured only on the engines that complete them.
function crossEngineGroups() {
  const copy = (workloads) => workloads.map((workload) => ({ ...workload }));
  return [
    {
      binary: "all",
      engines: ENGINES.filter((engine) => engine !== "mlua-lua54"),
      workloads: copy(COMMON_WORKLOADS),
    },
    { binary: "lua54", engines: ["mlua-lua54"], workloads: copy(COMMON_WORKLOADS) },
    {
      binary: "all",
      engines: ["rustia", "luaur", "mlua-luau", "mlua-luau-jit", "ruau"],
      workloads: [...copy(STRESS_WORKLOADS), ...copy(TODO_WORKLOADS)],
    },
    { binary: "all", engines: ["koto"], workloads: [...copy(STRESS_WORKLOADS), ...copy(TODO_WORKLOADS)] },
    { binary: "all", engines: ["boa", "rquickjs"], workloads: copy(STRESS_WORKLOADS) },
    { binary: "lua54", engines: ["mlua-lua54"], workloads: copy(STRESS_WORKLOADS) },
  ];
}

function fail(message) {
  throw new Error(message);
}

// Every engine/workload case a groups block expands to, in run order.
function expandCases(groups) {
  return groups.flatMap((group) =>
    group.engines.flatMap((engine) =>
      group.workloads.map((workload) => ({ engine, ...workload })),
    ),
  );
}

function createConfig(commit) {
  const base = rustiaOnly.toDedicated(rustiaOnly.createConfig(commit));
  const groups = crossEngineGroups();
  return {
    ...base,
    measurement: {
      ...base.measurement,
      cohort_id: `cross-engine-${commit.slice(0, 12)}`,
      groups,
    },
    // Mode "memory" is what makes the runner build all-engines and mlua-lua54
    // rather than the rustia-only shortcut. It also skips the wasm32 quota
    // probe, which belongs to the rustia-only cohort.
    publication: { mode: "memory", rows: expandCases(groups).length },
  };
}

function parseOutput(argv) {
  if (argv.length !== 2 || argv[0] !== "--output") {
    fail(
      "usage: create-cross-engine-memory-config.js --output " +
        "bench/cloud/dedicated-cross-engine-memory-COMMIT.json",
    );
  }
  const relative = argv[1].replaceAll("\\", "/");
  if (!SAFE_OUTPUT.test(relative)) fail("--output must be a safe per-commit bench/cloud JSON path");
  const output = path.resolve(ROOT, relative);
  if (path.relative(ROOT, output).startsWith("..")) fail("--output escapes the repository");
  return { relative, output };
}

function selfTest() {
  // The freeze reads source hashes out of git, so the checks run against a
  // real commit: HEAD.
  const commit = gitHead();
  const groups = crossEngineGroups();
  const cases = expandCases(groups);
  assert.equal(cases.length, 110);
  const identities = new Set(cases.map((entry) => `${entry.engine}\0${entry.id}`));
  assert.equal(identities.size, cases.length, "an engine/workload case is measured twice");
  assert.deepEqual(
    [...new Set(cases.map((entry) => entry.engine))].sort(),
    [...ENGINES].sort(),
    "the cohort must cover exactly the matrix's engines",
  );
  // The reference engine and Rustia must each complete every history
  // workload, or the ratio chart has nothing to draw.
  for (const engine of ["rustia", "mlua-luau-jit"]) {
    const covered = new Set(cases.filter((entry) => entry.engine === engine).map((entry) => entry.id));
    for (const workload of HISTORY_WORKLOADS) {
      assert(covered.has(workload), `${engine} does not measure ${workload}`);
    }
  }
  for (const group of groups) {
    assert(["all", "lua54"].includes(group.binary));
    assert.equal(group.engines.includes("mlua-lua54"), group.binary === "lua54");
  }

  // The Rustia cases must be the rustia-only cohort's cases exactly, or the
  // per-engine merge in the published data would treat the two cohorts'
  // Rustia rows as different measurements.
  const base = rustiaOnly.toDedicated(rustiaOnly.createConfig(commit));
  const rustiaCases = (measurement) =>
    expandCases(measurement.groups)
      .filter((entry) => entry.engine === "rustia")
      .map((entry) => `${entry.id}\0${entry.n}\0${entry.epochs}`)
      .sort();
  const config = createConfig(commit);
  assert.deepEqual(rustiaCases(config.measurement), rustiaCases(base.measurement));
  assert.equal(rustiaCases(config.measurement).length, 11);

  // Everything but the engine set, the cohort id and the publication shape
  // is inherited from the rustia-only dedicated freeze.
  assert.equal(config.schema, base.schema);
  assert.deepEqual(config.provider, base.provider);
  assert.deepEqual(config.source, base.source);
  assert.deepEqual(config.host_gate, base.host_gate);
  assert.equal(config.measurement.cohort_id, `cross-engine-${commit.slice(0, 12)}`);
  assert.deepEqual(
    { ...config.measurement, cohort_id: null, groups: null },
    { ...base.measurement, cohort_id: null, groups: null },
  );
  assert.deepEqual(config.publication, { mode: "memory", rows: 110 });

  assert.deepEqual(parseOutput(["--output", "bench/cloud/dedicated-cross-engine-memory-abc1234.json"]).relative,
    "bench/cloud/dedicated-cross-engine-memory-abc1234.json");
  assert.throws(() => parseOutput(["--output", "bench/cloud/dedicated-rustia-ci-memory-abc1234.json"]));
  assert.throws(() => parseOutput(["--output", "bench/cloud/linode-cross-engine-memory-abc1234.json"]));
  assert.throws(() => parseOutput(["--dedicated", "--output", "bench/cloud/dedicated-cross-engine-memory-abc1234.json"]));
  process.stdout.write("cross-engine memory config self-test passed\n");
}

function gitHead() {
  const commit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }).trim();
  if (!FULL_COMMIT.test(commit)) fail("HEAD did not resolve to a full lowercase commit");
  return commit;
}

function main() {
  const argv = process.argv.slice(2);
  if (argv.length === 1 && argv[0] === "self-test") {
    selfTest();
    return;
  }
  const { relative, output } = parseOutput(argv);
  const commit = gitHead();
  if (fs.existsSync(output)) fail(`refusing to overwrite ${relative}`);
  fs.writeFileSync(output, `${JSON.stringify(createConfig(commit), null, 2)}\n`, {
    flag: "wx",
    mode: 0o600,
  });
  process.stdout.write(`${relative}\n`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`create-cross-engine-memory-config: ${error.message}\n`);
    process.exitCode = 1;
  }
}

// Exported so the runner's self-test can validate exactly what this writes.
module.exports = { createConfig, crossEngineGroups, expandCases, ENGINES, HISTORY_WORKLOADS };
