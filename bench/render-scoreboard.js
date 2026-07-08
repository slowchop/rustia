#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = resolveArg("--source", path.join(__dirname, "current-results.json"));
const wasmSourcePath = resolveArg("--wasm-source", path.join(__dirname, "wasm", "current-results.json"));
const outputPath = resolveArg("--output", path.join(__dirname, "SCOREBOARD.md"));
const checkOnly = process.argv.includes("--check");

const workloadLabels = new Map([
  ["dense_tables", "dense"],
  ["sparse_tables", "sparse"],
  ["closures_upvalues", "closures"],
  ["generic_iteration", "generic"],
  ["stdlib_math_table", "stdlib math"],
]);

const engineOrder = [
  "rustia",
  "mlua-luau",
  "mlua-lua54",
  "koto",
  "rhai",
  "roto",
  "rquickjs",
  "boa",
  "wasmi",
  "wasmtime",
];

function fail(message) {
  console.error(`render-scoreboard: ${message}`);
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

function ensureCurrentResults(data) {
  if (data.schema_version !== 1 || data.schema !== "rustia.bench.current-results.v1") {
    fail(`${relative(sourcePath)} must use rustia.bench.current-results.v1`);
  }
  if (!Array.isArray(data.rows) || data.rows.length === 0) {
    fail(`${relative(sourcePath)} must contain rows`);
  }
  return data;
}

function ensureWasmResults(data) {
  if (data.schema_version !== 1 || data.schema !== "rustia.bench.wasm-gate-current-results.v1") {
    fail(`${relative(wasmSourcePath)} must use rustia.bench.wasm-gate-current-results.v1`);
  }
  if (!Array.isArray(data.runs)) {
    fail(`${relative(wasmSourcePath)} must contain runs`);
  }
  return data;
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

function formatUs(ns) {
  if (ns == null) {
    return "not supported";
  }
  const us = ns / 1000;
  if (us >= 1000) {
    return `${formatNumber(us, 0)} us`;
  }
  if (us >= 100) {
    return `${formatNumber(us, 1)} us`;
  }
  if (us >= 10) {
    return `${formatNumber(us, 2)} us`;
  }
  return `${formatNumber(us, 3)} us`;
}

function formatNumber(value, digits = 2) {
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: digits,
    minimumFractionDigits: 0,
  }).format(value);
}

function formatRatio(value) {
  if (value == null) {
    return "not supported";
  }
  if (value >= 100) {
    return `${formatNumber(value, 0)}x`;
  }
  if (value >= 10) {
    return `${formatNumber(value, 2)}x`;
  }
  return `${formatNumber(value, 3)}x`;
}

function formatIso(timestamp) {
  return new Date(timestamp * 1000).toISOString().replace("T", " ").replace(".000Z", " UTC");
}

function workloadHeader(workloads) {
  return workloads.map((workload) => workloadLabels.get(workload) || workload).join(" | ");
}

function workloadSeparator(workloads) {
  return workloads.map(() => "---:").join("|");
}

function workloadValues(row, workloads) {
  return workloads.map((workload) => formatUs(row.byWorkload.get(workload))).join(" | ");
}

function engineSortKey(engine) {
  const index = engineOrder.indexOf(engine);
  return index === -1 ? engineOrder.length : index;
}

function collectNativeTargets(data) {
  const workloads = data.metadata.workloads;
  const targets = [];
  for (const [target, rows] of groupBy(data.rows, (row) => row.target)) {
    const engines = [...new Set([...data.metadata.engines, ...rows.map((row) => row.engine)])]
      .sort((left, right) => engineSortKey(left) - engineSortKey(right) || left.localeCompare(right));
    targets.push({
      kind: "native",
      target,
      workloads,
      rows: collectTargetRows(rows, engines, workloads),
    });
  }
  return targets.sort((left, right) => left.target.localeCompare(right.target));
}

function collectWasmTargets(data, nativeEngines, nativeWorkloads) {
  const targets = [];
  for (const [target, runs] of groupBy(data.runs, (run) => run.target)) {
    const rows = [];
    for (const run of runs) {
      for (const row of run.rows || []) {
        rows.push({
          target,
          engine: run.adapter.id,
          dialect: run.adapter.dialect,
          workload: row.workload,
          unit: row.unit,
          commit: run.commit,
          timestamp_unix: run.timestamp_unix,
          ns_per_unit_median: row.ns_per_unit_median,
          ns_per_unit_min: row.ns_per_unit_min,
          n: row.n,
        });
      }
    }
    const engines = [...new Set([...nativeEngines, ...runs.map((run) => run.adapter.id)])]
      .sort((left, right) => engineSortKey(left) - engineSortKey(right) || left.localeCompare(right));
    targets.push({
      kind: "wasm",
      target,
      workloads: nativeWorkloads,
      rows: collectTargetRows(rows, engines, nativeWorkloads),
    });
  }
  return targets.sort((left, right) => left.target.localeCompare(right.target));
}

