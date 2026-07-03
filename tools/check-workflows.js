#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const workflowDir = path.join(root, ".github", "workflows");
const ciPath = path.join(workflowDir, "ci.yml");
const benchmarksPath = path.join(workflowDir, "benchmarks.yml");

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

function requireRegex(text, regex, label, description) {
  if (!regex.test(text)) {
    fail(`${label} must ${description}`);
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

function benchmarkRunLabels(text) {
  const match = text.match(/runs-on:\n((?: {6}- [^\n]+\n)+)/);
  if (!match) {
    fail("benchmark workflow must use list-form runs-on labels");
  }
  return match[1]
    .trim()
    .split(/\r?\n/)
    .map((line) => line.trim().replace(/^- /, ""));
}

function collectListLabels(lines, startIndex) {
  const labels = [];
  for (let index = startIndex + 1; index < lines.length; index += 1) {
    const line = lines[index];
    const match = line.match(/^ {6}- ([^\n]+)$/);
    if (!match) {
      break;
    }
    labels.push(match[1].trim());
  }
  return labels;
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
      if (value === "") {
        const labels = collectListLabels(lines, index);
        if (labels.join(",") !== "self-hosted,bench") {
          fail(`${label} list-form runs-on labels must be exactly self-hosted + bench, got ${labels.join(", ")}`);
        }
        continue;
      }
      if (!hostedRunners.has(value)) {
        fail(`${label} hosted runs-on value must be one of ${[...hostedRunners].join(", ")}, got ${value}`);
      }
    }
  }
}

function checkCi(text) {
  const label = ".github/workflows/ci.yml";
  requireIncludes(text, "name: CI\n", label);
  requireIncludes(text, "\"on\":\n  pull_request:\n  push:\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "runs-on: ubuntu-latest", label);
  requireIncludes(text, "node tools/check-release-notes.js", label);
  requireIncludes(text, "node tools/check-crate-metadata.js --check", label);
  requireIncludes(text, "node tools/check-package-files.js --check", label);
  requireIncludes(text, "node tools/check-publishability.js --check", label);
  requireIncludes(text, "node tools/check-package-dry-runs.js", label);
  requireIncludes(text, "node tools/check-workflows.js", label);
  requireIncludes(text, "node design/render-progress.js --check", label);
  requireIncludes(text, "node design/show-progress.js --no-color --width 88 >/dev/null", label);
  requireIncludes(text, "node tools/check-project-status.js", label);
  requireIncludes(text, "node tools/check-benchmark-results.js", label);
  requireIncludes(text, "node bench/render-current-results.js --check", label);
  requireIncludes(text, "node bench/render-claims.js --check", label);
  requireIncludes(text, "cargo test -p rustia --examples --no-run", label);
  requireIncludes(text, "cargo test --workspace", label);
  forbidIncludes(text, "cargo run -p script-bench-harness", label);
  checkActionPins(text, label);
}

function checkBenchmarks(text) {
  const label = ".github/workflows/benchmarks.yml";
  requireIncludes(text, "name: Benchmark Measurement\n", label);
  requireIncludes(text, "\"on\":\n  workflow_dispatch:\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  forbidIncludes(text, "pull_request:", label);
  forbidIncludes(text, "push:", label);
  forbidIncludes(text, "schedule:", label);
  forbidIncludes(text, "contents: write", label);
  forbidIncludes(text, "git push", label);
  forbidIncludes(text, "secrets.", label);
  const labels = benchmarkRunLabels(text);
  if (labels.join(",") !== "self-hosted,bench") {
    fail(`${label} runner labels must be exactly self-hosted + bench, got ${labels.join(", ")}`);
  }
  requireIncludes(text, "git diff --quiet --ignore-submodules --", label);
  requireIncludes(text, "git diff --cached --quiet --ignore-submodules --", label);
  requireIncludes(text, "--features all-engines", label);
  requireIncludes(text, "--features mlua-lua54", label);
  requireIncludes(text, "--commit \"$(git rev-parse --short HEAD)\"", label);
  requireIncludes(text, "cargo run -p script-bench-harness -- --refresh-results", label);
  requireIncludes(text, "node bench/render-current-results.js", label);
  requireIncludes(text, "node bench/render-claims.js", label);
  requireIncludes(text, "uses: actions/upload-artifact@v4", label);
  requireRegex(text, /bench\/history\/runs\.jsonl[\s\S]*bench\/CLAIMS\.md/, label, "upload benchmark history and claim artifacts");
  checkActionPins(text, label);
}

const files = workflowFiles();
for (const required of ["ci.yml", "benchmarks.yml"]) {
  if (!files.includes(required)) {
    fail(`missing .github/workflows/${required}`);
  }
}
checkRunnerLabels(files);
checkCi(readWorkflow(ciPath));
checkBenchmarks(readWorkflow(benchmarksPath));

console.log(`workflow policy ok for ${files.length} workflow file(s)`);
