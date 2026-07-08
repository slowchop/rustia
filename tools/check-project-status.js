#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");

function fail(message) {
  console.error(`check-project-status: ${message}`);
  process.exit(1);
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function relative(filePath) {
  return slash(path.relative(root, filePath));
}

function readText(relativePath) {
  const filePath = path.join(root, relativePath);
  try {
    return fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(filePath)}: ${error.message}`);
  }
}

function readJson(relativePath) {
  try {
    return JSON.parse(readText(relativePath));
  } catch (error) {
    fail(`could not parse ${relativePath}: ${error.message}`);
  }
}

function requireIncludes(relativePath, text, needle, description) {
  if (!text.includes(needle)) {
    fail(`${relativePath} must mention ${description}: ${JSON.stringify(needle)}`);
  }
}

function assertEqual(actual, expected, description) {
  if (actual !== expected) {
    fail(`${description}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertArraySetEqual(actual, expected, description) {
  const left = [...actual].sort();
  const right = [...expected].sort();
  if (left.join("\n") !== right.join("\n")) {
    fail(`${description}: expected ${right.join(", ")}, got ${left.join(", ")}`);
  }
}

function dataRows(relativePath) {
  return readText(relativePath)
    .split(/\r?\n/)
    .filter((line) => line !== "" && !line.startsWith("#"));
}

function conformanceSummary() {
  const manifestRows = dataRows("conformance/MANIFEST.tsv");
  const refsRows = dataRows("conformance/REFS.tsv");
  const oracleCounts = new Map();

  for (const [index, row] of manifestRows.entries()) {
    const fields = row.split("\t");
    if (fields.length !== 7) {
      fail(`conformance/MANIFEST.tsv row ${index + 2} must have 7 tab-separated columns`);
    }
    oracleCounts.set(fields[4], (oracleCounts.get(fields[4]) || 0) + 1);
  }

  for (const [index, row] of refsRows.entries()) {
    const fields = row.split("\t");
    if (fields.length !== 5) {
      fail(`conformance/REFS.tsv row ${index + 2} must have 5 tab-separated columns`);
    }
  }

  return {
    fixtures: manifestRows.length,
    refs: refsRows.length,
    luau0709: oracleCounts.get("luau-0.709") || 0,
    pendingOracle: oracleCounts.get("pending-oracle") || 0,
    pendingImpl: oracleCounts.get("pending-impl") || 0,
    rustiaOnly: oracleCounts.get("rustia-only") || 0,
  };
}

function benchmarkInventory() {
  let stdout;
  try {
    stdout = childProcess.execFileSync(
      "cargo",
      ["run", "-p", "script-bench-harness", "--", "--list-json"],
      {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`cargo run -p script-bench-harness -- --list-json failed${stderr ? `\n${stderr}` : ""}`);
  }

  try {
    return JSON.parse(stdout);
  } catch (error) {
    fail(`script-bench --list-json did not emit valid JSON: ${error.message}`);
  }
}

function checkProgressGoals(progress, readme) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(progress.metadata.date)) {
    fail("design/progress.json metadata.date must use YYYY-MM-DD format");
  }
  if (!progress.metadata.excludes.includes("Actual grabby integration work")) {
    fail("design/progress.json metadata.excludes must keep grabby integration out of scope");
  }
  requireIncludes(
    "README.md",
    readme,
    `as of ${progress.metadata.date}`,
    "the progress snapshot date",
  );
  requireIncludes(
    "README.md",
    readme,
    "[`design/progress.md`](design/progress.md)",
    "the generated Markdown progress report",
  );
  requireIncludes(
    "README.md",
    readme,
    "actual grabby integration work is explicitly out of scope",
    "the grabby integration exclusion",
  );

  for (const goal of progress.overall_goals) {
    requireIncludes(
      "README.md",
      readme,
      `| ${goal.name} | ${goal.complete}% | ${goal.confidence}% |`,
      `${goal.name} progress row from design/progress.json`,
    );
  }
}

function checkConformance(progress, readme, current) {
  const summary = conformanceSummary();
  const expected = progress.metadata.canonical_facts.conformance;
  assertEqual(expected.fixtures, summary.fixtures, "progress conformance fixture count");
  assertEqual(expected.refs, summary.refs, "progress conformance ref count");
  assertEqual(
    expected.pending,
    summary.pendingOracle + summary.pendingImpl,
    "progress conformance pending count",
  );
  assertEqual(summary.pendingOracle, 0, "conformance pending-oracle rows");
  assertEqual(summary.pendingImpl, 0, "conformance pending-impl rows");
  assertEqual(summary.rustiaOnly, 0, "conformance rustia-only rows");

  const facts = [
    `${summary.fixtures} fixtures`,
    `${summary.refs} refs`,
    `${summary.pendingOracle} pending-oracle`,
    `${summary.pendingImpl} pending-impl`,
  ];
  for (const fact of facts) {
    requireIncludes("README.md", readme, fact, `current conformance fact ${fact}`);
    requireIncludes("design/CURRENT.md", current, fact, `current conformance fact ${fact}`);
  }
  requireIncludes("README.md", readme, `${expected.pending} pending`, "compact pending summary");
}

function checkBenchmark(progress, readme, current) {
  const inventory = benchmarkInventory();
  const currentResults = readJson("bench/current-results.json");
  const facts = progress.metadata.canonical_facts.benchmark_adapters;
  const workloadFacts = progress.metadata.canonical_facts.benchmark_workloads;
  if (!workloadFacts) {
    fail("design/progress.json canonical_facts must include benchmark_workloads");
  }
  const implementedEngines = inventory.engines
    .filter((engine) => engine.implemented)
    .map((engine) => engine.id);
  const currentEngines = currentResults.metadata.engines;

  assertEqual(facts.requested, facts.names.length, "requested benchmark adapter count");
  assertEqual(facts.present, facts.names.length, "present benchmark adapter count");
  assertArraySetEqual(implementedEngines, facts.names, "implemented benchmark adapters");
  assertArraySetEqual(currentEngines, facts.names, "current-result benchmark adapters");
  assertEqual(inventory.workloads.length, workloadFacts.present, "benchmark inventory workload count");
  assertEqual(
    inventory.workloads.filter((workload) => workload.default).length,
    workloadFacts.default,
    "benchmark default workload count",
  );
  for (const workloadId of workloadFacts.source_covered_non_default) {
    const workload = inventory.workloads.find((entry) => entry.id === workloadId);
    if (!workload) {
      fail(`benchmark inventory must include ${workloadId}`);
    }
    assertEqual(workload.default, false, `${workloadId} default flag`);
  }
  const currentWorkloadCount = currentResults.metadata.workloads.length;
  assertEqual(currentWorkloadCount, workloadFacts.current_results, "current-results workload count");
  assertEqual(currentResults.metadata.row_count, currentResults.rows.length, "current-results row count");
  assertEqual(
    currentResults.rows.length,
    currentEngines.length * currentWorkloadCount * currentResults.metadata.targets.length,
    "current-results target/engine/workload coverage",
  );

  requireIncludes(
    "README.md",
    readme,
    `all ${facts.present} requested adapters`,
    "requested benchmark adapter count",
  );
  requireIncludes("README.md", readme, "`string_building`", "string_building benchmark status");
  requireIncludes(
    "design/CURRENT.md",
    current,
    "12 workload entries including cross-adapter `string_building` source coverage",
    "current benchmark workload coverage",
  );
  requireIncludes(
    "design/CURRENT.md",
    current,
    "approves no blanket speed headline",
    "generated benchmark claim posture",
  );
}

function checkPublishability(readme, current) {
  const publishability = readJson("design/publishability.json");
  assertEqual(publishability.summary.package_count, 8, "publishability package count");
  assertEqual(publishability.summary.publish_false_count, 8, "publishability publish=false count");
  assertEqual(publishability.summary.blocked_count, 8, "publishability blocked count");
  requireIncludes(
    "README.md",
    readme,
    "Current publish blockers are generated at",
    "generated publishability blockers",
  );
  requireIncludes(
    "design/CURRENT.md",
    current,
    "all 8 blocked by publish=false",
    "current publishability status",
  );
}

const progress = readJson("design/progress.json");
const readme = readText("README.md");
const current = readText("design/CURRENT.md");

checkProgressGoals(progress, readme);
checkConformance(progress, readme, current);
checkBenchmark(progress, readme, current);
checkPublishability(readme, current);

const conformance = progress.metadata.canonical_facts.conformance;
const adapters = progress.metadata.canonical_facts.benchmark_adapters;
console.log(
  `project status facts ok: ${conformance.fixtures} fixtures, ${conformance.refs} refs, ${adapters.present} adapters`,
);