function collectTargetRows(rows, engines, workloads) {
  const byEngine = groupBy(rows, (row) => row.engine);
  const rustia = summarizeEngine("rustia", byEngine.get("rustia") || [], workloads);
  return engines
    .map((engine) => summarizeEngine(engine, byEngine.get(engine) || [], workloads, rustia.totalNs))
    .sort((left, right) => {
      if (left.totalNs == null && right.totalNs == null) {
        return engineSortKey(left.engine) - engineSortKey(right.engine) || left.engine.localeCompare(right.engine);
      }
      if (left.totalNs == null) {
        return 1;
      }
      if (right.totalNs == null) {
        return -1;
      }
      return left.totalNs - right.totalNs;
    });
}

function summarizeEngine(engine, rows, workloads, rustiaTotalNs) {
  const byWorkload = new Map();
  let dialect = null;
  let commit = null;
  let timestamp = null;
  for (const row of rows) {
    byWorkload.set(row.workload, row.ns_per_unit_median);
    dialect = dialect || row.dialect;
    commit = commit || row.commit;
    timestamp = Math.max(timestamp || 0, row.timestamp_unix || 0);
  }
  const values = workloads.map((workload) => byWorkload.get(workload)).filter((value) => value != null);
  const complete = values.length === workloads.length;
  const totalNs = complete ? values.reduce((sum, value) => sum + value, 0) : null;
  return {
    engine,
    dialect: dialect || "not supported",
    commit: commit || "not measured",
    timestamp,
    measured: values.length,
    byWorkload,
    totalNs,
    ratio: totalNs != null && rustiaTotalNs != null ? rustiaTotalNs / totalNs : null,
    status: complete ? "measured" : values.length === 0 ? "not supported" : `${values.length}/${workloads.length} measured`,
  };
}

function targetSection(target) {
  let text = `## ${target.target}\n\n`;
  text += target.kind === "wasm"
    ? "Wasm adapter results for this runtime target. Missing adapters are shown as `not supported`.\n\n"
    : "Native/host benchmark results. Total score is the sum of per-workload medians; lower is better.\n\n";
  text += `| rank | lang/engine | dialect | ratio to rustia | total score (lower better) | measured | ${workloadHeader(target.workloads)} | commit | status |\n`;
  text += `|---:|---|---|---:|---:|---:|${workloadSeparator(target.workloads)}|---|---|\n`;
  let rank = 0;
  for (const row of target.rows) {
    if (row.totalNs != null) {
      rank += 1;
    }
    text += `| ${row.totalNs == null ? "-" : rank} | ${row.engine} | ${row.dialect} | ${formatRatio(row.ratio)} | ${formatUs(row.totalNs)} | ${row.measured}/${target.workloads.length} | ${workloadValues(row, target.workloads)} | ${row.commit} | ${row.status} |\n`;
  }
  return `${text}\n`;
}

function render(nativeData, wasmData) {
  const nativeTargets = collectNativeTargets(nativeData);
  const wasmTargets = collectWasmTargets(wasmData, nativeData.metadata.engines, nativeData.metadata.workloads);
  const window = nativeData.metadata.measurement_window_unix;
  let text = `# Benchmark Scoreboard

Latest generated benchmark scoreboard for rustia.

- Native source: [current-results.json](current-results.json)
- Wasm source: [wasm/current-results.json](wasm/current-results.json)
- History: \`${nativeData.metadata.history}\`
- Native rows: ${nativeData.metadata.row_count}
- Native measurement window: ${formatIso(window.start)} to ${formatIso(window.end)}

How to read this:

- **ratio to rustia** is based on the total score for that target; higher is faster.
- **total score** is the sum of median ns/unit for all listed workloads, displayed in microseconds; lower is better.
- Missing wasm adapters are shown as \`not supported\` so unsupported engines are visible instead of silently absent.

## Native Targets

`;
  for (const target of nativeTargets) {
    text += targetSection(target);
  }
  text += "## Wasm Targets\n\n";
  for (const target of wasmTargets) {
    text += targetSection(target);
  }
  return text;
}

const nativeData = ensureCurrentResults(readJson(sourcePath));
const wasmData = ensureWasmResults(readJson(wasmSourcePath));
const output = render(nativeData, wasmData);

if (checkOnly) {
  let existing;
  try {
    existing = fs.readFileSync(outputPath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(outputPath)}: ${error.message}`);
  }
  if (existing !== output) {
    fail(`${relative(outputPath)} is stale; run node bench/render-scoreboard.js`);
  }
  console.log(`${relative(outputPath)} is current`);
} else {
  fs.writeFileSync(outputPath, output);
  console.log(`wrote ${relative(outputPath)}`);
}
