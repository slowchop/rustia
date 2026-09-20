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
  memoryHistoryModel,
  memoryHistorySection,
  memoryResultsSection,
  optimizationEvidenceSection,
  operationsStatusSection,
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
  const operationsHtml = operationsStatusSection({
    runner_classes: {
      "general-linux": { environment: "self-hosted-shared", timing_authority: false },
      "bench-solo": {
        environment: "self-hosted-owned-qualified",
        timing_authority: true,
        host: "ci-bench",
        qualification: { pinned_cpu: 4 },
      },
    },
    measurement: { mode: "attested-owned-host", scheduled_billing_allowed: false },
    publication: {
      state: "active",
      reason: "Publication reason.",
      next_action: "Keep artifact publication healthy.",
    },
    progress: {
      state: "pending",
      measured_commit: "a".repeat(40),
      measured_at_unix: 100,
      latest_relevant_commit: "b".repeat(40),
      pending_relevant_commit_count: 2,
      next_action: "Measure the pending commit.",
    },
    landscape: {
      state: "paused",
      measured_commit: "a".repeat(40),
      measured_at_unix: 100,
      reason: "Landscape reason.",
      next_action: "Implement landscape campaigns.",
    },
  });
  assert.match(operationsHtml, /Automatic publication is active/);
  assert.match(operationsHtml, /2 relevant commits waiting/);
  // The page must keep saying the shared pool cannot produce a timing, and must
  // keep naming the machine that did. Asserting the host means the page can
  // never quietly stop attributing its own numbers.
  assert.match(operationsHtml, /The shared pool is still not a timing authority/);
  assert.match(operationsHtml, /no result measured on the shared pool may enter a claim/i);
  assert.match(operationsHtml, /ci-bench/);
  assert.match(operationsHtml, /isolated core 4/);
  assert.match(operationsHtml, /no scheduled job may start a billed instance/i);

  const optimizationHtml = optimizationEvidenceSection([{
    workload: "generic_iteration",
    createdAt: "2026-08-14T08:00:35.308Z",
    baselineCommit: "a".repeat(40),
    candidateCommit: "b".repeat(40),
    delta: 1.31805462010519,
    lower: -4.57625251012931,
    upper: 3.93058467710712,
    legCount: 8,
    sampleCount: 808,
    verdict: "material_regression_excluded",
    published: true,
  }]);
  assert.match(optimizationHtml, /Accepted Optimization Checks/);
  assert.match(optimizationHtml, /\+1\.32%/);
  assert.match(optimizationHtml, /-4\.58% to \+3\.93%/);
  assert.match(optimizationHtml, /Material regression excluded/);
  assert.match(optimizationHtml, /8 legs · 808 samples/);

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

  const memoryHistoryWorkloads = [
    ...REQUIRED_COMMON_WORKLOADS,
    "memory_burst",
    "memory_churn",
    "todo_history",
    "todo_steady",
  ];
  const memoryHistoryRow = (engine, workload, peak, retained) => ({
    target: "linux-x86_64",
    engine,
    workload,
    metrics: {
      peak_delta_kib: { median: peak },
      quiescent_retained_delta_anon_kib: { median: retained },
    },
  });
  const memoryHistory = [{
    schema_version: 1,
    schema: "rustia.bench.memory-cohort.v1",
    cohort_id: "baseline",
    commit: "baseline-commit",
    created_at: "2026-07-01T00:00:00.000Z",
    rows: memoryHistoryWorkloads.flatMap((workload) => [
      memoryHistoryRow("mlua-luau-jit", workload, 100, 50),
      memoryHistoryRow("rustia", workload, 200, 200),
    ]),
  }, {
    schema_version: 1,
    schema: "rustia.bench.memory-cohort.v1",
    cohort_id: "current",
    commit: "current-commit",
    created_at: "2026-07-02T00:00:00.000Z",
    rows: memoryHistoryWorkloads.map((workload) =>
      memoryHistoryRow("rustia", workload, 50, 25)
    ),
  }];
  const memoryHistoryModelValue = memoryHistoryModel(memoryHistory);
  assert.equal(memoryHistoryModelValue.points.length, 2);
  assert.equal(memoryHistoryModelValue.points[0].peak, 0.5);
  assert.equal(memoryHistoryModelValue.points[0].retained, 0.25);
  assert.equal(memoryHistoryModelValue.points[1].peak, 2);
  assert.equal(memoryHistoryModelValue.points[1].retained, 2);
  const memoryHistoryHtml = memoryHistorySection(memoryHistory);
  assert.match(memoryHistoryHtml, /rustia Memory History/);
  assert.match(memoryHistoryHtml, /data-memory-history-metric="peak" aria-pressed="true"/);
  assert.match(memoryHistoryHtml, /data-memory-history-metric="retained" aria-pressed="false"/);
  assert.match(memoryHistoryHtml, /data-history-axis="date" aria-pressed="true"/);
  assert.match(memoryHistoryHtml, /data-history-axis="commit" aria-pressed="false"/);
  assert.equal(count(memoryHistoryHtml, 'data-memory-history-metric-view="peak"'), 2);
  assert.equal(count(memoryHistoryHtml, 'data-memory-history-metric-view="retained" hidden'), 2);
  assert.match(memoryHistoryHtml, /mlua<\/strong> \/ Luau \/ JIT · 1×/);

  // Without a same-host competitor cohort there is no reference to divide by, so the panel has to
  // say so instead of plotting a line against a host it no longer trusts.
  const rustiaOnlyHistory = [{
    schema_version: 1,
    schema: "rustia.bench.memory-cohort.v1",
    cohort_id: "ci-bench-only",
    commit: "ci-bench-commit",
    created_at: "2026-07-03T00:00:00.000Z",
    rows: memoryHistoryWorkloads.map((workload) =>
      memoryHistoryRow("rustia", workload, 50, 25)
    ),
  }];
  assert.equal(memoryHistoryModel(rustiaOnlyHistory), null);
  const rustiaOnlyHistoryHtml = memoryHistorySection(rustiaOnlyHistory);
  assert.match(rustiaOnlyHistoryHtml, /1 cohort\(s\) on <code>ci-bench<\/code>; no reference to plot against yet\./);
  assert.match(rustiaOnlyHistoryHtml, /The chart needs a competitor reference from the same machine\./);
  assert.match(rustiaOnlyHistoryHtml, /Rustia cohorts recorded so far: <code>ci-bench-only<\/code>\./);
  assert.equal(count(rustiaOnlyHistoryHtml, "<svg"), 0);

  const emptyHistoryHtml = memoryHistorySection([]);
  assert.match(emptyHistoryHtml, /No cohorts\./);
  assert.match(emptyHistoryHtml, /The series starts again on <code>ci-bench<\/code>\./);
  assert.equal(count(emptyHistoryHtml, "<svg"), 0);

  const memoryResultRow = (engine, workload) => ({
    target: "linux-x86_64",
    engine,
    dialect: engine === "rustia" ? "luau" : "koto",
    execution_mode: "interpreter",
    workload,
    classification: "stable_in_tested_window",
    metrics: {
      spawned_anon_kib: { median: 2_048 },
      prepared_delta_anon_kib: { median: 128 },
      peak_delta_kib: { median: engine === "rustia" ? 1_024 : 256 },
      quiescent_retained_delta_anon_kib: { median: engine === "rustia" ? 512 : 16 },
      tail_growth_kib_per_epoch: { median: 0 },
    },
  });
  const memoryResults = (engines) => ({
    schema_version: 1,
    schema: "rustia.bench.memory-current-results.v1",
    status: "ready",
    detector_coverage: engines.map((engine) => ({ engine, detector_validated: true })),
    rows: engines.flatMap((engine) =>
      memoryWorkloads.map((workload) => memoryResultRow(engine, workload))
    ),
  });

  // One engine cannot be a comparison: the panel reads rustia's own figures and drops the charts.
  const singleEngineMemoryHtml = memoryResultsSection(memoryResults(["rustia"]));
  assert.match(singleEngineMemoryHtml, /<h2>rustia Memory on ci-bench<\/h2>/);
  assert.match(singleEngineMemoryHtml, /No competitor engines yet\./);
  assert.doesNotMatch(singleEngineMemoryHtml, /Linux Memory Comparison/);
  assert.equal(count(singleEngineMemoryHtml, 'class="memory-cohort-summary"'), 0);

  const comparativeMemoryHtml = memoryResultsSection(memoryResults(["rustia", "koto"]));
  assert.match(comparativeMemoryHtml, /<h2>Linux Memory Comparison<\/h2>/);
  assert.doesNotMatch(comparativeMemoryHtml, /No competitor engines yet/);
  assert.equal(count(comparativeMemoryHtml, 'class="memory-cohort-summary"'), 3);

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
