#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const resultsPath = path.join(root, "bench", "wasm", "current-results.json");
const gatePath = path.join(root, "bench", "wasm-gate.json");

const expectedWorkloads = new Map([
  ["dense_tables", "build+sum+sort of 256-elem array"],
  ["sparse_tables", "fill+read 128 hash entries"],
  ["closures_upvalues", "one upvalue-mutating closure call"],
  ["generic_iteration", "__iter(32)+pairs(5)+ipairs(32) pass"],
  ["stdlib_math_table", "math fastcalls + buffered push"],
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
  const families = new Set();
  for (const [runIndex, run] of data.runs.entries()) {
    const label = `results runs[${runIndex}]`;
    assertEqual(run.schema, "rustia.bench.wasm-runtime-results.v1", `${label}.schema`);
    assertText(run.target, `${label}.target`);
    assertObject(run.runtime, `${label}.runtime`);
    assertText(run.runtime.name, `${label}.runtime.name`);
    assertText(run.runtime.family, `${label}.runtime.family`);
    assertText(run.runtime.version, `${label}.runtime.version`);
    assertText(run.runtime.host, `${label}.runtime.host`);
    families.add(run.runtime.family);
    assertObject(run.wasm, `${label}.wasm`);
    assertEqual(run.wasm.bench_abi_version, 1, `${label}.wasm.bench_abi_version`);
    assertEqual(run.wasm.bytes, data.wasm.bytes, `${label}.wasm.bytes`);

    if (!Array.isArray(run.rows) || run.rows.length !== expectedWorkloads.size) {
      fail(`${label}.rows must contain ${expectedWorkloads.size} workload rows`);
    }
    const workloads = new Set();
    for (const [rowIndex, row] of run.rows.entries()) {
      const rowLabel = `${label}.rows[${rowIndex}]`;
      assertText(row.workload, `${rowLabel}.workload`);
      assertText(row.unit, `${rowLabel}.unit`);
      assertText(row.checksum, `${rowLabel}.checksum`);
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

  assertEqual(Boolean(data.requirements["v8-production-rustia-wasm"]), families.has("V8"), "V8 requirement summary");
  assertEqual(
    Boolean(data.requirements["javascriptcore-production-rustia-wasm"]),
    families.has("JavaScriptCore"),
    "JavaScriptCore requirement summary",
  );
  assertEqual(
    Boolean(data.requirements["spidermonkey-production-rustia-wasm"]),
    families.has("SpiderMonkey"),
    "SpiderMonkey requirement summary",
  );
  return families;
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

const results = readJson(resultsPath);
const gate = readJson(gatePath);
const families = validateResults(results);
validateGate(gate, results);
console.log(
  `wasm gate results ok: ${results.runs.length} runtime(s), ${[...families].sort().join(", ")}`,
);
