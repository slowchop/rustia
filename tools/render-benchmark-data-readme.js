#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = resolveArg("--source", path.join(root, "bench", "current-results.json"));
const historyPath = resolveArg("--history", path.join(root, "bench", "history", "runs.jsonl"));
const wasmSourcePath = resolveArg("--wasm-source", path.join(root, "bench", "wasm", "current-results.json"));
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

function readOptionalJson(filePath) {
  if (!fs.existsSync(filePath)) {
    return null;
  }
  return readJson(filePath);
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

function validateWasmData(data) {
  if (data == null) {
    return;
  }
  if (data.schema_version !== 1 || data.schema !== "rustia.bench.wasm-gate-current-results.v1") {
    fail("wasm current results must use rustia.bench.wasm-gate-current-results.v1");
  }
  if (!Array.isArray(data.runs)) {
    fail("wasm current results runs must be an array");
  }
  data.runs.forEach(validateWasmRun);
}

function validateWasmRun(run, index) {
  const label = `wasm runs[${index}]`;
  if (run.schema_version !== 1 || run.schema !== "rustia.bench.wasm-runtime-results.v1") {
    fail(`${label} must use rustia.bench.wasm-runtime-results.v1`);
  }
  for (const key of ["target", "commit"]) {
    if (typeof run[key] !== "string" || run[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  if (!Number.isFinite(run.timestamp_unix) || run.timestamp_unix <= 0) {
    fail(`${label}.timestamp_unix must be a positive finite number`);
  }
  if (!run.runtime || typeof run.runtime !== "object" || Array.isArray(run.runtime)) {
    fail(`${label}.runtime must be an object`);
  }
  for (const key of ["name", "family", "host"]) {
    if (typeof run.runtime[key] !== "string" || run.runtime[key].trim() === "") {
      fail(`${label}.runtime.${key} must be a non-empty string`);
    }
  }
  if (!run.adapter || typeof run.adapter !== "object" || Array.isArray(run.adapter)) {
    fail(`${label}.adapter must be an object`);
  }
  for (const key of ["id", "dialect"]) {
    if (typeof run.adapter[key] !== "string" || run.adapter[key].trim() === "") {
      fail(`${label}.adapter.${key} must be a non-empty string`);
    }
  }
  if (!Array.isArray(run.rows) || run.rows.length === 0) {
    fail(`${label}.rows must be a non-empty array`);
  }
  run.rows.forEach((row, rowIndex) => validateWasmRow(row, `${label}.rows[${rowIndex}]`));
}

function validateWasmRow(row, label) {
  for (const key of ["engine", "dialect", "workload", "unit", "checksum"]) {
    if (typeof row[key] !== "string" || row[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  for (const key of ["n", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
    if (!Number.isFinite(row[key]) || row[key] <= 0) {
      fail(`${label}.${key} must be a positive finite number`);
    }
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

function targetSummaries(data, targetLabels = new Map()) {
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
    return {
      target,
      label: targetLabels.get(target) ?? target,
      rowsByEngine,
      workloadCount: workloadGroups.size,
    };
  });
}

function workloadLeaders(data, targetLabels = new Map()) {
  return [...groupBy(data.rows, (row) => `${row.target}\0${row.workload}`).entries()]
    .map(([key, rows]) => {
      const [target, workload] = key.split("\0");
      const sorted = rows.slice().sort((left, right) => left.ns_per_unit_median - right.ns_per_unit_median);
      const rustia = rows.find((row) => row.engine === "rustia");
      const rank = rustia ? sorted.findIndex((row) => row.engine === "rustia") + 1 : null;
      return {
        target,
        label: targetLabels.get(target) ?? target,
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

function flattenWasmData(wasmData) {
  if (wasmData == null) {
    return null;
  }
  const rows = [];
  const targetLabels = new Map();
  for (const run of wasmData.runs) {
    targetLabels.set(
      run.target,
      `${run.target} (${run.runtime.family}/${run.runtime.name} on ${run.runtime.host})`,
    );
    for (const row of run.rows) {
      rows.push({
        target: run.target,
        engine: run.adapter.id,
        dialect: run.adapter.dialect,
        workload: row.workload,
        unit: row.unit,
        commit: run.commit,
        timestamp_unix: run.timestamp_unix,
        n: row.n,
        ns_per_unit_min: row.ns_per_unit_min,
        ns_per_unit_median: row.ns_per_unit_median,
        units_per_sec: row.units_per_sec,
        checksum: row.checksum,
        speed_vs_rustia: null,
      });
    }
  }

  for (const rowsForWorkload of groupBy(rows, (row) => `${row.target}\0${row.workload}`).values()) {
    const rustia = rowsForWorkload.find((row) => row.engine === "rustia");
    if (!rustia) {
      continue;
    }
    for (const row of rowsForWorkload) {
      row.speed_vs_rustia = rustia.ns_per_unit_median / row.ns_per_unit_median;
    }
  }

  return {
    rows,
    targetLabels,
    metadata: {
      targets: unique(rows.map((row) => row.target)),
      engines: unique(rows.map((row) => row.engine)),
      workloads: unique(rows.map((row) => row.workload)),
      commits: unique(rows.map((row) => row.commit)),
    },
  };
}

function renderComparisonSections(lines, title, description, data, targetLabels = new Map()) {
  lines.push(`## ${title}`);
  lines.push("");
  lines.push(description);
  lines.push("");

  for (const target of targetSummaries(data, targetLabels)) {
    lines.push(`### ${target.label}`);
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
}

function renderWorkloadLeaders(lines, title, data, targetLabels = new Map()) {
  const leaders = workloadLeaders(data, targetLabels);
  lines.push(`## ${title}`);
  lines.push("");
  lines.push("| target | workload | fastest engine | fastest median ns/unit | rustia median ns/unit | rustia rank |");
  lines.push("| --- | --- | --- | ---: | ---: | ---: |");
  for (const row of leaders) {
    lines.push([
      `| ${markdownCell(row.label)}`,
      markdownCell(row.workload),
      markdownCell(row.fastest.engine),
      formatNumber(row.fastest.ns_per_unit_median),
      row.rustia ? formatNumber(row.rustia.ns_per_unit_median) : "n/a",
      `${row.rustiaRank ? `${row.rustiaRank}/${row.engineCount}` : "n/a"} |`,
    ].join(" | "));
  }
  lines.push("");
}

function render(data, historyCount, wasmData) {
  const window = data.metadata.measurement_window_unix;
  const commits = data.metadata.commits ?? [];
  const wasm = flattenWasmData(wasmData);
  const lines = [];

  lines.push("# rustia benchmark data");
  lines.push("");
  lines.push("Generated by the `slowchop/rustia` benchmark workflow. Do not edit this file by hand.");
  lines.push("");
  lines.push(`- History records: ${formatNumber(historyCount)}`);
  lines.push(`- Current rows: ${formatNumber(data.rows.length)}`);
  lines.push(`- Targets: ${data.metadata.targets.map(markdownCell).join(", ")}`);
  if (wasm) {
    lines.push(`- Wasm adapter rows: ${formatNumber(wasm.rows.length)}`);
    lines.push(`- Wasm adapter targets: ${wasm.metadata.targets.map((target) => markdownCell(wasm.targetLabels.get(target) ?? target)).join(", ")}`);
  }
  lines.push(`- Workloads: ${data.metadata.workloads.map(markdownCell).join(", ")}`);
  lines.push(`- Source commits in current snapshot: ${commits.map((commit) => `\`${commit}\``).join(", ")}`);
  if (window) {
    lines.push(`- Measurement window: ${formatTime(window.start)} to ${formatTime(window.end)}`);
  }
  lines.push("");
  renderComparisonSections(
    lines,
    "Native/host comparison",
    "Speed is the geometric mean of latest rows on that native target, relative to Rustia. Higher is faster. Bars are log-scaled.",
    data,
  );

  if (wasm) {
    renderComparisonSections(
      lines,
      "Wasm adapter comparison",
      "These rows are wasm adapter runs grouped by runtime target. They are intentionally separate from native targets, so an adapter's wasm result is never mixed with its native-host result.",
      wasm,
      wasm.targetLabels,
    );
  }

  renderWorkloadLeaders(lines, "Native workload leaders", data);
  if (wasm) {
    renderWorkloadLeaders(lines, "Wasm workload leaders", wasm, wasm.targetLabels);
  }

  lines.push("## Files");
  lines.push("");
  lines.push("- [`bench/history/runs.jsonl`](bench/history/runs.jsonl): active coherent native measurement snapshot");
  lines.push("- [`bench/current-results.json`](bench/current-results.json): latest rows by target, engine, and workload");
  lines.push("- [`bench/RESULTS.md`](bench/RESULTS.md): generated detailed result table");
  lines.push("- [`bench/current-results.html`](bench/current-results.html): standalone rendered report");
  lines.push("- [`bench/grabby-builds/results.json`](bench/grabby-builds/results.json): selected Grabby application build results and paired samples, separate from Rustia workloads");
  lines.push("- [`bench/CLAIMS.md`](bench/CLAIMS.md): generated public-claim gate");
  lines.push("- [`bench/operations-status.json`](bench/operations-status.json): publication, progress, and landscape freshness");
  if (wasm) {
    lines.push("- [`bench/wasm/current-results.json`](bench/wasm/current-results.json): wasm adapter result snapshot");
    lines.push("- [`bench/wasm/dispatch-current-results.json`](bench/wasm/dispatch-current-results.json): wasm dispatch microbenchmark snapshot");
    lines.push("- [`bench/wasm-gate.json`](bench/wasm-gate.json): wasm pre-lock gate tracker");
  }
  lines.push("");

  return `${lines.join("\n")}`;
}

if (!outputPath) {
  fail("--output is required so the source repository README is not overwritten accidentally");
}

const data = readJson(sourcePath);
validateData(data);
const wasmData = readOptionalJson(wasmSourcePath);
validateWasmData(wasmData);
const historyCount = readHistoryCount(historyPath);
const rendered = render(data, historyCount, wasmData);

if (checkOnly) {
  let existing;
  try {
    existing = fs.readFileSync(outputPath, "utf8");
  } catch (error) {
    fail(`could not read ${outputPath}: ${error.message}`);
  }
  if (existing !== rendered) {
    fail(`${outputPath} is stale; run node tools/render-benchmark-data-readme.js --source ${sourcePath} --history ${historyPath} --wasm-source ${wasmSourcePath} --output ${outputPath}`);
  }
  console.log(`${outputPath} is current`);
} else {
  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, rendered);
  console.log(`wrote ${outputPath}`);
}
