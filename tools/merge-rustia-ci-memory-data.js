#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const CURRENT_SCHEMA = "rustia.bench.memory-current-results.v1";
const COHORT_SCHEMA = "rustia.bench.memory-cohort.v1";

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { command: "self-test" };
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value == null) fail(`${flag ?? "argument"} requires a value`);
    if (values.has(flag)) fail(`${flag} was supplied more than once`);
    values.set(flag, value);
  }
  const take = (flag) => {
    const value = values.get(flag);
    values.delete(flag);
    if (value == null) fail(`missing ${flag}`);
    return path.resolve(value);
  };
  const config = {
    command: "merge",
    generatedCurrent: take("--generated-current"),
    generatedHistory: take("--generated-history"),
    dataRoot: take("--data-root"),
  };
  if (values.size !== 0) fail(`unknown argument(s): ${[...values.keys()].join(", ")}`);
  return config;
}

function readJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot parse ${label}: ${error.message}`);
  }
}

function readHistory(file) {
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8").split(/\r?\n/).flatMap((raw, index) => {
    const line = raw.trim();
    if (line === "") return [];
    let record;
    try {
      record = JSON.parse(line);
    } catch (error) {
      fail(`${file} line ${index + 1} is invalid JSON: ${error.message}`);
    }
    if (record.schema_version !== 1 || record.schema !== COHORT_SCHEMA) {
      fail(`${file} line ${index + 1} has the wrong schema`);
    }
    return [record];
  });
}

function writeAtomic(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomBytes(6).toString("hex")}`;
  fs.writeFileSync(temporary, text, { flag: "wx" });
  fs.renameSync(temporary, file);
}

function validateCurrent(current, label, rustiaOnly) {
  if (
    current?.schema_version !== 1 ||
    current?.schema !== CURRENT_SCHEMA ||
    current?.status !== "ready" ||
    !Array.isArray(current.rows) ||
    current.rows.length === 0 ||
    !Array.isArray(current.detector_coverage)
  ) {
    fail(`${label} is not a ready memory-current-results snapshot`);
  }
  if (rustiaOnly && current.rows.some((row) => row.engine !== "rustia")) {
    fail(`${label} must contain only Rustia rows`);
  }
}

// Every row that will stand in the merged table must name its machine. The
// generated rows are checked whole; the stored table is checked only on the
// rows the merge keeps, because a stored Rustia row is about to be replaced
// and refusing over it would make a stale, unlabelled Rustia row permanent --
// the one row this merge exists to replace.
function requireLabelledRows(rows, label) {
  const unlabelled = rows.filter((row) => typeof row.host !== "string" || row.host === "");
  if (unlabelled.length !== 0) {
    fail(
      `${label} has ${unlabelled.length} row(s) that do not name the machine they were measured on ` +
      `(${[...new Set(unlabelled.map((row) => row.engine))].sort().join(", ")}); ` +
      "a row that cannot say where it came from cannot be merged with one that can",
    );
  }
}

