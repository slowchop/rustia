#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");
const assert = require("assert/strict");

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
      ["run", "--locked", "-p", "script-bench-harness", "--", "--list-json"],
      {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`cargo run --locked -p script-bench-harness -- --list-json failed${stderr ? `\n${stderr}` : ""}`);
  }

  try {
    return JSON.parse(stdout);
  } catch (error) {
    fail(`script-bench --list-json did not emit valid JSON: ${error.message}`);
  }
}

function checkProgressMetadata(progress) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(progress.metadata.date)) {
    fail("design/progress.json metadata.date must use YYYY-MM-DD format");
  }
  if (!progress.metadata.excludes.includes("Actual grabby integration work")) {
    fail("design/progress.json metadata.excludes must keep grabby integration out of scope");
  }
}

// The README can omit counts or use everyday language. Any numeric claims it
// does display must still agree with the executable corpus, including bold or
// line-wrapped prose and either "named behaviors" or the engineering "refs".
function readmeConformanceProblems(readme, summary) {
  const prose = readme.replace(/```[\s\S]*?```/g, "").replace(/\*\*|__/g, "").replace(/\s+/g, " ");
  const rules = [
    [/\b([\d,]+)\s+fixtures\b/gi, summary.fixtures, "fixture count"],
    [/\b([\d,]+)\s+(?:named\s+behavio(?:u)?rs|refs)\b/gi, summary.refs, "named behavior count"],
    [/\b([\d,]+)\s+pending-oracle\b/gi, summary.pendingOracle, "pending oracle count"],
    [/\b([\d,]+)\s+pending-impl\b/gi, summary.pendingImpl, "pending implementation count"],
    [/\b([\d,]+)\s+pending\b(?!-)/gi, summary.pendingOracle + summary.pendingImpl, "pending case count"],
  ];
  const problems = [];
  for (const [pattern, expected, label] of rules) {
    for (const match of prose.matchAll(pattern)) {
      const actual = Number(match[1].replace(/,/g, ""));
      if (actual !== expected) problems.push(`README.md ${label}: corpus has ${expected}, README claims ${match[1]}`);
    }
  }
  return problems;
}

function testReadmeConformance(summary) {
  const valid = `**${summary.fixtures} fixtures** cover **${summary.refs} named\nbehaviors**, with **0 pending** cases.`;
  assert.deepEqual(readmeConformanceProblems(valid, summary), []);
  assert.deepEqual(readmeConformanceProblems("See the compatibility coverage guide.", summary), []);
  assert.deepEqual(readmeConformanceProblems(`${summary.refs} refs, 0 pending-oracle, 0 pending-impl`, summary), []);
  for (const [claim, expected] of [
    [`${summary.fixtures + 1} fixtures`, "fixture count"],
    [`${summary.refs + 1} named behaviors`, "named behavior count"],
    [`${summary.refs + 1} refs`, "named behavior count"],
    ["1 pending-oracle", "pending oracle count"],
    ["1 pending-impl", "pending implementation count"],
    ["1 pending", "pending case count"],
  ]) {
    const problems = readmeConformanceProblems(`**${claim}**`, summary);
    assert.equal(problems.length, 1, `must reject incorrect README claim: ${claim}`);
    assert.ok(problems[0].includes(expected), problems[0]);
  }
}

function checkConformance(progress, current, readme) {
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
    requireIncludes("design/CURRENT.md", current, fact, `current conformance fact ${fact}`);
  }
  testReadmeConformance(summary);
  for (const problem of readmeConformanceProblems(readme, summary)) fail(problem);
}

