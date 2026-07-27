#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const workflowDir = path.join(root, ".github", "workflows");
const ciPath = path.join(workflowDir, "ci.yml");
const rustiaMemoryPath = path.join(workflowDir, "linode-rustia-memory.yml");
const rustToolchainPath = path.join(root, "rust-toolchain.toml");

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
    const action = line.slice("uses: ".length);
    if (!/^actions\/[a-z0-9-]+@v\d+$/.test(action)) {
      fail(`${label} action must be pinned to an actions/* major version: ${line}`);
    }
  }
}

function checkRunnerLabels(files) {
  const hostedRunners = new Set(["ubuntu-latest", "macos-latest", "windows-latest"]);
  for (const file of files) {
    const text = readWorkflow(path.join(workflowDir, file));
    const label = `.github/workflows/${file}`;
    const lines = text.split(/\r?\n/);
    for (let index = 0; index < lines.length; index += 1) {
      const line = lines[index];
      const inline = line.match(/^ {4}runs-on:\s*(.+)$/);
      if (!inline) {
        continue;
      }
      const value = inline[1].trim();
      if (value.startsWith("[")) {
        fail(`${label} must use expanded runs-on form so runner labels are reviewable`);
      }
      if (value === "") fail(`${label} must not use a self-hosted/list-form runner`);
      if (!hostedRunners.has(value)) {
        fail(`${label} hosted runs-on value must be one of ${[...hostedRunners].join(", ")}, got ${value}`);
      }
    }
    forbidIncludes(text, "self-hosted", label);
    forbidIncludes(text, "cargo run -p script-bench-harness", label);
    forbidIncludes(text, "node tools/run-wasm-gate.js", label);
    forbidIncludes(text, "node tools/run-wasm-dispatch-gate.js", label);
  }
}

function checkCi(text) {
  const label = ".github/workflows/ci.yml";
  const rustChannel = pinnedRustChannel();
  requireIncludes(text, "name: CI\n", label);
  requireIncludes(text, "\"on\":\n  pull_request:\n  push:\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "runs-on: ubuntu-latest", label);
  requireIncludes(
    text,
    `rustup toolchain install ${rustChannel} --profile minimal --component clippy --component rustfmt`,
    label,
  );
  requireIncludes(text, `rustup default ${rustChannel}`, label);
  requireIncludes(text, "cargo fmt --all -- --check", label);
  requireIncludes(text, "cargo clippy --workspace --all-targets -- -D warnings", label);
  forbidIncludes(text, "rustup toolchain install stable", label);
  forbidIncludes(text, "rustup default stable", label);
  requireIncludes(text, "node tools/check-release-notes.js", label);
  requireIncludes(text, "node tools/check-crate-metadata.js --check", label);
  requireIncludes(text, "node tools/check-package-files.js --check", label);
  requireIncludes(text, "node tools/check-publishability.js --check", label);
  requireIncludes(text, "node tools/check-package-dry-runs.js", label);
  requireIncludes(text, "node tools/check-workflows.js", label);
  requireIncludes(text, "node design/render-progress.js --check", label);
  requireIncludes(text, "node tools/check-project-status.js", label);
  requireIncludes(text, "node bench/analyze-fixed-samples.js --self-test", label);
  requireIncludes(text, "node tools/check-benchmark-results.js", label);
  requireIncludes(text, "node tools/check-wasm-gate-results.js", label);
  requireIncludes(text, "node tools/check-current-report-summary.js", label);
  requireIncludes(text, "node tools/merge-rustia-ci-memory-data.js self-test", label);
  requireIncludes(text, "node bench/render-current-results.js --check", label);
  requireIncludes(text, "node bench/render-claims.js --check", label);
  requireIncludes(text, "cargo test -p rustia --examples --no-run", label);
  requireIncludes(text, "cargo test --workspace", label);
  forbidIncludes(text, "cargo run -p script-bench-harness", label);
  checkActionPins(text, label);
}

function checkRustiaMemory(text) {
  const label = ".github/workflows/linode-rustia-memory.yml";
  requireIncludes(text, "name: Rustia Linode Memory\n", label);
  requireIncludes(text, "\"on\":\n  workflow_run:\n", label);
  requireIncludes(text, "workflows:\n      - CI\n", label);
  requireIncludes(text, "types:\n      - completed\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "cancel-in-progress: false\n", label);
  requireIncludes(text, "github.event.workflow_run.conclusion == 'success'", label);
  requireIncludes(text, "github.event.workflow_run.event == 'push'", label);
  requireIncludes(text, "github.event.workflow_run.head_branch == 'main'", label);
  requireIncludes(
    text,
    "github.event.workflow_run.head_repository.full_name == github.repository",
    label,
  );
  requireIncludes(text, "ref: ${{ github.event.workflow_run.head_sha }}", label);
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
  forbidIncludes(text, "contents: write", label);
  checkActionPins(text, label);
}

const files = workflowFiles();
for (const required of ["ci.yml", "linode-rustia-memory.yml"]) {
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

console.log(`workflow policy ok for ${files.length} workflow file(s)`);