function merge(config) {
  const generated = readJson(config.generatedCurrent, "generated current memory results");
  validateCurrent(generated, "generated current memory results", true);
  requireLabelledRows(generated.rows, "generated current memory results");
  const generatedHistory = readHistory(config.generatedHistory);
  const cohortId = generated.metadata?.cohort_id;
  const commit = generated.metadata?.commit;
  const generatedCohorts = generatedHistory.filter(
    (record) => record.cohort_id === cohortId && record.commit === commit,
  );
  if (generatedCohorts.length !== 1) {
    fail("generated history must contain exactly one cohort matching generated current results");
  }

  const currentPath = path.join(config.dataRoot, "bench", "memory", "current-results.json");
  const historyPath = path.join(config.dataRoot, "bench", "memory", "history", "runs.jsonl");
  const existingCurrent = fs.existsSync(currentPath) ? readJson(currentPath, "stored current memory results") : null;
  if (existingCurrent != null) validateCurrent(existingCurrent, "stored current memory results", false);
  const existingHistory = readHistory(historyPath);
  const duplicate = existingHistory.find((record) => record.cohort_id === cohortId);
  if (duplicate != null && JSON.stringify(duplicate) !== JSON.stringify(generatedCohorts[0])) {
    fail(`stored memory cohort ${cohortId} differs from the generated cohort`);
  }
  const mergedHistory = duplicate == null
    ? [...existingHistory, generatedCohorts[0]]
    : existingHistory;

  const retainedRows = (existingCurrent?.rows ?? []).filter((row) => row.engine !== "rustia");
  requireLabelledRows(retainedRows, "stored current memory results");
  const rows = [
    ...retainedRows,
    ...generated.rows,
  ].sort(
    (left, right) =>
      left.workload.localeCompare(right.workload) ||
      left.engine.localeCompare(right.engine),
  );
  // This merge is per-engine: a Rustia-only run replaces the Rustia rows and
  // keeps every competitor row standing. That is how a table came to hold six
  // weeks of rented-cloud competitor rows under metadata announcing the owned
  // host -- the mix was invisible because no row said where it came from. Now
  // they do, so the merge stops rather than publishing a table whose rows were
  // measured on different machines. The way forward is to re-measure the
  // retained engines on the same machine, or to retire their rows; it is not to
  // publish a comparison between two boxes as though it were one.
  const hosts = [...new Set(rows.map((row) => row.host))].sort();
  if (hosts.length !== 1) {
    const byHost = hosts.map(
      (host) => `${host}: ${[...new Set(rows.filter((row) => row.host === host).map((row) => row.engine))].sort().join(", ")}`,
    );
    fail(
      `merging would publish a memory table measured on ${hosts.length} machines ` +
      `(${byHost.join("; ")}); re-measure the retained engines on ${generated.rows[0].host} or retire their rows`,
    );
  }
  const detectorCoverage = [
    ...(existingCurrent?.detector_coverage ?? []).filter((entry) => entry.engine !== "rustia"),
    ...generated.detector_coverage,
  ].sort((left, right) => left.engine.localeCompare(right.engine));
  const sourceCohorts = [
    ...new Set([
      ...(existingCurrent?.metadata?.source_cohorts ?? []),
      existingCurrent?.metadata?.cohort_id,
      cohortId,
    ].filter(Boolean)),
  ];
  const mergedCurrent = {
    ...generated,
    metadata: {
      ...generated.metadata,
      description: "Latest compatible dedicated-Linux memory rows by engine; Rustia is refreshed by successful main CI.",
      history: "history/runs.jsonl",
      targets: [...new Set(rows.map((row) => row.target))].sort(),
      hosts,
      engines: [...new Set(rows.map((row) => row.engine))].sort(),
      workloads: [...new Set(rows.map((row) => row.workload))].sort(),
      row_count: rows.length,
      raw_sample_count: rows.reduce(
        (total, row) => total + Number(row.metrics?.peak_delta_kib?.sample_count ?? 0),
        0,
      ),
      source_cohorts: sourceCohorts,
    },
    detector_coverage: detectorCoverage,
    rows,
  };

  writeAtomic(historyPath, `${mergedHistory.map((record) => JSON.stringify(record)).join("\n")}\n`);
  writeAtomic(currentPath, `${JSON.stringify(mergedCurrent, null, 2)}\n`);
  return {
    cohortId,
    commit,
    historyCount: mergedHistory.length,
    rowCount: rows.length,
  };
}

