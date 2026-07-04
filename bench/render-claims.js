#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = resolveArg("--source", path.join(__dirname, "current-results.json"));
const outputPath = resolveArg("--output", path.join(__dirname, "CLAIMS.md"));
const checkOnly = process.argv.includes("--check");

function fail(message) {
  console.error(`render-claims: ${message}`);
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

function readData() {
  let data;
  try {
    data = JSON.parse(fs.readFileSync(sourcePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(sourcePath)}: ${error.message}`);
  }
  if (data.schema_version !== 1 || data.schema !== "rustia.bench.current-results.v1") {
    fail("current-results JSON must use rustia.bench.current-results.v1");
  }
  if (!data.metadata || typeof data.metadata !== "object") {
    fail("metadata is required");
  }
  if (!Array.isArray(data.rows) || data.rows.length === 0) {
    fail("rows must contain at least one result");
  }
  for (const key of ["targets", "engines", "workloads"]) {
    if (!Array.isArray(data.metadata[key]) || data.metadata[key].length === 0) {
      fail(`metadata.${key} must be a non-empty array`);
    }
  }
  data.rows.forEach(validateRow);
  return data;
}

function validateRow(row, index) {
  const label = `rows[${index}]`;
  for (const key of ["target", "engine", "dialect", "workload", "unit", "commit"]) {
    if (typeof row[key] !== "string" || row[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  for (const key of ["timestamp_unix", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec", "n"]) {
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

function targetKind(target) {
  const value = String(target);
  if (value.startsWith("wasm32")) {
    return "wasm32";
  }
  for (const candidate of ["aarch64", "arm64", "x86_64", "x86", "arm32", "armv7"]) {
    if (value.includes(candidate)) {
      return candidate;
    }
  }
  const parts = value.split("-").filter(Boolean);
  return parts.length === 0 ? value : parts[parts.length - 1];
}

function formatTime(timestampUnix) {
  return new Date(timestampUnix * 1000).toISOString().replace("T", " ").replace(".000Z", " UTC");
}

function markdownCell(value) {
  return String(value).replace(/\|/g, "\\|").replace(/\n/g, " ");
}

function claimPosture(rustiaRow, luauRow) {
  if (!rustiaRow) {
    return { ready: false, text: "blocked: missing rustia baseline" };
  }
  if (!luauRow || !Number.isFinite(luauRow.speed_vs_rustia)) {
    return { ready: false, text: "blocked: missing mlua-luau baseline" };
  }
  if (luauRow.speed_vs_rustia > 1.05) {
    return {
      ready: false,
      text: `not public-ready: rustia trails mlua-luau by ${formatSpeed(luauRow.speed_vs_rustia)}`,
    };
  }
  if (luauRow.speed_vs_rustia >= 0.95) {
    return { ready: true, text: "eligible: rustia is near mlua-luau for this workload" };
  }
  return {
    ready: true,
    text: `eligible: rustia leads mlua-luau by ${formatSpeed(1 / luauRow.speed_vs_rustia)}`,
  };
}

function summarizeWorkloads(data) {
  const groups = [...groupBy(data.rows, (row) => `${row.target}\0${row.workload}`).entries()]
    .map(([key, rows]) => {
      const [target, workload] = key.split("\0");
      const sorted = rows.slice().sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min);
      const rustiaRow = rows.find((row) => row.engine === "rustia");
      const luauRow = rows.find((row) => row.engine === "mlua-luau");
      const kotoRow = rows.find((row) => row.engine === "koto");
      const posture = claimPosture(rustiaRow, luauRow);
      return {
        target,
        workload,
        unit: rustiaRow ? rustiaRow.unit : sorted[0].unit,
        rustiaRow,
        luauRow,
        kotoRow,
        fastest: sorted[0],
        rustiaRank: rustiaRow ? sorted.findIndex((row) => row.engine === "rustia") + 1 : null,
        engineCount: sorted.length,
        posture,
      };
    });
  return groups.sort((left, right) => (
    left.target.localeCompare(right.target) ||
    left.workload.localeCompare(right.workload)
  ));
}

function summarizeEngines(data) {
  const workloadGroups = groupBy(data.rows, (row) => `${row.target}\0${row.workload}`);
  const bestCounts = new Map();
  for (const rows of workloadGroups.values()) {
    const sorted = rows.slice().sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min);
    const key = `${sorted[0].engine}\0${sorted[0].target}`;
    bestCounts.set(key, (bestCounts.get(key) ?? 0) + 1);
  }
  return [...groupBy(data.rows, (row) => `${row.engine}\0${row.target}`).entries()]
    .map(([key, rows]) => {
      const [engine, target] = key.split("\0");
      return {
        engine,
        target,
        targetKind: targetKind(target),
        geomeanVsRustia: geometricMean(rows.map((row) => row.speed_vs_rustia)),
        bestCount: bestCounts.get(key) ?? 0,
        workloadCount: unique(rows.map((row) => row.workload)).length,
      };
    })
    .sort((left, right) => {
      const leftSpeed = left.geomeanVsRustia ?? -Infinity;
      const rightSpeed = right.geomeanVsRustia ?? -Infinity;
      return (
        rightSpeed - leftSpeed ||
        left.engine.localeCompare(right.engine) ||
        left.target.localeCompare(right.target)
      );
    });
}

function render(data) {
  const claims = summarizeWorkloads(data);
  const engines = summarizeEngines(data);
  const readyCount = claims.filter((claim) => claim.posture.ready).length;
  const allReady = readyCount === claims.length;
  const window = data.metadata.measurement_window_unix;
  const windowText = window
    ? `${formatTime(window.start)} to ${formatTime(window.end)}`
    : "n/a";
  const headline = allReady
    ? "eligible only as per-workload claims; do not collapse to one multiplier"
    : "not approved from this snapshot";
  const reason = allReady
    ? "Every current workload is at or above the mlua-luau reference threshold, but claims still need per-workload wording."
    : `${claims.length - readyCount} of ${claims.length} current workload claims are not public-ready against the mlua-luau reference row.`;

  const workloadRows = claims.map((claim) => [
    claim.target,
    claim.workload,
    claim.unit,
    claim.rustiaRow ? formatNumber(claim.rustiaRow.ns_per_unit_min) : "n/a",
    claim.rustiaRank == null ? "n/a" : `${claim.rustiaRank} / ${claim.engineCount}`,
    claim.fastest.engine,
    formatSpeed(claim.fastest.speed_vs_rustia),
    claim.luauRow ? formatSpeed(claim.luauRow.speed_vs_rustia) : "n/a",
    claim.kotoRow ? formatSpeed(claim.kotoRow.speed_vs_rustia) : "n/a",
    claim.posture.text,
  ]);

  const engineRows = engines.map((engine) => [
    `${engine.engine} (${engine.targetKind})`,
    formatSpeed(engine.geomeanVsRustia),
    `${engine.bestCount} / ${engine.workloadCount}`,
    engine.target,
    `${engine.workloadCount} workloads`,
  ]);

  return `# script-bench-harness Benchmark Claims

Generated from [current-results.json](current-results.json) by [render-claims.js](render-claims.js).

This is a release-gate aid, not raw benchmark data. Keep public benchmark language
per workload and per target. Do not publish a single blanket multiplier from this
snapshot.

## Snapshot

- Source history: \`${data.metadata.history}\`
- Rows: ${data.metadata.row_count}
- Targets: ${data.metadata.targets.map((target) => `\`${target}\``).join(", ")}
- Workloads: ${data.metadata.workloads.map((workload) => `\`${workload}\``).join(", ")}
- Measurement window: ${windowText}
- Global public speed headline: **${headline}**
- Reason: ${reason}

## Per-Workload Claim Gate

| target | workload | unit | rustia min ns/unit | rustia rank | fastest engine | fastest vs rustia | mlua-luau vs rustia | koto vs rustia | claim posture |
|---|---|---|---:|---:|---|---:|---:|---:|---|
${workloadRows.map((row) => `| ${row.map(markdownCell).join(" | ")} |`).join("\n")}

## Engineering Context Only

These geomeans are useful for triage, but they are not public headline claims.

| engine target | geomean vs rustia | fastest workloads | target detail | coverage |
|---|---:|---:|---|---|
${engineRows.map((row) => `| ${row.map(markdownCell).join(" | ")} |`).join("\n")}
`;
}

function main() {
  const data = readData();
  const output = render(data).replace(/[ \t]+$/gm, "").replace(/\n+$/, "\n");
  if (checkOnly) {
    let current;
    try {
      current = fs.readFileSync(outputPath, "utf8");
    } catch (error) {
      fail(`could not read ${relative(outputPath)}: ${error.message}`);
    }
    if (current !== output) {
      fail(`${relative(outputPath)} is stale; run node bench/render-claims.js`);
    }
    console.log(`${relative(outputPath)} is current`);
  } else {
    fs.writeFileSync(outputPath, output, "utf8");
    console.log(`wrote ${relative(outputPath)} from ${relative(sourcePath)}`);
  }
}

main();
