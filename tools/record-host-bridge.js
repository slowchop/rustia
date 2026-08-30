#!/usr/bin/env node

"use strict";

// Records what changed when the cross-engine matrix moved machines.
//
// The dashboard publishes ratios against the mlua/Luau/JIT reference measured
// in the same run, so a change of host cancels there. The absolute nanoseconds
// do not cancel, and once the rented instance's rows are deleted there is no way
// to recover how the two machines compared. This writes that comparison down
// first, per engine and per workload, so a later reader can tell a host change
// from an engine change in a series that spans both.
//
// It only ever reads history. It is not a measurement and it never appends one.

const fs = require("node:fs");
const path = require("node:path");

const REPO = path.resolve(__dirname, "..");
const HISTORY = path.join(REPO, "bench/history/runs.jsonl");
const SCHEMA = "rustia.bench.host-bridge.v1";
const FULL_COMMIT = /^[0-9a-f]{40}$/;

function fail(message) {
  throw new Error(message);
}

function loadRows() {
  const text = fs.readFileSync(HISTORY, "utf8");
  if (!text.endsWith("\n")) fail("bench/history/runs.jsonl must be newline terminated");
  return text
    .trim()
    .split("\n")
    .map((line, index) => {
      try {
        return JSON.parse(line);
      } catch (error) {
        fail(`bench/history/runs.jsonl row ${index + 1} is not JSON: ${error.message}`);
      }
    });
}

function cohort(rows, commit) {
  const selected = rows.filter((row) => row.commit === commit);
  if (selected.length === 0) fail(`no rows for commit ${commit}`);
  return selected;
}

function slotKey(row) {
  return `${row.engine}/${row.dialect}/${row.workload}`;
}

// The comparison itself: same engine, same workload, one machine over the other.
// A slot present on only one side is reported rather than dropped, because a
// cohort that gained or lost an engine is exactly the kind of difference this
// file exists to make visible.
function compare(fromRows, toRows) {
  const before = new Map(fromRows.map((row) => [slotKey(row), row]));
  const after = new Map(toRows.map((row) => [slotKey(row), row]));
  const slots = [];
  const missing = { only_before: [], only_after: [] };
  for (const key of before.keys()) {
    if (!after.has(key)) missing.only_before.push(key);
  }
  for (const [key, row] of after) {
    if (!before.has(key)) {
      missing.only_after.push(key);
      continue;
    }
    const previous = before.get(key);
    // The matrix protocol chooses N to hit a target duration, so N and the
    // checksum over N units both differ legitimately between two runs -- and
    // will differ by more between two machines than between two runs on one.
    // Requiring them equal here would refuse every real comparison. What must
    // match is the unit: `ns_per_unit_min` is only comparable if one unit means
    // the same work on both sides, and that string is where the protocol says so.
    if (previous.unit !== row.unit) {
      fail(
        `${key} changed its unit between the two cohorts ` +
          `(${JSON.stringify(previous.unit)} -> ${JSON.stringify(row.unit)}); a per-unit ratio ` +
          "between different units would not mean anything",
      );
    }
    slots.push({
      engine: row.engine,
      dialect: row.dialect,
      workload: row.workload,
      unit: row.unit,
      before_n: previous.n,
      after_n: row.n,
      before_ns_per_unit_min: previous.ns_per_unit_min,
      after_ns_per_unit_min: row.ns_per_unit_min,
      // Above 1 means the new host is slower on this slot.
      ratio: row.ns_per_unit_min / previous.ns_per_unit_min,
    });
  }
  slots.sort((left, right) =>
    `${left.engine}${left.workload}`.localeCompare(`${right.engine}${right.workload}`),
  );
  return { slots, missing };
}

function median(values) {
  if (values.length === 0) return null;
  const ordered = [...values].sort((left, right) => left - right);
  const middle = Math.floor(ordered.length / 2);
  return ordered.length % 2 === 0 ? (ordered[middle - 1] + ordered[middle]) / 2 : ordered[middle];
}

function parseArgs(argv) {
  const parsed = { from: undefined, to: undefined, out: undefined, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--self-test") parsed.selfTest = true;
    else if (option === "--from") parsed.from = argv[++index];
    else if (option === "--to") parsed.to = argv[++index];
    else if (option === "--out") parsed.out = argv[++index];
    else return null;
  }
  if (parsed.selfTest) return parsed.from || parsed.to || parsed.out ? null : parsed;
  if (!parsed.from || !parsed.to) return null;
  return parsed;
}