function selfTest() {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-memory-data-"));
  try {
    const dataRoot = path.join(temporary, "data");
    const generatedCurrent = path.join(temporary, "generated-current.json");
    const generatedHistory = path.join(temporary, "generated-history.jsonl");
    const currentPath = path.join(dataRoot, "bench", "memory", "current-results.json");
    const historyPath = path.join(dataRoot, "bench", "memory", "history", "runs.jsonl");
    const metric = { sample_count: 9, median: 16 };
    const row = (engine, commit, host = "ci-bench") => ({
      commit,
      target: "linux-x86_64",
      host,
      engine,
      workload: "dense_tables",
      metrics: { peak_delta_kib: metric },
    });
    const existingCohort = {
      schema_version: 1,
      schema: COHORT_SCHEMA,
      cohort_id: "baseline",
      commit: "a".repeat(40),
      rows: [row("mlua-luau-jit", "a".repeat(40))],
    };
    const generatedCohort = {
      schema_version: 1,
      schema: COHORT_SCHEMA,
      cohort_id: "rustia-ci-bbbbbbb",
      commit: "b".repeat(40),
      rows: [row("rustia", "b".repeat(40))],
    };
    writeAtomic(historyPath, `${JSON.stringify(existingCohort)}\n`);
    writeAtomic(currentPath, `${JSON.stringify({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: "baseline" },
      detector_coverage: [{ engine: "mlua-luau-jit" }],
      rows: existingCohort.rows,
    })}\n`);
    writeAtomic(generatedCurrent, `${JSON.stringify({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: generatedCohort.cohort_id, commit: generatedCohort.commit },
      detector_coverage: [{ engine: "rustia" }],
      rows: generatedCohort.rows,
    })}\n`);
    writeAtomic(generatedHistory, `${JSON.stringify(existingCohort)}\n${JSON.stringify(generatedCohort)}\n`);
    const first = merge({ generatedCurrent, generatedHistory, dataRoot });
    const second = merge({ generatedCurrent, generatedHistory, dataRoot });
    const current = readJson(currentPath, "self-test current");
    if (
      first.historyCount !== 2 ||
      second.historyCount !== 2 ||
      current.rows.length !== 2 ||
      !current.rows.some((entry) => entry.engine === "mlua-luau-jit") ||
      !current.rows.some((entry) => entry.engine === "rustia")
    ) {
      fail("self-test did not preserve competitors or idempotent history");
    }
    if (JSON.stringify(current.metadata.hosts) !== JSON.stringify(["ci-bench"])) {
      fail("self-test did not record the machine behind the merged table");
    }

    // The defect this merge produced once: a Rustia-only run refreshes the
    // Rustia rows and every competitor row stays, so a table can end up holding
    // rows from two machines under metadata naming only one. It must now stop.
    writeAtomic(currentPath, `${JSON.stringify({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: "baseline" },
      detector_coverage: [{ engine: "mlua-luau-jit" }],
      rows: [row("mlua-luau-jit", "a".repeat(40), "a-retired-rental")],
    })}\n`);
    let refused = false;
    try {
      merge({ generatedCurrent, generatedHistory, dataRoot });
    } catch (error) {
      refused = /2 machines/.test(error.message);
    }
    if (!refused) fail("self-test merged competitor rows measured on a different machine");

    // A stored Rustia row that predates host labels is about to be replaced,
    // so it must not block the merge that replaces it; a stored competitor row
    // that predates them is about to be published, so it must.
    const storedTable = (storedRows) => ({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: "baseline" },
      detector_coverage: [{ engine: "mlua-luau-jit" }],
      rows: storedRows,
    });
    writeAtomic(currentPath, `${JSON.stringify(storedTable([
      row("mlua-luau-jit", "a".repeat(40)),
      { ...row("rustia", "a".repeat(40)), host: undefined },
    ]))}\n`);
    const replaced = merge({ generatedCurrent, generatedHistory, dataRoot });
    const replacedCurrent = readJson(currentPath, "self-test current");
    if (
      replaced.rowCount !== 2 ||
      !replacedCurrent.rows.every((entry) => entry.host === "ci-bench")
    ) {
      fail("self-test did not replace an unlabelled stored Rustia row");
    }
    writeAtomic(currentPath, `${JSON.stringify(storedTable([
      { ...row("mlua-luau-jit", "a".repeat(40)), host: undefined },
    ]))}\n`);
    let refusedUnlabelled = false;
    try {
      merge({ generatedCurrent, generatedHistory, dataRoot });
    } catch (error) {
      refusedUnlabelled = /do not name the machine/.test(error.message) && /mlua-luau-jit/.test(error.message);
    }
    if (!refusedUnlabelled) fail("self-test merged an unlabelled stored competitor row");
    const generatedUnlabelled = path.join(temporary, "generated-unlabelled.json");
    writeAtomic(generatedUnlabelled, `${JSON.stringify({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: generatedCohort.cohort_id, commit: generatedCohort.commit },
      detector_coverage: [{ engine: "rustia" }],
      rows: [{ ...row("rustia", "b".repeat(40)), host: "" }],
    })}\n`);
    let refusedGenerated = false;
    try {
      merge({ generatedCurrent: generatedUnlabelled, generatedHistory, dataRoot });
    } catch (error) {
      refusedGenerated = /generated current memory results has 1 row/.test(error.message);
    }
    if (!refusedGenerated) fail("self-test merged a generated row with an empty host");
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
  process.stdout.write("Rustia CI memory data merge self-test passed\n");
}

try {
  const config = parseArgs(process.argv.slice(2));
  if (config.command === "self-test") {
    selfTest();
  } else {
    const result = merge(config);
    process.stdout.write(
      `merged ${result.cohortId} (${result.commit.slice(0, 12)}): ${result.rowCount} current rows, ${result.historyCount} history cohorts\n`,
    );
  }
} catch (error) {
  process.stderr.write(`merge-rustia-ci-memory-data: ${error.message}\n`);
  process.exitCode = 1;
}
