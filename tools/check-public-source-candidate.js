#!/usr/bin/env node
"use strict";

// Validate or render workflow candidates without changing any repository settings.
const fs = require("node:fs");
const path = require("node:path");
const assert = require("node:assert/strict");
const privateProfile = require("./check-public-workflow-profile.js");
const PRIVATE_PR = "github.event.repository.private == true && (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository)";
const PRIVATE_MANUAL = "github.event.repository.private == true && github.event_name == 'workflow_dispatch'";
const PUBLIC = "github.event.repository.private == false";
const PUBLIC_MANUAL = `${PUBLIC} && github.event_name == 'workflow_dispatch'`;
const FILES = ["ci.yml", "error-fuzz.yml"];
const RUNNERS = { "general-linux": "ubuntu-24.04", "general-macos": "macos-15", "general-windows": "windows-2025" };
const REHEARSALS = { "ci.yml": "public-ci-rehearsal.yml", "error-fuzz.yml": "public-error-fuzz-rehearsal.yml" };

function jobIds(text) {
  const start = text.indexOf("\njobs:\n");
  assert(start >= 0, "workflow jobs are missing");
  return [...text.slice(start).matchAll(/^  ([a-z][a-z0-9-]*):\n/gm)].map((match) => match[1]);
}

function proposedWorkflow(name, original) {
  let text = original;
  if (name === "ci.yml") {
    privateProfile.checkCi(text);
    const safari = text.indexOf("\n  safari-runtime:\n");
    assert(safari > 0, "private Safari job must exist before explicit public deferral");
    text = text.replace(/\n  safari-runtime:\n[\s\S]*?(?=\n  [a-z][a-z0-9-]*:\n|$)/, "");
    const macSetup = /      - name: Install pinned Rust toolchain\n[\s\S]*?(?=\n\n      - name: Verify native)/;
    assert(macSetup.test(text), "missing native macOS toolchain installation");
    text = text.replace(macSetup, privateProfile.HOSTED_MAC_SETUP);
    const triggers = text.indexOf('"on":\n');
    const permissions = text.indexOf("\npermissions:\n", triggers);
    assert(triggers > 0 && permissions > triggers, "private trigger boundary missing");
    text = `name: CI\n\n# Run public correctness checks on standard GitHub-hosted runners.\n${text.slice(triggers, permissions).replace(/  workflow_dispatch:[\s\S]*/, "  workflow_dispatch:\n")}${text.slice(permissions)}`;
  } else {
    assert.equal(name, "error-fuzz.yml");
    privateProfile.checkErrorFuzz(text);
  }
  text = text.replace('"on":\n', '"on":\n  public:\n');
  const group = name === "ci.yml" ? "github.ref" : "github.event.pull_request.number || github.ref";
  text = text.replace(`\${{ ${group} }}`, `\${{ github.event_name == 'public' && 'launch' || ${group} }}`);
  text = text.replaceAll(PRIVATE_PR, PUBLIC).replaceAll(PRIVATE_MANUAL, PUBLIC_MANUAL);
  text = text.replace(/runs-on:\n      group: (general-linux|general-macos|general-windows)\n      labels: \[[^\n]+\]/g,
    (_whole, group) => `runs-on: ${RUNNERS[group]}`);
  // Cold standard VMs have smaller resources than the private warm pool. Bound
  // jobs without deleting checks or relying on a cache to finish correctly.
  text = text.replace(/timeout-minutes: (\d+)/g, (_whole, value) => `timeout-minutes: ${Number(value) === 60 ? 180 : Math.max(Number(value), 60)}`);
  if (name === "ci.yml") text = text.replace(/(  release-gate:[\s\S]*?    runs-on: ubuntu-24\.04\n)(    steps:)/, "$1    timeout-minutes: 5\n$2");
  return text;
}

