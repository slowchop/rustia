#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const EXPECTED_FILES = Object.freeze(["ci.yml", "error-fuzz.yml"]);
const CI_JOBS = Object.freeze({
  "release-preflight": "ubuntu-24.04",
  "release-gate": "ubuntu-24.04",
  "macos-arm64-runtime": "macos-15",
  "portable-engine-build": "ubuntu-24.04",
  "wasm-node-runtime": "ubuntu-24.04",
  "windows-runtime": "windows-2022",
  "webkit-runtime": "ubuntu-24.04",
});
const FUZZ_JOBS = Object.freeze({
  "differential-smoke": "ubuntu-24.04",
  campaign: "ubuntu-24.04",
});
const PINNED_ACTIONS = new Set([
  "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
  "actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
  "actions/setup-node@820762786026740c76f36085b0efc47a31fe5020",
  "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
]);
const FORBIDDEN_WORKFLOW_TEXT = Object.freeze([
  "self-hosted",
  "workflow_run",
  "pull_request_target",
  "schedule:",
  "repository_dispatch",
  "workflow_call",
  "contents: write",
  "permissions: write-all",
  "secrets.",
  "${{ secrets",
  "LINODE_TOKEN",
  "BENCH_DATA_DEPLOY_KEY",
  "run-dedicated-benchmark.js",
  "script-bench-harness",
  "cargo bench",
  "node tools/run-wasm-gate.js",
  "node tools/run-wasm-dispatch-gate.js",
  "run-linux-history-backfill.js",
  "run-linux-matrix-benchmark.js",
  "run-linux-memory-benchmark.js",
  "run-linode-benchmark.js",
  "push-benchmark-data-publication.js",
  "land-matrix-publication.js",
  "merge-rustia-ci-memory-data.js",
  "cargo publish",
  "npm publish",
  "twine upload",
]);
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const BENCHMARK_BRIDGE_STATES = Object.freeze([
  "pending-private-trusted-pipeline",
  "private-maintainer-cohorts-v1",
]);

function reject(message) {
  throw new Error(message);
}

function requireText(text, needle, label) {
  if (!text.includes(needle)) reject(`${label} must include ${JSON.stringify(needle)}`);
}

function forbidText(text, needle, label) {
  if (text.includes(needle)) reject(`${label} must not include ${JSON.stringify(needle)}`);
}

function jobBlocks(text, label) {
  const lines = text.split(/\r?\n/);
  const start = lines.findIndex((line) => line === "jobs:");
  if (start < 0) reject(`${label} must declare jobs`);
  const starts = [];
  for (let index = start + 1; index < lines.length; index += 1) {
    const match = lines[index].match(/^  ([A-Za-z0-9_-]+):\s*$/);
    if (match) starts.push({ name: match[1], index });
  }
  if (starts.length === 0) reject(`${label} must declare at least one job`);
  return new Map(starts.map(({ name, index }, at) => [
    name,
    lines.slice(index, starts[at + 1]?.index ?? lines.length).join("\n"),
  ]));
}

function checkJobRunners(text, expected, label) {
  const blocks = jobBlocks(text, label);
  const names = [...blocks.keys()].sort();
  const expectedNames = Object.keys(expected).sort();
  if (JSON.stringify(names) !== JSON.stringify(expectedNames)) {
    reject(`${label} jobs must be exactly ${expectedNames.join(", ")}; found ${names.join(", ")}`);
  }
  for (const [name, runner] of Object.entries(expected)) {
    const declarations = [...blocks.get(name).matchAll(/^    runs-on:\s*(.*?)\s*$/gm)].map((match) => match[1]);
    if (declarations.length !== 1 || declarations[0] !== runner) {
      reject(`${label} job ${name} must run on ${runner}`);
    }
  }
}

