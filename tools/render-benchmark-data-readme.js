#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = resolveArg("--source", path.join(root, "bench", "current-results.json"));
const historyPath = resolveArg("--history", path.join(root, "bench", "history", "runs.jsonl"));
const outputPath = argValue("--output");
const checkOnly = process.argv.includes("--check");

function fail(message) {
  console.error(`render-benchmark-data-readme: ${message}`);
  process.exit(1);
}

function argValue(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) {
    return null;
  }
  const value = process.argv[index + 1];
  if (value == null || value.startsWith("--")) {
    fail(`${name} requires a path`);
  }
  return value;
}

function resolveArg(name, defaultPath) {
  const value = argValue(name);
  if (value == null) {
    return defaultPath;
  }
  return path.isAbsolute(value) ? value : path.resolve(root, value);
}

function relative(filePath) {
  return path.relative(root, filePath).split(path.sep).join("/");
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(filePath)}: ${error.message}`);
  }
}

function readHistoryCount(filePath) {
  let text;
  try {
    text = fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(filePath)}: ${error.message}`);
  }
  return text.split(/\r?\n/).filter((line) => line.trim() !== "").length;
}

function validateData(data) {
  if (data.schema_version !== 1 || data.schema !== "rustia.bench.current-results.v1") {
    fail("current results must use rustia.bench.current-results.v1");
  }
  if (!data.metadata || typeof data.metadata !== "object" || Array.isArray(data.metadata)) {
    fail("metadata must be an object");
  }
  for (const key of ["targets", "engines", "workloads", "commits"]) {
    if (!Array.isArray(data.metadata[key])) {
      fail(`metadata.${key} must be an array`);
    }
  }
  if (!Array.isArray(data.rows) || data.rows.length === 0) {
    fail("rows must contain at least one result");
  }
  if (data.metadata.row_count !== data.rows.length) {
    fail("metadata.row_count must match rows.length");
  }
  data.rows.forEach(validateRow);
}

