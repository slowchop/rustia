#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = path.join(__dirname, "current-results.json");
const outputPath = path.join(__dirname, "current-results.html");

function fail(message) {
  console.error(`render-current-results: ${message}`);
  process.exit(1);
}

function readData() {
  let data;
  try {
    data = JSON.parse(fs.readFileSync(sourcePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(sourcePath)}: ${error.message}`);
  }

  if (data.schema_version !== 1) {
    fail("schema_version must be 1");
  }
  if (data.schema !== "rustia.bench.current-results.v1") {
    fail('schema must be "rustia.bench.current-results.v1"');
  }
  if (data.source !== "script-bench-harness") {
    fail('source must be "script-bench-harness"');
  }
  if (data.metadata == null || typeof data.metadata !== "object" || Array.isArray(data.metadata)) {
    fail("metadata must be an object");
  }
  for (const key of ["targets", "engines", "workloads", "commits"]) {
    if (!Array.isArray(data.metadata[key])) {
      fail(`metadata.${key} must be an array`);
    }
  }
  if (!Array.isArray(data.rows)) {
    fail("rows must be an array");
  }
  if (!Number.isInteger(data.metadata.row_count) || data.metadata.row_count !== data.rows.length) {
    fail("metadata.row_count must match rows.length");
  }
  if (data.rows.length === 0) {
    fail("rows must contain at least one result");
  }
  if (typeof data.metadata.history !== "string" || data.metadata.history.trim() === "") {
    fail("metadata.history must be a non-empty string");
  }
  if (typeof data.metadata.history_schema !== "string" || data.metadata.history_schema.trim() === "") {
    fail("metadata.history_schema must be a non-empty string");
  }
  const window = data.metadata.measurement_window_unix;
  if (window == null || typeof window !== "object" || Array.isArray(window)) {
    fail("metadata.measurement_window_unix must be an object");
  }
  if (!Number.isFinite(window.start) || !Number.isFinite(window.end) || window.start <= 0 || window.end < window.start) {
    fail("metadata.measurement_window_unix must contain positive start/end values");
  }

  data.rows.forEach(validateRow);
  validateMetadataMatchesRows(data);
  return data;
}

function validateRow(row, index) {
  const label = `rows[${index}]`;
  for (const key of ["commit", "target", "engine", "dialect", "workload", "unit", "checksum"]) {
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

function validateMetadataMatchesRows(data) {
  const expected = {
    targets: unique(data.rows.map((row) => row.target)),
    engines: unique(data.rows.map((row) => row.engine)),
    workloads: unique(data.rows.map((row) => row.workload)),
    commits: unique(data.rows.map((row) => row.commit)),
  };

  for (const [key, values] of Object.entries(expected)) {
    const actual = data.metadata[key].slice().sort((left, right) => left.localeCompare(right));
    if (JSON.stringify(actual) !== JSON.stringify(values)) {
      fail(`metadata.${key} must match rows`);
    }
  }

  const timestamps = data.rows.map((row) => row.timestamp_unix);
  if (
    data.metadata.measurement_window_unix.start !== Math.min(...timestamps) ||
    data.metadata.measurement_window_unix.end !== Math.max(...timestamps)
  ) {
    fail("metadata.measurement_window_unix must match rows");
  }
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function relative(filePath) {
  return path.relative(root, filePath);
}

function unique(values) {
  return [...new Set(values)].sort((left, right) => left.localeCompare(right));
}

function formatNumber(value) {
  if (!Number.isFinite(value)) {
    return "n/a";
  }
  return new Intl.NumberFormat("en-US", {
    maximumFractionDigits: value >= 100 ? 0 : value >= 10 ? 1 : 3,
  }).format(value);
}

function formatInteger(value) {
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(value);
}

function formatTime(timestampUnix) {
  return new Date(timestampUnix * 1000).toISOString().replace("T", " ").replace(".000Z", " UTC");
}

function formatSpeed(value) {
  return Number.isFinite(value) ? `${formatNumber(value)}x` : "n/a";
}

function slug(value) {
  return String(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
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

function geometricMean(values) {
  const finite = values.filter((value) => Number.isFinite(value) && value > 0);
  if (finite.length === 0) {
    return null;
  }
  return Math.exp(finite.reduce((sum, value) => sum + Math.log(value), 0) / finite.length);
}

function compareRows(left, right) {
  return (
    left.target.localeCompare(right.target) ||
    left.workload.localeCompare(right.workload) ||
    left.ns_per_unit_min - right.ns_per_unit_min ||
    left.engine.localeCompare(right.engine)
  );
}

function summarize(data) {
  const rows = data.rows.slice().sort(compareRows);
  const timestamps = rows.map((row) => row.timestamp_unix);
  const targetRows = [...groupBy(rows, (row) => row.target)]
    .map(([target, values]) => ({ target, values }))
    .sort((left, right) => left.target.localeCompare(right.target));
  const engineRows = [...groupBy(rows, (row) => row.engine)]
    .map(([engine, values]) => ({
      engine,
      targetCount: unique(values.map((row) => row.target)).length,
      workloadCount: unique(values.map((row) => row.workload)).length,
      bestCount: 0,
      geoSpeed: geometricMean(values.map((row) => row.speed_vs_rustia)),
      medianNs: median(values.map((row) => row.ns_per_unit_min)),
    }))
    .sort((left, right) => {
      const leftSpeed = left.geoSpeed ?? -Infinity;
      const rightSpeed = right.geoSpeed ?? -Infinity;
      return rightSpeed - leftSpeed || left.engine.localeCompare(right.engine);
    });

  for (const group of groupBy(rows, (row) => `${row.target}\0${row.workload}`).values()) {
    group.sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min);
    const winner = engineRows.find((engine) => engine.engine === group[0].engine);
    if (winner) {
      winner.bestCount += 1;
    }
  }

  return {
    rows,
    targetRows,
    engineRows,
    minTimestamp: Math.min(...timestamps),
    maxTimestamp: Math.max(...timestamps),
  };
}

function median(values) {
  const sorted = values
    .filter((value) => Number.isFinite(value))
    .sort((left, right) => left - right);
  if (sorted.length === 0) {
    return null;
  }
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 1
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
}

function metric(value, label) {
  return `
    <div class="metric">
      <strong>${escapeHtml(value)}</strong>
      <span>${escapeHtml(label)}</span>
    </div>
  `;
}

function engineSummaryTable(engineRows, totalMatchups) {
  const maxSpeed = Math.max(...engineRows.map((row) => row.geoSpeed ?? 0), 1);
  return `
    <section class="panel">
      <div class="section-head">
        <div>
          <h2>Engine Summary</h2>
          <p>Geomean uses available speed-vs-rustia ratios from the current snapshot.</p>
        </div>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Engine</th>
              <th>Geomean vs rustia</th>
              <th>Best results</th>
              <th>Median min ns/unit</th>
              <th>Coverage</th>
            </tr>
          </thead>
          <tbody>
            ${engineRows.map((row) => {
              const width = row.geoSpeed ? Math.max(2, (row.geoSpeed / maxSpeed) * 100) : 0;
              return `
                <tr>
                  <td><strong>${escapeHtml(row.engine)}</strong></td>
                  <td>
                    <div class="bar-cell">
                      <span>${formatSpeed(row.geoSpeed)}</span>
                      <div class="bar" aria-hidden="true"><span style="width: ${width.toFixed(3)}%"></span></div>
                    </div>
                  </td>
                  <td>${formatInteger(row.bestCount)} / ${formatInteger(totalMatchups)}</td>
                  <td>${formatNumber(row.medianNs)}</td>
                  <td>${formatInteger(row.targetCount)} targets, ${formatInteger(row.workloadCount)} workloads</td>
                </tr>
              `;
            }).join("")}
          </tbody>
        </table>
      </div>
    </section>
  `;
}

function workloadSection(target, workload, rows) {
  const sorted = rows.slice().sort((left, right) => left.ns_per_unit_min - right.ns_per_unit_min || left.engine.localeCompare(right.engine));
  const fastest = sorted[0].ns_per_unit_min;
  const slowest = sorted[sorted.length - 1].ns_per_unit_min;
  const maxSpeed = Math.max(...sorted.map((row) => row.speed_vs_rustia ?? 0), 1);
  return `
    <section class="workload" id="${escapeHtml(slug(`${target}-${workload}`))}">
      <div class="workload-head">
        <div>
          <h3>${escapeHtml(workload)}</h3>
          <p>${escapeHtml(sorted[0].unit)} · fastest ${formatNumber(fastest)} ns/unit · slowest ${formatNumber(slowest)} ns/unit</p>
        </div>
      </div>
      <div class="bars">
        ${sorted.map((row) => resultBar(row, fastest, maxSpeed)).join("")}
      </div>
    </section>
  `;
}

function resultBar(row, fastest, maxSpeed) {
  const speedWidth = row.speed_vs_rustia ? Math.max(2, (row.speed_vs_rustia / maxSpeed) * 100) : 0;
  const timeRatio = fastest / row.ns_per_unit_min;
  const timeWidth = Math.max(2, Math.min(100, timeRatio * 100));
  return `
    <article class="result-row">
      <div class="result-label">
        <strong>${escapeHtml(row.engine)}</strong>
        <span>${escapeHtml(row.dialect)} · ${escapeHtml(row.commit)} · ${formatTime(row.timestamp_unix)}</span>
      </div>
      <div class="result-values">
        <span>${formatNumber(row.ns_per_unit_min)} ns min</span>
        <span>${formatSpeed(row.speed_vs_rustia)} vs rustia</span>
      </div>
      <div class="bars-pair">
        <div class="mini-bar time" title="Relative to fastest row in this workload"><span style="width: ${timeWidth.toFixed(3)}%"></span></div>
        <div class="mini-bar speed" title="Speed relative to rustia"><span style="width: ${speedWidth.toFixed(3)}%"></span></div>
      </div>
    </article>
  `;
}

function targetSection(group) {
  const workloads = [...groupBy(group.values, (row) => row.workload)]
    .map(([workload, rows]) => ({ workload, rows }))
    .sort((left, right) => left.workload.localeCompare(right.workload));
  return `
    <section class="target" id="${escapeHtml(slug(group.target))}">
      <div class="target-head">
        <div>
          <h2>${escapeHtml(group.target)}</h2>
          <p>${formatInteger(group.values.length)} rows across ${formatInteger(workloads.length)} workloads</p>
        </div>
      </div>
      ${workloads.map((entry) => workloadSection(group.target, entry.workload, entry.rows)).join("")}
    </section>
  `;
}

function render(data) {
  const summary = summarize(data);
  const totalMatchups = groupBy(summary.rows, (row) => `${row.target}\0${row.workload}`).size;
  const latestCommit = summary.rows
    .slice()
    .sort((left, right) => right.timestamp_unix - left.timestamp_unix || right.commit.localeCompare(left.commit))[0].commit;

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>rustia Current Benchmark Results</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f5f7fb;
      --panel: #ffffff;
      --panel-soft: #eef3f6;
      --text: #17202a;
      --muted: #5d6875;
      --line: #d8e0e7;
      --accent: #0f766e;
      --accent-2: #2563eb;
      --warn: #b45309;
      --shadow: 0 16px 34px rgba(30, 45, 62, 0.08);
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font: 15px/1.5 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    main {
      width: min(1180px, calc(100% - 32px));
      margin: 0 auto;
      padding: 30px 0 54px;
    }

    h1, h2, h3, p { margin-top: 0; }

    .hero {
      display: grid;
      grid-template-columns: minmax(0, 1.4fr) minmax(260px, 0.8fr);
      gap: 18px;
      margin-bottom: 18px;
    }

    .hero-panel,
    .metric,
    .panel,
    .target {
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 8px;
      box-shadow: var(--shadow);
    }

    .hero-panel {
      padding: 28px;
    }

    .eyebrow {
      margin-bottom: 8px;
      color: var(--accent);
      font-size: 12px;
      font-weight: 800;
      letter-spacing: 0;
      text-transform: uppercase;
    }

    h1 {
      margin-bottom: 12px;
      font-size: 46px;
      line-height: 1.05;
      letter-spacing: 0;
    }

    .lead {
      max-width: 760px;
      margin-bottom: 18px;
      color: var(--muted);
      font-size: 16px;
    }

    .source-line {
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin: 0;
      color: var(--muted);
      font-size: 13px;
    }

    code {
      padding: 2px 6px;
      border-radius: 6px;
      background: var(--panel-soft);
      color: var(--text);
    }

    .metrics {
      display: grid;
      gap: 12px;
    }

    .metric {
      padding: 16px;
    }

    .metric strong {
      display: block;
      margin-bottom: 2px;
      font-size: 25px;
      line-height: 1.1;
    }

    .metric span,
    .section-head p,
    .target-head p,
    .workload-head p,
    .result-label span {
      color: var(--muted);
      font-size: 13px;
    }

    .nav {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin: 0 0 18px;
    }

    .nav a {
      padding: 7px 10px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: var(--panel);
      color: var(--text);
      font-size: 13px;
      font-weight: 700;
      text-decoration: none;
    }

    .nav a:hover {
      border-color: var(--accent);
      color: var(--accent);
    }

    .panel,
    .target {
      margin-bottom: 18px;
      overflow: hidden;
    }

    .section-head,
    .target-head,
    .workload-head {
      display: flex;
      justify-content: space-between;
      gap: 16px;
      align-items: flex-start;
      padding: 18px 20px;
      border-bottom: 1px solid var(--line);
      background: var(--panel-soft);
    }

    .section-head h2,
    .target-head h2,
    .workload-head h3 {
      margin-bottom: 2px;
      font-size: 22px;
    }

    .table-wrap {
      overflow-x: auto;
    }

    table {
      width: 100%;
      border-collapse: collapse;
      min-width: 760px;
    }

    th,
    td {
      padding: 12px 14px;
      border-bottom: 1px solid var(--line);
      text-align: left;
      vertical-align: middle;
      white-space: nowrap;
    }

    th {
      color: var(--muted);
      font-size: 12px;
      text-transform: uppercase;
    }

    tr:last-child td {
      border-bottom: 0;
    }

    .bar-cell {
      display: grid;
      grid-template-columns: 82px minmax(120px, 1fr);
      gap: 12px;
      align-items: center;
    }

    .bar,
    .mini-bar {
      overflow: hidden;
      border-radius: 999px;
      background: #e2e8ef;
    }

    .bar {
      height: 10px;
    }

    .bar span,
    .mini-bar span {
      display: block;
      height: 100%;
      border-radius: inherit;
      background: linear-gradient(90deg, var(--accent), var(--accent-2));
    }

    .workload {
      border-bottom: 1px solid var(--line);
    }

    .workload:last-child {
      border-bottom: 0;
    }

    .workload-head {
      background: #fff;
    }

    .bars {
      display: grid;
    }

    .result-row {
      display: grid;
      grid-template-columns: minmax(180px, 0.9fr) minmax(160px, 0.7fr) minmax(220px, 1.4fr);
      gap: 16px;
      align-items: center;
      padding: 13px 20px;
      border-top: 1px solid var(--line);
    }

    .result-label {
      min-width: 0;
    }

    .result-label strong,
    .result-label span {
      display: block;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .result-values {
      display: grid;
      gap: 2px;
      color: var(--text);
      font-variant-numeric: tabular-nums;
    }

    .result-values span:last-child {
      color: var(--muted);
      font-size: 13px;
    }

    .bars-pair {
      display: grid;
      gap: 6px;
    }

    .mini-bar {
      height: 8px;
    }

    .mini-bar.time span {
      background: linear-gradient(90deg, var(--warn), var(--accent));
    }

    .footnote {
      color: var(--muted);
      font-size: 13px;
    }

    @media (max-width: 780px) {
      main {
        width: min(100% - 20px, 1180px);
        padding-top: 18px;
      }

      .hero {
        grid-template-columns: 1fr;
      }

      h1 {
        font-size: 34px;
      }

      .result-row {
        grid-template-columns: 1fr;
        gap: 8px;
      }
    }
  </style>
</head>
<body>
  <main>
    <section class="hero">
      <div class="hero-panel">
        <p class="eyebrow">script-bench current snapshot</p>
        <h1>rustia Current Benchmark Results</h1>
        <p class="lead">Static HTML generated from the committed current benchmark JSON. Bars are rough relative comparisons: lower time bars compare each row to the fastest engine in that workload, while speed bars use the harness speed-vs-rustia ratio.</p>
        <p class="source-line">
          <span>Source <code>${escapeHtml(relative(sourcePath))}</code></span>
          <span>History <code>${escapeHtml(data.metadata.history)}</code></span>
          <span>Generated by <code>${escapeHtml(relative(__filename))}</code></span>
        </p>
      </div>
      <div class="metrics">
        ${metric(formatInteger(data.metadata.row_count), "current result rows")}
        ${metric(formatInteger(data.metadata.engines.length), "engines")}
        ${metric(formatInteger(data.metadata.workloads.length), "workloads")}
        ${metric(escapeHtml(latestCommit), "latest commit in snapshot")}
        ${metric(`${formatTime(summary.minTimestamp)} to ${formatTime(summary.maxTimestamp)}`, "measurement window")}
      </div>
    </section>

    <nav class="nav" aria-label="Targets">
      ${summary.targetRows.map((group) => `<a href="#${escapeHtml(slug(group.target))}">${escapeHtml(group.target)}</a>`).join("")}
    </nav>

    ${engineSummaryTable(summary.engineRows, totalMatchups)}
    ${summary.targetRows.map(targetSection).join("")}

    <p class="footnote">Schema <code>${escapeHtml(data.schema)}</code>. History <code>${escapeHtml(data.metadata.history)}</code> (${escapeHtml(data.metadata.history_schema)}). The dashboard is deterministic and contains no external assets or runtime JavaScript. Refresh data with the benchmark harness before rendering when current history changes.</p>
  </main>
</body>
</html>
`;
}

function main() {
  const data = readData();
  const html = render(data).replace(/[ \t]+$/gm, "").replace(/\n+$/, "");
  fs.writeFileSync(outputPath, `${html}\n`);
  console.log(`wrote ${relative(outputPath)} from ${relative(sourcePath)}`);
}

main();
