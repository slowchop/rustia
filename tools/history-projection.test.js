"use strict";
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const { loadProjection, resolveHistoricalCommit, verifyProjection } = require("./history-projection.js");
const root = path.resolve(__dirname, "..");
const enabled = fs.existsSync(path.join(root, "design/public-source-profile.json"));
function fixture(change) {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-projection-test-"));
  fs.mkdirSync(path.join(temp, "design"));
  const marker = JSON.parse(fs.readFileSync(path.join(root, "design/public-source-profile.json")));
  const projection = JSON.parse(fs.readFileSync(path.join(root, "design/history-projection.json")));
  change(projection, marker);
  fs.writeFileSync(path.join(temp, "design/public-source-profile.json"), JSON.stringify(marker));
  fs.writeFileSync(path.join(temp, "design/history-projection.json"), JSON.stringify(projection));
  return temp;
}
function rejects(change, pattern) {
  const temp = fixture(change);
  try { assert.throws(() => loadProjection(temp), pattern); }
  finally { fs.rmSync(temp, { recursive: true, force: true }); }
}
test("private checkout needs no projection", () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-no-projection-"));
  try { assert.equal(loadProjection(temp), null); assert.equal(resolveHistoricalCommit("a".repeat(40), temp), "a".repeat(40)); }
  finally { fs.rmSync(temp, { recursive: true, force: true }); }
});
test("public projection is complete, reachable and preserves protected sources", { skip: !enabled }, () => {
  const value = verifyProjection(root);
  assert.equal(resolveHistoricalCommit(value.original_head, root), value.projected_head);
});
test("projection rejects absent, duplicate, malformed and mismatched records", { skip: !enabled }, () => {
  rejects(value => value.entries.pop(), /incomplete/);
  rejects(value => value.entries[1].projected = value.entries[0].projected, /duplicate/);
  rejects(value => value.entries[1].protected_sha256 = "bad", /invalid/);
  rejects(value => value.protected_paths.pop(), /protected paths/);
  rejects(value => value.metadata_projection[1] = value.metadata_projection[0], /duplicate metadata/);
  rejects((value, marker) => marker.historical_head = "f".repeat(40), /disagree/);
});

test("verification rejects unreachable mappings and altered protected fingerprints", { skip: !enabled }, () => {
  for (const [change, pattern] of [
    [value => value.entries[0].projected = "f".repeat(40), /unreachable/],
    [value => value.entries[0].protected_sha256 = "f".repeat(64), /protected source mismatch/],
  ]) {
    const temp = fixture(change);
    const gitDir = execFileSync("git", ["rev-parse", "--absolute-git-dir"], { cwd: root, encoding: "utf8" }).trim();
    fs.writeFileSync(path.join(temp, ".git"), `gitdir: ${gitDir}\n`);
    try { assert.throws(() => verifyProjection(temp), pattern); }
    finally { fs.rmSync(temp, { recursive: true, force: true }); }
  }
});