function checkCandidate(name, actual, original = null) {
  if (original !== null) {
    assert.equal(actual, proposedWorkflow(name, original), `${name}: dormant proposal differs from the complete reviewed transformation`);
    assert.deepEqual(jobIds(actual), jobIds(original).filter((id) => id !== "safari-runtime"),
      `${name}: public jobs must preserve every non-Safari private job`);
  }
  const launchGroup = name === "ci.yml"
    ? "ci-${{ github.workflow }}-${{ github.event_name == 'public' && 'launch' || github.ref }}"
    : "error-fuzz-${{ github.event_name == 'public' && 'launch' || github.event.pull_request.number || github.ref }}";
  assert(actual.includes(`\nconcurrency:\n  group: ${launchGroup}\n  cancel-in-progress: true\n`),
    "the visibility run must have its own concurrency group");
  const runnerLines = [...actual.matchAll(/^    runs-on: (.+)$/gm)].map((m) => m[1]);
  const expected = name === "ci.yml" ? 8 : 2;
  assert.equal(runnerLines.length, expected, `${name}: unexpected public job inventory`);
  assert(runnerLines.every((runner) => ["ubuntu-24.04", "macos-15", "windows-2025"].includes(runner)), "only standard fixed hosted labels are permitted");
  for (const forbidden of ["self-hosted", "group: general-", "group: bench", "secrets.", "secrets:",
    "pull_request_target", "workflow_run", "repository_dispatch", "contents: write", "id-token:",
    "actions/cache@", "cache: true", "persist-credentials: true", "continue-on-error: true"]) {
    assert(!actual.includes(forbidden), `${name}: forbidden public boundary: ${forbidden}`);
  }
  assert(!actual.includes("github.event.pull_request.head.repo.full_name"), "public fork PRs must execute correctness jobs");
  assert(!actual.includes("safari-runtime:"), "private desktop Safari remains explicitly deferred");
  if (name === "ci.yml") privateProfile.checkCi(actual, "public");
  else privateProfile.checkErrorFuzz(actual, "public");
  return { jobs: expected };
}

function rehearsalWorkflow(name, proposed) {
  const trigger = proposed.indexOf('"on":\n');
  const end = proposed.indexOf("\npermissions:\n", trigger);
  assert(trigger >= 0 && end > trigger, "public trigger boundary missing");
  let text = `name: ${name === "ci.yml" ? "Public CI rehearsal" : "Public error smoke rehearsal"}\n\n${privateProfile.REHEARSAL_TRIGGERS}${proposed.slice(end)}`;
  // Replace longer guards first so all non-required/manual work stays opt-in.
  text = text.replaceAll(PUBLIC_MANUAL, `${privateProfile.REHEARSAL} && inputs.extended`)
    .replaceAll(PUBLIC, privateProfile.REHEARSAL);
  text = text.replace(/^    name: (.+)$/gm, "    name: Public rehearsal / $1");
  if (name === "error-fuzz.yml") text = text.replace("  differential-smoke:\n", "  differential-smoke:\n    name: Public rehearsal / differential-smoke\n");
  if (name === "error-fuzz.yml") text = text.replace("group: error-fuzz-", "group: public-error-fuzz-rehearsal-");
  return text;
}

