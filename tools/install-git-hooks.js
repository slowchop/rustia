#!/usr/bin/env node
"use strict";
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");
const root = path.resolve(__dirname, "..");
const current = spawnSync("git", ["config", "--get", "core.hooksPath"], { cwd: root, encoding: "utf8" });
const defaultHook = execFileSync("git", ["rev-parse", "--git-path", "hooks/pre-push"], { cwd: root, encoding: "utf8" }).trim();
if ((current.status === 0 && current.stdout.trim() !== ".githooks") ||
    (current.status === 1 && fs.existsSync(path.resolve(root, defaultHook)))) {
  console.error("Existing Git hooks found; refusing to replace them. Integrate tools/pre-push.js into the existing hook.");
  process.exit(1);
}
if (![0, 1].includes(current.status)) process.exit(1);
// Ensure --worktree cannot silently fall back to shared repository config.
execFileSync("git", ["config", "--local", "extensions.worktreeConfig", "true"], { cwd: root });
// Worktree-local: older checkouts without the hook remain usable.
execFileSync("git", ["config", "--worktree", "core.hooksPath", ".githooks"], { cwd: root });
console.log("Installed the full validation pre-push hook for this checkout.");
