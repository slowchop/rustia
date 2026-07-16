#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const resultsPath = path.join(root, "bench", "wasm", "current-results.json");
const dispatchResultsPath = path.join(root, "bench", "wasm", "dispatch-current-results.json");
const gatePath = path.join(root, "bench", "wasm-gate.json");
const requiredDispatchFamilies = ["V8", "JavaScriptCore", "SpiderMonkey"];

const expectedWorkloads = new Map([
  ["dense_tables", "build+sum+sort of 256-elem array"],
  ["sparse_tables", "fill+read 128 hash entries"],
  ["closures_upvalues", "one upvalue-mutating closure call"],
  ["generic_iteration", "__iter(32)+pairs(5)+ipairs(32) pass"],
  ["stdlib_math_table", "math fastcalls + buffered push"],
]);
const expectedAdapters = new Map([
  ["rustia", "luau"],
  ["koto", "koto"],
  ["rhai", "rhai"],
]);

function fail(message) {
  console.error(`check-wasm-gate-results: ${message}`);
  process.exit(1);
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function relative(filePath) {
  return slash(path.relative(root, filePath));
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(filePath)}: ${error.message}`);
  }
}

function assertEqual(actual, expected, description) {
  if (actual !== expected) {
    fail(`${description}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertText(value, description) {
  if (typeof value !== "string" || value.trim() === "") {
    fail(`${description} must be a non-empty string`);
  }
}

function assertPositiveFinite(value, description) {
  if (!Number.isFinite(value) || value <= 0) {
    fail(`${description} must be positive and finite, got ${JSON.stringify(value)}`);
  }
}

function assertObject(value, description) {
  if (value == null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${description} must be an object`);
  }
}

function validateResults(data) {
  assertEqual(data.schema_version, 1, "results schema_version");
  assertEqual(data.schema, "rustia.bench.wasm-gate-current-results.v1", "results schema");
  assertEqual(data.source, "script-bench-harness", "results source");
  assertText(data.updated, "results updated");
  assertText(data.commit, "results commit");
  assertObject(data.wasm, "results wasm");
  assertText(data.wasm.manifest, "results wasm.manifest");
  assertText(data.wasm.artifact, "results wasm.artifact");
  assertPositiveFinite(data.wasm.bytes, "results wasm.bytes");
  assertObject(data.requirements, "results requirements");

  if (!Array.isArray(data.runs) || data.runs.length === 0) {
    fail("results runs must be a non-empty array");
  }
  const rustiaFamilies = new Set();
  const adaptersByTarget = new Map();
  for (const [runIndex, run] of data.runs.entries()) {
    const label = `results runs[${runIndex}]`;
    assertEqual(run.schema, "rustia.bench.wasm-runtime-results.v1", `${label}.schema`);
    assertText(run.target, `${label}.target`);
    assertObject(run.runtime, `${label}.runtime`);
    assertText(run.runtime.name, `${label}.runtime.name`);
    assertText(run.runtime.family, `${label}.runtime.family`);
    assertText(run.runtime.version, `${label}.runtime.version`);
    assertText(run.runtime.host, `${label}.runtime.host`);
    assertObject(run.adapter, `${label}.adapter`);
    assertText(run.adapter.id, `${label}.adapter.id`);
    assertText(run.adapter.dialect, `${label}.adapter.dialect`);
    const expectedDialect = expectedAdapters.get(run.adapter.id);
    if (expectedDialect == null) {
      fail(`${label}.adapter.id is not expected for wasm target: ${run.adapter.id}`);
    }
    assertEqual(run.adapter.dialect, expectedDialect, `${label}.adapter.dialect`);
    if (run.adapter.id === "rustia") {
      rustiaFamilies.add(run.runtime.family);
    }
    const adapters = adaptersByTarget.get(run.target) ?? new Set();
    adapters.add(run.adapter.id);
    adaptersByTarget.set(run.target, adapters);
    assertObject(run.wasm, `${label}.wasm`);
    assertEqual(run.wasm.bench_abi_version, 1, `${label}.wasm.bench_abi_version`);
    assertEqual(run.wasm.bytes, data.wasm.bytes, `${label}.wasm.bytes`);

    if (!Array.isArray(run.rows) || run.rows.length !== expectedWorkloads.size) {
      fail(`${label}.rows must contain ${expectedWorkloads.size} workload rows`);
    }
    const workloads = new Set();
    for (const [rowIndex, row] of run.rows.entries()) {
      const rowLabel = `${label}.rows[${rowIndex}]`;
      assertText(row.engine, `${rowLabel}.engine`);
      assertText(row.dialect, `${rowLabel}.dialect`);
      assertText(row.workload, `${rowLabel}.workload`);
      assertText(row.unit, `${rowLabel}.unit`);
      assertText(row.checksum, `${rowLabel}.checksum`);
      assertEqual(row.engine, run.adapter.id, `${rowLabel}.engine`);
      assertEqual(row.dialect, run.adapter.dialect, `${rowLabel}.dialect`);
      for (const field of ["n", "reps", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
        assertPositiveFinite(row[field], `${rowLabel}.${field}`);
      }
      if (row.ns_per_unit_median < row.ns_per_unit_min) {
        fail(`${rowLabel}.ns_per_unit_median must be >= ns_per_unit_min`);
      }
      const expectedUnit = expectedWorkloads.get(row.workload);
      if (expectedUnit == null) {
        fail(`${rowLabel}.workload is not expected: ${row.workload}`);
      }
      assertEqual(row.unit, expectedUnit, `${rowLabel}.unit`);
      workloads.add(row.workload);
    }
    for (const workload of expectedWorkloads.keys()) {
      if (!workloads.has(workload)) {
        fail(`${label}.rows is missing workload ${workload}`);
      }
    }
  }

  for (const [target, adapters] of adaptersByTarget) {
    for (const adapter of expectedAdapters.keys()) {
      if (!adapters.has(adapter)) {
        fail(`${target} is missing wasm adapter ${adapter}`);
      }
    }
  }

  assertEqual(Boolean(data.requirements["v8-production-rustia-wasm"]), rustiaFamilies.has("V8"), "V8 requirement summary");
  assertEqual(
    Boolean(data.requirements["javascriptcore-production-rustia-wasm"]),
    rustiaFamilies.has("JavaScriptCore"),
    "JavaScriptCore requirement summary",
  );
  assertEqual(
    Boolean(data.requirements["spidermonkey-production-rustia-wasm"]),
    rustiaFamilies.has("SpiderMonkey"),
    "SpiderMonkey requirement summary",
  );
  return rustiaFamilies;
}

function validateGate(gate, results) {
  const requirements = new Map(gate.requirements.map((requirement) => [requirement.id, requirement]));
  for (const [id, complete] of Object.entries(results.requirements)) {
    const requirement = requirements.get(id);
    if (!requirement) {
      fail(`${relative(gatePath)} is missing requirement ${id}`);
    }
    const measured = requirement.status === "measured";
    if (measured !== complete) {
      fail(`${id} gate status ${requirement.status} does not match current wasm results`);
    }
  }
}

function validateDispatchResults(data) {
  assertEqual(data.schema_version, 1, "dispatch results schema_version");
  assertEqual(data.schema, "rustia.bench.wasm-dispatch-current-results.v1", "dispatch results schema");
  assertEqual(data.source, "script-bench-harness", "dispatch results source");
  assertText(data.updated, "dispatch results updated");
  assertText(data.commit, "dispatch results commit");
  assertObject(data.requirements, "dispatch results requirements");
  if (!Array.isArray(data.required_families)) {
    fail("dispatch results required_families must be an array");
  }
  for (const family of requiredDispatchFamilies) {
    if (!data.required_families.includes(family)) {
      fail(`dispatch results required_families is missing ${family}`);
    }
  }
  if (!Array.isArray(data.runs) || data.runs.length === 0) {
    fail("dispatch results runs must be a non-empty array");
  }
  if (!Array.isArray(data.skipped)) {
    fail("dispatch results skipped must be an array");
  }

  const measuredFamilies = new Set();
  for (const [runIndex, run] of data.runs.entries()) {
    const label = `dispatch results runs[${runIndex}]`;
    assertEqual(run.schema_version, 1, `${label}.schema_version`);
    assertEqual(run.schema, "rustia.bench.wasm-dispatch-results.v1", `${label}.schema`);
    assertEqual(run.source, "script-bench-harness", `${label}.source`);
    assertText(run.target, `${label}.target`);
    assertObject(run.runtime, `${label}.runtime`);
    assertText(run.runtime.name, `${label}.runtime.name`);
    assertText(run.runtime.family, `${label}.runtime.family`);
    assertText(run.runtime.version, `${label}.runtime.version`);
    assertText(run.runtime.host, `${label}.runtime.host`);
    assertObject(run.microbench, `${label}.microbench`);
    assertEqual(run.microbench.id, "call_ref_vs_br_table", `${label}.microbench.id`);
    assertEqual(run.microbench.dispatches_per_unit, 1, `${label}.microbench.dispatches_per_unit`);
    assertObject(run.comparison, `${label}.comparison`);
    if (run.comparison.status === "measured") {
      measuredFamilies.add(run.runtime.family);
      assertObject(run.verification, `${label}.verification`);
      assertPositiveFinite(run.verification.n, `${label}.verification.n`);
      assertText(run.verification.checksum, `${label}.verification.checksum`);
      assertEqual(run.comparison.baseline, "br_table", `${label}.comparison.baseline`);
      assertEqual(run.comparison.candidate, "call_ref", `${label}.comparison.candidate`);
      assertPositiveFinite(
        run.comparison.candidate_vs_baseline_min,
        `${label}.comparison.candidate_vs_baseline_min`,
      );
      assertPositiveFinite(
        run.comparison.candidate_vs_baseline_median,
        `${label}.comparison.candidate_vs_baseline_median`,
      );
      if (!Array.isArray(run.rows) || run.rows.length !== 2) {
        fail(`${label}.rows must contain br_table and call_ref rows`);
      }
      const styles = new Set();
      for (const [rowIndex, row] of run.rows.entries()) {
        const rowLabel = `${label}.rows[${rowIndex}]`;
        assertText(row.style, `${rowLabel}.style`);
        assertText(row.feature, `${rowLabel}.feature`);
        assertPositiveFinite(row.n, `${rowLabel}.n`);
        assertPositiveFinite(row.reps, `${rowLabel}.reps`);
        assertPositiveFinite(row.wasm_bytes, `${rowLabel}.wasm_bytes`);
        assertPositiveFinite(row.ns_per_dispatch_min, `${rowLabel}.ns_per_dispatch_min`);
        assertPositiveFinite(row.ns_per_dispatch_median, `${rowLabel}.ns_per_dispatch_median`);
        assertPositiveFinite(row.dispatches_per_sec, `${rowLabel}.dispatches_per_sec`);
        assertText(row.checksum, `${rowLabel}.checksum`);
        if (row.ns_per_dispatch_median < row.ns_per_dispatch_min) {
          fail(`${rowLabel}.ns_per_dispatch_median must be >= ns_per_dispatch_min`);
        }
        styles.add(row.style);
      }
      for (const style of ["br_table", "call_ref"]) {
        if (!styles.has(style)) {
          fail(`${label}.rows is missing ${style}`);
        }
      }
    } else if (run.comparison.status !== "unsupported") {
      fail(`${label}.comparison.status must be measured or unsupported`);
    }
  }

  const expectedComplete = requiredDispatchFamilies.every((family) => measuredFamilies.has(family));
  assertEqual(
    Boolean(data.requirements["call-ref-vs-br-table"]),
    expectedComplete,
    "call_ref vs br_table requirement summary",
  );
  return measuredFamilies;
}

function validateDispatchGate(gate, dispatchResults) {
  const requirements = new Map(gate.requirements.map((requirement) => [requirement.id, requirement]));
  const requirement = requirements.get("call-ref-vs-br-table");
  if (!requirement) {
    fail(`${relative(gatePath)} is missing requirement call-ref-vs-br-table`);
  }
  const measured = requirement.status === "measured";
  const complete = Boolean(dispatchResults.requirements["call-ref-vs-br-table"]);
  if (measured !== complete) {
    fail(`call-ref-vs-br-table gate status ${requirement.status} does not match current dispatch results`);
  }
}

const results = readJson(resultsPath);
const dispatchResults = readJson(dispatchResultsPath);
const gate = readJson(gatePath);
const families = validateResults(results);
const dispatchFamilies = validateDispatchResults(dispatchResults);
validateGate(gate, results);
validateDispatchGate(gate, dispatchResults);
console.log(
  `wasm gate results ok: ${results.runs.length} adapter runtime(s), rustia on ${
    [...families].sort().join(", ")
  }; dispatch on ${[...dispatchFamilies].sort().join(", ")}`,
);
