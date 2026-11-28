#!/usr/bin/env node
"use strict";

const assert = require("assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");

function requireFact(condition, message) {
  if (!condition) throw new Error(message);
}

// The quickstart uses the released Cargo package. Benchmark provenance and other source links
// still need to resolve against this checkout's history.
function verifySourceReferences(readme, checkReference) {
  const installs = readme.match(/^cargo add rustia\b[^\r\n]*/gm) ?? [];
  requireFact(
    installs.length === 1 &&
      /^cargo add rustia@\d+\.\d+\.\d+$/.test(installs[0]),
    "README must show how to add a versioned Rustia release from crates.io",
  );
  for (const match of readme.matchAll(/https:\/\/github\.com\/slowchop\/rustia\/(blob|tree|commit|raw)\/(main|[a-f0-9]{40})(\/[^\s)>\]"`#]+)?/g)) {
    const sourcePath = match[3] ? decodeURIComponent(match[3].slice(1)) : null;
    requireFact(match[1] === "commit" || sourcePath, "source links need a file or directory");
    checkReference(match[2], sourcePath);
  }
}

function checkSourceReferences(readme) {
  const git = (args) => execFileSync("git", args, {
    cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
  });
  const reachable = new Set(git(["rev-list", "HEAD"]).trim().split("\n"));
  const checkReference = (commit, sourcePath) => {
    const resolved = commit === "main" ? git(["rev-parse", "HEAD"]).trim() : commit;
    requireFact(reachable.has(resolved), `README source commit is unreachable: ${commit}`);
    if (sourcePath) {
      try { git(["cat-file", "-e", `${resolved}:${sourcePath}`]); }
      catch (_error) { throw new Error(`README source target is absent: ${commit}:${sourcePath}`); }
    }
  };
  verifySourceReferences(readme, checkReference);
  assert.throws(() => verifySourceReferences(`${readme}\nhttps://github.com/slowchop/rustia/blob/main/__missing_current_readme_artifact__`, checkReference),
    /source target is absent/);
  assert.throws(() => verifySourceReferences(readme.replace(/^cargo add rustia@\d+\.\d+\.\d+$/m, "cargo add rustia --git https://github.com/slowchop/rustia --branch main"), checkReference),
    /versioned Rustia release from crates.io/);
  const immutable = readme.match(/https:\/\/github\.com\/slowchop\/rustia\/blob\/([a-f0-9]{40})\/[^\s)>]+/);
  requireFact(immutable, "README must link immutable benchmark evidence");
  assert.throws(() => verifySourceReferences(readme.replace(immutable[0],
    `https://github.com/slowchop/rustia/blob/${immutable[1]}/__missing_readme_artifact__`), checkReference),
    /source target is absent/);
  assert.throws(() => verifySourceReferences(readme.replace(immutable[1], "f".repeat(40)), checkReference),
    /source commit is unreachable/);
}

