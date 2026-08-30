#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const workflowDir = path.join(root, ".github", "workflows");
const ciPath = path.join(workflowDir, "ci.yml");
const rustiaMemoryPath = path.join(workflowDir, "linode-rustia-memory.yml");
const benchmarkPublicationPath = path.join(workflowDir, "publish-benchmark-data.yml");
const nightlyMatrixPath = path.join(workflowDir, "nightly-matrix-bench.yml");
const rustToolchainPath = path.join(root, "rust-toolchain.toml");
const releasePreflightPath = path.join(root, "tools", "run-release-preflight.js");

function fail(message) {
  console.error(`check-workflows: ${message}`);
  process.exit(1);
}

function readWorkflow(filePath) {
  try {
    return fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${path.relative(root, filePath)}: ${error.message}`);
  }
}

function pinnedRustChannel() {
  const text = readWorkflow(rustToolchainPath);
  const match = text.match(/^channel = "([^"]+)"$/m);
  if (!match) {
    fail("rust-toolchain.toml must declare a quoted channel");
  }
  return match[1];
}

function requireIncludes(text, needle, label) {
  if (!text.includes(needle)) {
    fail(`${label} must include ${JSON.stringify(needle)}`);
  }
}

function forbidIncludes(text, needle, label) {
  if (text.includes(needle)) {
    fail(`${label} must not include ${JSON.stringify(needle)}`);
  }
}

function workflowFiles() {
  try {
    return fs.readdirSync(workflowDir)
      .filter((name) => name.endsWith(".yml") || name.endsWith(".yaml"))
      .sort();
  } catch (error) {
    fail(`could not read ${path.relative(root, workflowDir)}: ${error.message}`);
  }
}

function usesLines(text) {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.startsWith("uses: "));
}

function checkActionPins(text, label) {
  for (const line of usesLines(text)) {
    const action = line.slice("uses: ".length).split(/\s+/, 1)[0];
    const official = /^actions\/[a-z0-9-]+@[0-9a-f]{40}$/.test(action);
    const rustToolchain = action === "actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659";
    if (!official && !rustToolchain) {
      fail(`${label} action must be an approved action pinned to a full commit SHA: ${line}`);
    }
  }
}

// A job-level `runs-on:`. The value group must be allowed to match empty,
// because the expanded form carries its group and labels on the following
// lines. This pattern previously required at least one character after the
// colon, so a bare `    runs-on:` never matched and the expanded-form check in
// checkRunnerLabels was unreachable for every workflow in the repository. The
// tripwire below fails loudly if that regression is ever reintroduced.
const RUNS_ON_PATTERN = /^ {4}runs-on:\s*(.*)$/;
for (const [sample, shouldMatch] of [
  ["    runs-on:", true],
  ["    runs-on: ubuntu-latest", true],
  ["  runs-on: ubuntu-latest", false],
]) {
  if (RUNS_ON_PATTERN.test(sample) !== shouldMatch) {
    fail(
      `runs-on pattern no longer ${shouldMatch ? "matches" : "rejects"} ${JSON.stringify(sample)}; ` +
        "the expanded-form runner check depends on it",
    );
  }
}

// Runner classes. The distinction that matters is not which file is allowed to
// name a self-hosted runner, but which machine may produce a timing. The shared
// pool is busy by design and must never be a timing authority; a qualified owned
// host is quiet, attested against the kernel before it builds, and takes one job
// at a time. Encoding that as a property of the machine rather than of a
// filename is what lets the second kind exist without weakening the first.
const RUNNER_CLASSES = new Map([
  [
    "general-linux",
    {
      block: [
        "    runs-on:",
        "      group: general-linux",
        "      labels: [self-hosted, Linux, X64, general-linux]",
      ].join("\n"),
      timingAuthority: false,
    },
  ],
]);

// Every workflow must appear here, so adding one is a deliberate act rather than
// a file that quietly inherits whatever the loop happens to allow. `null` means
// the workflow may only use GitHub-hosted runners.
const WORKFLOW_RUNNER_CLASS = new Map([
  ["ci.yml", "general-linux"],
  ["nightly-matrix-bench.yml", "general-linux"],
  ["publish-benchmark-data.yml", "general-linux"],
  ["linode-rustia-memory.yml", null],
]);

// Commands that take a timing. Until now only the first three were forbidden,
// and the four on-host runners were forbidden nowhere -- a workflow on the
// shared pool could have invoked any of them and no rule would have stopped it.
// That was the real hole; the raw-harness ban was the visible part of it.
const MEASUREMENT_COMMANDS = Object.freeze([
  "cargo run -p script-bench-harness",
  "node tools/run-wasm-gate.js",
  "node tools/run-wasm-dispatch-gate.js",
  "node tools/run-linux-fixed-benchmark.js",
  "node tools/run-linux-matrix-benchmark.js",
  "node tools/run-linux-history-backfill.js",
  "node tools/run-linux-memory-benchmark.js",
]);

// The single entrypoint a timing-authoritative runner may use. It attests the
// machine -- CPU model, logical count, SMT, and that every pinned core is in the
// kernel's isolated set -- before it builds anything, so measurement is
// reachable only through the code that checks the host is what the config says.
const ATTESTED_ENTRYPOINT = "node tools/run-dedicated-benchmark.js";

// Cloud coordinators are deliberately absent from MEASUREMENT_COMMANDS. They
// take no timing themselves: they provision a separate qualified instance and
// measure there, which is the retired model nightly-matrix-bench.yml still
// carries. Adding them here would require deleting that workflow, and it is
// still the only producer of the cross-engine reference every published Rustia
// number is normalised against. Phase-out happens when the matrix moves to the
// owned host, not before.

// contents: write can create a commit on main. Forbidden everywhere except the
// workflows named here, so a third holder cannot appear by omission.
const CONTENTS_WRITE_WORKFLOWS = new Set(["nightly-matrix-bench.yml"]);

function checkRunnerLabels(files) {
  const hostedRunners = new Set(["ubuntu-latest", "macos-latest", "windows-latest"]);
  for (const file of files) {
    const text = readWorkflow(path.join(workflowDir, file));
    const label = `.github/workflows/${file}`;
    if (!WORKFLOW_RUNNER_CLASS.has(file)) {
      fail(`${label} is not registered in WORKFLOW_RUNNER_CLASS; a new workflow must name its runner class`);
    }
    const className = WORKFLOW_RUNNER_CLASS.get(file);
    const runnerClass = className === null ? null : RUNNER_CLASSES.get(className);
    if (className !== null && runnerClass === undefined) {
      fail(`${label} names unknown runner class ${JSON.stringify(className)}`);
    }

    // Action pins are checked for every workflow here rather than only inside
    // the four per-file checkers, where a fifth workflow's pins went unchecked.
    checkActionPins(text, label);

    const lines = text.split(/\r?\n/);
    for (let index = 0; index < lines.length; index += 1) {
      const inline = lines[index].match(RUNS_ON_PATTERN);
      if (!inline) continue;
      const value = inline[1].trim();
      if (value.startsWith("[")) {
        fail(`${label} must use expanded runs-on form so runner labels are reviewable`);
      }
      if (value === "") {
        if (runnerClass === null) {
          fail(`${label} may only use GitHub-hosted runners, but declares an expanded runs-on block`);
        }
        const runnerBlock = lines.slice(index, index + 3).join("\n");
        if (runnerBlock !== runnerClass.block) {
          fail(`${label} expanded runs-on block must match the reviewed ${className} class exactly`);
        }
        continue;
      }
      if (!hostedRunners.has(value)) {
        fail(`${label} hosted runs-on value must be one of ${[...hostedRunners].join(", ")}, got ${value}`);
      }
    }

    // A workflow may name exactly the runners its class allows, and no others.
    // Counting occurrences catches a bench workflow that also names the shared
    // pool, which a plain allowlist cannot express.
    const selfHostedMentions = text.split("self-hosted").length - 1;
    const allowedMentions = runnerClass === null
      ? 0
      : text.split(runnerClass.block).length - 1;
    if (selfHostedMentions !== allowedMentions) {
      fail(
        `${label} mentions "self-hosted" ${selfHostedMentions} time(s) but declares the ` +
          `${className ?? "hosted-only"} runner block ${allowedMentions} time(s); a workflow may name ` +
          `only its own class`,
      );
    }

    for (const command of MEASUREMENT_COMMANDS) forbidIncludes(text, command, label);
    if (runnerClass === null || !runnerClass.timingAuthority) {
      forbidIncludes(text, ATTESTED_ENTRYPOINT, label);
    }

    // No workflow needs either trigger, and workflow_run is the canonical way to
    // hand a privileged token to a job that ran fork code.
    forbidIncludes(text, "pull_request_target", label);
    forbidIncludes(text, "workflow_run", label);
    if (!CONTENTS_WRITE_WORKFLOWS.has(file)) forbidIncludes(text, "contents: write", label);
  }
}

function checkCi(text) {
  const label = ".github/workflows/ci.yml";
  const rustChannel = pinnedRustChannel();
  const releasePreflightJob = text.split("\n  portable-engine-build:\n", 1)[0];
  requireIncludes(text, "name: CI\n", label);
  requireIncludes(text, "\"on\":\n  pull_request:\n  push:\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(
    text,
    "runs-on:\n      group: general-linux\n      labels: [self-hosted, Linux, X64, general-linux]",
    label,
  );
  requireIncludes(text, "max-parallel: 2\n", label);
  requireIncludes(text, "persist-credentials: false\n", label);
  requireIncludes(
    text,
    "uses: actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
    label,
  );
  requireIncludes(text, `toolchain: ${rustChannel}`, label);
  requireIncludes(text, "components: clippy, rustfmt", label);
  requireIncludes(text, "cache: false", label);
  forbidIncludes(text, "rustup toolchain install stable", label);
  forbidIncludes(text, "rustup default stable", label);
  requireIncludes(text, "node tools/run-release-preflight.js", label);
  requireIncludes(releasePreflightJob, "fetch-depth: 0", `${label} release-preflight job`);
  forbidIncludes(text, "cargo clippy --workspace", label);
  forbidIncludes(text, "cargo test --workspace", label);
  forbidIncludes(text, "node tools/check-package-files.js", label);
  forbidIncludes(text, "cargo run -p script-bench-harness", label);
  requireIncludes(text, "portable-engine-build:\n", label);
  requireIncludes(text, "name: Engine build (${{ matrix.target }})", label);
  requireIncludes(text, "fail-fast: false\n", label);
  for (const target of [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-ios",
    "aarch64-linux-android",
    "wasm32-unknown-unknown",
  ]) {
    requireIncludes(text, `          - ${target}\n`, label);
  }
  requireIncludes(text, "target: ${{ matrix.target }}", label);
  requireIncludes(
    text,
    "cargo check --locked -p rustia --lib --target ${{ matrix.target }}",
    label,
  );
  requireIncludes(text, "wasm-node-runtime:\n", label);
  requireIncludes(text, "name: Wasm runtime (Node/V8)", label);
  requireIncludes(
    text,
    "cargo install wasm-bindgen-cli --version 0.2.126 --locked",
    label,
  );
  requireIncludes(
    text,
    "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER: wasm-bindgen-test-runner",
    label,
  );
  requireIncludes(
    text,
    "cargo test --locked -p rustia --target wasm32-unknown-unknown --no-default-features --test bare_wasm_live",
    label,
  );
  requireIncludes(
    text,
    "cargo test --locked -p rustia --target wasm32-unknown-unknown --no-default-features --test mixed_heap_pressure",
    label,
  );
  requireIncludes(
    text,
    "cargo test --locked -p rustia --target wasm32-unknown-unknown --test web_live",
    label,
  );
  checkActionPins(text, label);
}

function checkReleasePreflight(text) {
  const label = "tools/run-release-preflight.js";
  for (const needle of [
    '"git", ["diff", "--quiet", "--", "Cargo.lock"]',
    '"cargo", ["metadata", "--locked"',
    '"cargo", ["fmt", "--all"',
    '"cargo", ["clippy", "--locked", "--workspace", "--all-targets"',
    '"tools/check-release-notes.js"',
    '"tools/check-crate-metadata.js"',
    '"tools/check-workflows.js"',
    '"tools/check-package-files.js"',
    '"tools/check-publishability.js"',
    '"tools/check-package-dry-runs.js"',
    '"tools/verify-conformance-package.js"',
    '"design/render-progress.js"',
    '"tools/check-project-status.js"',
    '"bench/analyze-fixed-samples.js"',
    '"tools/run-digitalocean-benchmark.js"',
    '"tools/run-linode-benchmark.js"',
    '"tools/run-linux-matrix-benchmark.js"',
    // The owned-host lane. Its self-tests carry the rules that keep a source
    // repair honest and a host attested, so preflight must keep running them.
    '"tools/run-dedicated-benchmark.js"',
    '"tools/check-benchmark-series.js"',
    '"tools/run-linux-history-backfill.js"',
    '"tools/freeze-history-backfill.js"',
    '"bench/run-memory-supervisor.js"',
    '"bench/analyze-memory-samples.js"',
    '"bench/run-memory-cohort.js"',
    '"bench/publish-memory-analysis.js"',
    '"tools/run-linux-memory-benchmark.js"',
    '"tools/run-vultr-benchmark.js"',
    '"tools/run-scaleway-benchmark.js"',
    '"tools/run-wasm-gate.js"',
    '"tools/run-wasm-dispatch-gate.js"',
    '"tools/merge-rustia-ci-memory-data.js"',
    '"tools/check-benchmark-results.js"',
    '"tools/check-wasm-gate-results.js"',
    '"tools/render-benchmark-operations-status.js", "--check"',
    '"tools/check-current-report-summary.js"',
    '"bench/render-current-results.js"',
    '"bench/render-claims.js"',
    '"cargo", ["fmt", "--all", "--manifest-path", "fuzz/Cargo.toml"',
    '"clippy",\n  "--locked",\n  "--manifest-path",\n  "fuzz/Cargo.toml"',
    '"cargo", ["test", "--locked", "--manifest-path", "fuzz/Cargo.toml"]',
    '"cargo", ["test", "--locked", "--workspace"]',
  ]) {
    requireIncludes(text, needle, label);
  }
}

function checkRustiaMemory(text) {
  const label = ".github/workflows/linode-rustia-memory.yml";
  requireIncludes(text, "name: Rustia Linode Memory\n", label);
  requireIncludes(text, "\"on\":\n  workflow_dispatch:\n", label);
  requireIncludes(text, "commit_sha:\n", label);
  requireIncludes(text, "required: true\n", label);
  requireIncludes(text, "type: string\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "cancel-in-progress: false\n", label);
  requireIncludes(text, "if: github.ref == 'refs/heads/main'", label);
  requireIncludes(text, "ref: main", label);
  requireIncludes(text, "BENCHMARK_COMMIT: ${{ inputs.commit_sha }}", label);
  requireIncludes(text, "^[0-9a-f]{40}$", label);
  requireIncludes(text, 'git cat-file -e "${BENCHMARK_COMMIT}^{commit}"', label);
  requireIncludes(text, 'git merge-base --is-ancestor "$BENCHMARK_COMMIT" origin/main', label);
  requireIncludes(text, 'test "$(git rev-parse HEAD)" = "$BENCHMARK_COMMIT"', label);
  requireIncludes(text, "LINODE_TOKEN: ${{ secrets.LINODE_TOKEN }}", label);
  requireIncludes(text, "node tools/create-rustia-ci-memory-config.js", label);
  requireIncludes(text, "node tools/run-linode-benchmark.js bench", label);
  requireIncludes(text, "node tools/summarize-rustia-ci-memory.js", label);
  requireIncludes(text, "BENCH_DATA_DEPLOY_KEY: ${{ secrets.BENCH_DATA_DEPLOY_KEY }}", label);
  requireIncludes(text, "git@github.com:slowchop/rustia-bench-data.git", label);
  requireIncludes(text, "node tools/merge-rustia-ci-memory-data.js", label);
  requireIncludes(text, "git -C \"$data_dir\" push origin HEAD:main", label);
  requireIncludes(text, "git -C \"$pages_dir\" push origin HEAD:gh-pages", label);
  requireIncludes(text, "node tools/run-linode-benchmark.js destroy", label);
  forbidIncludes(text, "actions/upload-artifact", label);
  forbidIncludes(text, "retention-days:", label);
  forbidIncludes(text, "pull_request_target", label);
  forbidIncludes(text, "workflow_run", label);
  forbidIncludes(text, "contents: write", label);
  checkActionPins(text, label);
}

function checkNightlyMatrix(text) {
  const label = ".github/workflows/nightly-matrix-bench.yml";
  requireIncludes(text, "name: Cross-Engine Matrix (manual)\n", label);
  // Manual dispatch only. The nightly schedule created a billed Linode
  // whenever main gained an engine commit; measurement has moved to the owned
  // host, so nothing in this repository may start a paid instance on a timer.
  // Restoring a cron here has to change this line too, which is the point.
  requireIncludes(text, "\"on\":\n  workflow_dispatch:\n", label);
  forbidIncludes(text, "cron:", label);
  // The single sanctioned contents-write grant: this workflow may push
  // accepted matrix cohorts to main and nothing else may.
  requireIncludes(text, "permissions:\n  contents: write\n  actions: write\n", label);
  requireIncludes(text, "publish-benchmark-data.yml/dispatches", label);
  requireIncludes(text, "cancel-in-progress: false\n", label);
  requireIncludes(text, "if: github.ref == 'refs/heads/main'", label);
  requireIncludes(text, "ref: main", label);
  requireIncludes(
    text,
    "runs-on:\n      group: general-linux\n      labels: [self-hosted, Linux, X64, general-linux]",
    label,
  );
  requireIncludes(
    text,
    "uses: actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
    label,
  );
  requireIncludes(text, "toolchain: 1.95.0", label);
  requireIncludes(text, "cache: false", label);
  requireIncludes(
    text,
    "678efc1cfbd5fa5a88375daa7e2f3864a049d6d63a0296df925a2ae5f516cb56",
    label,
  );
  requireIncludes(text, "sha256sum -c -", label);
  requireIncludes(text, "pending_relevant_commit_count", label);
  requireIncludes(text, "steps.gate.outputs.pending != '0'", label);
  requireIncludes(text, "LINODE_TOKEN: ${{ secrets.LINODE_TOKEN }}", label);
  requireIncludes(text, "node tools/create-linode-matrix-config.js", label);
  requireIncludes(text, "node tools/register-matrix-cohort.js", label);
  requireIncludes(text, "node tools/run-linode-benchmark.js bench", label);
  requireIncludes(text, "node tools/check-benchmark-results.js", label);
  requireIncludes(text, "node tools/run-linode-benchmark.js destroy", label);
  requireIncludes(text, "rm -f .rustia-benchmark-state/linode/token", label);
  forbidIncludes(text, "actions/upload-artifact", label);
  forbidIncludes(text, "retention-days:", label);
  forbidIncludes(text, "pull_request_target", label);
  forbidIncludes(text, "workflow_run", label);
  forbidIncludes(text, "BENCH_DATA_DEPLOY_KEY", label);
  checkActionPins(text, label);
}

function checkBenchmarkPublication(text) {
  const label = ".github/workflows/publish-benchmark-data.yml";
  requireIncludes(text, "name: Publish Benchmark Data\n", label);
  requireIncludes(text, "push:\n    branches:\n      - main\n    paths:\n", label);
  requireIncludes(text, "workflow_dispatch:\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "cancel-in-progress: false\n", label);
  requireIncludes(text, "if: github.ref == 'refs/heads/main'", label);
  requireIncludes(
    text,
    "runs-on:\n      group: general-linux\n      labels: [self-hosted, Linux, X64, general-linux]",
    label,
  );
  requireIncludes(text, "ref: main", label);
  requireIncludes(text, "persist-credentials: false", label);
  requireIncludes(
    text,
    "uses: actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
    label,
  );
  requireIncludes(text, "toolchain: 1.95.0", label);
  requireIncludes(text, "cache: false", label);
  requireIncludes(text, "node tools/check-benchmark-results.js", label);
  requireIncludes(text, "node tools/check-wasm-gate-results.js", label);
  requireIncludes(text, "node tools/render-benchmark-operations-status.js --check", label);
  requireIncludes(text, "BENCH_DATA_DEPLOY_KEY: ${{ secrets.BENCH_DATA_DEPLOY_KEY }}", label);
  requireIncludes(text, "git@github.com:slowchop/rustia-bench-data.git", label);
  requireIncludes(text, "bench/operations-status.json", label);
  // Publishing the chart without the rows behind it leaves a reader unable to
  // check a number they can see.
  requireIncludes(text, "bench/history/backfills/**", label);
  requireIncludes(text, "for backfill in bench/history/backfills/*.json; do", label);
  requireIncludes(text, "node tools/check-benchmark-series.js --rows-root", label);
  requireIncludes(text, "node tools/render-benchmark-data-readme.js", label);
  requireIncludes(text, "publish_dir=\"$(mktemp -d \"$RUNNER_TEMP/rustia-bench-publish.XXXXXX\")\"", label);
  requireIncludes(text, "rm -rf -- \"$publish_dir\"", label);
  requireIncludes(text, "trap cleanup EXIT", label);
  requireIncludes(text, "UserKnownHostsFile=$ssh_dir/known_hosts", label);
  requireIncludes(text, "git -C \"$data_dir\" push origin HEAD:main", label);
  requireIncludes(text, "git -C \"$pages_dir\" push origin HEAD:gh-pages", label);
  for (const forbidden of [
    "LINODE_TOKEN",
    "run-linode-benchmark.js bench",
    "run-digitalocean-benchmark.js bench",
    "run-vultr-benchmark.js bench",
    "run-scaleway-benchmark.js bench",
    "workflow_run",
    "pull_request_target",
    "contents: write",
  ]) forbidIncludes(text, forbidden, label);
  checkActionPins(text, label);
}

const files = workflowFiles();
for (const required of [
  "ci.yml",
  "linode-rustia-memory.yml",
  "nightly-matrix-bench.yml",
  "publish-benchmark-data.yml",
]) {
  if (!files.includes(required)) {
    fail(`missing .github/workflows/${required}`);
  }
}
if (files.includes("benchmarks.yml")) {
  fail(".github/workflows/benchmarks.yml must remain absent; fresh timing runs belong to the explicit Linode lifecycle");
}
checkRunnerLabels(files);
checkCi(readWorkflow(ciPath));
checkRustiaMemory(readWorkflow(rustiaMemoryPath));
checkNightlyMatrix(readWorkflow(nightlyMatrixPath));
checkBenchmarkPublication(readWorkflow(benchmarkPublicationPath));
checkReleasePreflight(readWorkflow(releasePreflightPath));

console.log(`workflow policy ok for ${files.length} workflow file(s)`);