function checkPermissions(text, label) {
  const blocks = text.match(/^permissions:\s*$/gm) ?? [];
  const declarations = text.match(/^permissions:\s*\n  contents: read\s*$/gm) ?? [];
  if (blocks.length !== 1 || declarations.length !== 1) reject(`${label} must grant only top-level contents: read`);
  if (/^\s{2,}[A-Za-z0-9_-]+:\s*(?:write|write-all)\s*$/m.test(text)) {
    reject(`${label} must not grant any write permission`);
  }
}

function checkTriggers(text, expected, label) {
  const start = text.indexOf('"on":\n');
  const end = text.indexOf("\npermissions:", start);
  const actual = start < 0 || end < 0 ? "" : `${text.slice(start, end).trimEnd()}\n`;
  if (actual !== `${expected.trimEnd()}\n`) {
    reject(`${label} must use only the reviewed public workflow triggers`);
  }
}

function checkActionPins(text, label) {
  for (const line of text.split(/\r?\n/).map((value) => value.trim()).filter((value) => value.startsWith("uses: "))) {
    const action = line.slice("uses: ".length).split(/\s+/, 1)[0];
    if (!PINNED_ACTIONS.has(action)) reject(`${label} uses an unapproved or unpinned action ${action}`);
  }
  const lines = text.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    if (!/^\s*uses:\s*actions\/checkout@/.test(lines[index])) continue;
    let start = index;
    while (start > 0 && !/^\s*-\s/.test(lines[start])) start -= 1;
    let end = index + 1;
    while (end < lines.length && !/^\s*-\s/.test(lines[end])) end += 1;
    const step = lines.slice(start, end).join("\n");
    requireText(step, "persist-credentials: false", label);
    if (/persist-credentials:\s*true/.test(step)) reject(`${label} checkout must not persist credentials`);
  }
  const setupLines = lines.map((line, index) => ({ line: line.trim(), index }))
    .filter(({ line }) => line.startsWith("uses: actions-rust-lang/setup-rust-toolchain@"));
  for (const { index } of setupLines) {
    let end = index + 1;
    while (end < lines.length && !/^\s*-\s/.test(lines[end])) end += 1;
    requireText(lines.slice(index, end).join("\n"), "cache: false", label);
  }
  if (text.includes("actions/cache@")) reject(`${label} must keep Actions caching disabled`);
}

