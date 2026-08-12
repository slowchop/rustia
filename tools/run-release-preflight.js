#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const path = require("path");

const root = path.resolve(__dirname, "..");

function run(command, args) {
  process.stdout.write(`+ ${command} ${args.join(" ")}\n`);
  try {
    childProcess.execFileSync(command, args, {
      cwd: root,
      env: process.env,
      stdio: "inherit",
    });
  } catch (error) {
    const status = Number.isInteger(error.status) ? ` (exit ${error.status})` : "";
    console.error(`release-preflight: ${command} ${args.join(" ")} failed${status}`);
    process.exit(1);
  }
}

function node(script, ...args) {
  run("node", [script, ...args]);
}

run("git", ["diff", "--quiet", "--", "Cargo.lock"]);
run("git", ["diff", "--cached", "--quiet", "--", "Cargo.lock"]);
run("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"]);
run("cargo", ["fmt", "--all", "--", "--check"]);
run("cargo", ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"]);

node("tools/check-release-notes.js");
node("tools/check-crate-metadata.js", "--check");
node("tools/check-workflows.js");
node("tools/check-package-files.js", "--check");
node("tools/check-publishability.js", "--check");
node("tools/check-package-dry-runs.js");
node("tools/verify-conformance-package.js");
node("design/render-progress.js", "--check");
node("tools/check-project-status.js");

node("bench/analyze-fixed-samples.js", "--self-test");
node("tools/run-digitalocean-benchmark.js", "self-test");
node("tools/run-linode-benchmark.js", "self-test");
node("tools/run-linux-matrix-benchmark.js", "self-test");
node("bench/run-memory-supervisor.js", "self-test");
node("bench/analyze-memory-samples.js", "self-test");
node("bench/run-memory-cohort.js", "self-test");
node("bench/publish-memory-analysis.js", "self-test");
node("tools/run-linux-memory-benchmark.js", "self-test");
node("tools/run-vultr-benchmark.js", "self-test");
node("tools/run-scaleway-benchmark.js", "self-test");
node("tools/run-wasm-gate.js", "self-test");
node("tools/run-wasm-dispatch-gate.js", "self-test");
node("tools/merge-rustia-ci-memory-data.js", "self-test");
node("tools/check-benchmark-results.js");
node("tools/check-wasm-gate-results.js");
node("tools/render-benchmark-operations-status.js", "--check");
node("tools/check-current-report-summary.js");
node("bench/render-current-results.js", "--check");
node("bench/render-claims.js", "--check");

run("cargo", ["fmt", "--all", "--manifest-path", "fuzz/Cargo.toml", "--", "--check"]);
run("cargo", [
  "clippy",
  "--locked",
  "--manifest-path",
  "fuzz/Cargo.toml",
  "--all-targets",
  "--",
  "-D",
  "warnings",
]);
run("cargo", ["test", "--locked", "--manifest-path", "fuzz/Cargo.toml"]);
run("cargo", ["test", "--locked", "-p", "rustia", "--examples", "--no-run"]);
run("cargo", ["test", "--locked", "--workspace"]);
