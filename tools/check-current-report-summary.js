#!/usr/bin/env node
"use strict";

const assert = require("assert/strict");
const {
  REQUIRED_COMMON_WORKLOADS,
  bestAvailableRows,
  coldStartSection,
  implementationLabel,
  languageLabel,
  linuxComparisonSection,
  languagesByTargetSection,
  memoryComparisonCharts,
  summarizeLanguageTargets,
  wasmTargetRows,
} = require("../bench/render-current-results.js");

function measurement({
  target,
  engine,
  dialect,
  workload,
  timestamp,
  commit,
  minimum,
  speed,
}) {
  return {
    target,
    targetKind: "x86_64",
    runtime: null,
    engine,
    dialect,
    workload,
    timestamp_unix: timestamp,
    commit,
    ns_per_unit_min: minimum,
    speed_vs_rustia: speed,
  };
}

function count(text, fragment) {
  return text.split(fragment).length - 1;
}

function main() {
  assert.deepEqual(
    ["javascript", "koto", "lua54", "luau", "rhai", "roto", "wasm"].map(languageLabel),
    ["JavaScript", "Koto", "Lua 5.4", "Luau", "Rhai", "Roto", "WebAssembly"],
  );
  assert.deepEqual(
    ["rustia", "mlua-luau", "mlua-luau-jit", "mlua-lua54", "rquickjs"].map(implementationLabel),
    ["rustia", "mlua", "mlua", "mlua", "rquickjs"],
  );

  const rows = [];
  const rustiaMinimums = [100, 200, 300, 400, 500];
  const kotoMinimums = [50, 300, 150, 800, 250];
  REQUIRED_COMMON_WORKLOADS.forEach((workload, index) => {
    rows.push(measurement({
      target: "linux-complete",
      engine: "rustia",
      dialect: "luau",
      workload,
      timestamp: 100 + index,
      commit: "complete-revision",
      minimum: rustiaMinimums[index],
      speed: 1,
    }));
    rows.push(measurement({
      target: "linux-complete",
      engine: "koto",
      dialect: "koto",
      workload,
      timestamp: 105 + index,
      commit: "complete-revision",
      minimum: kotoMinimums[index],
      speed: rustiaMinimums[index] / kotoMinimums[index],
    }));
  });
  rows.push(measurement({
    target: "linux-partial",
    engine: "rustia",
    dialect: "luau",
    workload: "dense_tables",
    timestamp: 200,
    commit: "partial-revision-a",
    minimum: 100,
    speed: 1,
  }));
  rows.push(measurement({
    target: "linux-partial",
    engine: "rustia",
    dialect: "luau",
    workload: "sparse_tables",
    timestamp: 210,
    commit: "partial-revision-b",
    minimum: 200,
    speed: 1,
  }));
  rows.push(measurement({
    target: "linux-partial",
    engine: "rustia",
    dialect: "luau",
    workload: "oop_metamethods",
    timestamp: 999,
    commit: "excluded-revision",
    minimum: 1,
    speed: 1,
  }));

  const summaries = summarizeLanguageTargets(rows);
  assert.equal(summaries.length, 2);

  const complete = summaries.find((row) => row.target === "linux-complete");
  assert.equal(complete.status, "Complete");
  assert.equal(complete.revisionStatus, "Single revision");
  assert.deepEqual(complete.commits, ["complete-revision"]);
  assert.equal(complete.minTimestamp, 100);
  assert.equal(complete.maxTimestamp, 109);
  const completeKoto = complete.implementationRows.find((row) => row.engine === "koto");
  const completeRustia = complete.implementationRows.find((row) => row.engine === "rustia");
  assert.deepEqual(complete.implementationRows.map((row) => row.engine), ["koto", "rustia"]);
  assert.equal(completeKoto.language, "Koto");
  assert.equal(completeKoto.coverage, 5);
  assert.equal(completeKoto.bestCount, 3);
  assert.equal(completeKoto.rank, 1);
  assert.ok(completeKoto.geoSpeed > 1);
  assert.ok(completeKoto.geoTime < completeRustia.geoTime);
  assert.equal(completeRustia.bestCount, 2);
  assert.equal(completeRustia.rank, 2);
  assert.equal(completeRustia.geoSpeed, 1);

  const partial = summaries.find((row) => row.target === "linux-partial");
  assert.equal(partial.status, "Partial");
  assert.equal(partial.revisionStatus, "Mixed revisions");
  assert.deepEqual(partial.commits, ["partial-revision-a", "partial-revision-b"]);
  assert.equal(partial.minTimestamp, 200);
  assert.equal(partial.maxTimestamp, 210);
  assert.equal(partial.implementationRows[0].coverage, 2);
  assert.equal(partial.implementationRows[0].rank, null);
  assert.equal(partial.implementationRows[0].geoSpeed, null);
  assert.equal(partial.implementationRows[0].geoTime, null);
  assert.equal(partial.implementationRows[0].bestCount, null);

  const html = languagesByTargetSection(summaries);
  assert.equal(count(html, 'class="language-target-summary"'), 2);
  assert.equal(count(html, 'data-target="linux-complete"'), 1);
  assert.equal(count(html, 'data-target="linux-partial"'), 1);
  assert.match(html, /<h2>Interpreter-only Languages by Target<\/h2>/);
  assert.match(html, />Complete<\/span>/);
  assert.match(html, />Partial<\/span>/);
  assert.match(html, />Single revision<\/span>/);
  assert.match(html, />Mixed revisions<\/span>/);
  assert.match(html, /Measured 1970-01-01 00:01:40 UTC to 1970-01-01 00:01:49 UTC/);
  assert.match(html, /commits <code>partial-revision-a<\/code>, <code>partial-revision-b<\/code>/);
  assert.match(html, /<strong>koto<\/strong><span>Koto<\/span>/);
  assert.match(html, /<strong>rustia<\/strong><span>Luau<\/span>/);
  assert.match(html, /data-metric="relative" aria-pressed="true"/);
  assert.match(html, /data-metric="absolute" aria-pressed="false"/);
  assert.match(html, /class="language-heatmap"/);
  assert.match(html, /data-relative="2x" data-absolute="50 ns"/);
  assert.ok(html.indexOf('data-implementation="koto"') < html.indexOf('data-implementation="rustia"'));
  assert.doesNotMatch(html, /oop_metamethods|excluded-revision/);

  const jitSummary = {
    ...complete,
    implementationRows: [{
      ...completeKoto,
      engine: "mlua-luau-jit",
      language: "Luau",
      mode: "jit",
    }],
  };
  const comparisonHtml = linuxComparisonSection({
    all: [jitSummary],
    interpreter: summaries,
    accelerated: [jitSummary],
  });
  assert.match(comparisonHtml, /<h2>Linux Language Comparison<\/h2>/);
  assert.match(comparisonHtml, /data-mode-filter="all" aria-pressed="true"/);
  assert.match(comparisonHtml, /data-mode-filter="interpreter" aria-pressed="false"/);
  assert.match(comparisonHtml, /data-mode-filter="accelerated" aria-pressed="false"/);
  assert.match(comparisonHtml, /data-mode-view="all"/);
  assert.match(comparisonHtml, /data-mode-view="interpreter" hidden/);
  assert.match(comparisonHtml, /data-mode-view="accelerated" hidden/);
  assert.match(comparisonHtml, /<strong>mlua<\/strong><span>Luau \/ JIT<\/span>/);

  const modeRows = [
    { ...rows[0], engine: "mlua-luau", mode_family: "mlua-luau", execution_mode: "interpreter" },
    { ...rows[0], engine: "mlua-luau-jit", mode_family: "mlua-luau", execution_mode: "jit" },
    { ...rows[0], engine: "rustia", mode_family: "rustia", execution_mode: "interpreter" },
  ];
  assert.deepEqual(bestAvailableRows(modeRows).map((row) => row.engine).sort(), ["mlua-luau-jit", "rustia"]);

  const coldHtml = coldStartSection([{
    target: "linux-complete",
    engine: "mlua-luau-jit",
    dialect: "luau",
    mode: "jit",
    prepare: 2_000_000,
    firstCall: 500_000,
    total: 2_500_000,
    coverage: 5,
    nativeVerified: true,
    nativeVerification: "ab-native-sentinel",
    nativeVerificationRatio: 3.95,
  }]);
  assert.match(coldHtml, /Compile and First-call Cost/);
  assert.match(coldHtml, /3\.95x A\/B/);

  const memoryWorkloads = [
    ...REQUIRED_COMMON_WORKLOADS,
    "memory_burst",
    "memory_churn",
    "todo_steady",
    "todo_history",
  ];
  const memoryRows = ["rustia", "koto"].flatMap((engine) =>
    memoryWorkloads.map((workload) => ({
      engine,
      dialect: engine === "rustia" ? "luau" : "koto",
      execution_mode: "interpreter",
      workload,
      metrics: {
        peak_delta_kib: { median: engine === "rustia" ? 1_024 : 256 },
        quiescent_retained_delta_anon_kib: { median: engine === "rustia" ? 512 : 16 },
      },
    }))
  );
  memoryRows.push({
    engine: "rustia",
    dialect: "luau",
    execution_mode: "interpreter",
    workload: "memory_retained_control",
    metrics: {
      peak_delta_kib: { median: 65_536 },
      quiescent_retained_delta_anon_kib: { median: 65_536 },
    },
  });
  const memoryHtml = memoryComparisonCharts(memoryRows);
  assert.equal(count(memoryHtml, 'class="memory-cohort-summary"'), 3);
  assert.equal(count(memoryHtml, 'class="memory-ranked-headline"'), 6);
  assert.match(memoryHtml, /Shared benchmark workloads/);
  assert.match(memoryHtml, /Allocation stress/);
  assert.match(memoryHtml, /Stateful todo application/);
  assert.match(memoryHtml, /geometric mean peak Δ · lower is better/);
  assert.match(memoryHtml, /arithmetic mean retained Δ · lower is better/);
  assert.ok(memoryHtml.indexOf("<strong>koto</strong>") < memoryHtml.indexOf("<strong>rustia</strong>"));
  assert.doesNotMatch(memoryHtml, /memory_retained_control|64 MiB/);

  const wasmRows = wasmTargetRows({
    runs: [
      ["wasm-target-a", "rustia", "luau", 100],
      ["wasm-target-a", "koto", "koto", 50],
      ["wasm-target-b", "rustia", "luau", 1_000],
      ["wasm-target-b", "koto", "koto", 500],
    ].map(([target, adapter, dialect, minimum], index) => ({
      target,
      timestamp_unix: 300 + index,
      commit: "wasm-revision",
      runtime: { family: "V8", name: "node" },
      adapter: { id: adapter, dialect },
      rows: [{
        workload: "dense_tables",
        unit: "unit",
        ns_per_unit_min: minimum,
        ns_per_unit_median: minimum,
      }],
    })),
  });
  assert.equal(
    wasmRows.find((row) => row.target === "wasm-target-a" && row.adapter === "koto").speed_vs_rustia,
    2,
  );
  assert.equal(
    wasmRows.find((row) => row.target === "wasm-target-b" && row.adapter === "koto").speed_vs_rustia,
    2,
  );

  console.log("current report target-first language summary checks passed");
}

main();