function readCheckout(root, write = false) {
  const directory = path.join(root, ".github/public-ci");
  const profile = JSON.parse(fs.readFileSync(path.join(root, "design/public-source-profile.json"), "utf8"));
  const hosted = profile.correctness_ci === "public-hosted";
  if (write) fs.mkdirSync(directory, { recursive: true });
  for (const name of FILES) {
    const original = fs.readFileSync(path.join(root, ".github/workflows", name), "utf8");
    const file = path.join(directory, name);
    if (write) fs.writeFileSync(file, hosted ? original : proposedWorkflow(name, original));
    if (hosted) {
      assert.equal(original, fs.readFileSync(file, "utf8"), "activated workflows must match the reviewed hosted proposals");
      checkCandidate(name, original);
      continue;
    }
    checkCandidate(name, fs.readFileSync(file, "utf8"), original);
    const rehearsal = path.join(root, ".github/workflows", REHEARSALS[name]);
    const expected = rehearsalWorkflow(name, fs.readFileSync(file, "utf8"));
    if (write) fs.writeFileSync(rehearsal, expected);
    assert.equal(fs.readFileSync(rehearsal, "utf8"), expected, "rehearsal must execute the exact public proposal with only bounded private dispatch guards");
    if (name === "ci.yml") privateProfile.checkCi(expected, "rehearsal");
    else privateProfile.checkErrorFuzz(expected, "rehearsal");
  }
  assert.deepEqual(fs.readdirSync(directory).filter((name) => /\.ya?ml$/.test(name)).sort(), FILES.slice().sort(), "dormant workflow inventory must be exact");
  return { workflows: 2, jobs: 10, active: hosted };
}

