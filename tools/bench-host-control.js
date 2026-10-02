#!/usr/bin/env node
"use strict";

// Host-change control for the live per-commit series.
//
// ci-bench installs operating-system updates by itself, and a new kernel, libc
// or microcode can move every number after it. The series alone cannot show
// that: its next point is a new commit, so a host step and a code step look the
// same. So when the host's software fingerprint (run-dedicated-benchmark.js
// host-fingerprint) differs from the last one recorded, the per-commit lane
// first measures the last recorded commit again and compares it with its own
// recorded row. Same code, same protocol, different host: whatever moved is the
// host.
//
// A move beyond the calibrated same-host spread (bench/pr-screening.json) is
// flagged for a person, who decides whether to re-measure the history on the
// updated host. Nothing here rewrites a row. Every control is appended to the
// controls file whether or not it moved, which is how the spread between
// controls gets measured rather than assumed.

const fs = require("node:fs");
const path = require("node:path");

const REPO = path.resolve(__dirname, "..");
const SCHEMA = "rustia.bench.host-control.v1";
const FINGERPRINT = /^[0-9a-f]{64}$/;

function fail(message) {
  console.error(`bench-host-control: ${message}`);
  process.exit(1);
}

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter((line) => line.trim() !== "")
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        fail(`${file} line ${index + 1} is not JSON: ${error.message}`);
      }
    });
}

function readFingerprint(file) {
  const value = JSON.parse(fs.readFileSync(file, "utf8"));
  if (!FINGERPRINT.test(value.host_fingerprint ?? "")) fail(`${file} carries no host_fingerprint`);
  for (const field of ["kernel", "microcode", "packages_sha256"]) {
    if (typeof value[field] !== "string" || value[field] === "") fail(`${file} carries no ${field}`);
  }
  return value;
}

/// The fingerprint the host last measured under: the newer of the last series
/// row and the last control. A series row from before rows carried one says
/// nothing about the host, so it reads as unknown, and unknown counts as
/// changed -- the first run after this lane appeared controls rather than
/// assuming nothing happened since.
function lastKnownFingerprint(rows, controls) {
  const lastRow = rows.at(-1);
  const lastControl = controls.at(-1);
  const newest =
    lastControl && (!lastRow || lastControl.timestamp_unix >= lastRow.timestamp_unix) ? lastControl : lastRow;
  return newest?.host_fingerprint ?? null;
}

function status(rows, controls, fingerprint) {
  if (rows.length === 0) return { changed: false, reason: "no series rows yet; nothing to control against" };
  const previous = lastKnownFingerprint(rows, controls);
  if (previous === fingerprint.host_fingerprint) {
    return { changed: false, reason: "host software unchanged since the last measurement" };
  }
  return {
    changed: true,
    reason: previous === null ? "no recorded host fingerprint" : "host software changed",
    previous_fingerprint: previous,
    control_commit: rows.at(-1).commit,
  };
}

/// Per workload, the control's median against the recorded one. `moved` is
/// any workload outside its calibrated threshold, in either direction: a host
/// that got faster shifts the series just as much as one that got slower.
function compare(recordedRows, controlRows, thresholds) {
  const workloads = {};
  let moved = false;
  for (const recorded of recordedRows) {
    const control = controlRows.find((row) => row.workload === recorded.workload);
    if (!control) fail(`the control run has no ${recorded.workload} row`);
    if (control.checksum !== recorded.checksum) {
      fail(`${recorded.workload} checksum changed between the recorded row and its control`);
    }
    const threshold = thresholds[recorded.workload]?.material_threshold_percent;
    if (!Number.isFinite(threshold) || threshold <= 0) {
      fail(`bench/pr-screening.json has no threshold for ${recorded.workload}`);
    }
    // Rounded before the comparison, so a change that prints as the threshold
    // is not flagged by the last bit of a float.
    const changePercent = Number(((control.ns_per_unit_median / recorded.ns_per_unit_median - 1) * 100).toFixed(3));
    const outside = Math.abs(changePercent) > threshold;
    moved ||= outside;
    workloads[recorded.workload] = {
      recorded_ns_per_unit_median: recorded.ns_per_unit_median,
      control_ns_per_unit_median: control.ns_per_unit_median,
      change_percent: changePercent,
      threshold_percent: threshold,
      outside_threshold: outside,
    };
  }
  return { workloads, moved };
}

