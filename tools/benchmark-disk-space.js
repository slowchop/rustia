"use strict";

// Shared by the coordinator and each shipped runner. Capacity metadata contains
// no filesystem/device names or paths; the caller supplies the working root.
const fs = require("node:fs");
const path = require("node:path");
const MINIMUM_FREE_GIB = 20;
const MINIMUM_FREE_FRACTION = 0.20;

function diskSpaceProblems(totalKib, freeKib) {
  if (!Number.isSafeInteger(totalKib) || totalKib <= 0 ||
      !Number.isSafeInteger(freeKib) || freeKib < 0 || freeKib > totalKib) {
    return ["free space: filesystem capacity is missing or invalid"];
  }
  const problems = [];
  if (freeKib < MINIMUM_FREE_GIB * 1024 * 1024) {
    problems.push(`free space: at least ${MINIMUM_FREE_GIB} GiB needed under $HOME, host reports ${(freeKib / 1024 / 1024).toFixed(1)} GiB`);
  }
  if (freeKib / totalKib < MINIMUM_FREE_FRACTION) {
    problems.push(`free space: at least ${MINIMUM_FREE_FRACTION * 100}% of the filesystem must be available, host reports ${(100 * freeKib / totalKib).toFixed(2)}%`);
  }
  return problems;
}

// Called after builds, before quiet/timed windows. Save refusals before throwing
// so the runner's existing failed-export path retains the capacity evidence.
function checkDiskSpace(directory, evidenceFile, statfs = fs.statfsSync) {
  const record = { schema: "rustia.bench.disk-space.v1", captured_at: new Date().toISOString(),
    minimum_free_gib: MINIMUM_FREE_GIB, minimum_free_fraction: MINIMUM_FREE_FRACTION };
  try {
    const stats = statfs(directory);
    record.total_kib = stats.blocks * stats.bsize / 1024;
    record.free_kib = stats.bavail * stats.bsize / 1024;
    record.problems = diskSpaceProblems(record.total_kib, record.free_kib);
    if (record.problems.length === 0) record.free_fraction = record.free_kib / record.total_kib;
  } catch (error) {
    record.problems = [`free space: unable to read filesystem capacity (${error.code ?? "unknown"})`];
  }
  record.accepted = record.problems.length === 0;
  fs.mkdirSync(path.dirname(evidenceFile), { recursive: true });
  fs.writeFileSync(evidenceFile, `${JSON.stringify(record, null, 2)}\n`);
  if (!record.accepted) throw new Error(record.problems.join("; "));
  return record;
}

function selfTest() {
  const assert = require("node:assert/strict");
  const os = require("node:os");
  const gib = 1024 * 1024;
  assert.deepEqual(diskSpaceProblems(100 * gib, 20 * gib), []);
  assert.match(diskSpaceProblems(200 * gib, 30 * gib).join("; "), /at least 20%/);
  assert.match(diskSpaceProblems(50 * gib, 19 * gib).join("; "), /at least 20 GiB/);
  for (const [total, free] of [[undefined, 30 * gib], [0, 30 * gib], [100 * gib, undefined],
    [100 * gib, NaN], [Infinity, 30 * gib], [100 * gib, -1], [100 * gib, 101 * gib], [1.5, 1]]) {
    assert.match(diskSpaceProblems(total, free).join("; "), /missing or invalid/);
  }
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-disk-test-"));
  const file = path.join(dir, "evidence.json");
  try {
    const stats = (free) => ({ blocks: 200 * gib, bavail: free * gib, bsize: 1024 });
    assert.throws(() => checkDiskSpace("/fixture", file, () => stats(30)), /at least 20%/);
    assert.equal(JSON.parse(fs.readFileSync(file, "utf8")).accepted, false);
    const good = checkDiskSpace("/fixture", file, () => stats(40));
    assert.equal(good.accepted, true);
    assert.equal(good.free_fraction, 0.2);
    assert(!JSON.stringify(good).includes("/fixture"));
    assert.throws(() => checkDiskSpace("/fixture", file, () => { throw Object.assign(new Error("private path"), { code: "EIO" }); }), /EIO/);
    const failed = JSON.parse(fs.readFileSync(file, "utf8"));
    assert.equal(failed.accepted, false);
    assert(!JSON.stringify(failed).includes("private path"));
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

module.exports = { MINIMUM_FREE_GIB, MINIMUM_FREE_FRACTION, diskSpaceProblems, checkDiskSpace, selfTest };
