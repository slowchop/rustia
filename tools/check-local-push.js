#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");
const { performance } = require("node:perf_hooks");

const root = path.resolve(__dirname, "..");
const deadline = performance.now() + 50_000;
const checks = [
  ["git", ["show", "--format=", "--check", "HEAD"]],
  ["cargo", ["fmt", "--all", "--", "--check"]],
  ["cargo", ["fmt", "--manifest-path", "demos/web-live/Cargo.toml", "--", "--check"]],
  [process.execPath, ["tools/check-crate-metadata.js", "--check"]],
];

for (const [command, args] of checks) {
  const remaining = Math.floor(deadline - performance.now());
  if (remaining <= 0) {
    console.error("Local push checks exceeded 50 seconds.");
    process.exit(1);
  }

  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    timeout: remaining,
    killSignal: "SIGKILL",
  });
  if (result.status !== 0) {
    console.error(`Local push check failed: ${command} ${args.join(" ")}`);
    if (result.error) console.error(result.error.message);
    process.exit(result.status ?? 1);
  }
}
