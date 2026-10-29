#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const path = require("path");

const root = path.resolve(__dirname, "..");
for (const name of ["RUSTIA_UPDATE_C23_CENSUS", "RUSTIA_UPDATE_C23_O2_PREVIEW"]) {
  if (process.env[name] !== undefined) {
    throw new Error(`Validation refuses evidence-regeneration override ${name}`);
  }
}

const plan = [];
function run(command, args, extraEnv = {}) {
  plan.push({ command, args, extraEnv });
}

function execute({ command, args, extraEnv }) {
  process.stdout.write(`+ ${command} ${args.join(" ")}\n`);
  try {
    childProcess.execFileSync(command, args, {
      cwd: root,
      env: { ...process.env, ...extraEnv },
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
// shared crate updates the root lock and leaves theirs stale. The workspace
// build never reads them. The weekly matrix and the rustia-only memory lane
// build the benchmark adapter with --locked on the host, where a stale lock
// stops the run, and the demo's README documents a --locked build. Filtering
// the full tracked list here, not with a pathspec glob, covers a new lock file
// without an edit, and no git environment setting can shrink the list.
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
run("cargo", ["fmt", "--manifest-path", "demos/web-live/Cargo.toml", "--", "--check"]);
run("cargo", ["clippy", "--locked", "--manifest-path", "demos/web-live/Cargo.toml", "--target", "wasm32-unknown-unknown", "--all-targets", "--", "-D", "warnings"]);
run("cargo", ["clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"]);
// The three builds that otherwise compile only on the owned benchmark host,
// for the weekly matrix and the memory lanes: the competitor engines, the Lua
// 5.4 binary (the one build with rustia-engine off), and the wasm adapter (a
// standalone package that calls rustia's API). The workspace Clippy above sees
// none of their cfg paths: bench's default features are rustia-engine alone,
// and the adapter is a workspace of its own. A break in any of them showed up
// only as a failed host run after merge. Both host workflows inherit the
// toolchain action's default RUSTFLAGS=-D warnings, so a warning fails the
// host build too, and a plain `cargo check` would miss it. Clippy with
// -D warnings holds these builds to the bar the rest of the code meets. None
// needs its own CARGO_TARGET_DIR. Cargo keys artifacts by feature set, so the
// bench checks sit beside the workspace build without evicting it. The
// adapter builds in its own target/, as it does on the host.
run("cargo", [
  "clippy",
  "--locked",
  "--manifest-path",
  "bench/Cargo.toml",
  "--bin",
  "script-bench",
  "--no-default-features",
  "--features",
  "all-engines",
  "--",
  "-D",
  "warnings",
]);
run("cargo", [
  "clippy",
  "--locked",
  "--manifest-path",
  "bench/Cargo.toml",
  "--bin",
  "script-bench",
  "--no-default-features",
  "--features",
  "mlua-lua54",
  "--",
  "-D",
  "warnings",
]);
run("cargo", [
  "clippy",
  "--locked",
  "--manifest-path",
  "bench/wasm/adapter-runner/Cargo.toml",
  "--target",
  "wasm32-unknown-unknown",
  "--",
  "-D",
  "warnings",
]);

node("tools/check-release-notes.js");
node("tools/check-crate-metadata.js", "--check");
node("tools/check-preflight.js");
run("node", ["--test", "tools/browser/result.test.cjs"]);
node("tools/check-pre-push.js");
node("tools/check-workflows.js");
node("tools/check-workflows.js", "--self-test");
run("python3", ["tools/check-browser-correctness.py"]);
node("tools/check-package-files.js", "--check");
node("tools/check-publishability.js", "--check");
node("tools/check-package-dry-runs.js");
node("tools/verify-conformance-package.js");
node("design/render-progress.js", "--check");
node("tools/check-project-status.js");
node("tools/check-readme-examples.js");
node("tools/check-facade-api.js");
node("tools/check-external-facade.js");
node("tools/check-readme-comparisons.js");
run("cargo", ["doc", "--locked", "--workspace", "--lib", "--no-deps"], {
  RUSTDOCFLAGS: [process.env.RUSTDOCFLAGS, "-D warnings"].filter(Boolean).join(" "),
});

node("bench/analyze-fixed-samples.js", "--self-test");
node("tools/run-linode-benchmark.js", "self-test");
node("tools/run-dedicated-benchmark.js", "self-test");
node("tools/check-benchmark-series.js", "--self-test");
node("tools/bench-host-control.js", "--self-test");
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
run("node", ["--test", "tools/history-projection.test.js"]);
node("tools/check-benchmark-commit-reachability.js");
node("tools/check-benchmark-commit-reachability.js", "--self-test");
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
run("cargo", ["run", "--locked", "-p", "rustia", "--example", "game_objects"]);
run("cargo", ["run", "--locked", "-p", "rustia", "--example", "mutable_objects"]);
run("cargo", ["run", "--locked", "-p", "rustia", "--example", "hardened_decision"]);
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

// One command inventory serves local validation and every CI stage.
const stages = ["quick", "workspace", "release"];
const packageScripts = new Set([
  "tools/check-package-dry-runs.js", "tools/verify-conformance-package.js",
  "tools/check-external-facade.js", "tools/check-readme-examples.js",
  "tools/check-facade-api.js",
]);
function stageOf({ command, args }) {
  if (command === "cargo") {
    if (args[0] === "test" && args.includes("--workspace")) return "workspace";
    return ["fmt", "metadata", "fetch"].includes(args[0]) ? "quick" : "release";
  }
  return command === "node" && packageScripts.has(args[0]) ? "release" : "quick";
}
// Catch ordinary compiler regressions before clean-package rebuilds or long corpora.
const compilerCheck = { command: "cargo", args: ["test", "--locked", "-p", "rustia-compiler", "--lib"], extraEnv: {} };
plan.push(compilerCheck);
function assignedStage(entry) {
  return entry === compilerCheck ? "quick" : stageOf(entry);
}
const argv = process.argv.slice(2);
if (argv.length === 1 && argv[0] === "--list") {
  console.log(JSON.stringify(plan.map(entry => ({ ...entry, stage: assignedStage(entry) })), null, 2));
} else if (argv.length === 2 && argv[0] === "--stage" && stages.includes(argv[1])) {
  for (const entry of plan.filter(entry => assignedStage(entry) === argv[1] ||
    (argv[1] !== "quick" && entry.command === "cargo" && ["metadata", "fetch"].includes(entry.args[0])))) execute(entry);
} else if (argv.length === 0) {
  for (const entry of plan.filter(entry => assignedStage(entry) === "quick")) execute(entry);
  // Package construction/doc generation stay serialized in release; workspace
  // execution can overlap them. Separate build trees prevent shared-artifact races.
  const children = ["workspace", "release"].map(stage => childProcess.spawn(
    process.execPath, [__filename, "--stage", stage], {
      cwd: root, stdio: "inherit",
      env: { ...process.env, CARGO_TARGET_DIR: stage === "workspace"
        ? (process.env.CARGO_TARGET_DIR || path.join(root, "target"))
        : path.join(root, "target", "preflight-release") },
    },
  ));
  Promise.all(children.map(child => new Promise(resolve => {
    child.on("error", error => { console.error(error.message); resolve(false); });
    child.on("exit", code => resolve(code === 0));
  }))).then(results => { if (results.some(ok => !ok)) process.exitCode = 1; });
} else {
  console.error("usage: run-release-preflight.js [--stage quick|workspace|release|--list]");
  process.exitCode = 2;
}