function readCohort(snapshot, history) {
  requireFact(/^[a-f0-9]{40}$/.test(snapshot.commit), "snapshot needs a full source commit SHA");
  requireFact(snapshot.target === "linux-x86_64", "snapshot must name the Linux x86-64 target");
  requireFact(Number.isSafeInteger(snapshot.start) && Number.isSafeInteger(snapshot.end)
    && snapshot.start <= snapshot.end, "snapshot needs a valid measurement window");
  const day = timestamp => new Date(timestamp * 1000).toISOString().slice(0, 10);
  requireFact(day(snapshot.start) === day(snapshot.end), "snapshot must name one UTC date");
  requireFact(Object.values(snapshot.workloads).every(({ label, unit }) =>
    typeof label === "string" && label.length > 0 && typeof unit === "string" && unit.length > 0),
  "snapshot workloads need display labels and exact harness units");
  return history.filter(row => row.commit === snapshot.commit && row.target === snapshot.target
    && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
}

function measuredTime(cohort, engine, mode, workload, unit) {
  const label = engine + "/" + workload;
  const matches = cohort.filter(row => row.engine === engine && row.workload === workload);
  assert.equal(matches.length, 1, "expected one history row for " + label + ", found " + matches.length);
  const [row] = matches;
  assert.ok(row.host === "ci-bench" && row.execution_mode === mode, label + " must be a ci-bench " + mode + " result");
  const family = engine === "mlua-luau-jit" ? "mlua-luau" : engine;
  assert.equal(row.mode_family, family, label + " has the wrong execution-mode family");
  assert.equal(row.unit, unit, label + ": unexpected history unit " + JSON.stringify(row.unit));
  assert.ok(Number.isFinite(row.ns_per_unit_min) && row.ns_per_unit_min > 0, label + " has no valid minimum time");
  if (mode === "jit") {
    assert.ok(row.native_verified === true && row.native_verification === "ab-native-sentinel"
      && Number.isFinite(row.native_verification_ratio) && row.native_verification_ratio >= 1.5,
    label + " lacks its verified Luau JIT sentinel");
  } else if (mode === "native") {
    assert.ok(row.native_verified === true && row.native_verification === "cranelift-function-construction",
      label + " lacks verified Cranelift function construction");
  }
  return row.ns_per_unit_min;
}

function validateComparison(snapshot, cohort) {
  const workloads = ["dense_tables", "sparse_tables", "closures_upvalues", "generic_iteration", "stdlib_math_table"];
  const engineModes = {
    rustia: "interpreter",
    luaur: "interpreter",
    ruau: "interpreter",
    "mlua-luau": "interpreter",
    "mlua-luau-jit": "jit",
    koto: "interpreter",
    rhai: "interpreter",
    roto: "native",
  };
  assert.deepEqual(Object.keys(snapshot.workloads).sort(), [...workloads].sort(),
    "comparison requires all five retained workloads");
  return Object.entries(engineModes).map(([engine, mode]) => workloads.map(workload =>
    measuredTime(cohort, engine, mode, workload, snapshot.workloads[workload].unit)));
}

function selfTest(snapshot, history) {
  const cohort = readCohort(snapshot, history);
  const firstWorkload = Object.keys(snapshot.workloads)[0];
  const first = cohort.find(row => row.engine === "rustia" && row.workload === firstWorkload);
  assert.ok(first, "chart needs a Rustia measurement");
  const measuredTimes = data => validateComparison(snapshot, readCohort(snapshot, data));
  assert.throws(() => measuredTimes(history.filter(row => row !== first)), /expected one history row/);
  assert.throws(() => measuredTimes([...history, first]), /expected one history row/);
  for (const invalid of [{ host: "local" }, { execution_mode: "jit" }]) {
    assert.throws(() => measuredTimes(history.map(row => row === first ? { ...row, ...invalid } : row)),
      /ci-bench interpreter/);
  }
  assert.throws(() => measuredTimes(history.map(row => row === first ? { ...row, unit: "different work" } : row)),
    /unexpected history unit/);
  const jit = history.find(row => row.commit === snapshot.commit && row.engine === "mlua-luau-jit"
    && row.workload === first.workload && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
  const native = history.find(row => row.commit === snapshot.commit && row.engine === "roto"
    && row.workload === first.workload && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
  assert.ok(jit && native, "chart needs both compiled engines");
  assert.throws(() => measuredTimes([...history, jit]), /expected one history row/);
  for (const [row, change, failure] of [
    [jit, { commit: "0".repeat(40) }, /expected one history row/],
    [jit, { unit: "different work" }, /unexpected history unit/],
    [jit, { execution_mode: "interpreter" }, /ci-bench jit/],
    [jit, { mode_family: "rustia" }, /wrong execution-mode family/],
    [jit, { native_verification_ratio: 1 }, /verified Luau JIT/],
    [jit, { native_verified: false }, /verified Luau JIT/],
    [native, { native_verification: "not-applicable" }, /verified Cranelift/],
    [native, { native_verified: false }, /verified Cranelift/],
  ]) {
    assert.throws(() => measuredTimes(history.map(item => item === row ? { ...item, ...change } : item)), failure);
  }
  // Newer data must not change a chart for a selected cohort.
  assert.deepEqual(measuredTimes([...history, { ...first, timestamp_unix: snapshot.end + 604800 }]),
    measuredTimes(history));
}

try {
  const readme = fs.readFileSync(path.join(root, "README.md"), "utf8");
  const history = fs.readFileSync(path.join(root, "bench/history/runs.jsonl"), "utf8")
    .trim().split(/\r?\n/).map((line) => JSON.parse(line));
  checkSourceReferences(readme);
  const snapshot = JSON.parse(fs.readFileSync(path.join(root, "bench/history/site-comparison.json"), "utf8"));
  selfTest(snapshot, history);
  requireFact(process.argv.length === 2, "usage: node tools/check-readme-comparisons.js");
  validateComparison(snapshot, readCohort(snapshot, history));
  require("./sync-luau-comparison.js").syncComparison();
  console.log(`README sources reachable and website benchmark evidence valid at ${snapshot.commit.slice(0, 7)}`);
} catch (error) {
  console.error(`check-readme-comparisons: ${error.message}`);
  process.exit(1);
}
