#!/usr/bin/env node
"use strict";

const assert = require("assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("fs");
const path = require("path");

const { measuredTime, renderPerformance } = require("./render-readme-performance.js");

const root = path.resolve(__dirname, "..");

// Keep the friendly description tied to the harness's exact unit. Timings
// still come only from history; no benchmark values are duplicated here.
const workUnits = {
  dense_tables: ["Build, sum, and sort a 256-item array", "build+sum+sort of 256-elem array"],
  sparse_tables: ["Fill and read 128 hash entries", "fill+read 128 hash entries"],
  closures_upvalues: ["One closure call updating a captured value", "one upvalue-mutating closure call"],
  generic_iteration: ["Custom, keyed, and array iteration pass", "__iter(32)+pairs(5)+ipairs(32) pass"],
  stdlib_math_table: ["Math library calls and buffered appends", "math fastcalls + buffered push"],
};

function requireFact(condition, message) {
  if (!condition) throw new Error(message);
}

function tableCells(line) {
  requireFact(/^\|.*\|$/.test(line), "snapshot table must use pipe-delimited rows");
  return line.slice(1, -1).split("|").map((cell) => cell.trim());
}

// Installation pins and immutable/current source links must resolve in this checkout's
// history. Measurement source IDs remain unchanged: the history projection is
// provenance, not an automatic rewrite of a user's Cargo dependency or URL.
function verifySourceReferences(readme, checkReference) {
  const dependencies = [...readme.matchAll(/\brustia\s*=\s*\{[^}\n]*\}/g)]
    .map((match) => match[0])
    .filter((value) => /git\s*=\s*"https:\/\/github\.com\/slowchop\/rustia(?:\.git)?"/.test(value));
  requireFact(dependencies.length === 1, "README must contain one pinned Rustia Git dependency");
  const pin = dependencies[0].match(/\brev\s*=\s*"([a-f0-9]{40})"/);
  requireFact(pin, "README Git dependency needs a full source commit SHA");
  checkReference(pin[1], null);
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
  assert.throws(() => verifySourceReferences(readme.replace(/rev = "[a-f0-9]{40}"/, 'rev = "main"'), checkReference),
    /full source commit SHA/);
  assert.throws(() => verifySourceReferences(readme.replace(/rev = "[a-f0-9]{40}"/, `rev = "${"f".repeat(40)}"`), checkReference),
    /source commit is unreachable/);
  assert.throws(() => verifySourceReferences(readme.replace(/rev = "[a-f0-9]{40}"/, 'branch = "main"'), checkReference),
    /full source commit SHA/);
  const immutable = readme.match(/https:\/\/github\.com\/slowchop\/rustia\/blob\/([a-f0-9]{40})\/[^\s)>]+/);
  requireFact(immutable, "README must link immutable benchmark evidence");
  assert.throws(() => verifySourceReferences(readme.replace(immutable[0],
    `https://github.com/slowchop/rustia/blob/${immutable[1]}/__missing_readme_artifact__`), checkReference),
    /source target is absent/);
  assert.throws(() => verifySourceReferences(readme.replace(immutable[1], "f".repeat(40)), checkReference),
    /source commit is unreachable/);
}