function summaryMarkdown(entry, kernelBefore) {
  const lines = [
    `## Host changed: control re-measure of \`${entry.commit.slice(0, 8)}\``,
    "",
    entry.moved
      ? "**Moved beyond the calibrated same-host spread.** The series has a host step here; " +
        "decide whether to re-measure the history on the updated host."
      : "Within the calibrated same-host spread on every workload.",
    "",
    `Kernel ${kernelBefore ?? "unrecorded"} → ${entry.kernel}, microcode ${entry.microcode}.` +
      (entry.control_binary_sha256 === entry.recorded_binary_sha256
        ? " Same binary as the recorded row."
        : " **The binary differs from the recorded row** (toolchain or linker changed on the host)."),
    "",
    "| Workload | Recorded ns/unit | Control ns/unit | Change | Threshold |",
    "|---|---:|---:|---:|---:|",
  ];
  for (const [workload, result] of Object.entries(entry.workloads)) {
    const change = `${result.change_percent >= 0 ? "+" : ""}${result.change_percent.toFixed(2)}%`;
    lines.push(
      `| ${workload} | ${result.recorded_ns_per_unit_median.toFixed(2)} | ` +
        `${result.control_ns_per_unit_median.toFixed(2)} | ${result.outside_threshold ? `**${change}**` : change} | ` +
        `±${result.threshold_percent}% |`,
    );
  }
  return `${lines.join("\n")}\n`;
}

function record(options) {
  const rows = readJsonl(options.rows);
  const controls = readJsonl(options.controls);
  const fingerprint = readFingerprint(options.fingerprint);
  const decision = status(rows, controls, fingerprint);
  if (!decision.changed) fail(`nothing to record: ${decision.reason}`);
  const commit = decision.control_commit;
  const recordedRows = rows.filter((row) => row.commit === commit);
  const controlRows = readJsonl(options.controlRows);
  if (controlRows.length === 0) fail("the control run recorded no rows");
  if (controlRows.some((row) => row.commit !== commit)) fail(`the control rows are not for ${commit}`);
  if (controlRows.some((row) => row.host_fingerprint !== fingerprint.host_fingerprint)) {
    fail("the control rows were measured under a different fingerprint than this run reported");
  }
  const screening = JSON.parse(fs.readFileSync(options.thresholds, "utf8"));
  if (screening.schema !== "rustia.bench.pr-screening.v1" || screening.host !== "ci-bench") {
    fail(`${options.thresholds} is not ci-bench's calibrated screening file`);
  }
  const { workloads, moved } = compare(recordedRows, controlRows, screening.workloads);
  const entry = {
    schema: SCHEMA,
    timestamp_unix: Math.max(...controlRows.map((row) => row.timestamp_unix)),
    host: "ci-bench",
    commit,
    recorded_timestamp_unix: recordedRows[0].timestamp_unix,
    previous_fingerprint: decision.previous_fingerprint,
    host_fingerprint: fingerprint.host_fingerprint,
    kernel: fingerprint.kernel,
    microcode: fingerprint.microcode,
    packages_sha256: fingerprint.packages_sha256,
    recorded_binary_sha256: recordedRows[0].binary_sha256 ?? null,
    control_binary_sha256: controlRows[0].binary_sha256 ?? null,
    thresholds_calibrated_at: screening.calibrated_at,
    workloads,
    moved,
  };
  fs.mkdirSync(path.dirname(options.controls), { recursive: true });
  fs.appendFileSync(options.controls, `${JSON.stringify(entry)}\n`);
  if (options.summary) fs.appendFileSync(options.summary, summaryMarkdown(entry, recordedRows[0].kernel));
  process.stdout.write(`${moved ? "moved" : "steady"}\n`);
}

