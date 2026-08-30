#!/usr/bin/env node
"use strict";

// Validates the live per-commit Rustia series against its committed protocol.
//
// The protocol (bench/history/series/*.json) lives in this repository, where it
// is reviewed. The rows it describes live in the published data repository,
// following the memory lane, because they grow with every commit. That split
// means this checker usually runs inside the publication workflow with the data
// repository already cloned, and is given the rows path explicitly.
//
// The load-bearing rule is checksum equality. Identical checksums across every
// commit are what make the series a measurement of speed rather than of a
// drifting workload; the 75-revision reconstruction demonstrated exactly that
// property, and a live series is worth nothing without it.

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const SERIES_DIR = path.join(REPO, "bench/history/series");
const FULL_COMMIT = /^[0-9a-f]{40}$/;

function fail(message) {
  console.error(`check-benchmark-series: ${message}`);
  process.exit(1);
}

function loadJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`could not read ${label}: ${error.message}`);
  }
}

function validateManifest(manifest, label) {
  if (manifest.schema !== "rustia.bench.history-series.v1" || manifest.schema_version !== 1) {
    fail(`${label} must declare rustia.bench.history-series.v1`);
  }
  for (const key of ["host", "harness_commit", "protocol", "native_target", "rows_file"]) {
    if (typeof manifest[key] !== "string" || manifest[key] === "") {
      fail(`${label}.${key} must be a non-empty string`);
    }
  }
  if (!FULL_COMMIT.test(manifest.harness_commit)) {
    fail(`${label}.harness_commit must be a full 40-hex commit`);
  }
  if (!Number.isSafeInteger(manifest.pinned_core)) fail(`${label}.pinned_core must be an integer`);
  if (!Number.isSafeInteger(manifest.repetitions) || manifest.repetitions < 1) {
    fail(`${label}.repetitions must be a positive integer`);
  }
  if (!Array.isArray(manifest.workloads) || manifest.workloads.length === 0) {
    fail(`${label}.workloads must be a non-empty array`);
  }
  for (const workload of manifest.workloads) {
    if (typeof manifest.fixed_n?.[workload] !== "number") {
      fail(`${label}.fixed_n is missing ${workload}`);
    }
    if (typeof manifest.expected_checksums?.[workload] !== "string") {
      fail(`${label}.expected_checksums is missing ${workload}`);
    }
  }
  return manifest;
}

function parseRows(file, label) {
  const text = fs.readFileSync(file, "utf8");
  const rows = [];
  for (const [index, line] of text.split("\n").entries()) {
    if (line.trim() === "") continue;
    try {
      rows.push({ row: JSON.parse(line), line: index + 1 });
    } catch (error) {
      fail(`${label} line ${index + 1} is not JSON: ${error.message}`);
    }
  }
  return rows;
}

function isAncestorOfHead(commit) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", commit, "HEAD"], {
      cwd: REPO,
      stdio: "ignore",
    });
    return true;
  } catch {
    return false;
  }
}

function validateRows(manifest, entries, label, { checkAncestry }) {
  const workloads = new Set(manifest.workloads);
  const perCommit = new Map();
  let previousTimestamp = 0;
  for (const { row, line } of entries) {
    const at = `${label} line ${line}`;
    if (row.engine !== "rustia") fail(`${at}: series is rustia-only, got engine ${row.engine}`);
    if (row.target !== manifest.native_target) {
      fail(`${at}: target must be ${manifest.native_target}, got ${row.target}`);
    }
    if (row.host !== manifest.host) {
      fail(`${at}: host must be ${manifest.host}, got ${row.host ?? "unrecorded"}`);
    }
    if (!workloads.has(row.workload)) fail(`${at}: unknown workload ${row.workload}`);
    if (row.n !== manifest.fixed_n[row.workload]) {
      fail(`${at}: ${row.workload} must use n=${manifest.fixed_n[row.workload]}, got ${row.n}`);
    }
    if (row.reps !== manifest.repetitions) {
      fail(`${at}: reps must be ${manifest.repetitions}, got ${row.reps}`);
    }
    // The rule the whole series rests on. A changed checksum means the workload
    // changed, so the points either side of it are not measuring the same thing
    // and the line through them is not a performance trend.
    if (row.checksum !== manifest.expected_checksums[row.workload]) {
      fail(
        `${at}: ${row.workload} checksum ${row.checksum} does not match the protocol's ` +
          `${manifest.expected_checksums[row.workload]}; the workload changed, so open a new era ` +
          `rather than extending this one`,
      );
    }
    if (!FULL_COMMIT.test(row.commit ?? "")) fail(`${at}: commit must be a full 40-hex SHA`);
    if (!Number.isSafeInteger(row.timestamp_unix) || row.timestamp_unix <= 0) {
      fail(`${at}: timestamp_unix must be a positive integer`);
    }
    if (row.timestamp_unix < previousTimestamp) {
      fail(`${at}: timestamp_unix must not precede the previous row`);
    }
    previousTimestamp = row.timestamp_unix;
    if (checkAncestry && !isAncestorOfHead(row.commit)) {
      fail(`${at}: ${row.commit.slice(0, 8)} is not an ancestor of HEAD`);
    }
    const seen = perCommit.get(row.commit) ?? new Set();
    if (seen.has(row.workload)) fail(`${at}: duplicate ${row.workload} for ${row.commit.slice(0, 8)}`);
    seen.add(row.workload);
    perCommit.set(row.commit, seen);
  }
  for (const [commit, seen] of perCommit) {
    if (seen.size !== workloads.size) {
      fail(
        `${label}: ${commit.slice(0, 8)} has ${seen.size} of ${workloads.size} workloads; ` +
          `a commit is recorded complete or not at all`,
      );
    }
  }
  return { commits: perCommit.size, rows: entries.length };
}

