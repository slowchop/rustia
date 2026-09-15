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

// The manifest beside every committed Cargo.lock. The wasm runners are
// standalone packages with their own lock files, so a dependency added to a
// shared crate updates the root lock and leaves theirs stale. CI never builds
// them. The weekly matrix and the rustia-only memory lane build the benchmark
// adapter with --locked on the host, where a stale lock stops the run, and the
// demo's README documents a --locked build. Filtering the full tracked list
// here, not with a pathspec glob, covers a new lock file without an edit, and
// no git environment setting can shrink the list.
function lockedManifests() {
  const tracked = childProcess.execFileSync("git", ["ls-files", "-z"], {
    cwd: root,
    encoding: "utf8",
  });
  const locks = tracked.split("\0").filter((file) => /(^|\/)Cargo\.lock$/.test(file));
  if (!locks.includes("Cargo.lock")) {
    console.error("release-preflight: git ls-files did not list the root Cargo.lock");
    process.exit(1);
  }
  return locks.map((lock) => path.posix.join(path.posix.dirname(lock), "Cargo.toml"));
}

run("git", ["diff", "--quiet", "--", "Cargo.lock"]);
run("git", ["diff", "--cached", "--quiet", "--", "Cargo.lock"]);
run("cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"]);
// The same command the matrix lane runs on the host before it builds.
for (const manifest of lockedManifests()) {
  run("cargo", ["fetch", "--locked", "--manifest-path", manifest]);
}
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
node("tools/run-linode-benchmark.js", "self-test");
node("tools/run-dedicated-benchmark.js", "self-test");
node("tools/check-benchmark-series.js", "--self-test");
node("tools/record-host-bridge.js", "--self-test");
node("tools/retire-cloud-history.js", "--self-test");
node("tools/run-linux-history-backfill.js", "self-test");
node("tools/freeze-history-backfill.js", "--self-test");
node("tools/check-dedicated-config.js");
node("tools/check-dedicated-config.js", "--self-test");
node("tools/push-benchmark-data-publication.js", "self-test");
node("tools/run-linux-matrix-benchmark.js", "self-test");
node("tools/land-matrix-publication.js", "self-test");
node("bench/run-memory-supervisor.js", "self-test");
node("bench/analyze-memory-samples.js", "self-test");
node("bench/run-memory-cohort.js", "self-test");
node("bench/publish-memory-analysis.js", "self-test");
node("tools/run-linux-memory-benchmark.js", "self-test");
node("tools/create-cross-engine-memory-config.js", "self-test");
node("tools/run-wasm-gate.js", "self-test");
node("tools/run-wasm-dispatch-gate.js", "self-test");
node("tools/merge-rustia-ci-memory-data.js", "self-test");
node("tools/summarize-rustia-ci-memory.js", "self-test");
node("tools/check-benchmark-results.js");
node("tools/check-wasm-gate-results.js");
node("tools/render-benchmark-operations-status.js", "--check");
node("tools/check-current-report-summary.js");
node("tools/check-grabby-build-results.js");
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
run("cargo", [
  "clippy",
  "--locked",
  "-p",
  "rustia-compiler",
  "--all-targets",
  "--features",
  "table-write-diagnostics",
  "--",
  "-D",
  "warnings",
]);
run("cargo", [
  "test",
  "--locked",
  "-p",
  "rustia-compiler",
  "--features",
  "table-write-diagnostics",
  "--test",
  "array_slot_write_coverage",
  "--",
  "--nocapture",
]);
