#!/usr/bin/env node

"use strict";

const fs = require("node:fs");
const path = require("node:path");

const METRICS = [
  ["peak_delta_kib", "Peak Δ", "KiB", false],
  ["quiescent_retained_delta_anon_kib", "Retained Δ", "KiB", false],
  ["tail_growth_kib_per_epoch", "Tail growth", "KiB/epoch", false],
  ["recovery_ratio", "Recovery", "%", true],
];

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { selfTest: true };
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value == null) fail(`${flag ?? "argument"} requires a value`);
    if (values.has(flag)) fail(`${flag} was supplied twice`);
    values.set(flag, value);
  }
  const take = (name) => {
    const value = values.get(name);
    values.delete(name);
    if (value == null) fail(`missing ${name}`);
    return path.resolve(value);
  };
  const config = {
    current: take("--current"),
    history: take("--history"),
    output: take("--output"),
  };
  if (values.size !== 0) fail(`unknown argument(s): ${[...values.keys()].join(", ")}`);
  return config;
}

function readJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot read ${label}: ${error.message}`);
  }
}

function readHistory(file) {
  return fs.readFileSync(file, "utf8")
    .split(/\r?\n/)
    .filter((line) => line.trim() !== "")
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        fail(`history line ${index + 1} is invalid: ${error.message}`);
      }
    });
}

function identity(row) {
  return [
    row.target,
    row.engine,
    row.execution_mode,
    row.workload,
    row.n,
    row.epochs,
  ].join("\0");
}

function median(row, metric) {
  const value = row?.metrics?.[metric]?.median;
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function formatValue(value, unit) {
  if (value == null) return "—";
  if (unit === "%") return `${(value * 100).toFixed(1)}%`;
  if (Math.abs(value) >= 100) return value.toFixed(0);
  if (Math.abs(value) >= 10) return value.toFixed(1);
  return value.toFixed(2);
}

function formatChange(before, after, higherIsBetter) {
  if (before == null || after == null) return "—";
  if (before === 0) {
    if (after === 0) return "0%";
    return `${after > 0 ? "+" : ""}${after.toFixed(2)}`;
  }
  const percent = ((after - before) / Math.abs(before)) * 100;
  const better = higherIsBetter ? percent > 0 : percent < 0;
  const worse = higherIsBetter ? percent < 0 : percent > 0;
  const marker = better ? "↓better" : worse ? "↑worse" : "same";
  return `${percent >= 0 ? "+" : ""}${percent.toFixed(1)}% ${marker}`;
}

function summarize(current, history) {
  if (current?.status !== "ready" || !Array.isArray(current.rows) || current.rows.length === 0) {
    fail("current Rustia CI memory results are not ready");
  }
  // The summary is the commit-over-commit Rustia comparison, whichever cohort
  // the lane measured: a cross-engine cohort's competitor rows are the
  // dashboard's business, not this table's.
  const currentRows = current.rows.filter((row) => row.engine === "rustia");
  if (currentRows.length === 0) fail("current CI memory result contains no Rustia rows");
  const cohortId = current.metadata?.cohort_id;
  const previousRows = new Map();
  let comparisonCohort = null;
  for (const cohort of history) {
    if (cohort.cohort_id === cohortId || !Array.isArray(cohort.rows)) continue;
    for (const row of cohort.rows) {
      if (row.engine !== "rustia") continue;
      const key = identity(row);
      if (currentRows.some((candidate) => identity(candidate) === key)) {
        previousRows.set(key, row);
        comparisonCohort = cohort;
      }
    }
  }
  if (previousRows.size === 0 || comparisonCohort == null) {
    fail("no prior comparable Rustia memory cohort exists");
  }

  const lines = [
    "# Rustia memory on the owned host",
    "",
    `Commit \`${current.metadata.commit.slice(0, 12)}\` compared with ` +
      `\`${comparisonCohort.commit.slice(0, 12)}\` (${comparisonCohort.cohort_id}). ` +
      "Rustia rows only; the cohorts compared may differ in which other engines they measured.",
    "",
    "| Workload | Metric | Before | After | Change |",
    "|---|---:|---:|---:|---:|",
  ];
  for (const row of [...currentRows].sort((left, right) => left.workload.localeCompare(right.workload))) {
    const before = previousRows.get(identity(row));
    if (!before) continue;
    for (const [metric, label, unit, higherIsBetter] of METRICS) {
      const beforeValue = median(before, metric);
      const afterValue = median(row, metric);
      lines.push(
        `| ${row.workload} | ${label} (${unit}) | ${formatValue(beforeValue, unit)} | ` +
          `${formatValue(afterValue, unit)} | ${formatChange(beforeValue, afterValue, higherIsBetter)} |`,
      );
    }
  }
  lines.push("", "Lower is better except for recovery.");
  return `${lines.join("\n")}\n`;
}

function main() {
  const config = parseArgs(process.argv.slice(2));
  if (config.selfTest) {
    const metric = (median) => ({ median });
    const row = (commit, peak, engine = "rustia") => ({
      commit,
      target: "linux-x86_64",
      engine,
      execution_mode: "interpreter",
      workload: "memory_churn",
      n: 1,
      epochs: 40,
      metrics: {
        peak_delta_kib: metric(peak),
        quiescent_retained_delta_anon_kib: metric(32),
        tail_growth_kib_per_epoch: metric(0),
        recovery_ratio: metric(0.75),
      },
    });
    const current = {
      status: "ready",
      metadata: { cohort_id: "current", commit: "b".repeat(40) },
      rows: [row("b".repeat(40), 50)],
    };
    const history = [{
      cohort_id: "baseline",
      commit: "a".repeat(40),
      rows: [row("a".repeat(40), 100)],
    }];
    const markdown = summarize(current, history);
    if (!markdown.includes("| memory_churn | Peak Δ (KiB) | 100 | 50.0 | -50.0% ↓better |")) {
      fail("self-test summary did not report the expected improvement");
    }
    // A cross-engine cohort carries competitor rows; they are left out of the
    // table rather than aborting the job after the measurement succeeded.
    const crossEngine = {
      status: "ready",
      metadata: { cohort_id: "cross-engine", commit: "c".repeat(40) },
      rows: [row("c".repeat(40), 25), row("c".repeat(40), 999, "mlua-luau-jit")],
    };
    const crossMarkdown = summarize(crossEngine, [
      ...history,
      { cohort_id: "current", commit: "b".repeat(40), rows: current.rows },
    ]);
    if (
      !crossMarkdown.includes("| memory_churn | Peak Δ (KiB) | 50.0 | 25.0 | -50.0% ↓better |") ||
      crossMarkdown.includes("999") ||
      crossMarkdown.includes("mlua-luau-jit")
    ) {
      fail("self-test summary did not leave the competitor row out");
    }
    let refused = false;
    try {
      summarize({ ...crossEngine, rows: [row("c".repeat(40), 999, "mlua-luau-jit")] }, history);
    } catch (error) {
      refused = /no Rustia rows/.test(error.message);
    }
    if (!refused) fail("self-test summarised a cohort without Rustia rows");
    process.stdout.write("Rustia CI memory summary self-test passed\n");
    return;
  }
  const markdown = summarize(readJson(config.current, "current results"), readHistory(config.history));
  fs.writeFileSync(config.output, markdown);
  process.stdout.write(markdown);
}

try {
  main();
} catch (error) {
  process.stderr.write(`summarize-rustia-ci-memory: ${error.message}\n`);
  process.exitCode = 1;
}