function selfTest() {
  const assert = require("node:assert/strict");
  const manifest = {
    schema: "rustia.bench.history-series.v1",
    schema_version: 1,
    host: "ci-bench",
    pinned_core: 4,
    harness_commit: "a".repeat(40),
    protocol: "rustia_main_series_v1",
    native_target: "linux-x86_64",
    workloads: ["w1", "w2"],
    fixed_n: { w1: 10, w2: 20 },
    repetitions: 5,
    expected_checksums: { w1: "number:1", w2: "number:2" },
    rows_file: "x.jsonl",
  };
  validateManifest(manifest, "fixture");

  const base = (over = {}) => ({
    engine: "rustia", target: "linux-x86_64", host: "ci-bench", workload: "w1",
    n: 10, reps: 5, checksum: "number:1", commit: "b".repeat(40), timestamp_unix: 100, ...over,
  });
  const rows = (list) => list.map((row, index) => ({ row, line: index + 1 }));
  const expectFail = (name, list) => {
    const realFail = process.exit;
    let failed = false;
    process.exit = () => { failed = true; throw new Error("exited"); };
    const realError = console.error;
    console.error = () => {};
    try { validateRows(manifest, rows(list), "fixture", { checkAncestry: false }); } catch {}
    process.exit = realFail; console.error = realError;
    if (!failed) fail(`self-test: ${name} should have been rejected`);
  };

  const complete = [base(), base({ workload: "w2", n: 20, checksum: "number:2" })];
  const okResult = validateRows(manifest, rows(complete), "fixture", { checkAncestry: false });
  assert.equal(okResult.commits, 1);
  assert.equal(okResult.rows, 2);

  expectFail("a changed checksum", [base({ checksum: "number:999" })]);
  expectFail("a wrong host", [base({ host: "somewhere-else" })]);
  expectFail("a non-rustia engine", [base({ engine: "koto" })]);
  expectFail("a wrong fixed n", [base({ n: 11 })]);
  expectFail("a wrong repetition count", [base({ reps: 9 })]);
  expectFail("a partial commit", [base()]);
  expectFail("a duplicate workload", [base(), base(), base({ workload: "w2", n: 20, checksum: "number:2" })]);
  expectFail("time going backwards", [
    base({ timestamp_unix: 200 }),
    base({ workload: "w2", n: 20, checksum: "number:2", timestamp_unix: 100 }),
  ]);

  process.stdout.write("check-benchmark-series self-test ok\n");
}

function usage() {
  process.stdout.write(
    "Usage: node tools/check-benchmark-series.js [--rows-root DIR]\n" +
      "       node tools/check-benchmark-series.js --self-test\n" +
      "\n" +
      "Validates every bench/history/series/*.json protocol against its rows.\n" +
      "--rows-root names the checkout holding the rows (the data repository);\n" +
      "without it a missing rows file is reported rather than assumed empty.\n",
  );
}

function main(argv) {
  if (argv.includes("--self-test")) {
    if (argv.length !== 1) fail("--self-test takes no other options");
    selfTest();
    return;
  }
  let rowsRoot = null;
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--rows-root") {
      rowsRoot = argv[++index];
      if (!rowsRoot) { usage(); process.exitCode = 2; return; }
    } else {
      usage();
      process.exitCode = 2;
      return;
    }
  }
  if (!fs.existsSync(SERIES_DIR)) {
    process.stdout.write("no benchmark series declared\n");
    return;
  }
  const manifests = fs.readdirSync(SERIES_DIR).filter((name) => name.endsWith(".json")).sort();
  if (manifests.length === 0) {
    process.stdout.write("no benchmark series declared\n");
    return;
  }
  let total = 0;
  for (const name of manifests) {
    const manifestPath = path.join(SERIES_DIR, name);
    const label = `bench/history/series/${name}`;
    const manifest = validateManifest(loadJson(manifestPath, label), label);
    if (rowsRoot === null) {
      process.stdout.write(`${label}: protocol valid (rows not checked; pass --rows-root)\n`);
      continue;
    }
    const rowsPath = path.join(rowsRoot, manifest.rows_file);
    if (!fs.existsSync(rowsPath)) {
      // An absent rows file is the state before the first measured commit, and
      // is reported rather than treated as a failure or as an empty success.
      process.stdout.write(`${label}: no rows yet at ${manifest.rows_file}\n`);
      continue;
    }
    const result = validateRows(
      manifest,
      parseRows(rowsPath, manifest.rows_file),
      manifest.rows_file,
      { checkAncestry: true },
    );
    total += result.rows;
    process.stdout.write(`${label}: ${result.rows} rows across ${result.commits} commits\n`);
  }
  process.stdout.write(`benchmark series ok (${total} row(s))\n`);
}

main(process.argv.slice(2));
