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

function validateCurrent(current, label, generated) {
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
  // Both cohorts the lane measures carry Rustia; a cohort without it was
  // produced by something other than the lane and must not be merged.
  if (generated && !current.rows.some((row) => row.engine === "rustia")) {
    fail(`${label} must contain Rustia rows`);
  }
}

// Each engine's rows come from the newest cohort that measured that engine.
// A rustia-only cohort therefore swaps the Rustia rows and keeps every
// competitor row; a cross-engine cohort swaps them all. Prior rows for an
// engine the cohort measured are dropped whole rather than merged by
// workload, so one engine never mixes two cohorts' measurements.
function merge(config) {
  const generated = readJson(config.generatedCurrent, "generated current memory results");
  validateCurrent(generated, "generated current memory results", true);
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

  const measured = new Set(generated.rows.map((row) => row.engine));
  const survivors = (existingCurrent?.rows ?? []).filter((row) => !measured.has(row.engine));
  const rows = [...survivors, ...generated.rows].sort(
    (left, right) =>
      left.workload.localeCompare(right.workload) ||
      left.engine.localeCompare(right.engine),
  );
  const detectorCoverage = [
    ...(existingCurrent?.detector_coverage ?? []).filter((entry) => !measured.has(entry.engine)),
    ...generated.detector_coverage,
  ].sort((left, right) => left.engine.localeCompare(right.engine));
  // The cohorts whose rows are in the table, in history order. A cohort whose
  // every row the new cohort replaced leaves the list with those rows; the
  // new cohort is in the history by now and always contributes.
  const rowKeys = new Set(rows.map((row) => `${row.engine}\0${row.commit}`));
  const sourceCohorts = mergedHistory
    .filter((record) => (record.rows ?? []).some((row) => rowKeys.has(`${row.engine}\0${row.commit}`)))
    .map((record) => record.cohort_id);
  const mergedCurrent = {
    ...generated,
    metadata: {
      ...generated.metadata,
      description:
        "Latest dedicated-Linux memory rows by engine; each engine's rows come from the newest " +
        "cohort that measured it, and Rustia is refreshed by every memory cohort.",
      history: "history/runs.jsonl",
      targets: [...new Set(rows.map((row) => row.target))].sort(),
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
    const row = (engine, commit) => ({
      commit,
      target: "linux-x86_64",
      engine,
      workload: "dense_tables",
      metrics: { peak_delta_kib: metric },
    });
    const cohort = (id, commit, engines) => ({
      schema_version: 1,
      schema: COHORT_SCHEMA,
      cohort_id: id,
      commit,
      rows: engines.map((engine) => row(engine, commit)),
    });
    const snapshot = (record) => ({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: record.cohort_id, commit: record.commit },
      detector_coverage: record.rows.map((entry) => ({ engine: entry.engine })),
      rows: record.rows,
    });
    const engineCommits = () =>
      Object.fromEntries(readJson(currentPath, "self-test current").rows.map((entry) => [entry.engine, entry.commit]));
    const stage = (generated, history) => {
      writeAtomic(generatedCurrent, `${JSON.stringify(snapshot(generated))}\n`);
      writeAtomic(generatedHistory, `${history.map((record) => JSON.stringify(record)).join("\n")}\n`);
    };

    // A rustia-only cohort against a stored competitor row: the competitor
    // stays, Rustia is added, and a second merge changes nothing.
    const baseline = cohort("baseline", "a".repeat(40), ["mlua-luau-jit"]);
    const rustiaB = cohort("rustia-ci-bbbbbbb", "b".repeat(40), ["rustia"]);
    writeAtomic(historyPath, `${JSON.stringify(baseline)}\n`);
    writeAtomic(currentPath, `${JSON.stringify(snapshot(baseline))}\n`);
    stage(rustiaB, [baseline, rustiaB]);
    const first = merge({ generatedCurrent, generatedHistory, dataRoot });
    const second = merge({ generatedCurrent, generatedHistory, dataRoot });
    if (first.historyCount !== 2 || second.historyCount !== 2) fail("self-test history is not idempotent");
    if (JSON.stringify(engineCommits()) !== JSON.stringify({ "mlua-luau-jit": "a".repeat(40), rustia: "b".repeat(40) })) {
      fail("self-test did not preserve competitors under a rustia-only cohort");
    }
    let current = readJson(currentPath, "self-test current");
    if (JSON.stringify(current.metadata.source_cohorts) !== JSON.stringify(["baseline", "rustia-ci-bbbbbbb"])) {
      fail(`self-test source cohorts after rustia-only: ${current.metadata.source_cohorts}`);
    }

    // A cross-engine cohort replaces every engine it measured: no engine
    // appears twice, the stale competitor row is gone, and the cohorts whose
    // rows were replaced leave the source list with them.
    const crossC = cohort("cross-engine-ccccccc", "c".repeat(40), ["rustia", "mlua-luau-jit", "koto"]);
    stage(crossC, [baseline, rustiaB, crossC]);
    const third = merge({ generatedCurrent, generatedHistory, dataRoot });
    merge({ generatedCurrent, generatedHistory, dataRoot });
    current = readJson(currentPath, "self-test current");
    if (third.historyCount !== 3 || third.rowCount !== 3 || current.rows.length !== 3) {
      fail("self-test cross-engine merge did not replace the rows wholesale");
    }
    const commitsAfterCross = engineCommits();
    if (Object.keys(commitsAfterCross).length !== 3 || Object.values(commitsAfterCross).some((c) => c !== "c".repeat(40))) {
      fail("self-test cross-engine merge left a row from another cohort");
    }
    if (JSON.stringify(current.metadata.source_cohorts) !== JSON.stringify(["cross-engine-ccccccc"])) {
      fail(`self-test source cohorts after cross-engine: ${current.metadata.source_cohorts}`);
    }
    if (current.detector_coverage.length !== 3) fail("self-test detector coverage was not replaced");

    // The next rustia-only cohort swaps only the Rustia rows: the competitor
    // rows from the cross-engine cohort stay and are counted once.
    const rustiaD = cohort("rustia-ci-ddddddd", "d".repeat(40), ["rustia"]);
    stage(rustiaD, [baseline, rustiaB, crossC, rustiaD]);
    const fourth = merge({ generatedCurrent, generatedHistory, dataRoot });
    current = readJson(currentPath, "self-test current");
    const expected = { koto: "c".repeat(40), "mlua-luau-jit": "c".repeat(40), rustia: "d".repeat(40) };
    if (fourth.rowCount !== 3 || JSON.stringify(engineCommits()) !== JSON.stringify(expected)) {
      fail("self-test rustia-only merge after a cross-engine cohort did not swap only Rustia");
    }
    if (JSON.stringify(current.metadata.source_cohorts) !== JSON.stringify(["cross-engine-ccccccc", "rustia-ci-ddddddd"])) {
      fail(`self-test source cohorts after the follow-on: ${current.metadata.source_cohorts}`);
    }
    if (current.metadata.engines.join() !== "koto,mlua-luau-jit,rustia" || current.metadata.row_count !== 3) {
      fail("self-test metadata does not describe the merged rows");
    }
    // Two routine cohorts in a row: the earlier one's rows are all replaced,
    // so its id leaves the source list rather than accumulating.
    const rustiaE = cohort("rustia-ci-eeeeeee", "e".repeat(40), ["rustia"]);
    stage(rustiaE, [baseline, rustiaB, crossC, rustiaD, rustiaE]);
    const fifth = merge({ generatedCurrent, generatedHistory, dataRoot });
    current = readJson(currentPath, "self-test current");
    if (
      fifth.rowCount !== 3 ||
      engineCommits().rustia !== "e".repeat(40) ||
      JSON.stringify(current.metadata.source_cohorts) !== JSON.stringify(["cross-engine-ccccccc", "rustia-ci-eeeeeee"])
    ) {
      fail(`self-test source cohorts after back-to-back routine cohorts: ${current.metadata.source_cohorts}`);
    }

    // A cohort without Rustia rows did not come from the lane.
    stage(cohort("foreign", "f".repeat(40), ["koto"]), [cohort("foreign", "f".repeat(40), ["koto"])]);
    let refused = false;
    try {
      merge({ generatedCurrent, generatedHistory, dataRoot });
    } catch (error) {
      refused = /must contain Rustia rows/.test(error.message);
    }
    if (!refused) fail("self-test accepted a cohort without Rustia rows");
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