function checkBenchmark(progress, current) {
  const inventory = benchmarkInventory();
  const currentResults = readJson("bench/current-results.json");
  const facts = progress.metadata.canonical_facts.benchmark_adapters;
  const workloadFacts = progress.metadata.canonical_facts.benchmark_workloads;
  if (!workloadFacts) {
    fail("design/progress.json canonical_facts must include benchmark_workloads");
  }
  const implementedEngines = inventory.engines
    .filter((engine) => engine.implemented)
    .map((engine) => engine.mode_family || engine.id);
  const currentEngines = currentResults.rows.map((row) => row.mode_family || row.engine);

  assertEqual(facts.requested, facts.names.length, "requested benchmark adapter count");
  assertEqual(facts.present, facts.names.length, "present benchmark adapter count");
  assertArraySetEqual(new Set(implementedEngines), facts.names, "implemented benchmark adapter families");
  assertArraySetEqual(new Set(currentEngines), facts.names, "current-result benchmark adapter families");
  const jitSlot = inventory.engines.find((engine) => engine.id === "mlua-luau-jit");
  if (!jitSlot || jitSlot.mode_family !== "mlua-luau" || jitSlot.execution_mode !== "jit") {
    fail("benchmark inventory must expose mlua-luau-jit as the mlua-luau adapter family's JIT mode");
  }
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
  const resultKeys = currentResults.rows.map(
    (row) => `${row.target}\0${row.engine}\0${row.workload}`,
  );
  assertEqual(
    new Set(resultKeys).size,
    resultKeys.length,
    "current-results unique target/engine/workload rows",
  );
  assertArraySetEqual(
    new Set(currentResults.rows.map((row) => row.target)),
    currentResults.metadata.targets,
    "current-results target metadata coverage",
  );
  assertArraySetEqual(
    new Set(currentResults.rows.map((row) => row.engine)),
    currentResults.metadata.engines,
    "current-results engine metadata coverage",
  );
  assertArraySetEqual(
    new Set(currentResults.rows.map((row) => row.workload)),
    currentResults.metadata.workloads,
    "current-results workload metadata coverage",
  );

  requireIncludes(
    "design/CURRENT.md",
    current,
    "20 workload entries including cross-adapter `string_building` source coverage",
    "current benchmark workload coverage",
  );
  requireIncludes(
    "design/CURRENT.md",
    current,
    "approves no blanket speed headline",
    "generated benchmark claim posture",
  );
}

function checkPublishability(current) {
  const publishability = readJson("design/publishability.json");
  assertEqual(publishability.summary.package_count, 8, "publishability package count");
  assertEqual(publishability.summary.publish_false_count, 8, "publishability publish=false count");
  assertEqual(publishability.summary.blocked_count, 8, "publishability blocked count");
  requireIncludes(
    "design/CURRENT.md",
    current,
    "all 8 blocked by publish=false",
    "current publishability status",
  );
}

function checkReadmeEvidence(readme) {
  // The landing page is for people choosing a scripting engine. Keep the
  // engineering snapshot in design/ and link readers to the audited evidence
  // without prescribing progress tables, terminology, or sentence wording.
  const links = [...readme.matchAll(/\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g)];
  for (const [target, description] of [
    ["conformance/COVERAGE.md", "compatibility coverage"],
    ["bench/CLAIMS.md", "the generated benchmark claim limits"],
    ["design/90-release-readiness.md", "the release readiness checklist"],
  ]) {
    if (!links.some((match) => match[1].split(/[?#]/, 1)[0].endsWith(target))) {
      fail(`README.md must link to ${description}: ${target}`);
    }
  }
}

const progress = readJson("design/progress.json");
const readme = readText("README.md");
const current = readText("design/CURRENT.md");

checkProgressMetadata(progress);
checkConformance(progress, current, readme);
checkBenchmark(progress, current);
checkPublishability(current);
checkReadmeEvidence(readme);

const conformance = progress.metadata.canonical_facts.conformance;
const adapters = progress.metadata.canonical_facts.benchmark_adapters;
console.log(
  `project status facts ok: ${conformance.fixtures} fixtures, ${conformance.refs} refs, ${adapters.present} adapters`,
);
