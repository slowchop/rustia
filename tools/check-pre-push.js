#!/usr/bin/env node
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-push-guard-"));
const environment = { ...process.env };
for (const name of execFileSync("git", ["rev-parse", "--local-env-vars"], { encoding: "utf8" }).trim().split("\n")) delete environment[name];
const git = (...args) => execFileSync("git", args, { cwd: temporary, env: environment, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
try {
  git("init"); git("config", "commit.gpgsign", "false"); git("config", "user.name", "Guard test"); git("config", "user.email", "guard@example.invalid");
  fs.mkdirSync(path.join(temporary, "tools"));
  fs.copyFileSync(path.join(__dirname, "pre-push.js"), path.join(temporary, "tools/pre-push.js"));
  fs.writeFileSync(path.join(temporary, "tools/run-release-preflight.js"), `
    const fs = require('node:fs');
    if (process.env.GUARD_TEST_MODE === 'fail') process.exit(1);
    if (process.env.GUARD_TEST_MODE === 'inherited-git' &&
        ['GIT_DIR', 'GIT_WORK_TREE', 'GIT_INDEX_FILE'].some(name => process.env[name] !== undefined)) process.exit(2);
    if (process.env.GUARD_TEST_MODE === 'preflight-root' &&
        process.env.RUSTIA_PREFLIGHT_ROOT !== undefined) process.exit(3);
    if (process.env.GUARD_TEST_MODE === 'dirty') fs.writeFileSync('changed', 'x');
  `);
  git("add", "."); git("commit", "-m", "fixture");
  const first = git("rev-parse", "HEAD");
  fs.writeFileSync(path.join(temporary, "content"), "second");
  git("add", "."); git("commit", "-m", "second");
  const head = git("rev-parse", "HEAD");
  const run = (sha = head, mode = "", extraEnv = {}) => spawnSync(process.execPath, ["tools/pre-push.js"], {
    cwd: temporary, encoding: "utf8", env: { ...environment, ...extraEnv, GUARD_TEST_MODE: mode },
    input: `refs/heads/test ${sha} refs/heads/test ${"0".repeat(40)}\n`,
  });
  assert.equal(run().status, 0);
  // Linked-worktree pushes export GIT_DIR. Full validation must clear this
  // context before any helper creates a temporary consumer/test repository.
  const protectedFiles = ["HEAD", "config", "index"].map(name => path.join(temporary, ".git", name));
  const before = protectedFiles.map(file => fs.readFileSync(file));
  assert.equal(run(head, "inherited-git", {
    GIT_DIR: path.join(temporary, ".git"),
    GIT_WORK_TREE: temporary,
    GIT_INDEX_FILE: path.join(temporary, ".git", "index"),
  }).status, 0, "full validation inherited the pushing repository's Git context");
  protectedFiles.forEach((file, index) => assert.deepEqual(fs.readFileSync(file), before[index]));
  assert.equal(run(head, "preflight-root", {
    RUSTIA_PREFLIGHT_ROOT: path.join(temporary, "other-checkout"),
  }).status, 0, "full validation accepted an inherited preflight root override");
  assert.notEqual(run(first).status, 0, "other source commit accepted");
  assert.notEqual(run(head, "fail").status, 0, "failed validation accepted");
  assert.notEqual(run(head, "dirty").status, 0, "validation mutation accepted");
  assert.notEqual(run().status, 0, "dirty tree accepted");
  assert.equal(run("0".repeat(40)).status, 0, "deletion unexpectedly ran validation");
  console.log("Pre-push rejects wrong commits, test failures, dirty trees, and mid-check edits.");
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