function checkNoForbidden(text, label) {
  for (const needle of FORBIDDEN_WORKFLOW_TEXT) forbidText(text, needle, label);
  if (/^\s*git config --global --add safe\.directory\s+.*\*.*$/m.test(text)) {
    reject(`${label} must not use wildcard git safe.directory scope`);
  }
  if (/^\s*uses:\s*[^\s]+@(?![0-9a-f]{40}\b)/m.test(text)) {
    reject(`${label} must pin every action to a full commit SHA`);
  }
  if (/^\s*uses:\s*\.\//m.test(text)) reject(`${label} must not call a reusable workflow or local action`);
}

function checkCi(text) {
  const label = ".github/workflows/ci.yml";
  checkTriggers(text, '"on":\n  pull_request:\n  push:\n    branches:\n      - main\n  workflow_dispatch:\n', label);
  checkPermissions(text, label);
  checkJobRunners(text, CI_JOBS, label);
  checkActionPins(text, label);
  checkNoForbidden(text, label);
  requireText(text, "stage: [quick, workspace, release]", label);
  requireText(text, 'node tools/run-release-preflight.js --stage "${{ matrix.stage }}"', label);
  requireText(text, "fail-fast: false", label);
  const jobs = jobBlocks(text, label);
  requireText(text, "max-parallel: 2", label);
  requireText(jobs.get("release-preflight"), "fetch-depth: 0", `${label} release-preflight`);
  requireText(jobs.get("release-gate"), "needs: [release-preflight, webkit-runtime]", `${label} release-gate`);
  requireText(jobs.get("release-gate"), "if: always()", `${label} release-gate`);
  requireText(jobs.get("release-gate"), "PREFLIGHT_RESULT: ${{ needs.release-preflight.result }}", `${label} release-gate`);
  requireText(jobs.get("release-gate"), "WEBKIT_RESULT: ${{ needs.webkit-runtime.result }}", `${label} release-gate`);
  requireText(
    jobs.get("release-gate"),
    'run: test "$PREFLIGHT_RESULT" = success && test "$WEBKIT_RESULT" = success',
    `${label} release-gate`,
  );
  const webkit = jobs.get("webkit-runtime");
  const safeDirectory = '          git config --global --add safe.directory "$GITHUB_WORKSPACE"';
  const browserCommand = "          bash tools/run-browser-correctness.sh webkit 1.64.1";
  requireText(webkit, [
    "      - name: Execute both feature modes in headless WebKit",
    "        run: |",
    safeDirectory,
    browserCommand,
  ].join("\n"), `${label} webkit-runtime`);
  requireText(webkit, [
    "      - name: Execute documented browser embedding example",
    "        run: |",
    "          cargo build --locked --manifest-path demos/web-live/Cargo.toml --target wasm32-unknown-unknown --release --target-dir demos/web-live/target",
    "          wasm-bindgen --target web --out-dir demos/web-live/pkg --out-name rustia_example demos/web-live/target/wasm32-unknown-unknown/release/rustia_web_live_example.wasm",
    "          node tools/browser/web-live-example.cjs",
  ].join("\n"), `${label} web-live recipe`);
  for (const textFragment of [
    "name: Engine build (${{ matrix.target }})",
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
    "aarch64-apple-ios",
    "aarch64-linux-android",
    "wasm32-unknown-unknown",
    "bash tools/run-private-platform-golden.sh native default x86_64-unknown-linux-gnu",
    "bash tools/run-private-platform-golden.sh native bare x86_64-unknown-linux-gnu",
    "bash tools/run-private-platform-golden.sh native default aarch64-apple-darwin",
    "bash tools/run-private-platform-golden.sh native bare aarch64-apple-darwin",
    "bash tools/run-private-platform-golden.sh wasm bare",
    "bash tools/run-private-platform-golden.sh wasm default",
    "for mode in default bare; do",
    'bash tools/run-private-platform-golden.sh native "$mode" x86_64-pc-windows-msvc',
    "bash tools/run-browser-correctness.sh webkit 1.64.1",
  ]) requireText(text, textFragment, label);
  const windows = jobs.get("windows-runtime");
  requireText(windows, "if: github.event_name == 'workflow_dispatch'", `${label} windows-runtime`);
  const release = jobs.get("release-preflight");
  if (!release.includes("stage: [quick, workspace, release]")) reject(`${label} must run all canonical preflight stages`);
}

function checkErrorFuzz(text) {
  const label = ".github/workflows/error-fuzz.yml";
  checkTriggers(text, '"on":\n  pull_request:\n    types: [opened, synchronize, reopened]\n  workflow_dispatch:\n', label);
  checkPermissions(text, label);
  checkJobRunners(text, FUZZ_JOBS, label);
  checkActionPins(text, label);
  checkNoForbidden(text, label);
  requireText(text, "name: Verify release diagnostic parity against pinned Luau", label);
  requireText(text, "cargo test --locked --manifest-path fuzz/Cargo.toml generated_differential_smoke -- --ignored", label);
  requireText(text, "timeout-minutes: 20", `${label} differential-smoke`);
  const jobs = jobBlocks(text, label);
  const campaign = jobs.get("campaign");
  requireText(campaign, "if: github.event_name == 'workflow_dispatch'", `${label} campaign`);
  requireText(campaign, "timeout-minutes: 170", `${label} campaign`);
  requireText(campaign, "-max_total_time=1800", `${label} campaign`);
  requireText(campaign, "-timeout=15", `${label} campaign`);
  requireText(campaign, "-rss_limit_mb=2048", `${label} campaign`);
  const durations = [...campaign.matchAll(/-max_total_time=(\d+)/g)].map((match) => Number(match[1]));
  if (durations.length !== 2 || !durations.includes(1800) || !durations.includes(7200) ||
      durations.some((duration) => duration > 7200)) {
    reject(`${label} fuzz campaigns must have exactly two bounded durations, each at most 7200 seconds`);
  }
  if (campaign.includes("github.event.pull_request")) reject(`${label} long campaigns must not run from pull requests`);
  requireText(campaign, "if: github.event_name == 'workflow_dispatch'", `${label} campaign`);
  requireText(campaign, "if: always()", `${label} retained fuzz evidence`);
  requireText(campaign, "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02", `${label} retained fuzz evidence`);
}

function validatePublicProfile({ profile, files, workflows }) {
  if (profile?.schema !== "rustia.public-source-profile.v1" || profile?.state !== "private-rehearsal" ||
      !FULL_COMMIT.test(profile.historical_head ?? "") || profile.historical_host_label !== "ci-bench" ||
      !BENCHMARK_BRIDGE_STATES.includes(profile.benchmark_bridge) || profile.package_publication !== false ||
      profile.visibility_authorized !== false) {
    reject("design/public-source-profile.json must be the valid private-rehearsal public-source profile");
  }
  const sortedFiles = files.slice().sort();
  if (JSON.stringify(sortedFiles) !== JSON.stringify(EXPECTED_FILES)) {
    reject(`active workflows must be exactly ${EXPECTED_FILES.join(" and ")}; found ${sortedFiles.join(", ")}`);
  }
  for (const file of EXPECTED_FILES) {
    const text = workflows[file];
    if (typeof text !== "string") reject(`missing active workflow ${file}`);
    checkNoForbidden(text, `.github/workflows/${file}`);
  }
  checkCi(workflows["ci.yml"]);
  checkErrorFuzz(workflows["error-fuzz.yml"]);
  return { files: sortedFiles.length, profile: profile.state };
}

function readCheckout(root) {
  const profileFile = path.join(root, "design", "public-source-profile.json");
  let profile;
  try {
    profile = JSON.parse(fs.readFileSync(profileFile, "utf8"));
  } catch (error) {
    reject(`could not read design/public-source-profile.json: ${error.message}`);
  }
  const workflowDir = path.join(root, ".github", "workflows");
  let files;
  try {
    files = fs.readdirSync(workflowDir).filter((name) => name.endsWith(".yml") || name.endsWith(".yaml"));
  } catch (error) {
    reject(`could not read .github/workflows: ${error.message}`);
  }
  const workflows = Object.fromEntries(files.map((file) => [file, fs.readFileSync(path.join(workflowDir, file), "utf8")]));
  return validatePublicProfile({ profile, files, workflows });
}

function selfTest(root) {
  const assert = require("node:assert/strict");
  const fs = require("node:fs");
  const os = require("node:os");
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-public-workflow-profile-"));
  const profile = {
    schema: "rustia.public-source-profile.v1", state: "private-rehearsal", historical_head: "a".repeat(40),
    historical_host_label: "ci-bench", benchmark_bridge: "pending-private-trusted-pipeline",
    package_publication: false, visibility_authorized: false,
  };
  const workflows = {
    "ci.yml": fs.readFileSync(path.join(root, ".github/workflows/ci.yml"), "utf8"),
    "error-fuzz.yml": fs.readFileSync(path.join(root, ".github/workflows/error-fuzz.yml"), "utf8"),
  };
  const files = EXPECTED_FILES.slice();
  const run = (changedProfile = profile, changedFiles = files, changedWorkflows = workflows) =>
    validatePublicProfile({ profile: changedProfile, files: changedFiles, workflows: changedWorkflows });
  const expectFail = (label, fn, pattern) => {
    assert.throws(fn, pattern, label);
  };
  try {
    assert.deepEqual(run(), { files: 2, profile: "private-rehearsal" });
    assert.deepEqual(run({ ...profile, benchmark_bridge: "private-maintainer-cohorts-v1" }), {
      files: 2, profile: "private-rehearsal",
    });
    expectFail("extra workflow", () => run(profile, [...files, "main-bench.yml"], workflows), /active workflows must be exactly/);
    expectFail("missing workflow", () => run(profile, ["ci.yml"], workflows), /active workflows must be exactly/);
    expectFail("bad marker", () => run({ ...profile, state: "public" }), /valid private-rehearsal/);
    for (const [label, key, value] of [
      ["bad historical label", "historical_host_label", "runner.internal"],
      ["unknown private bridge state", "benchmark_bridge", "enabled"],
      ["arbitrary bridge state", "benchmark_bridge", "available"],
      ["package publication", "package_publication", true],
      ["visibility authorization", "visibility_authorized", true],
    ]) {
      expectFail(label, () => run({ ...profile, [key]: value }), /valid private-rehearsal/);
    }
    for (const [label, file, transform, expected] of [
      ["self-hosted runner", "ci.yml", (s) => s.replace("runs-on: ubuntu-24.04", "runs-on: [self-hosted, Linux]"), /self-hosted/],
      ["untrusted action ref", "ci.yml", (s) => s.replace(/actions\/checkout@([0-9a-f]{40})/, "actions/checkout@v7"), /pin every action/],
      ["write permission", "ci.yml", (s) => s.replace("contents: read", "contents: write"), /contents: write/],
      ["secret use", "error-fuzz.yml", (s) => `${s}\n# secrets.X\n`, /secrets/],
      ["private workflow trigger", "error-fuzz.yml", (s) => s.replace("workflow_dispatch:", "schedule:\n    - cron: '0 0 * * *'\n  workflow_dispatch:"), /schedule/],
      ["benchmark command", "ci.yml", (s) => `${s}\n# node tools/run-dedicated-benchmark.js\n`, /run-dedicated-benchmark/],
      ["unbounded fuzz", "error-fuzz.yml", (s) => s.replace("-max_total_time=7200", "-max_total_time=9000"), /bounded durations/],
      ["campaign on PR label", "error-fuzz.yml", (s) => s.replace("if: github.event_name == 'workflow_dispatch'", "if: github.event_name == 'workflow_dispatch' || contains(github.event.pull_request.labels.*.name, 'run-error-fuzz')"), /long campaigns must not run from pull requests/],
      ["credentials persisted", "ci.yml", (s) => s.replace("persist-credentials: false", "persist-credentials: true"), /persist-credentials: false/],
      ["enabled build cache", "ci.yml", (s) => s.replace("cache: false", "cache: true"), /cache: false/],
      ["missing preflight aggregate", "ci.yml", (s) => s.replace("run: test \"$PREFLIGHT_RESULT\" = success && test \"$WEBKIT_RESULT\" = success", "run: true"), /run: test.*PREFLIGHT_RESULT/],
      ["missing WebKit safe-directory setup", "ci.yml", (s) => s.replace('          git config --global --add safe.directory "$GITHUB_WORKSPACE"\n', ""), /safe\.directory/],
      ["missing browser recipe build", "ci.yml", (s) => s.replace("cargo build --locked --manifest-path demos/web-live/Cargo.toml", "true"), /web-live recipe/],
      ["missing browser recipe execution", "ci.yml", (s) => s.replace("node tools/browser/web-live-example.cjs", "true"), /web-live recipe/],
      [
        "wildcard WebKit safe-directory setup",
        "ci.yml",
        (s) => s.replace('git config --global --add safe.directory "$GITHUB_WORKSPACE"', "git config --global --add safe.directory '*'"),
        /wildcard git safe\.directory/,
      ],
    ]) {
      const mutated = { ...workflows, [file]: transform(workflows[file]) };
      expectFail(label, () => run(profile, files, mutated), expected);
    }
    return { cases: 22, passed: true };
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

module.exports = { validatePublicProfile, checkCi, checkErrorFuzz, readCheckout, selfTest };
