#!/usr/bin/env node
"use strict";

// Every commit hash a committed benchmark artifact cites must stay reachable
// from some ref. A gate's evidence directory, its frozen config, and its
// retained run state all name commits by hash; once the branch that held one is
// deleted, the object survives only until `git gc` runs. The evidence still
// LOOKS complete at that point -- the directory is there, the hashes are there,
// the analysis still validates -- but the comparison can no longer be
// reproduced. That is a believable failure rather than a loud one, which is
// exactly the kind this repository prefers to make mechanical.
//
// The check is deliberately ROLE-BLIND. An earlier version of this rule was
// "tag any control commit a gate's evidence cites", which was shaped by the
// instance that prompted it and would have missed the candidate and
// orchestration commits that were actually orphaned. Scanning the artifacts for
// anything that resolves to a commit does not require anyone to have
// enumerated the roles correctly in advance.

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const REPO = path.resolve(__dirname, "..");
const SCAN_ROOTS = ["bench/cloud", "bench/history"];
const HASH = /\b[0-9a-f]{40}\b/g;

function git(args) {
  return execFileSync("git", args, {
    cwd: REPO,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function trackedFiles() {
  return git(["ls-files", "-z", ...SCAN_ROOTS]).split("\0").filter(Boolean);
}

function citedHashes() {
  const cited = new Map();
  for (const file of trackedFiles()) {
    let text;
    try {
      text = fs.readFileSync(path.join(REPO, file), "utf8");
    } catch {
      continue;
    }
    for (const match of text.matchAll(HASH)) {
      const hash = match[0];
      if (!cited.has(hash)) cited.set(hash, new Set());
      cited.get(hash).add(file);
    }
  }
  return cited;
}

function reachableCommits() {
  return new Set(git(["rev-list", "--all"]).split("\n").filter(Boolean));
}

function isCommit(hash) {
  try {
    return git(["cat-file", "-t", hash]).trim() === "commit";
  } catch {
    return false;
  }
}

function subjectOf(hash) {
  try {
    return git(["log", "-1", "--format=%s", hash]).trim();
  } catch {
    return "(unknown)";
  }
}

// Proves the two halves of the check on a commit built for the purpose: an
// orphan commit must be recognised as a commit AND as unreachable. Without
// this, a checker that silently classified everything as reachable would pass
// on a clean repository and report nothing on a dirty one.
function selfTest() {
  const tree = git(["rev-parse", "HEAD^{tree}"]).trim();
  const orphan = git(["commit-tree", tree, "-m", "reachability self-test"]).trim();
  const reachable = reachableCommits();
  const head = git(["rev-parse", "HEAD"]).trim();
  const failures = [];
  if (!isCommit(orphan)) failures.push("orphan commit was not recognised as a commit");
  if (reachable.has(orphan)) failures.push("orphan commit was reported reachable");
  if (!reachable.has(head)) failures.push("HEAD was reported unreachable");
  if (!isCommit(head)) failures.push("HEAD was not recognised as a commit");
  if (failures.length > 0) {
    process.stderr.write(`reachability self-test failed:\n  ${failures.join("\n  ")}\n`);
    process.exitCode = 1;
    return;
  }
  process.stdout.write("benchmark commit reachability self-test ok\n");
}

function main() {
  if (process.argv.includes("--self-test")) {
    selfTest();
    return;
  }
  const cited = citedHashes();
  const reachable = reachableCommits();
  const orphaned = [];
  let commits = 0;

  for (const [hash, files] of cited) {
    if (reachable.has(hash)) {
      commits += 1;
      continue;
    }
    if (!isCommit(hash)) continue; // blob/tree hashes and checksums are not our concern
    commits += 1;
    orphaned.push({ hash, files: [...files].sort() });
  }

  if (orphaned.length === 0) {
    process.stdout.write(
      `benchmark commit reachability ok: ${commits} cited commit(s) across ${cited.size} hash(es) are reachable\n`,
    );
    return;
  }

  process.stderr.write(
    `${orphaned.length} cited benchmark commit(s) are unreachable and will be lost at the next \`git gc\`.\n` +
      `Their evidence would still look complete while ceasing to be reproducible.\n\n`,
  );
  for (const { hash, files } of orphaned) {
    const short = hash.slice(0, 7);
    process.stderr.write(`  ${short}  ${subjectOf(hash)}\n`);
    process.stderr.write(`    cited by ${files.length} committed artifact(s), e.g. ${files[0]}\n`);
    process.stderr.write(
      `    fix: git tag -a bench-evidence/${short} -m "benchmark evidence commit ${short}" ${hash} && git push origin bench-evidence/${short}\n\n`,
    );
  }
  process.exitCode = 1;
}

main();