function selfTest() {
  const assert = require("node:assert/strict");
  const row = (over) => ({
    commit: "a".repeat(40),
    engine: "rustia",
    dialect: "luau",
    workload: "dense_tables",
    target: "linux-x86_64",
    unit: "build+sum+sort of 256-elem array",
    n: 10,
    ns_per_unit_min: 100,
    ...over,
  });
  const before = [row({}), row({ engine: "koto", dialect: "koto", ns_per_unit_min: 200 })];
  const after = [
    row({ commit: "b".repeat(40), ns_per_unit_min: 80 }),
    row({ commit: "b".repeat(40), engine: "koto", dialect: "koto", ns_per_unit_min: 220 }),
  ];
  const result = compare(before, after);
  assert.equal(result.slots.length, 2);
  assert.equal(result.slots.find((slot) => slot.engine === "rustia").ratio, 0.8);
  assert.equal(result.slots.find((slot) => slot.engine === "koto").ratio, 1.1);
  assert.deepEqual(result.missing, { only_before: [], only_after: [] });

  // A slot on one side only is reported, not silently dropped.
  const lopsided = compare(before, [after[0]]);
  assert.deepEqual(lopsided.missing.only_before, ["koto/koto/dense_tables"]);
  assert.equal(lopsided.slots.length, 1);

  // A changed unit means the two sides are not measuring the same work, so no
  // per-unit ratio between them would mean anything.
  assert.throws(
    () => compare(before, [row({ commit: "b".repeat(40), unit: "something else" })]),
    /changed its unit between the two cohorts/,
  );
  // A different N is expected and must not be refused: this protocol chooses N
  // to hit a target duration, so two machines land on different values and
  // ns_per_unit is exactly the figure that survives that.
  const rescaled = compare(before, [row({ commit: "b".repeat(40), n: 9_999, ns_per_unit_min: 50 })]);
  assert.equal(rescaled.slots.length, 1);
  assert.equal(rescaled.slots[0].ratio, 0.5);
  assert.equal(rescaled.slots[0].before_n, 10);
  assert.equal(rescaled.slots[0].after_n, 9_999);

  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  assert.equal(median([]), null);
  assert.equal(parseArgs(["--from", "a", "--to", "b"]).to, "b");
  assert.equal(parseArgs(["--from", "a"]), null);
  assert.equal(parseArgs(["--self-test", "--from", "a"]), null);
  process.stdout.write("record-host-bridge self-test ok\n");
}

function main(argv) {
  const parsed = parseArgs(argv);
  if (parsed === null) {
    process.stdout.write(
      "Usage: node tools/record-host-bridge.js --from COMMIT --to COMMIT [--out FILE]\n" +
        "       node tools/record-host-bridge.js --self-test\n",
    );
    process.exitCode = 2;
    return;
  }
  if (parsed.selfTest) {
    selfTest();
    return;
  }
  for (const [name, value] of [
    ["--from", parsed.from],
    ["--to", parsed.to],
  ]) {
    if (!FULL_COMMIT.test(value)) fail(`${name} must be a full lowercase commit`);
  }
  const rows = loadRows();
  const fromRows = cohort(rows, parsed.from);
  const toRows = cohort(rows, parsed.to);
  const { slots, missing } = compare(fromRows, toRows);
  const ratios = slots.map((slot) => slot.ratio);
  const bridge = {
    schema: SCHEMA,
    recorded_at: new Date().toISOString(),
    why:
      "The cross-engine matrix moved from a rented instance to the owned host. Published figures " +
      "are ratios against the reference measured in the same run, so a host change cancels there; " +
      "absolute nanoseconds do not. This is that comparison, written down before the rented rows " +
      "are deleted and it becomes unrecoverable.",
    from_commit: parsed.from,
    to_commit: parsed.to,
    slot_count: slots.length,
    ratio_summary: {
      note: "after / before on ns_per_unit_min; above 1 means the new host is slower on that slot",
      median: median(ratios),
      minimum: ratios.length ? Math.min(...ratios) : null,
      maximum: ratios.length ? Math.max(...ratios) : null,
    },
    unmatched_slots: missing,
    slots,
  };
  const destination = parsed.out
    ? path.resolve(REPO, parsed.out)
    : path.join(REPO, "bench/evidence/matrix", parsed.to, "host-bridge.json");
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  fs.writeFileSync(destination, `${JSON.stringify(bridge, null, 1)}\n`);
  process.stdout.write(
    `wrote ${path.relative(REPO, destination)}: ${slots.length} slots, ` +
      `median ratio ${bridge.ratio_summary.median?.toFixed(4)}\n`,
  );
}

try {
  main(process.argv.slice(2));
} catch (error) {
  process.stderr.write(`record-host-bridge: ${error.message}\n`);
  process.exitCode = 1;
}
