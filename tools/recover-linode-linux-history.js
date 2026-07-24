#!/usr/bin/env node
"use strict";

// Rebuild the append-only Linux benchmark ledger from the published matrix
// snapshots that were accidentally replaced in 2026-07-20..2026-07-23.
// Deliberately excludes pre-Linode/macOS and mixed-host records.

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const OUTPUT = path.join(ROOT, "bench", "history", "runs.jsonl");
const TARGET = "linux-x86_64";
const SNAPSHOTS = Object.freeze([
  { commit: "514982c1d9b91200b0a65eec3b2a6eeff60f8da4", sourceRecords: 540, records: 340 },
  { commit: "a6b8ac897bf3b82a364c59cd30fcb9ac1b5bc3b1", records: 50 },
  { commit: "3bc12d88f25a57e165293a7a6d962ffe9626d999", records: 50 },
  { commit: "cc3deb9d6c43189dfd0f31be3f6847240032f640", records: 50 },
  { commit: "1de5cc3c44da9928bb4d02c43292449876b0e575", records: 50 },
  { commit: "4548534c0089b7ed20c7e61c773788bdb1378ca9", records: 60 },
  { commit: "b9f74406dfd2653a5c41630e203123cd65fb1ad7", records: 60 },
  { commit: "023933953c14fdd0bccf131fdf7a5e67d3652097", records: 60 },
  { commit: "7c64bc732fe8b51827ee2982614d46010a537a34", records: 60 },
  { commit: "a86a3e2a4a591410406fa3d0e24d3dbeaefbfc1a", records: 60 },
  { commit: "01e77983483d080c46123c2213d948f495c93e4f", records: 65 },
]);

function fail(message) {
  process.stderr.write(`recover-linode-linux-history: ${message}\n`);
  process.exit(1);
}

function readSnapshot(snapshot) {
  const result = spawnSync("git", ["show", `${snapshot.commit}:bench/history/runs.jsonl`], {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
  });
  if (result.status !== 0) {
    fail(`could not read ${snapshot.commit}:bench/history/runs.jsonl: ${String(result.stderr).trim()}`);
  }
  const rows = String(result.stdout).trim().split(/\r?\n/).map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      fail(`${snapshot.commit} row ${index + 1} is not JSON: ${error.message}`);
    }
  });
  if (rows.length !== (snapshot.sourceRecords ?? snapshot.records)) {
    fail(`${snapshot.commit} has ${rows.length} rows, expected ${snapshot.sourceRecords ?? snapshot.records}`);
  }
  const targetRows = rows.filter((row) => row.target === TARGET);
  if (targetRows.length !== snapshot.records) {
    fail(`${snapshot.commit} has ${targetRows.length} ${TARGET} rows, expected ${snapshot.records}`);
  }
  return targetRows;
}

const rows = SNAPSHOTS.flatMap(readSnapshot).sort((left, right) =>
  left.timestamp_unix - right.timestamp_unix ||
  left.commit.localeCompare(right.commit) ||
  left.engine.localeCompare(right.engine) ||
  left.workload.localeCompare(right.workload),
);
const uniqueRows = new Set(rows.map((row) => JSON.stringify(row)));
if (uniqueRows.size !== rows.length) fail("recovered snapshots contain duplicate records");
const digest = crypto.createHash("sha256").update(rows.map((row) => JSON.stringify(row)).join("\n")).digest("hex");
const output = `${rows.map((row) => JSON.stringify(row)).join("\n")}\n`;

if (process.argv[2] === "--check") {
  if (fs.readFileSync(OUTPUT, "utf8") !== output) fail("ledger differs from the recoverable Linux snapshot set");
  process.stdout.write(`verified ${rows.length} recovered Linux records (${digest})\n`);
} else if (process.argv.length === 2) {
  fs.writeFileSync(OUTPUT, output);
  process.stdout.write(`wrote ${rows.length} recovered Linux records to ${path.relative(ROOT, OUTPUT)} (${digest})\n`);
} else {
  fail("usage: recover-linode-linux-history.js [--check]");
}
