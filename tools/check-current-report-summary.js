#!/usr/bin/env node
"use strict";

const assert = require("assert/strict");
const {
  REQUIRED_COMMON_WORKLOADS,
  languageLabel,
  languagesByTargetSection,
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
  assert.equal(partial.implementationRows[0].bestCount, null);

  const html = languagesByTargetSection(summaries);
  assert.equal(count(html, 'class="language-target-summary"'), 2);
  assert.equal(count(html, 'data-target="linux-complete"'), 1);
  assert.equal(count(html, 'data-target="linux-partial"'), 1);
  assert.match(html, /<h2>Languages by Target<\/h2>/);
  assert.match(html, />Complete<\/span>/);
  assert.match(html, />Partial<\/span>/);
  assert.match(html, />Single revision<\/span>/);
  assert.match(html, />Mixed revisions<\/span>/);
  assert.match(html, /Measured 1970-01-01 00:01:40 UTC to 1970-01-01 00:01:49 UTC/);
  assert.match(html, /commits <code>partial-revision-a<\/code>, <code>partial-revision-b<\/code>/);
  assert.match(html, /<strong>Koto<\/strong>/);
  assert.match(html, /<strong>Luau<\/strong>/);
  assert.ok(html.indexOf('data-implementation="koto"') < html.indexOf('data-implementation="rustia"'));
  assert.doesNotMatch(html, /oop_metamethods|excluded-revision/);

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
