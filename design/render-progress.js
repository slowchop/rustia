#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const sourcePath = path.join(__dirname, "progress.json");
const htmlOutputPath = path.join(__dirname, "progress.html");
const markdownOutputPath = path.join(__dirname, "progress.md");
const checkOnly = process.argv.includes("--check");

function fail(message) {
  console.error(`render-progress: ${message}`);
  process.exit(1);
}

function assertPercent(value, label) {
  if (!Number.isInteger(value) || value < 0 || value > 100) {
    fail(`${label} must be an integer percentage from 0 to 100`);
  }
}

function assertEntry(entry, label) {
  for (const key of ["name", "state", "future_plan"]) {
    if (typeof entry[key] !== "string" || entry[key].trim() === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  assertPercent(entry.complete, `${label}.complete`);
  assertPercent(entry.confidence, `${label}.confidence`);
}

function readData() {
  let data;
  try {
    data = JSON.parse(fs.readFileSync(sourcePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${path.relative(root, sourcePath)}: ${error.message}`);
  }

  if (!data.metadata || typeof data.metadata !== "object") {
    fail("metadata is required");
  }
  for (const key of ["date", "scope"]) {
    if (typeof data.metadata[key] !== "string" || data.metadata[key].trim() === "") {
      fail(`metadata.${key} must be a non-empty string`);
    }
  }
  if (!Array.isArray(data.metadata.excludes) || data.metadata.excludes.length === 0) {
    fail("metadata.excludes must include the out-of-scope items");
  }
  if (!Array.isArray(data.overall_goals) || data.overall_goals.length === 0) {
    fail("overall_goals must contain at least one entry");
  }
  data.overall_goals.forEach((entry, index) => assertEntry(entry, `overall_goals[${index}]`));

  const expectedAreas = [
    "Design",
    "Luau target",
    "Conformance",
    "Frontend",
    "VM/runtime",
    "Determinism",
    "Embedded API",
    "Benchmarks",
    "Wasm/perf",
    "Tooling",
    "Packaging"
  ];
  if (!Array.isArray(data.areas) || data.areas.length !== expectedAreas.length) {
    fail(`areas must contain exactly ${expectedAreas.length} groups`);
  }
  data.areas.forEach((area, areaIndex) => {
    if (area.area !== expectedAreas[areaIndex]) {
      fail(`areas[${areaIndex}].area must be "${expectedAreas[areaIndex]}"`);
    }
    if (!Array.isArray(area.entries) || area.entries.length === 0) {
      fail(`${area.area}.entries must contain at least one entry`);
    }
    area.entries.forEach((entry, entryIndex) => {
      assertEntry(entry, `areas[${areaIndex}].entries[${entryIndex}]`);
    });
  });

  return data;
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function escapeMarkdown(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\r?\n/g, " ")
    .replace(/\|/g, "\\|")
    .replace(/\s+/g, " ")
    .trim();
}

function average(values) {
  if (values.length === 0) {
    return 0;
  }
  return Math.round(values.reduce((sum, value) => sum + value, 0) / values.length);
}

function confidenceClass(value) {
  if (value >= 80) {
    return "high";
  }
  if (value >= 65) {
    return "medium";
  }
  return "low";
}

function progressBar(value, label) {
  const escapedLabel = escapeHtml(label);
  return `
    <div class="meter" aria-label="${escapedLabel}: ${value}% complete">
      <div class="meter-fill" style="width: ${value}%"></div>
    </div>
  `;
}

function goalCard(goal) {
  return `
    <article class="goal-card">
      <div class="card-head">
        <h3>${escapeHtml(goal.name)}</h3>
        <span class="percent">${goal.complete}%</span>
      </div>
      ${progressBar(goal.complete, goal.name)}
      <div class="meta-line">
        <span class="chip ${confidenceClass(goal.confidence)}">${goal.confidence}% confidence</span>
      </div>
      <p>${escapeHtml(goal.state)}</p>
      <p class="plan"><strong>Next:</strong> ${escapeHtml(goal.future_plan)}</p>
    </article>
  `;
}

function entryRow(entry) {
  return `
    <article class="entry">
      <div class="entry-top">
        <div>
          <h4>${escapeHtml(entry.name)}</h4>
          <p>${escapeHtml(entry.state)}</p>
        </div>
        <div class="entry-score">
          <strong>${entry.complete}%</strong>
          <span class="chip ${confidenceClass(entry.confidence)}">${entry.confidence}% confidence</span>
        </div>
      </div>
      ${progressBar(entry.complete, entry.name)}
      <p class="plan"><strong>Future plan:</strong> ${escapeHtml(entry.future_plan)}</p>
    </article>
  `;
}

function areaSection(area) {
  const complete = average(area.entries.map((entry) => entry.complete));
  const confidence = average(area.entries.map((entry) => entry.confidence));
  return `
    <section class="area" id="${escapeHtml(slug(area.area))}">
      <div class="area-head">
        <div>
          <h2>${escapeHtml(area.area)}</h2>
          <p>${area.entries.length} tracked ${area.entries.length === 1 ? "item" : "items"}</p>
        </div>
        <div class="area-score">
          <span>${complete}% avg complete</span>
          <span>${confidence}% avg confidence</span>
        </div>
      </div>
      <div class="entries">
        ${area.entries.map(entryRow).join("")}
      </div>
    </section>
  `;
}

function slug(value) {
  return String(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");
}

function render(data) {
  const areaAverages = data.areas.map((area) => ({
    area: area.area,
    complete: average(area.entries.map((entry) => entry.complete))
  }));
  const overall = data.overall_goals[0];
  const embedded = data.overall_goals.find((goal) => goal.name === "Usable embedded scripting crate");
  const release = data.overall_goals.find((goal) => goal.name === "Public release readiness");
  const conformance = data.metadata.canonical_facts.conformance;
  const adapters = data.metadata.canonical_facts.benchmark_adapters;
  const scopeExclusions = data.metadata.excludes.join(", ");
  const notes = Array.isArray(data.metadata.notes) ? data.metadata.notes : [];

  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>rustia progress dashboard</title>
  <style>
    :root {
      color-scheme: light;
      --bg: #f6f8fb;
      --panel: #ffffff;
      --panel-soft: #eef4f8;
      --text: #17202a;
      --muted: #5c6875;
      --line: #d9e1e8;
      --accent: #0f766e;
      --accent-2: #2563eb;
      --good-bg: #dff6ed;
      --good-text: #0f5c45;
      --mid-bg: #e8f0ff;
      --mid-text: #1d4f91;
      --low-bg: #fff1d8;
      --low-text: #8a4b00;
      --shadow: 0 20px 45px rgba(30, 45, 62, 0.10);
    }

    * {
      box-sizing: border-box;
    }

    body {
      margin: 0;
      background: var(--bg);
      color: var(--text);
      font: 15px/1.5 ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    }

    main {
      width: min(1180px, calc(100% - 32px));
      margin: 0 auto;
      padding: 32px 0 56px;
    }

    .hero {
      display: grid;
      grid-template-columns: minmax(0, 1.5fr) minmax(300px, 0.75fr);
      gap: 20px;
      align-items: stretch;
      margin-bottom: 22px;
    }

    .hero-panel,
    .goal-card,
    .area {
      background: var(--panel);
      border: 1px solid var(--line);
      box-shadow: var(--shadow);
    }

    .hero-panel {
      padding: 30px;
      border-radius: 8px;
    }

    .eyebrow {
      margin: 0 0 10px;
      color: var(--accent);
      font-size: 12px;
      font-weight: 800;
      letter-spacing: 0;
      text-transform: uppercase;
    }

    h1,
    h2,
    h3,
    h4,
    p {
      margin-top: 0;
    }

    h1 {
      margin-bottom: 12px;
      font-size: 54px;
      line-height: 1.02;
      letter-spacing: 0;
    }

    .lead {
      max-width: 760px;
      margin-bottom: 18px;
      color: var(--muted);
      font-size: 17px;
    }

    .source-line {
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
      margin: 0;
      color: var(--muted);
      font-size: 13px;
    }

    .source-line code {
      padding: 2px 6px;
      border-radius: 6px;
      background: var(--panel-soft);
      color: var(--text);
    }

    .facts {
      display: grid;
      gap: 12px;
    }

    .fact {
      padding: 16px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--panel);
      box-shadow: var(--shadow);
    }

    .fact strong {
      display: block;
      margin-bottom: 2px;
      font-size: 24px;
      line-height: 1.1;
    }

    .fact span {
      color: var(--muted);
      font-size: 13px;
    }

    .goals {
      display: grid;
      grid-template-columns: repeat(5, minmax(0, 1fr));
      gap: 14px;
      margin-bottom: 22px;
    }

    .goal-card {
      min-width: 0;
      padding: 18px;
      border-radius: 8px;
    }

    .card-head,
    .entry-top,
    .area-head {
      display: flex;
      justify-content: space-between;
      gap: 16px;
      align-items: flex-start;
    }

    .goal-card h3 {
      margin-bottom: 0;
      font-size: 16px;
      line-height: 1.25;
    }

    .percent {
      color: var(--accent);
      font-size: 26px;
      font-weight: 850;
      line-height: 1;
      white-space: nowrap;
    }

    .meter {
      height: 9px;
      margin: 14px 0 12px;
      overflow: hidden;
      border-radius: 999px;
      background: #e6ebf0;
    }

    .meter-fill {
      height: 100%;
      border-radius: inherit;
      background: linear-gradient(90deg, var(--accent), var(--accent-2));
    }

    .meta-line {
      display: flex;
      gap: 8px;
      align-items: center;
      margin-bottom: 12px;
    }

    .chip {
      display: inline-flex;
      align-items: center;
      width: max-content;
      max-width: 100%;
      min-height: 24px;
      padding: 3px 9px;
      border-radius: 999px;
      font-size: 12px;
      font-weight: 750;
      white-space: nowrap;
    }

    .chip.high {
      background: var(--good-bg);
      color: var(--good-text);
    }

    .chip.medium {
      background: var(--mid-bg);
      color: var(--mid-text);
    }

    .chip.low {
      background: var(--low-bg);
      color: var(--low-text);
    }

    .goal-card p,
    .entry p {
      color: var(--muted);
    }

    .plan {
      margin-bottom: 0;
    }

    .plan strong {
      color: var(--text);
    }

    .area-nav {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      margin: 0 0 20px;
    }

    .area-nav a {
      padding: 7px 10px;
      border: 1px solid var(--line);
      border-radius: 999px;
      background: var(--panel);
      color: var(--text);
      font-size: 13px;
      font-weight: 700;
      text-decoration: none;
    }

    .area-nav a:hover {
      border-color: var(--accent);
      color: var(--accent);
    }

    .area {
      margin-bottom: 18px;
      overflow: hidden;
      border-radius: 8px;
    }

    .area-head {
      padding: 18px 20px;
      border-bottom: 1px solid var(--line);
      background: var(--panel-soft);
    }

    .area-head h2 {
      margin-bottom: 2px;
      font-size: 22px;
    }

    .area-head p,
    .area-score span {
      margin-bottom: 0;
      color: var(--muted);
      font-size: 13px;
    }

    .area-score {
      display: grid;
      gap: 4px;
      justify-items: end;
      white-space: nowrap;
    }

    .entries {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      gap: 1px;
      background: var(--line);
    }

    .entry {
      min-width: 0;
      padding: 18px 20px;
      background: var(--panel);
    }

    .entry h4 {
      margin-bottom: 6px;
      font-size: 16px;
    }

    .entry-score {
      display: grid;
      gap: 8px;
      justify-items: end;
      flex: 0 0 auto;
      text-align: right;
    }

    .entry-score strong {
      color: var(--accent);
      font-size: 22px;
      line-height: 1;
    }

    .notes {
      margin-top: 24px;
      padding: 18px 20px;
      border: 1px solid var(--line);
      border-radius: 8px;
      background: var(--panel);
      color: var(--muted);
      box-shadow: var(--shadow);
    }

    .notes h2 {
      margin-bottom: 8px;
      color: var(--text);
      font-size: 18px;
    }

    .notes ul {
      margin: 0;
      padding-left: 20px;
    }

    @media (max-width: 1020px) {
      .hero {
        grid-template-columns: 1fr;
      }

      .goals {
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }
    }

    @media (max-width: 720px) {
      main {
        width: min(100% - 20px, 1180px);
        padding-top: 18px;
      }

      .hero-panel,
      .goal-card,
      .area-head,
      .entry,
      .notes {
        padding: 16px;
      }

      h1 {
        font-size: 36px;
      }

      .goals,
      .entries {
        grid-template-columns: 1fr;
      }

      .card-head,
      .entry-top,
      .area-head {
        display: grid;
      }

      .entry-score,
      .area-score {
        justify-items: start;
        text-align: left;
      }
    }
  </style>
</head>
<body>
  <main>
    <section class="hero" aria-labelledby="title">
      <div class="hero-panel">
        <p class="eyebrow">Project progress</p>
        <h1 id="title">rustia progress dashboard</h1>
        <p class="lead">${escapeHtml(data.metadata.scope)} Actual grabby integration work is excluded from these estimates.</p>
        <p class="source-line">
          <span>Updated ${escapeHtml(data.metadata.date)}</span>
          <span>Source <code>design/progress.json</code></span>
          <span>Generated by <code>design/render-progress.js</code></span>
        </p>
      </div>
      <aside class="facts" aria-label="Current facts">
        <div class="fact">
          <strong>${overall.complete}%</strong>
          <span>Whole public rustia project, ${overall.confidence}% confidence</span>
        </div>
        <div class="fact">
          <strong>${embedded.complete}%</strong>
          <span>Usable embedded scripting crate, ${embedded.confidence}% confidence</span>
        </div>
        <div class="fact">
          <strong>${conformance.fixtures} / ${conformance.refs} / ${conformance.pending}</strong>
          <span>Conformance fixtures / refs / pending</span>
        </div>
        <div class="fact">
          <strong>${adapters.present}/${adapters.requested}</strong>
          <span>Requested benchmark adapters present</span>
        </div>
        <div class="fact">
          <strong>${release.complete}%</strong>
          <span>Public release readiness, ${release.confidence}% confidence</span>
        </div>
      </aside>
    </section>

    <section class="goals" aria-label="Overall goals">
      ${data.overall_goals.map(goalCard).join("")}
    </section>

    <nav class="area-nav" aria-label="Progress areas">
      ${areaAverages.map((area) => `<a href="#${escapeHtml(slug(area.area))}">${escapeHtml(area.area)} ${area.complete}%</a>`).join("")}
    </nav>

    ${data.areas.map(areaSection).join("")}

    <section class="notes">
      <h2>Scope Notes</h2>
      <ul>
        <li>Excluded: ${escapeHtml(scopeExclusions)}.</li>
        <li>${escapeHtml(adapters.note)}</li>
        ${notes.map((note) => `<li>${escapeHtml(note)}</li>`).join("\n        ")}
      </ul>
    </section>
  </main>
</body>
</html>
`;
}

function renderMarkdown(data) {
  const areaAverages = data.areas.map((area) => ({
    area: area.area,
    count: area.entries.length,
    complete: average(area.entries.map((entry) => entry.complete)),
    confidence: average(area.entries.map((entry) => entry.confidence)),
  }));
  const overall = data.overall_goals[0];
  const embedded = data.overall_goals.find((goal) => goal.name === "Usable embedded scripting crate");
  const release = data.overall_goals.find((goal) => goal.name === "Public release readiness");
  const conformance = data.metadata.canonical_facts.conformance;
  const adapters = data.metadata.canonical_facts.benchmark_adapters;
  const notes = Array.isArray(data.metadata.notes) ? data.metadata.notes : [];

  let text = `# rustia Progress Report

Generated project progress report for rustia.

- Updated: ${escapeMarkdown(data.metadata.date)}
- Scope: ${escapeMarkdown(data.metadata.scope)}
- Source: \`design/progress.json\`
- HTML dashboard: [progress.html](progress.html)
- Generated by: \`design/render-progress.js\`

These percentages are rough engineering estimates. Actual grabby integration work is excluded from these estimates.

## Current Facts

| fact | value |
|---|---:|
| Whole public rustia project | ${overall.complete}% complete / ${overall.confidence}% confidence |
| Usable embedded scripting crate | ${embedded.complete}% complete / ${embedded.confidence}% confidence |
| Conformance fixtures / refs / pending | ${conformance.fixtures} / ${conformance.refs} / ${conformance.pending} |
| Requested benchmark adapters present | ${adapters.present}/${adapters.requested} |
| Public release readiness | ${release.complete}% complete / ${release.confidence}% confidence |

## Overall Goals

| goal | complete | confidence |
|---|---:|---:|
`;

  for (const goal of data.overall_goals) {
    text += `| ${escapeMarkdown(goal.name)} | ${goal.complete}% | ${goal.confidence}% |\n`;
  }

  for (const goal of data.overall_goals) {
    text += `
### ${escapeMarkdown(goal.name)}

- Complete: ${goal.complete}%
- Confidence: ${goal.confidence}%
- State: ${escapeMarkdown(goal.state)}
- Next: ${escapeMarkdown(goal.future_plan)}
`;
  }

  text += `
## Area Summary

| area | tracked items | avg complete | avg confidence |
|---|---:|---:|---:|
`;

  for (const area of areaAverages) {
    text += `| [${escapeMarkdown(area.area)}](#${slug(area.area)}) | ${area.count} | ${area.complete}% | ${area.confidence}% |\n`;
  }

  text += "\n## Areas\n";

  for (const area of data.areas) {
    const complete = average(area.entries.map((entry) => entry.complete));
    const confidence = average(area.entries.map((entry) => entry.confidence));
    text += `
### ${escapeMarkdown(area.area)}

Average complete: ${complete}%. Average confidence: ${confidence}%.
`;

    for (const entry of area.entries) {
      text += `
#### ${escapeMarkdown(entry.name)}

- Complete: ${entry.complete}%
- Confidence: ${entry.confidence}%
- State: ${escapeMarkdown(entry.state)}
- Future plan: ${escapeMarkdown(entry.future_plan)}
`;
    }
  }

  text += `
## Scope Notes

- Excluded: ${escapeMarkdown(data.metadata.excludes.join(", "))}.
- ${escapeMarkdown(adapters.note)}
`;

  for (const note of notes) {
    text += `- ${escapeMarkdown(note)}\n`;
  }

  return text;
}

function stripTrailingWhitespace(value) {
  return value
    .split("\n")
    .map((line) => line.replace(/[ \t]+$/g, ""))
    .join("\n");
}

const data = readData();
const htmlOutput = stripTrailingWhitespace(render(data));
const markdownOutput = `${stripTrailingWhitespace(renderMarkdown(data))}\n`;

function checkGenerated(filePath, expected) {
  let current;
  try {
    current = fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${path.relative(root, filePath)}: ${error.message}`);
  }
  if (current !== expected) {
    fail(`${path.relative(root, filePath)} is stale; run node design/render-progress.js`);
  }
  console.log(`${path.relative(root, filePath)} is current`);
}

if (checkOnly) {
  checkGenerated(htmlOutputPath, htmlOutput);
  checkGenerated(markdownOutputPath, markdownOutput);
} else {
  fs.writeFileSync(htmlOutputPath, htmlOutput, "utf8");
  console.log(`Wrote ${path.relative(root, htmlOutputPath)}`);
  fs.writeFileSync(markdownOutputPath, markdownOutput, "utf8");
  console.log(`Wrote ${path.relative(root, markdownOutputPath)}`);
}