// A dated README may remain useful after weekly current-results publication.
// Match its immutable source cohort in history; never require today's latest row.
function verify(readme, history) {
  const markers = [...readme.matchAll(/<!-- rustia-benchmark-snapshot\s*([\s\S]*?)-->/g)];
  requireFact(markers.length === 1, "README must identify exactly one benchmark snapshot");
  const marker = markers[0];
  const snapshot = JSON.parse(marker[1]);
  requireFact(/^[a-f0-9]{40}$/.test(snapshot.commit), "snapshot needs a full source commit SHA");
  requireFact(typeof snapshot.target === "string" && snapshot.target.length > 0, "snapshot needs a target");
  requireFact(Number.isSafeInteger(snapshot.start) && Number.isSafeInteger(snapshot.end)
    && snapshot.start <= snapshot.end, "snapshot needs a valid measurement window");
  for (const field of ["engines", "workloads"]) {
    requireFact(Array.isArray(snapshot[field]) && snapshot[field].length > 0
      && snapshot[field].every((value) => typeof value === "string" && value.length > 0)
      && new Set(snapshot[field]).size === snapshot[field].length, `snapshot ${field} must be unique names`);
  }
  const before = readme.slice(0, marker.index).split("### Speed: a measured snapshot\n").at(-1);
  const date = new Date(snapshot.start * 1000).toISOString().slice(0, 10);
  requireFact(new Date(snapshot.end * 1000).toISOString().startsWith(date), "snapshot must name one UTC date");
  requireFact(before.includes(`${date} UTC at source commit \`${snapshot.commit.slice(0, 7)}\``),
    "visible snapshot date or source commit disagrees with evidence marker");
  requireFact(before.includes("Microseconds per work unit; lower is better")
    && before.includes("best observed") && before.includes("interpreter")
    && before.includes("Linux x86-64") && snapshot.target === "linux-x86_64"
    && before.includes("`ci-bench`"), "snapshot must explain units, statistic, mode, and target");

  const table = readme.slice(marker.index + marker[0].length).trimStart()
    .split(/\r?\n/).filter((line, index, lines) => index < lines.findIndex((value) => !value.startsWith("|")));
  requireFact(table.length === snapshot.workloads.length + 2, "snapshot table workload count disagrees with marker");
  const headers = tableCells(table[0]);
  requireFact(headers.length === snapshot.engines.length + 1, "snapshot table engine count disagrees with marker");
  const identity = (name) => name.toLowerCase().replace(/[^a-z0-9]/g, "");
  requireFact(headers.slice(1).every((header, index) => identity(header) === identity(snapshot.engines[index])),
    "snapshot table engine headers disagree with marker");
  const cohort = history.filter((row) => row.commit === snapshot.commit && row.target === snapshot.target
    && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
  let count = 0;
  snapshot.workloads.forEach((workload, rowIndex) => {
    const cells = tableCells(table[rowIndex + 2]);
    requireFact(cells.length === headers.length, `unexpected column count for ${workload}`);
    const workUnit = workUnits[workload];
    requireFact(workUnit, `no checked work-unit description for ${workload}`);
    requireFact(cells[0] === workUnit[0], `${workload}: work-unit label must be ${JSON.stringify(workUnit[0])}`);
    snapshot.engines.forEach((engine, columnIndex) => {
      const ns = measuredTime(cohort, engine, "interpreter", workload, workUnit[1]);
      const expected = (ns / 1000).toFixed(3);
      requireFact(cells[columnIndex + 1] === expected,
        `${engine}/${workload}: README has ${cells[columnIndex + 1]}, history requires ${expected} microseconds`);
      count += 1;
    });
  });
  return { count, snapshot, cohort };
}

function selfTest(readme, history) {
  const { snapshot } = verify(readme, history);
  const first = history.find((row) => row.commit === snapshot.commit
    && row.target === snapshot.target
    && row.engine === snapshot.engines[0] && row.workload === snapshot.workloads[0]
    && row.timestamp_unix >= snapshot.start && row.timestamp_unix <= snapshot.end);
  const value = (first.ns_per_unit_min / 1000).toFixed(3);
  assert.throws(() => verify(readme.replace(`| ${value} |`, "| 0.001 |"), history), /history requires/);
  const source = `${new Date(snapshot.start * 1000).toISOString().slice(0, 10)} UTC at source commit \`${snapshot.commit.slice(0, 7)}\``;
  const wrongSource = source.replace(snapshot.commit.slice(0, 7), "0000000");
  assert.throws(() => verify(readme.replace(source, wrongSource), history), /visible snapshot/);
  // The install example can legitimately pin the same revision as the table.
  // Mutate the visible provenance phrase, never an earlier dependency SHA.
  const matchingDependency = readme.replace(/rev = "[a-f0-9]{40}"/, `rev = "${snapshot.commit}"`);
  verify(matchingDependency, history);
  assert.throws(() => verify(matchingDependency.replace(source, wrongSource), history), /visible snapshot/);
  assert.throws(() => verify(readme, history.filter((row) => row !== first)), /expected one history row/);
  assert.throws(() => verify(readme, [...history, first]), /expected one history row/);
  const firstLabel = workUnits[snapshot.workloads[0]][0];
  const secondLabel = workUnits[snapshot.workloads[1]][0];
  assert.throws(() => verify(readme.replace(firstLabel, "Unrelated work"), history), /work-unit label/);
  const swappedLabels = readme.replace(firstLabel, "__swapped_work_unit__")
    .replace(secondLabel, firstLabel).replace("__swapped_work_unit__", secondLabel);
  assert.throws(() => verify(swappedLabels, history), /work-unit label/);
  assert.throws(() => verify(readme, history.map((row) => row === first ? { ...row, unit: "different work" } : row)),
    /unexpected history unit/);
  for (const invalid of [{ host: "local" }, { execution_mode: "jit" }]) {
    assert.throws(() => verify(readme, history.map((row) => row === first ? { ...row, ...invalid } : row)),
      /ci-bench interpreter/);
  }
  // The figure includes compiled engines that the interpreter-only table omits.
  const chart = data => {
    const { cohort } = verify(readme, data);
    return renderPerformance(snapshot, cohort, workUnits);
  };
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
  // Weekly publication is independent: newer rows must not invalidate a dated table.
  verify(readme, [...history, { ...first, timestamp_unix: snapshot.end + 604800 }]);
}

try {
  const readme = fs.readFileSync(path.join(root, "README.md"), "utf8");
  const history = fs.readFileSync(path.join(root, "bench/history/runs.jsonl"), "utf8")
    .trim().split(/\r?\n/).map((line) => JSON.parse(line));
  checkSourceReferences(readme);
  selfTest(readme, history);
  const { count, snapshot, cohort } = verify(readme, history);
  const file = path.join(root, "crates", "rustia", "performance.svg");
  const svg = renderPerformance(snapshot, cohort, workUnits);
  const args = process.argv.slice(2);
  if (args.length === 1 && args[0] === "--write-chart") {
    fs.writeFileSync(file, svg);
  } else {
    requireFact(args.length === 0, "usage: node tools/check-readme-comparisons.js [--write-chart]");
    requireFact(fs.readFileSync(file, "utf8") === svg,
      "crates/rustia/performance.svg has drifted; run node tools/check-readme-comparisons.js --write-chart");
    const pin = readme.match(/^!\[[^\]\n]+\]\(https:\/\/github\.com\/slowchop\/rustia\/raw\/([a-f0-9]{40})\/crates\/rustia\/performance\.svg\)$/m);
    requireFact(pin, "README must display performance.svg at an immutable source revision");
    const linkedSvg = execFileSync("git", ["show", `${pin[1]}:crates/rustia/performance.svg`], { cwd: root, encoding: "utf8" });
    requireFact(linkedSvg === svg, "README chart pin must point to the version generated from its benchmark snapshot");
  }
  console.log(`README sources reachable; ${count} table cells and the comparison chart match ci-bench history at ${snapshot.commit.slice(0, 7)}`);
} catch (error) {
  console.error(`check-readme-comparisons: ${error.message}`);
  process.exit(1);
}
