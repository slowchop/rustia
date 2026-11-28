#!/usr/bin/env node
"use strict";

const assert = require("assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("fs");
const path = require("path");

const { renderPerformance } = require("./render-readme-performance.js");

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

function selfTest(snapshot, history) {
  const cohort = readCohort(snapshot, history);
  const firstWorkload = Object.keys(snapshot.workloads)[0];
  const first = cohort.find(row => row.engine === "rustia" && row.workload === firstWorkload);
  assert.ok(first, "chart needs a Rustia measurement");
  const chart = data => renderPerformance(snapshot, readCohort(snapshot, data));
  assert.throws(() => chart(history.filter(row => row !== first)), /expected one history row/);
  assert.throws(() => chart([...history, first]), /expected one history row/);
  for (const invalid of [{ host: "local" }, { execution_mode: "jit" }]) {
    assert.throws(() => chart(history.map(row => row === first ? { ...row, ...invalid } : row)),
      /ci-bench interpreter/);
  }
  assert.throws(() => chart(history.map(row => row === first ? { ...row, unit: "different work" } : row)),
    /unexpected history unit/);
  const jit = history.find(row => row.commit === snapshot.commit && row.engine === "mlua-luau-jit"
    && row.workload === first.workload && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
  const native = history.find(row => row.commit === snapshot.commit && row.engine === "roto"
    && row.workload === first.workload && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
  assert.ok(jit && native, "chart needs both compiled engines");
  assert.throws(() => chart([...history, jit]), /expected one history row/);
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
    assert.throws(() => chart(history.map(item => item === row ? { ...item, ...change } : item)), failure);
  }
  // Newer data must not change a chart for a selected cohort.
  assert.equal(chart([...history, { ...first, timestamp_unix: snapshot.end + 604800 }]), chart(history));
}

try {
  const readme = fs.readFileSync(path.join(root, "README.md"), "utf8");
  const history = fs.readFileSync(path.join(root, "bench/history/runs.jsonl"), "utf8")
    .trim().split(/\r?\n/).map((line) => JSON.parse(line));
  checkSourceReferences(readme);
  const snapshot = JSON.parse(fs.readFileSync(path.join(root, "bench/history/site-comparison.json"), "utf8"));
  selfTest(snapshot, history);
  const file = path.join(root, "crates", "rustia", "performance.svg");
  const svg = renderPerformance(snapshot, readCohort(snapshot, history));
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "--write-chart") {
    fs.writeFileSync(file, svg);
  } else {
    requireFact(args.length === 0, "usage: node tools/check-readme-comparisons.js [--write-chart]");
    requireFact(fs.readFileSync(file, "utf8").replaceAll("\r\n", "\n") === svg,
      "crates/rustia/performance.svg has drifted; run node tools/check-readme-comparisons.js --write-chart");
    require("./sync-luau-comparison.js").syncComparison();
  }
  console.log(`README sources reachable; the comparison chart matches ci-bench history at ${snapshot.commit.slice(0, 7)}`);
} catch (error) {
  console.error(`check-readme-comparisons: ${error.message}`);
  process.exit(1);
}