function selfTest(root) {
  const profile = JSON.parse(fs.readFileSync(path.join(root, "design/public-source-profile.json"), "utf8"));
  const hosted = profile.correctness_ci === "public-hosted";
  const source = fs.readFileSync(path.join(root, ".github/workflows/ci.yml"), "utf8");
  const original = hosted ? null : source;
  const canonical = hosted ? source : proposedWorkflow("ci.yml", source);
  let cases = 0;
  for (const mutate of [
    (s) => s.replace("runs-on: ubuntu-24.04", "runs-on: [self-hosted, Linux]"),
    (s) => s.replace("runs-on: macos-15", "runs-on: macos-latest"),
    (s) => s.replace("runs-on: windows-2025", "runs-on: ${{ inputs.runner }}"),
    (s) => s.replace("contents: read", "contents: write"),
    (s) => s.replace("persist-credentials: false", "persist-credentials: true"),
    (s) => s.replace("cache: false", "cache: true"),
    (s) => s.replace("--stage \"${{ matrix.stage }}\"", "--stage quick"),
    (s) => s.replace("      - name: Run canonical release preflight\n", "      - name: Run canonical release preflight\n        if: false\n"),
    (s) => s.replace("      - name: Run canonical release preflight\n", "      - name: Run canonical release preflight\n        continue-on-error: true\n"),
    (s) => s.replace("needs: [release-preflight, webkit-runtime]", "needs: [release-preflight]"),
    (s) => s.replace("--test gc_accounting_paths -- --nocapture", "--test omitted -- --nocapture"),
    (s) => s.replace("node tools/browser/web-live-example.cjs", "true"),
    (s) => s.replace("sha512sum --check --status", "true"),
    (s) => s.replace(PUBLIC, PRIVATE_PR),
    (s) => s.replace("  public:\n", ""),
    (s) => s.replace("github.event_name == 'public' && 'launch' || ", ""),
    (s) => s.replace("pull_request:", "pull_request_target:"),
    (s) => `${s}\n# secrets.DEPLOY_KEY\n`,
  ]) {
    assert.throws(() => checkCandidate("ci.yml", mutate(canonical), original));
    assert.throws(() => checkCandidate("ci.yml", mutate(canonical)));
    cases += 1;
  }
  for (const action of ["actions/checkout@main", "other/action@" + "1".repeat(40)]) {
    const changed = canonical.replace(/- uses: actions\/checkout@[0-9a-f]{40}/, `- uses: ${action}`);
    assert.notEqual(changed, canonical);
    assert.throws(() => checkCandidate("ci.yml", changed));
    cases += 1;
  }
  const fuzzSource = fs.readFileSync(path.join(root, ".github/workflows/error-fuzz.yml"), "utf8");
  const fuzz = hosted ? fuzzSource : proposedWorkflow("error-fuzz.yml", fuzzSource);
  assert.throws(() => checkCandidate("error-fuzz.yml", fuzz.replace("run: node tools/audit-release-errors.mjs", "run: true")), /diagnostic parity/);
  cases += 1;
  if (!hosted) {
    const rehearsal = rehearsalWorkflow("ci.yml", canonical);
    for (const [from, to] of [
      ["github.actor_id == '31338'", "true"],
      ["github.ref == 'refs/heads/main'", "true"],
      ["github.sha == inputs.expected_sha", "true"],
      [" && inputs.confirm_hosted_rehearsal", ""],
      [" && inputs.extended", ""],
      ["type: boolean\n        default: false", "type: boolean\n        default: true"],
      ["runs-on: ubuntu-24.04", "runs-on: [self-hosted, Linux]"],
    ]) {
      assert(rehearsal.includes(from));
      assert.throws(() => privateProfile.checkCi(rehearsal.replace(from, to), "rehearsal"));
      cases += 1;
    }
    const fuzzOriginal = fs.readFileSync(path.join(root, ".github/workflows/error-fuzz.yml"), "utf8");
    const fuzzRehearsal = rehearsalWorkflow("error-fuzz.yml", proposedWorkflow("error-fuzz.yml", fuzzOriginal));
    assert(fuzzRehearsal.includes("group: public-error-fuzz-rehearsal-"));
    assert(!fuzzRehearsal.includes("group: error-fuzz-"));
  }
  assert.equal((canonical.match(/run: cargo check --locked -p rustia --lib --target/g) || []).length, 1);
  assert(!canonical.includes("safari_version"));
  readCheckout(root);
  // Exercise the actual selected-profile and filesystem paths in a disposable
  // activation fixture. No live workflow, setting or marker is changed.
  const temporary = fs.mkdtempSync(path.join(require("node:os").tmpdir(), "rustia-hosted-activation-"));
  try {
    const workflows = {};
    fs.mkdirSync(path.join(temporary, ".github/workflows"), { recursive: true });
    fs.mkdirSync(path.join(temporary, ".github/public-ci"), { recursive: true });
    fs.mkdirSync(path.join(temporary, "design"));
    for (const name of FILES) {
      workflows[name] = fs.readFileSync(path.join(root, ".github/public-ci", name), "utf8");
      fs.writeFileSync(path.join(temporary, ".github/workflows", name), workflows[name]);
      fs.writeFileSync(path.join(temporary, ".github/public-ci", name), workflows[name]);
    }
    fs.writeFileSync(path.join(temporary, "design/public-source-profile.json"), JSON.stringify({ ...profile, correctness_ci: "public-hosted" }));
    assert.equal(privateProfile.readCheckout(temporary).files, 2);
    assert.equal(readCheckout(temporary).active, true);
    assert.throws(() => privateProfile.validatePublicProfile({ profile: { ...profile, correctness_ci: "private-self-hosted" }, files: FILES, workflows }));
    // The actual entrypoint must also validate an activated fixture, including
    // its self-test path. This catches private-input assumptions after copying.
    if (!hosted) {
      fs.mkdirSync(path.join(temporary, "tools"));
      for (const name of ["check-workflows.js", "check-public-workflow-profile.js", "check-public-source-candidate.js"]) {
        fs.copyFileSync(path.join(root, "tools", name), path.join(temporary, "tools", name));
      }
      for (const args of [[], ["--self-test"]]) {
        const child = require("node:child_process").spawnSync(process.execPath,
          [path.join(temporary, "tools/check-workflows.js"), ...args], { encoding: "utf8" });
        assert.equal(child.status, 0, child.stderr);
      }
    }
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
  return { cases };
}

module.exports = { proposedWorkflow, rehearsalWorkflow, checkCandidate, readCheckout, selfTest };
if (require.main === module) {
  const root = path.resolve(__dirname, "..");
  const result = process.argv.includes("--self-test") ? selfTest(root) : readCheckout(root, process.argv.includes("--write"));
  console.log(`public-source workflow candidate ok: ${JSON.stringify(result)}`);
}
