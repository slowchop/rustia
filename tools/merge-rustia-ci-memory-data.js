#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const CURRENT_SCHEMA = "rustia.bench.memory-current-results.v1";
const COHORT_SCHEMA = "rustia.bench.memory-cohort.v1";
// The machine a merged row names is a provenance label published in row data,
// and this merge writes the memory table and history that are pushed to the
// public data repository, so it is the last place an address can be refused
// before it is published. A label, never an address: no dots and no colons, so
// it can be neither a fully qualified name nor an IPv4 or IPv6 address. Every
// emitter and validator carries this same literal; the self-test in
// tools/check-benchmark-results.js asserts it.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

function fail(message) {
  throw new Error(message);
}

// `subject` is who names the machine and how, so the refusal blames the side
// that is wrong.
function requireHostLabel(value, subject) {
  if (typeof value !== "string" || !HOST_LABEL.test(value)) {
    fail(
      `${subject} ${JSON.stringify(value)}, which is not a label: ` +
      "a label may contain only [A-Za-z0-9_-], never an address",
    );
  }
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

// Every row that will stand in the merged table must name its machine, and
// name it with a label. The generated rows are checked whole; the stored table
// is checked only on the rows the merge keeps, because a stored row for an
// engine the new cohort measured is about to be replaced, and refusing over it
// would make a stale, unlabelled row permanent -- the one row this merge exists
// to replace. That is also what lets a cross-engine cohort replace, whole, a
// table measured before hosts were recorded.
function requireLabelledRows(rows, label) {
  const unlabelled = rows.filter((row) => typeof row.host !== "string" || row.host === "");
  if (unlabelled.length !== 0) {
    fail(
      `${label} has ${unlabelled.length} row(s) that do not name the machine they were measured on ` +
      `(${[...new Set(unlabelled.map((row) => row.engine))].sort().join(", ")}); ` +
      "a row that cannot say where it came from cannot be merged with one that can",
    );
  }
  for (const row of rows) requireHostLabel(row.host, `${label} row ${row.engine}/${row.workload} names its machine`);
}

// A cohort is one run on one machine, however many engines it measured. The
// generated snapshot's rows must agree on that machine, and so must the two
// places that restate it: the snapshot's metadata.host, which the merged table
// inherits, and the history line appended beside it, which the memory history
// chart is keyed on. Both restatements are published, so each is checked as a
// label before it is compared with the rows: an address is refused as an
// address, not as a disagreement. Returns the machine.
function requireOneMachine(generated, record) {
  const label = "generated current memory results";
  const hosts = [...new Set(generated.rows.map((row) => row.host))].sort();
  if (hosts.length !== 1) {
    fail(`${label} was measured on ${hosts.length} machines (${hosts.join(", ")}); a cohort is one run on one machine`);
  }
  const [host] = hosts;
  if (generated.metadata?.host !== undefined) requireHostLabel(generated.metadata.host, `${label} names host`);
  if (record.host !== undefined) requireHostLabel(record.host, `generated history cohort ${record.cohort_id} names host`);
  for (const row of record.rows ?? []) {
    if (row.host !== undefined) {
      requireHostLabel(row.host, `generated history cohort ${record.cohort_id} row ${row.engine}/${row.workload} names its machine`);
    }
  }
  if (generated.metadata?.host !== undefined && generated.metadata.host !== host) {
    fail(`${label} names host ${JSON.stringify(generated.metadata.host)} but its rows name ${JSON.stringify(host)}`);
  }
  const strays = (record.rows ?? []).filter((row) => row.host !== host);
  if ((record.host !== undefined && record.host !== host) || strays.length !== 0) {
    fail(
      `generated history cohort ${record.cohort_id} does not name ${JSON.stringify(host)}, the machine its ` +
      "snapshot's rows were measured on; the snapshot and its history line are one run",
    );
  }
  return host;
}

// Each engine's rows come from the newest cohort that measured that engine.
// A rustia-only cohort therefore swaps the Rustia rows and keeps every
// competitor row; a cross-engine cohort swaps them all. Prior rows for an
// engine the cohort measured are dropped whole rather than merged by
// workload, so one engine never mixes two cohorts' measurements.
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
  const cohortHost = requireOneMachine(generated, generatedCohorts[0]);

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
  requireLabelledRows(survivors, "stored current memory results");
  const rows = [...survivors, ...generated.rows].sort(
    (left, right) =>
      left.workload.localeCompare(right.workload) ||
      left.engine.localeCompare(right.engine),
  );
  // This merge is per-engine: a rustia-only cohort replaces the Rustia rows and
  // keeps every competitor row standing. That is how a table came to hold six
  // weeks of rented-cloud competitor rows under metadata announcing the owned
  // host -- the mix was invisible because no row said where it came from. Now
  // they do, so the merge stops rather than publishing a table whose rows were
  // measured on different machines. The generated cohort is one machine, so any
  // other machine here is a survivor's. The way forward is to re-measure those
  // engines on the cohort's machine -- a cross-engine cohort replaces every
  // matrix engine at once -- or to retire their rows; it is not to publish a
  // comparison between two boxes as though it were one.
  const hosts = [...new Set(rows.map((row) => row.host))].sort();
  if (hosts.length !== 1) {
    const byHost = hosts.map(
      (host) => `${host}: ${[...new Set(rows.filter((row) => row.host === host).map((row) => row.engine))].sort().join(", ")}`,
    );
    const elsewhere = [...new Set(survivors.filter((row) => row.host !== cohortHost).map((row) => row.engine))].sort();
    fail(
      `merging would publish a memory table measured on ${hosts.length} machines ` +
      `(${byHost.join("; ")}); re-measure ${elsewhere.join(", ")} on ${cohortHost} ` +
      "(a cross-engine cohort measures every matrix engine) or retire their rows",
    );
  }
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
    // Shaped as bench/publish-memory-analysis.js writes them: the history line
    // and the snapshot's metadata each restate the one machine the rows name.
    const cohort = (id, commit, engines, host = "ci-bench") => ({
      schema_version: 1,
      schema: COHORT_SCHEMA,
      cohort_id: id,
      commit,
      host,
      rows: engines.map((engine) => row(engine, commit, host)),
    });
    const snapshot = (record) => ({
      schema_version: 1,
      schema: CURRENT_SCHEMA,
      status: "ready",
      metadata: { cohort_id: record.cohort_id, commit: record.commit, host: record.host },
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
    if (JSON.stringify(current.metadata.hosts) !== JSON.stringify(["ci-bench"]) || current.metadata.host !== "ci-bench") {
      fail("self-test did not record the machine behind the merged table");
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
    if (
      current.metadata.engines.join() !== "koto,mlua-luau-jit,rustia" ||
      current.metadata.row_count !== 3 ||
      current.metadata.hosts.join() !== "ci-bench"
    ) {
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

    // Machine provenance. A refused merge writes nothing, so each case below
    // stages its own stored table rather than inheriting the last case's.
    const refuses = (patterns, message) => {
      let refusal = null;
      try {
        merge({ generatedCurrent, generatedHistory, dataRoot });
      } catch (error) {
        refusal = error.message;
      }
      if (refusal == null) fail(`self-test ${message}`);
      if (!patterns.every((pattern) => pattern.test(refusal))) {
        fail(`self-test refused for the wrong reason (${message}): ${refusal}`);
      }
    };
    const store = (storedRows) =>
      writeAtomic(currentPath, `${JSON.stringify({
        schema_version: 1,
        schema: CURRENT_SCHEMA,
        status: "ready",
        metadata: { cohort_id: "stored" },
        detector_coverage: [...new Set(storedRows.map((entry) => entry.engine))].map((engine) => ({ engine })),
        rows: storedRows,
      })}\n`);
    // JSON.stringify drops an undefined key, so this is a row written before
    // hosts were recorded.
    const unlabelled = (entry) => ({ ...entry, host: undefined });
    const recorded = [baseline, rustiaB, crossC, rustiaD, rustiaE];

    // The defect this merge produced once: a rustia-only cohort refreshes the
    // Rustia rows and every competitor row stays, so a table can end up holding
    // rows from two machines under metadata naming only one. It must now stop.
    stage(rustiaE, recorded);
    store([row("mlua-luau-jit", "a".repeat(40), "a-retired-rental"), row("rustia", "d".repeat(40))]);
    refuses(
      [/2 machines/, /a-retired-rental: mlua-luau-jit/, /re-measure mlua-luau-jit on ci-bench/],
      "merged competitor rows measured on a different machine",
    );

    // A stored Rustia row that predates host labels is about to be replaced,
    // so it must not block the merge that replaces it; a stored competitor row
    // that predates them is about to be published, so it must.
    store([row("mlua-luau-jit", "c".repeat(40)), unlabelled(row("rustia", "a".repeat(40)))]);
    const replaced = merge({ generatedCurrent, generatedHistory, dataRoot });
    current = readJson(currentPath, "self-test current");
    if (replaced.rowCount !== 2 || !current.rows.every((entry) => entry.host === "ci-bench")) {
      fail("self-test did not replace an unlabelled stored Rustia row");
    }
    store([unlabelled(row("mlua-luau-jit", "c".repeat(40)))]);
    refuses(
      [/stored current memory results has 1 row/, /do not name the machine/, /mlua-luau-jit/],
      "merged an unlabelled stored competitor row",
    );

    // A cross-engine cohort is the way out of a mixed or unlabelled table: it
    // replaces every engine it measured, whatever machine -- or none -- the
    // stored rows name, and the table it leaves names one machine.
    const crossG = cohort("cross-engine-1111111", "1".repeat(40), ["rustia", "mlua-luau-jit", "koto"]);
    stage(crossG, [...recorded, crossG]);
    store([
      unlabelled(row("rustia", "a".repeat(40))),
      row("mlua-luau-jit", "a".repeat(40), "a-retired-rental"),
      unlabelled(row("koto", "a".repeat(40))),
    ]);
    const remeasured = merge({ generatedCurrent, generatedHistory, dataRoot });
    current = readJson(currentPath, "self-test current");
    if (
      remeasured.rowCount !== 3 ||
      !current.rows.every((entry) => entry.host === "ci-bench" && entry.commit === "1".repeat(40)) ||
      JSON.stringify(current.metadata.hosts) !== JSON.stringify(["ci-bench"]) ||
      current.metadata.host !== "ci-bench"
    ) {
      fail("self-test cross-engine cohort did not replace a table measured elsewhere");
    }
    // It does not excuse an engine it did not measure: a survivor measured on
    // another machine still makes the table two machines.
    store([row("boa", "a".repeat(40), "a-retired-rental"), row("rustia", "d".repeat(40))]);
    refuses(
      [/2 machines/, /a-retired-rental: boa/, /re-measure boa on ci-bench/],
      "let a cross-engine cohort keep a survivor measured on another machine",
    );

    // A cohort is one machine however many engines it measured, and the
    // snapshot's metadata and its history line must name the machine its rows do.
    // The stored table is one machine, so only the generated side can refuse.
    store([row("mlua-luau-jit", "1".repeat(40)), row("rustia", "1".repeat(40))]);
    const split = cohort("cross-engine-2222222", "2".repeat(40), ["rustia", "mlua-luau-jit", "koto"]);
    split.rows[2] = row("koto", split.commit, "other-box");
    stage(split, [...recorded, crossG, split]);
    refuses(
      [/generated current memory results was measured on 2 machines \(ci-bench, other-box\)/],
      "merged a cohort measured on two machines",
    );
    const misnamed = cohort("rustia-ci-3333333", "3".repeat(40), ["rustia"]);
    writeAtomic(generatedCurrent, `${JSON.stringify({
      ...snapshot(misnamed),
      metadata: { ...snapshot(misnamed).metadata, host: "other-box" },
    })}\n`);
    writeAtomic(generatedHistory, `${JSON.stringify(misnamed)}\n`);
    refuses(
      [/names host "other-box" but its rows name "ci-bench"/],
      "merged a snapshot whose metadata names another machine",
    );
    stage(misnamed, [{ ...misnamed, host: "other-box" }]);
    refuses(
      [/history cohort rustia-ci-3333333 does not name "ci-bench"/],
      "appended a history line naming another machine",
    );

    // A generated row must name its machine too; an empty label names none.
    const blank = cohort("rustia-ci-4444444", "4".repeat(40), ["rustia"]);
    blank.rows = [{ ...blank.rows[0], host: "" }];
    stage(blank, [blank]);
    refuses(
      [/generated current memory results has 1 row/, /do not name the machine/],
      "merged a generated row with an empty host",
    );

    // A label, never an address, wherever this merge would publish one: a
    // generated row, a stored row it keeps, the snapshot's metadata.host, and
    // the history line and its rows. Each case names the address in one place
    // only and expects the refusal to name that place.
    const literal = (text) => new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
    const notLabel = (subject, address) => literal(`${subject} ${JSON.stringify(address)}, which is not a label`);
    for (const [index, address] of ["bench.example.com", "10.0.0.5", "fe80::1", "user@box"].entries()) {
      const addressed = cohort(`rustia-ci-address-${index}`, "5".repeat(40), ["rustia"], address);
      stage(addressed, [addressed]);
      refuses(
        [notLabel("generated current memory results row rustia/dense_tables names its machine", address)],
        `merged a generated row naming ${address}`,
      );

      const labelled = cohort(`rustia-ci-labelled-${index}`, "6".repeat(40), ["rustia"]);
      stage(labelled, [labelled]);
      store([row("mlua-luau-jit", "c".repeat(40), address), row("rustia", "d".repeat(40))]);
      refuses(
        [notLabel("stored current memory results row mlua-luau-jit/dense_tables names its machine", address)],
        `kept a stored row naming ${address}`,
      );

      store([row("mlua-luau-jit", "1".repeat(40)), row("rustia", "1".repeat(40))]);
      writeAtomic(generatedCurrent, `${JSON.stringify({
        ...snapshot(labelled),
        metadata: { ...snapshot(labelled).metadata, host: address },
      })}\n`);
      writeAtomic(generatedHistory, `${JSON.stringify(labelled)}\n`);
      refuses(
        [notLabel("generated current memory results names host", address)],
        `merged a snapshot whose metadata names ${address}`,
      );

      stage(labelled, [{ ...labelled, host: address }]);
      refuses(
        [notLabel(`generated history cohort ${labelled.cohort_id} names host`, address)],
        `appended a history line naming ${address}`,
      );

      stage(labelled, [{ ...labelled, rows: labelled.rows.map((entry) => ({ ...entry, host: address })) }]);
      refuses(
        [notLabel(`generated history cohort ${labelled.cohort_id} row rustia/dense_tables names its machine`, address)],
        `appended a history row naming ${address}`,
      );
    }
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
