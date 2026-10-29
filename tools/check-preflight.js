#!/usr/bin/env node
"use strict";
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");
const root = path.resolve(__dirname, "..");
const runner = path.join(root, "tools/run-release-preflight.js");
const plan = JSON.parse(execFileSync(process.execPath, [runner, "--list"], { encoding: "utf8" }));
const stages = new Set(["quick", "workspace", "release"]);
assert(plan.every(entry => stages.has(entry.stage)));
const matches = (command, args) => plan.filter(entry => entry.command === command && JSON.stringify(entry.args) === JSON.stringify(args));
assert.equal(matches("cargo", ["test", "--locked", "--workspace"]).length, 1);
assert.equal(matches("cargo", ["test", "--locked", "-p", "rustia-compiler", "--lib"])[0].stage, "quick");
for (const example of ["embedding", "chunk_builder", "mutable_objects", "live_values", "game_objects", "hardened_decision"]) {
  assert.equal(matches("cargo", ["run", "--locked", "-p", "rustia", "--example", example]).length, 1);
}
assert.equal(matches("cargo", ["fmt", "--manifest-path", "demos/web-live/Cargo.toml", "--", "--check"]).length, 1);
assert.equal(matches("cargo", ["clippy", "--locked", "--manifest-path", "demos/web-live/Cargo.toml", "--target", "wasm32-unknown-unknown", "--all-targets", "--", "-D", "warnings"]).length, 1);
for (const script of ["check-package-dry-runs", "verify-conformance-package", "check-external-facade", "check-facade-api"]) {
  assert.equal(matches("node", [`tools/${script}.js`])[0].stage, "release");
}
for (const args of [["--stage", "typo"], ["--stage"], ["--skip-tests"]]) {
  assert.equal(spawnSync(process.execPath, [runner, ...args], { encoding: "utf8" }).status, 2);
}
const workflow = fs.readFileSync(path.join(root, ".github/workflows/ci.yml"), "utf8");
assert(workflow.includes("stage: [quick, workspace, release]"));
assert(workflow.includes('run: test "$PREFLIGHT_RESULT" = success'));
assert(workflow.includes('needs: [release-preflight, webkit-runtime]'));
assert(workflow.includes('&& test "$WEBKIT_RESULT" = success'));
console.log("Preflight inventory, stage boundaries, and invalid-option rejection pass.");
