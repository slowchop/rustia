#!/usr/bin/env node
"use strict";

const assert = require("assert/strict");
const {
  REQUIRED_COMMON_WORKLOADS,
  bestAvailableRows,
  coldStartSection,
  historySection,
  implementationLabel,
  languageLabel,
  linuxComparisonSection,
  languagesByTargetSection,
  memoryComparisonCharts,
  memoryCohortHost,
  memoryHistoryModel,
  memoryHistorySection,
  memoryResultsSection,
  mluaJitHistoryReference,
  normalizeHistoryRecord,
  optimizationEvidenceSection,
  operationsStatusSection,
  requireAuthorityHost,
  requirePublishedHosts,
  requireSingleHost,
  rustiaBackfillEras,
  rustiaHistoryEras,
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
  const memoryHistoryHtml = memoryHistorySection(memoryHistory, "ci-bench");
  assert.match(memoryHistoryHtml, /rustia Memory History/);
  assert.match(memoryHistoryHtml, /data-memory-history-metric="peak" aria-pressed="true"/);
  assert.match(memoryHistoryHtml, /data-memory-history-metric="retained" aria-pressed="false"/);
  assert.match(memoryHistoryHtml, /data-history-axis="date" aria-pressed="true"/);
  assert.match(memoryHistoryHtml, /data-history-axis="commit" aria-pressed="false"/);
  assert.equal(count(memoryHistoryHtml, 'data-memory-history-metric-view="peak"'), 2);
  assert.equal(count(memoryHistoryHtml, 'data-memory-history-metric-view="retained" hidden'), 2);
  assert.match(memoryHistoryHtml, /mlua<\/strong> \/ Luau \/ JIT · 1×/);

  // A cohort names one machine, read from its rows; the record may repeat it
  // but may not contradict it, and rows may not half-say it.
  const hostedRows = (host) => memoryHistory[0].rows.map((row) => ({ ...row, host }));
  assert.equal(memoryCohortHost({ rows: hostedRows("ci-bench") }, "fixture"), "ci-bench");
  assert.equal(memoryCohortHost({ rows: hostedRows(undefined) }, "fixture"), null);
  assert.equal(memoryCohortHost({ host: "ci-bench", rows: hostedRows("ci-bench") }, "fixture"), "ci-bench");
  assert.throws(
    () => memoryCohortHost({ rows: [...hostedRows("ci-bench"), { ...hostedRows("rented")[0], workload: "x" }] }, "fixture"),
    /fixture was measured on 2 machines \(ci-bench, rented\)/,
  );
  assert.throws(
    () => memoryCohortHost({ rows: [...hostedRows("ci-bench"), { ...hostedRows(undefined)[0], workload: "x" }] }, "fixture"),
    /2 machines \(unnamed, ci-bench\)/,
  );
  assert.throws(() => memoryCohortHost({ host: "ci-bench", rows: hostedRows("rented") }, "fixture"), /fixture\.host is "ci-bench"/);
  assert.throws(() => memoryCohortHost({ rows: hostedRows("") }, "fixture"), /non-empty string/);

  // The memory history is one machine's series. The reference must come from
  // the machine the latest cohort ran on; cohorts from any other machine are
  // counted and named as not drawn, never plotted on the same line.
  const hostedCohort = (record, host) => ({ ...record, host, rows: record.rows.map((row) => ({ ...row, host })) });
  const movedHistory = [hostedCohort(memoryHistory[0], "rented"), hostedCohort(memoryHistory[1], "ci-bench")];
  assert.equal(memoryHistoryModel(movedHistory), null);
  const movedHtml = memoryHistorySection(movedHistory, "ci-bench");
  assert.match(movedHtml, /1 cohort\(s\) on <code>ci-bench<\/code>; no reference to plot against yet/);
  assert.match(movedHtml, /no mlua\/JIT memory cohort has been measured on <code>ci-bench<\/code>/);
  assert.match(movedHtml, /1 cohort\(s\) measured on another machine is not drawn beside these/);
  // A cohort from before hosts were recorded is a different fact from a cohort
  // measured on another machine, and the prose says which.
  const predatingHtml = memoryHistorySection([memoryHistory[0], hostedCohort(memoryHistory[1], "ci-bench")], "ci-bench");
  assert.match(predatingHtml, /1 cohort\(s\) measured before hosts were recorded is not drawn beside these/);
  assert.doesNotMatch(predatingHtml, /another machine/);
  assert.doesNotMatch(movedHtml, /<code>baseline<\/code>/);
  const rehomedHistory = [
    ...movedHistory,
    hostedCohort({
      ...memoryHistory[0],
      cohort_id: "rehomed",
      commit: "rehomed-commit",
      created_at: "2026-07-03T00:00:00.000Z",
    }, "ci-bench"),
  ];
  const rehomedModel = memoryHistoryModel(rehomedHistory);
  assert.equal(rehomedModel.host, "ci-bench");
  assert.equal(rehomedModel.reference.cohort, "rehomed");
  assert.deepEqual(rehomedModel.points.map((point) => point.cohort), ["current", "rehomed"]);
  assert.equal(rehomedModel.points[0].peak, 2);
  assert.equal(rehomedModel.points[1].peak, 0.5);
  const rehomedHtml = memoryHistorySection(rehomedHistory, "ci-bench");
  assert.match(rehomedHtml, /2 complete Rustia cohorts on <code>ci-bench<\/code>/);
  assert.match(rehomedHtml, /1 cohort\(s\) measured on another machine is not drawn beside these/);
  assert.match(rehomedHtml, /measured on the same machine as every Rustia point/);
  // With no cohorts at all, the only machine the section can name is the
  // qualified timing authority it was handed.
  assert.match(memoryHistorySection([], "some-authority"), /starts again on <code>some-authority<\/code>/);

  // The memory panel names the machine its rows name, never a literal.
  const memoryResultRow = (engine, host) => ({
    commit: "c".repeat(40),
    target: "linux-x86_64",
    host,
    engine,
    dialect: engine === "rustia" ? "luau" : "koto",
    execution_mode: "interpreter",
    workload: "memory_churn",
    classification: "stable_in_tested_window",
    metrics: {
      spawned_anon_kib: { median: 1024 },
      prepared_delta_anon_kib: { median: 64 },
      peak_delta_kib: { median: 256 },
      quiescent_retained_delta_anon_kib: { median: 32 },
      tail_growth_kib_per_epoch: { median: 0 },
    },
  });
  const soloMemoryHtml = memoryResultsSection(
    { status: "ready", rows: [memoryResultRow("rustia", "box-a")], detector_coverage: [] },
    "some-authority",
  );
  assert.match(soloMemoryHtml, /<h2>rustia Memory on box-a<\/h2>/);
  assert.match(soloMemoryHtml, /Competitors return when the memory matrix runs on <code>box-a<\/code>/);
  assert.doesNotMatch(soloMemoryHtml, /some-authority|ci-bench/);
  const pendingMemoryHtml = memoryResultsSection({ status: "pending", rows: [], detector_coverage: [] }, "some-authority");
  assert.match(pendingMemoryHtml, /measured on <code>some-authority<\/code>/);
  assert.throws(
    () => memoryResultsSection(
      { status: "ready", rows: [memoryResultRow("rustia", "box-a"), memoryResultRow("koto", "box-b")], detector_coverage: [] },
      "some-authority",
    ),
    /2 machines/,
  );

  // The single-host rule, as the readers apply it.
  const hostRow = (engine, host) => ({ engine, host });
  assert.equal(requireSingleHost([hostRow("rustia", "box-a"), hostRow("koto", "box-a")], "fixture", "why"), "box-a");
  assert.throws(
    () => requireSingleHost([hostRow("rustia", "box-a"), hostRow("koto", "box-b")], "fixture", "every ratio divides one row by another"),
    /fixture holds rows measured on 2 machines \(box-a: rustia; box-b: koto\); every ratio divides one row by another/,
  );

  // Timing history rows keep their host through normalisation: absent stays
  // absent (null) rather than being guessed, and present must name something.
  const rawHistoryRecord = {
    commit: "d".repeat(40),
    target: "linux-x86_64",
    engine: "rustia",
    dialect: "luau",
    workload: "dense_tables",
    unit: "unit",
    checksum: "integer:1",
    timestamp_unix: 1_000,
    n: 10,
    reps: 3,
    ns_per_unit_min: 100,
    ns_per_unit_median: 110,
    units_per_sec: 1e7,
  };
  assert.equal(normalizeHistoryRecord(rawHistoryRecord, 1).host, null);
  assert.equal(normalizeHistoryRecord({ ...rawHistoryRecord, host: "ci-bench" }, 1).host, "ci-bench");
  assert.throws(() => normalizeHistoryRecord({ ...rawHistoryRecord, host: "" }, 1), /host must be a non-empty string when present/);
  assert.throws(() => normalizeHistoryRecord({ ...rawHistoryRecord, host: 7 }, 1), /host must be a non-empty string when present/);
  // A label, never an address: the page refuses a dotted name or an IP as a
  // machine label wherever it reads one.
  for (const address of ["bench.example.com", "10.0.0.5", "fe80::1"]) {
    assert.throws(() => normalizeHistoryRecord({ ...rawHistoryRecord, host: address }, 1), /not a label/);
    assert.throws(() => memoryCohortHost({ rows: hostedRows(address) }, "fixture"), /not a label/);
  }

  // Recorded timing eras are one machine's -- the machine the mlua/JIT
  // reference was measured on. Rows from another machine, or from before hosts
  // were recorded, are counted and named rather than drawn, and a history with
  // nothing from that machine is refused rather than drawn against it.
  // A matrix run stamps each row as it finishes, so a run's rows do not share
  // a timestamp; the fixture stamps them a second apart the same way.
  const snapshot = (timestamp, commit, minimum, host, engine = "rustia") =>
    REQUIRED_COMMON_WORKLOADS.map((workload, index) => ({
      target: "linux-x86_64",
      engine,
      dialect: "luau",
      workload,
      timestamp_unix: timestamp + index,
      commit,
      ns_per_unit_min: minimum,
      host,
    }));
  const eraReference = mluaJitHistoryReference(snapshot(1_000, "reference", 100, "ci-bench", "mlua-luau-jit"));
  assert.equal(eraReference.host, "ci-bench");
  // The reference is one machine, checked rather than assumed.
  assert.throws(
    () => mluaJitHistoryReference([
      ...snapshot(1_000, "reference", 100, "ci-bench", "mlua-luau-jit").slice(0, 3),
      ...snapshot(1_000, "reference", 100, "linode-ap-southeast-g6-dedicated-4", "mlua-luau-jit").slice(3),
    ]),
    /the mlua\/JIT reference holds rows measured on 2 machines/,
  );
  const day = 24 * 60 * 60;
  const oneHost = rustiaHistoryEras(
    [...snapshot(1_000, "first", 200, "ci-bench"), ...snapshot(1_000 + 2 * day, "second", 100, "ci-bench")],
    eraReference,
  );
  assert.deepEqual(oneHost.eras.map((era) => era.title), ["Earlier Linux history", "Current dedicated Linux history"]);
  assert.deepEqual(oneHost.elsewhere, []);
  const rented = "linode-ap-southeast-g6-dedicated-4";
  const twoHosts = rustiaHistoryEras(
    [
      ...snapshot(1_000, "rented-first", 400, rented),
      ...snapshot(2_000, "rented-second", 200, rented),
      ...snapshot(3_000, "owned-first", 100, "ci-bench"),
      ...snapshot(4_000, "owned-second", 50, "ci-bench"),
    ],
    eraReference,
  );
  assert.deepEqual(twoHosts.eras.map((era) => era.label), ["Recorded Linux history"]);
  // Only the reference machine's rows are on the chart, at their own 1×; the
  // rented rows are counted, never divided into ci-bench's reference.
  assert.deepEqual(twoHosts.eras[0].overall.map((point) => point.commit), ["owned-first", "owned-second"]);
  assert.deepEqual(twoHosts.eras[0].overall.map((point) => point.gain), [1, 2]);
  assert.deepEqual(twoHosts.elsewhere, [{ host: rented, runCount: 2 }]);
  const unnamed = rustiaHistoryEras(
    [...snapshot(1_000, "old", 400, undefined), ...snapshot(2_000, "new", 100, "ci-bench")],
    eraReference,
  );
  assert.deepEqual(unnamed.eras[0].overall.map((point) => point.commit), ["new"]);
  assert.deepEqual(unnamed.elsewhere, [{ host: null, runCount: 1 }]);
  // Undrawn runs are counted the way drawn ones are -- one row per workload
  // per run -- so two commits at one timestamp are two runs and a partial
  // matrix is at most one, whatever their timestamps say.
  const tied = rustiaHistoryEras(
    [
      ...snapshot(1_000, "rented-a", 400, rented),
      ...snapshot(1_000, "rented-b", 400, rented),
      ...snapshot(2_000, "partial", 400, "elsewhere").slice(0, 2),
      ...snapshot(3_000, "owned", 100, "ci-bench"),
    ],
    eraReference,
  );
  assert.deepEqual(tied.elsewhere, [{ host: "elsewhere", runCount: 1 }, { host: rented, runCount: 2 }]);
  assert.throws(
    () => rustiaHistoryEras(snapshot(1_000, "old", 400, rented), eraReference),
    /no recorded rustia rows measured on "ci-bench", the machine the mlua\/JIT reference was measured on/,
  );
  // The section's prose names the machine drawn and says, precisely, what is
  // not drawn: rows from another machine and rows from before hosts were
  // recorded are different facts.
  const mixedHistory = { rows: [...snapshot(1_000, "rented-first", 400, rented), ...snapshot(2_000, "old", 400, undefined), ...snapshot(3_000, "owned-first", 100, "ci-bench")] };
  const historyHtml = historySection(mixedHistory, [], snapshot(1_000, "reference", 100, "ci-bench", "mlua-luau-jit"), []);
  assert.match(historyHtml, /1 recorded Linux matrix runs on <code>ci-bench<\/code> across one era/);
  assert.match(historyHtml, /Not drawn: 1 run\(s\) measured before hosts were recorded; 1 run\(s\) measured on <code>linode-ap-southeast-g6-dedicated-4<\/code>\./);
  assert.doesNotMatch(historyHtml, /rented-first/);
  // When a live series (or a reconstruction) is the whole chart, the recorded
  // runs from other machines are still named as not drawn, and the series is
  // passed in rather than read from wherever the checkout happens to have it.
  const liveSeries = (host) => ({
    manifest: { harness_commit: "1".repeat(40), host },
    rows: [...snapshot(5_000, "series-a", 100, host), ...snapshot(6_000, "series-b", 50, host)],
  });
  const seriesHtml = historySection(mixedHistory, [], snapshot(1_000, "reference", 100, "ci-bench", "mlua-luau-jit"), [liveSeries("ci-bench")]);
  assert.match(seriesHtml, /2 historical Rustia revisions replayed against one fixed harness/);
  assert.match(seriesHtml, /Not drawn: 1 run\(s\) measured before hosts were recorded; 1 run\(s\) measured on <code>linode-ap-southeast-g6-dedicated-4<\/code>\./);
  assert.doesNotMatch(seriesHtml, /recorded Linux matrix runs/);
  // Every point on the chart is divided into the reference, so a series or a
  // reconstruction from another machine is refused, never drawn against it.
  assert.throws(
    () => rustiaBackfillEras([], eraReference, [liveSeries(rented)]),
    /the live series with harness 1111111 was measured on "linode-ap-southeast-g6-dedicated-4", but the mlua\/JIT reference was measured on "ci-bench"/,
  );
  assert.throws(
    () => rustiaBackfillEras([{ harness_commit: "2".repeat(40), host: rented, targets: [], rows: [] }], eraReference, []),
    /the reconstruction with harness 2222222 was measured on "linode-ap-southeast-g6-dedicated-4"/,
  );
  assert.throws(
    () => rustiaBackfillEras([{ harness_commit: "2".repeat(40), targets: [], rows: [] }], eraReference, []),
    /was measured on an unnamed machine, but the mlua\/JIT reference was measured on "ci-bench"/,
  );
  // The page pins each published table to the timing authority itself: the
  // publication lane installs the data repository's memory table after the
  // checker has run, so the checker's pin never sees the rows that are
  // rendered.
  assert.equal(requireAuthorityHost("ci-bench", "fixture", "ci-bench"), undefined);
  assert.throws(
    () => requireAuthorityHost(rented, "bench/memory/current-results.json", "ci-bench"),
    /bench\/memory\/current-results.json was measured on "linode-ap-southeast-g6-dedicated-4", but .*operations-status.json qualifies "ci-bench" as the timing authority; the page publishes only the qualified machine/,
  );
  assert.throws(() => requireAuthorityHost(null, "fixture", "ci-bench"), /was measured on null, but/);
  const policy = { runner_classes: { rented: { timing_authority: false, host: rented }, owned: { timing_authority: true, host: "ci-bench" } } };
  const table = (host) => ({ status: "ready", rows: [{ host }] });
  assert.equal(requirePublishedHosts(table("ci-bench"), table("ci-bench"), [{ host: rented }, { host: "ci-bench" }], policy), "ci-bench");
  assert.equal(requirePublishedHosts(table("ci-bench"), { status: "pending" }, [], policy), "ci-bench");
  assert.throws(() => requirePublishedHosts(table(rented), table("ci-bench"), [], policy), /bench\/current-results\.json was measured on "linode/);
  assert.throws(() => requirePublishedHosts(table("ci-bench"), table(rented), [], policy), /bench\/memory\/current-results\.json was measured on "linode/);
  assert.throws(
    () => requirePublishedHosts(table("ci-bench"), table("ci-bench"), [{ host: "ci-bench" }, { host: rented }], policy),
    /bench\/memory\/history\/runs\.jsonl latest cohort was measured on "linode/,
  );
  assert.throws(
    () => requirePublishedHosts(table("ci-bench"), table("ci-bench"), [{ host: null }], policy),
    /latest cohort was measured on null/,
  );

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