function validateRow(row, index) {
  const label = `rows[${index}]`;
  for (const key of ["target", "engine", "dialect", "workload", "unit", "commit"]) {
    if (typeof row[key] !== "string" || row[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  for (const key of ["timestamp_unix", "n", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
    if (!Number.isFinite(row[key]) || row[key] <= 0) {
      fail(`${label}.${key} must be a positive finite number`);
    }
  }
  if (row.speed_vs_rustia != null && (!Number.isFinite(row.speed_vs_rustia) || row.speed_vs_rustia <= 0)) {
    fail(`${label}.speed_vs_rustia must be null or a positive finite number`);
  }
}

function groupBy(values, keyFn) {
  const groups = new Map();
  for (const value of values) {
    const key = keyFn(value);
    const group = groups.get(key);
    if (group) {
      group.push(value);
    } else {
      groups.set(key, [value]);
    }
  }
  return groups;
}

function unique(values) {
  return [...new Set(values)].sort((left, right) => left.localeCompare(right));
}

function geometricMean(values) {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  if (finite.length === 0) {
    return null;
  }
  return Math.exp(finite.reduce((sum, value) => sum + Math.log(value), 0) / finite.length);
}

function formatNumber(value) {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: value >= 100 ? 0 : value >= 10 ? 2 : 3,
  }).format(value);
}

function formatSpeed(value) {
  return Number.isFinite(value) ? `${formatNumber(value)}x` : "n/a";
}

function formatTime(timestampUnix) {
  if (!Number.isFinite(timestampUnix) || timestampUnix <= 0) {
    return "n/a";
  }
  return new Date(timestampUnix * 1000).toISOString().replace("T", " ").replace(".000Z", " UTC");
}

function markdownCell(value) {
  return String(value).replace(/\|/g, "\\|").replace(/\n/g, " ");
}

function comparisonBar(speed) {
  if (!Number.isFinite(speed) || speed <= 0) {
    return "";
  }
  const minLog = -4;
  const maxLog = 8;
  const value = Math.max(minLog, Math.min(maxLog, Math.log2(speed)));
  const length = Math.max(1, Math.round(((value - minLog) / (maxLog - minLog)) * 32));
  return "#".repeat(length);
}

function targetSummaries(data) {
  return unique(data.rows.map((row) => row.target)).map((target) => {
    const targetRows = data.rows.filter((row) => row.target === target);
    const workloadGroups = groupBy(targetRows, (row) => row.workload);
    const wins = new Map();
    for (const rows of workloadGroups.values()) {
      const fastest = rows.slice().sort((left, right) => left.ns_per_unit_median - right.ns_per_unit_median)[0];
      wins.set(fastest.engine, (wins.get(fastest.engine) ?? 0) + 1);
    }
    const rowsByEngine = [...groupBy(targetRows, (row) => row.engine).entries()]
      .map(([engine, rows]) => ({
        engine,
        geomeanVsRustia: geometricMean(rows.map((row) => row.speed_vs_rustia)),
        workloads: unique(rows.map((row) => row.workload)).length,
        wins: wins.get(engine) ?? 0,
      }))
      .sort((left, right) => {
        const leftSpeed = left.geomeanVsRustia ?? -Infinity;
        const rightSpeed = right.geomeanVsRustia ?? -Infinity;
        return rightSpeed - leftSpeed || left.engine.localeCompare(right.engine);
      });
    return { target, rowsByEngine, workloadCount: workloadGroups.size };
  });
}

function workloadLeaders(data) {
  return [...groupBy(data.rows, (row) => `${row.target}\0${row.workload}`).entries()]
    .map(([key, rows]) => {
      const [target, workload] = key.split("\0");
      const sorted = rows.slice().sort((left, right) => left.ns_per_unit_median - right.ns_per_unit_median);
      const rustia = rows.find((row) => row.engine === "rustia");
      const rank = rustia ? sorted.findIndex((row) => row.engine === "rustia") + 1 : null;
      return {
        target,
        workload,
        fastest: sorted[0],
        rustia,
        rustiaRank: rank,
        engineCount: sorted.length,
      };
    })
    .sort((left, right) => (
      left.target.localeCompare(right.target) ||
      left.workload.localeCompare(right.workload)
    ));
}

function render(data, historyCount) {
  const window = data.metadata.measurement_window_unix;
  const commits = data.metadata.commits ?? [];
  const targets = targetSummaries(data);
  const leaders = workloadLeaders(data);
  const lines = [];

  lines.push("# rustia benchmark data");
  lines.push("");
  lines.push("Generated by the `slowchop/rustia` benchmark workflow. Do not edit this file by hand.");
  lines.push("");
  lines.push(`- History records: ${formatNumber(historyCount)}`);
  lines.push(`- Current rows: ${formatNumber(data.rows.length)}`);
  lines.push(`- Targets: ${data.metadata.targets.map(markdownCell).join(", ")}`);
  lines.push(`- Workloads: ${data.metadata.workloads.map(markdownCell).join(", ")}`);
  lines.push(`- Source commits in current snapshot: ${commits.map((commit) => `\`${commit}\``).join(", ")}`);
  if (window) {
    lines.push(`- Measurement window: ${formatTime(window.start)} to ${formatTime(window.end)}`);
  }
  lines.push("");
  lines.push("## Latest comparison");
  lines.push("");
  lines.push("Speed is the geometric mean of latest rows on that target, relative to Rustia. Higher is faster. Bars are log-scaled.");
  lines.push("");

  for (const target of targets) {
    lines.push(`### ${target.target}`);
    lines.push("");
    lines.push("| rank | engine | geomean vs rustia | fastest workloads | measured workloads | chart |");
    lines.push("| ---: | --- | ---: | ---: | ---: | --- |");
    target.rowsByEngine.forEach((row, index) => {
      lines.push([
        `| ${index + 1}`,
        markdownCell(row.engine),
        formatSpeed(row.geomeanVsRustia),
        `${row.wins}/${target.workloadCount}`,
        row.workloads,
        `\`${comparisonBar(row.geomeanVsRustia)}\` |`,
      ].join(" | "));
    });
    lines.push("");
  }

  lines.push("## Workload leaders");
  lines.push("");
  lines.push("| target | workload | fastest engine | fastest median ns/unit | rustia median ns/unit | rustia rank |");
  lines.push("| --- | --- | --- | ---: | ---: | ---: |");
  for (const row of leaders) {
    lines.push([
      `| ${markdownCell(row.target)}`,
      markdownCell(row.workload),
      markdownCell(row.fastest.engine),
      formatNumber(row.fastest.ns_per_unit_median),
      row.rustia ? formatNumber(row.rustia.ns_per_unit_median) : "n/a",
      `${row.rustiaRank ? `${row.rustiaRank}/${row.engineCount}` : "n/a"} |`,
    ].join(" | "));
  }
  lines.push("");
  lines.push("## Files");
  lines.push("");
  lines.push("- [`bench/history/runs.jsonl`](bench/history/runs.jsonl): append-only measurement history");
  lines.push("- [`bench/current-results.json`](bench/current-results.json): latest rows by target, engine, and workload");
  lines.push("- [`bench/RESULTS.md`](bench/RESULTS.md): generated detailed result table");
  lines.push("- [`bench/current-results.html`](bench/current-results.html): standalone rendered report");
  lines.push("- [`bench/CLAIMS.md`](bench/CLAIMS.md): generated public-claim gate");
  lines.push("");

  return `${lines.join("\n")}`;
}

if (!outputPath) {
  fail("--output is required so the source repository README is not overwritten accidentally");
}

const data = readJson(sourcePath);
validateData(data);
const historyCount = readHistoryCount(historyPath);
const rendered = render(data, historyCount);

if (checkOnly) {
  let existing;
  try {
    existing = fs.readFileSync(outputPath, "utf8");
  } catch (error) {
    fail(`could not read ${outputPath}: ${error.message}`);
  }
  if (existing !== rendered) {
    fail(`${outputPath} is stale; run node tools/render-benchmark-data-readme.js --source ${sourcePath} --history ${historyPath} --output ${outputPath}`);
  }
  console.log(`${outputPath} is current`);
} else {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, rendered);
  console.log(`wrote ${outputPath}`);
}