function selfTest() {
  const assert = require("node:assert/strict");
  const fp = (char) => ({
    kernel: "6.8.0-146-generic",
    microcode: "0x8701034",
    packages_sha256: "c".repeat(64),
    host_fingerprint: char.repeat(64),
  });
  const row = (over = {}) => ({
    commit: "a".repeat(40), workload: "w1", checksum: "number:1", ns_per_unit_median: 100,
    timestamp_unix: 100, binary_sha256: "e".repeat(64), ...over,
  });

  assert.equal(status([], [], fp("1")).changed, false);
  // Rows from before the fingerprint existed: unknown, so control.
  const legacy = status([row()], [], fp("1"));
  assert.equal(legacy.changed, true);
  assert.equal(legacy.previous_fingerprint, null);
  assert.equal(legacy.control_commit, "a".repeat(40));
  assert.equal(status([row({ host_fingerprint: "1".repeat(64) })], [], fp("1")).changed, false);
  assert.equal(status([row({ host_fingerprint: "1".repeat(64) })], [], fp("2")).changed, true);
  // A control newer than the last row is the last word on the host...
  const controlled = [{ timestamp_unix: 200, host_fingerprint: "2".repeat(64) }];
  assert.equal(status([row({ host_fingerprint: "1".repeat(64) })], controlled, fp("2")).changed, false);
  // ...and a row newer than the control is.
  assert.equal(
    status([row({ timestamp_unix: 300, host_fingerprint: "3".repeat(64) })], controlled, fp("2")).changed,
    true,
  );
  // The control re-measures the last commit, not the first.
  const two = [row(), row({ commit: "b".repeat(40), timestamp_unix: 150 })];
  assert.equal(status(two, [], fp("1")).control_commit, "b".repeat(40));

  const thresholds = { w1: { material_threshold_percent: 2 }, w2: { material_threshold_percent: 5 } };
  const recorded = [row(), row({ workload: "w2", checksum: "number:2", ns_per_unit_median: 200 })];
  const steady = compare(recorded, [
    row({ ns_per_unit_median: 101.5 }),
    row({ workload: "w2", checksum: "number:2", ns_per_unit_median: 192 }),
  ], thresholds);
  assert.equal(steady.moved, false);
  assert.equal(steady.workloads.w1.change_percent, 1.5);
  assert.equal(steady.workloads.w2.change_percent, -4);
  // Exactly at the threshold is inside it, float noise or not.
  assert.equal(compare([row()], [row({ ns_per_unit_median: 100 * 1.02 })], thresholds).moved, false);
  // Faster counts as moved too: the series shifts either way.
  const faster = compare(recorded, [
    row({ ns_per_unit_median: 97 }),
    row({ workload: "w2", checksum: "number:2", ns_per_unit_median: 200 }),
  ], thresholds);
  assert.equal(faster.moved, true);
  assert.equal(faster.workloads.w1.outside_threshold, true);
  assert.equal(faster.workloads.w2.outside_threshold, false);

  const summary = summaryMarkdown(
    {
      commit: "a".repeat(40), moved: true, kernel: "6.8.0-147-generic", microcode: "0x1",
      recorded_binary_sha256: "e".repeat(64), control_binary_sha256: "f".repeat(64), workloads: faster.workloads,
    },
    undefined,
  );
  assert.match(summary, /Moved beyond/);
  assert.match(summary, /binary differs/);
  assert.match(summary, /\*\*-3\.00%\*\*/);

  process.stdout.write("bench-host-control self-test ok\n");
}

function usage() {
  process.stdout.write(
    "Usage: node tools/bench-host-control.js status --rows FILE --controls FILE --fingerprint FILE\n" +
      "       node tools/bench-host-control.js record --rows FILE --controls FILE --fingerprint FILE\n" +
      "                                        --control-rows FILE [--summary FILE]\n" +
      "       node tools/bench-host-control.js --self-test\n" +
      "\n" +
      "status prints whether the host changed since the last measurement, as JSON, and\n" +
      "which commit to re-measure if so. record compares that re-measure with the\n" +
      "recorded rows, appends the result to the controls file and prints moved or steady.\n",
  );
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  if (!["status", "record"].includes(command)) return null;
  const options = { command, thresholds: path.join(REPO, "bench/pr-screening.json") };
  const flags = { "--rows": "rows", "--controls": "controls", "--fingerprint": "fingerprint" };
  if (command === "record") Object.assign(flags, { "--control-rows": "controlRows", "--summary": "summary" });
  for (let index = 0; index < rest.length; index += 1) {
    const key = flags[rest[index]];
    const value = rest[index + 1];
    if (!key || !value) return null;
    options[key] = value;
    index += 1;
  }
  for (const required of ["rows", "controls", "fingerprint"]) if (!options[required]) return null;
  if (command === "record" && !options.controlRows) return null;
  return options;
}

if (process.argv[2] === "--self-test") {
  selfTest();
} else {
  const options = parseArgs(process.argv.slice(2));
  if (options === null) {
    usage();
    process.exitCode = 2;
  } else if (options.command === "status") {
    process.stdout.write(
      `${JSON.stringify(status(readJsonl(options.rows), readJsonl(options.controls), readFingerprint(options.fingerprint)))}\n`,
    );
  } else {
    record(options);
  }
}
