#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const CORE_FILES = Object.freeze(["ci.yml", "error-fuzz.yml"]);
const QUICK_FILE = "quick.yml";
const BULK_FILE = "bulk.yml";
const EXPECTED_FILES = Object.freeze([...CORE_FILES, QUICK_FILE, BULK_FILE]);
const QUICK_TRIGGERS = '"on":\n  workflow_dispatch:\n    inputs:\n      repository_id:\n        description: Numeric target repository identity bound by the controller\n        type: string\n        required: true\n      admission_id:\n        description: Canonical controller admission identity\n        type: string\n        required: true\n      pull_request:\n        description: Numeric source pull-request identity\n        type: string\n        required: true\n      head_repository_id:\n        description: Numeric source repository identity\n        type: string\n        required: true\n      head_repository_full_name:\n        description: Source owner and repository used for the exact checkout\n        type: string\n        required: true\n      checkout_sha:\n        description: Exact source commit to check out\n        type: string\n        required: true\n      quick_intent_digest:\n        description: SHA-256 digest of the canonical Quick intent\n        type: string\n        required: true\n';
const BULK_TRIGGERS = '"on":\n  workflow_dispatch:\n    inputs:\n      repository_id:\n        description: Numeric repository identity bound by the controller\n        type: string\n        required: true\n      attempt_id:\n        description: Numeric controller attempt identity\n        type: string\n        required: true\n      checkout_sha:\n        description: Exact candidate commit to check out\n        type: string\n        required: true\n      manifest_digest:\n        description: SHA-256 digest of the canonical attempt manifest\n        type: string\n        required: true\n';
const HOSTED_MAC_SETUP = [
  "      - name: Install pinned Rust toolchain",
  "        uses: actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659 # v1",
  "        with:",
  "          toolchain: 1.95.0",
  "          target: aarch64-apple-darwin",
  "          cache: false",
].join("\n");
const PRIVATE_PR = "github.event.repository.private == true && (github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository)";
const PRIVATE_MANUAL = "github.event.repository.private == true && github.event_name == 'workflow_dispatch'";
const PRIVATE_SAFARI = `${PRIVATE_MANUAL} && inputs.safari`;
const PUBLIC_PR = "github.event.repository.private == false";
const PUBLIC_MANUAL = `${PUBLIC_PR} && github.event_name == 'workflow_dispatch'`;
const REHEARSAL = "github.event.repository.private == true && github.repository_id == '1437649541' && github.event_name == 'workflow_dispatch' && github.actor_id == '31338' && github.ref == 'refs/heads/main' && github.sha == inputs.expected_sha && inputs.confirm_hosted_rehearsal";
const REHEARSAL_TRIGGERS = '"on":\n  workflow_dispatch:\n    inputs:\n      expected_sha:\n        description: Exact reviewed main commit to rehearse\n        type: string\n        required: true\n      confirm_hosted_rehearsal:\n        description: Run the bounded standard-hosted readiness rehearsal\n        type: boolean\n        default: false\n      extended:\n        description: Also run non-required manual platform checks or long fuzz campaigns\n        type: boolean\n        default: false\n';
const LINUX_RUNNER = "runs-on:\n      group: general-linux\n      labels: [self-hosted, Linux, X64, general-linux]";
const CI_JOBS = Object.freeze({
  "release-preflight": LINUX_RUNNER,
  "release-gate": LINUX_RUNNER,
  "safari-runtime": "runs-on:\n      group: general-macos\n      labels: [self-hosted, macOS, ARM64, general-macos]",
  "macos-arm64-runtime": "runs-on:\n      group: general-macos\n      labels: [self-hosted, macOS, ARM64, general-macos]",
  "portable-engine-build": LINUX_RUNNER,
  "wasm-node-runtime": LINUX_RUNNER,
  "windows-runtime": "runs-on:\n      group: general-windows\n      labels: [self-hosted, Windows, X64, general-windows]",
  "firefox-runtime": "runs-on:\n      group: general-linux\n      labels: [self-hosted, Linux, X64, general-linux, docker]",
  "webkit-runtime": "runs-on:\n      group: general-linux\n      labels: [self-hosted, Linux, X64, general-linux, docker]",
});
const FUZZ_JOBS = Object.freeze({
  "differential-smoke": LINUX_RUNNER,
  campaign: LINUX_RUNNER,
});
const PINNED_ACTIONS = new Set([
  "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1",
  "actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659",
  "actions/setup-node@820762786026740c76f36085b0efc47a31fe5020",
  "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
]);
const FORBIDDEN_WORKFLOW_TEXT = Object.freeze([
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

function checkJobRunners(text, expected, label, mode = "private") {
  const blocks = jobBlocks(text, label);
  const names = [...blocks.keys()].sort();
  const expectedNames = Object.keys(expected).sort();
  if (JSON.stringify(names) !== JSON.stringify(expectedNames)) {
    reject(`${label} jobs must be exactly ${expectedNames.join(", ")}; found ${names.join(", ")}`);
  }
  for (const [name, runner] of Object.entries(expected)) {
    const declarations = [...blocks.get(name).matchAll(/^    runs-on:[^\n]*(?:\n      [^\n]*)*/gm)].map((match) => match[0].trimStart());
    if (declarations.length !== 1 || declarations[0] !== runner) {
      reject(`${label} job ${name} must run on ${runner}`);
    }
    const manual = ["campaign", "windows-runtime", "firefox-runtime"].includes(name);
    const base = mode === "rehearsal" ? REHEARSAL : mode === "public" ? PUBLIC_PR : PRIVATE_PR;
    const manualGuard = mode === "rehearsal" ? `${REHEARSAL} && inputs.extended` : mode === "public" ? PUBLIC_MANUAL : PRIVATE_MANUAL;
    const guard = name === "safari-runtime" ? PRIVATE_SAFARI : manual ? manualGuard
      : name === "release-gate" ? `always() && ${base}` : base;
    const conditions = [...blocks.get(name).matchAll(/^    if: (.*)$/gm)].map((match) => match[1]);
    if (conditions.length !== 1 || conditions[0] !== guard) {
      reject(`${label} job ${name} must retain its private trusted-event guard`);
    }
  }
}

function checkPermissions(text, label) {
  const blocks = text.match(/^permissions:\s*$/gm) ?? [];
  const declarations = text.match(/^permissions:\s*\n  contents: read\s*$/gm) ?? [];
  if (blocks.length !== 1 || declarations.length !== 1) reject(`${label} must grant only top-level contents: read`);
  if (/^\s{2,}(?:permissions|"permissions"|'permissions')\s*:/m.test(text)) {
    reject(`${label} must not override permissions inside a job`);
  }
  if (/^\s{2,}[A-Za-z0-9_-]+:\s*(?:write|write-all)\s*$/m.test(text)) {
    reject(`${label} must not grant any write permission`);
  }
  if (/github\s*(?:\.\s*token|\[\s*['"]token['"]\s*\])|GITHUB_TOKEN|tojson\s*\(\s*github\s*\)/i.test(text)) {
    reject(`${label} must not expose the GitHub token to workflow steps`);
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
  const normalized = (line) => line.trim().replace(/^-\s+/, "");
  for (const line of text.split(/\r?\n/).map(normalized).filter((value) => value.startsWith("uses: "))) {
    const action = line.slice("uses: ".length).split(/\s+/, 1)[0];
    if (!PINNED_ACTIONS.has(action)) reject(`${label} uses an unapproved or unpinned action ${action}`);
  }
  const lines = text.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    if (!/^\s*(?:-\s+)?uses:\s*actions\/checkout@/.test(lines[index])) continue;
    let start = index;
    while (start > 0 && !/^\s*-\s/.test(lines[start])) start -= 1;
    let end = index + 1;
    while (end < lines.length && !/^\s*-\s/.test(lines[end])) end += 1;
    const step = lines.slice(start, end).join("\n");
    requireText(step, "persist-credentials: false", label);
    if (/persist-credentials:\s*true/.test(step)) reject(`${label} checkout must not persist credentials`);
  }
  const setupLines = lines.map((line, index) => ({ line: normalized(line), index }))
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
  if (/^\s*continue-on-error:/m.test(text)) reject(`${label} must not suppress failed checks`);
  if (/^\s*git config --global --add safe\.directory\s+.*\*.*$/m.test(text)) {
    reject(`${label} must not use wildcard git safe.directory scope`);
  }
  if (/^\s*(?:-\s+)?uses:\s*[^\s]+@(?![0-9a-f]{40}\b)/m.test(text)) {
    reject(`${label} must pin every action to a full commit SHA`);
  }
  if (/^\s*(?:-\s+)?uses:\s*\.\//m.test(text)) reject(`${label} must not call a reusable workflow or local action`);
}

function exactStepBlocks(block, expected, label) {
  const lines = block.split(/\r?\n/);
  const start = lines.findIndex((line) => line === "    steps:");
  if (start < 0) reject(`${label} must declare steps`);
  const starts = [];
  for (let index = start + 1; index < lines.length; index += 1) {
    if (!/^      - /.test(lines[index])) continue;
    const match = lines[index].match(/^      - name: (.+)$/);
    if (!match) reject(`${label} must name every step`);
    starts.push({ name: match[1], index });
  }
  const names = starts.map(({ name }) => name);
  if (JSON.stringify(names) !== JSON.stringify(expected)) {
    reject(`${label} steps must be exactly ${expected.join(", ")}; found ${names.join(", ")}`);
  }
  return new Map(starts.map(({ name, index }, at) => [
    name,
    lines.slice(index, starts[at + 1]?.index ?? lines.length).join("\n").trimEnd(),
  ]));
}

function requireExactStep(steps, name, expected, label) {
  if (steps.get(name) !== expected.trimEnd()) {
    reject(`${label} must match the reviewed ${name} step`);
  }
}

function requireExactJobHeader(block, expected, label) {
  const lines = block.split(/\r?\n/);
  const steps = lines.findIndex((line) => line === "    steps:");
  if (steps < 0 || lines.slice(0, steps + 1).join("\n") !== expected.trimEnd()) {
    reject(`${label} must match the reviewed job header`);
  }
}

function checkBulk(text) {
  const label = ".github/workflows/bulk.yml";
  const expectedPrefix = [
    "name: Rustia Bulk trust fixture",
    "run-name: rustia-ci:${{ inputs.repository_id }}:${{ inputs.attempt_id }}:${{ inputs.checkout_sha }}:${{ inputs.manifest_digest }}",
    "",
    BULK_TRIGGERS.trimEnd(),
    "",
    "permissions:",
    "  contents: read",
    "",
    "jobs:",
    "",
  ].join("\n");
  if (!text.startsWith(expectedPrefix)) {
    reject(`${label} must retain the exact reviewed top-level configuration before jobs`);
  }
  checkTriggers(text, BULK_TRIGGERS, label);
  checkPermissions(text, label);
  checkActionPins(text, label);
  checkNoForbidden(text, label);
  const permissionsStart = text.indexOf("permissions:\n");
  const jobsStart = text.indexOf("jobs:\n", permissionsStart);
  if (permissionsStart < 0 || jobsStart < 0
      || text.slice(permissionsStart, jobsStart) !== "permissions:\n  contents: read\n\n") {
    reject(`${label} must put only top-level contents: read before jobs`);
  }
  requireText(text, "name: Rustia Bulk trust fixture\n", label);
  requireText(text,
    "run-name: rustia-ci:${{ inputs.repository_id }}:${{ inputs.attempt_id }}:${{ inputs.checkout_sha }}:${{ inputs.manifest_digest }}\n",
    label);

  const jobs = jobBlocks(text, label);
  const names = [...jobs.keys()].sort();
  if (JSON.stringify(names) !== JSON.stringify(["integrity", "test"])) {
    reject(`${label} jobs must be exactly integrity and test; found ${names.join(", ")}`);
  }
  for (const [name, block] of jobs) {
    const runners = [...block.matchAll(/^    runs-on: (.+)$/gm)].map((match) => match[1]);
    if (JSON.stringify(runners) !== JSON.stringify(["ubuntu-24.04"])) {
      reject(`${label} job ${name} must run once on ubuntu-24.04`);
    }
    requireText(block, `    name: ${name}\n`, `${label} ${name}`);
    requireText(block, "ref: ${{ inputs.checkout_sha }}", `${label} ${name}`);
    requireText(block, "fetch-depth: 1", `${label} ${name}`);
    requireText(block, "persist-credentials: false", `${label} ${name}`);
  }

  const integrity = jobs.get("integrity");
  requireExactJobHeader(integrity, `  integrity:
    name: integrity
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:`, `${label} integrity`);
  const integritySteps = exactStepBlocks(integrity, [
    "Validate dispatch inputs",
    "Checkout candidate",
    "Verify exact checkout",
  ], `${label} integrity`);
  requireExactStep(integritySteps, "Validate dispatch inputs", `      - name: Validate dispatch inputs
        shell: bash
        env:
          REPOSITORY_ID: \${{ inputs.repository_id }}
          ATTEMPT_ID: \${{ inputs.attempt_id }}
          CHECKOUT_SHA: \${{ inputs.checkout_sha }}
          MANIFEST_DIGEST: \${{ inputs.manifest_digest }}
        run: |
          set -euo pipefail
          [[ "$REPOSITORY_ID" =~ ^[1-9][0-9]*$ ]]
          [[ "$ATTEMPT_ID" =~ ^[1-9][0-9]*$ ]]
          [[ "$CHECKOUT_SHA" =~ ^[0-9a-f]{40}$ ]]
          [[ "$MANIFEST_DIGEST" =~ ^[0-9a-f]{64}$ ]]`, `${label} integrity`);
  requireExactStep(integritySteps, "Checkout candidate", `      - name: Checkout candidate
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          ref: \${{ inputs.checkout_sha }}
          fetch-depth: 1
          persist-credentials: false`, `${label} integrity`);
  requireExactStep(integritySteps, "Verify exact checkout", `      - name: Verify exact checkout
        shell: bash
        env:
          EXPECTED_CHECKOUT_SHA: \${{ inputs.checkout_sha }}
        run: |
          set -euo pipefail
          test "$(git rev-parse HEAD)" = "$EXPECTED_CHECKOUT_SHA"`, `${label} integrity`);
  if ((text.match(/      - name: Verify exact checkout\n/g) ?? []).length !== 1) {
    reject(`${label} must contain exactly one Verify exact checkout step`);
  }

  const test = jobs.get("test");
  requireExactJobHeader(test, `  test:
    name: test
    needs: integrity
    if: github.ref == 'refs/heads/main' && needs.integrity.result == 'success'
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:`, `${label} test`);
  const testSteps = exactStepBlocks(test, [
    "Checkout protected policy",
    "Checkout candidate",
    "Verify test checkout",
    "Check the protected workflow policy",
  ], `${label} test`);
  requireExactStep(testSteps, "Checkout protected policy", `      - name: Checkout protected policy
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          ref: \${{ github.sha }}
          path: trusted
          fetch-depth: 1
          persist-credentials: false`, `${label} test`);
  requireExactStep(testSteps, "Checkout candidate", `      - name: Checkout candidate
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          ref: \${{ inputs.checkout_sha }}
          path: candidate
          fetch-depth: 1
          persist-credentials: false`, `${label} test`);
  requireExactStep(testSteps, "Verify test checkout", `      - name: Verify test checkout
        shell: bash
        working-directory: candidate
        env:
          EXPECTED_CHECKOUT_SHA: \${{ inputs.checkout_sha }}
        run: |
          set -euo pipefail
          test "$(git rev-parse HEAD)" = "$EXPECTED_CHECKOUT_SHA"`, `${label} test`);
  requireExactStep(testSteps, "Check the protected workflow policy", `      - name: Check the protected workflow policy
        run: node trusted/tools/check-workflows.js`, `${label} test`);
  if ((text.match(/uses: actions\/checkout@/g) ?? []).length !== 3) {
    reject(`${label} must contain one candidate checkout per job and one protected-policy checkout`);
  }
}

function checkQuick(text) {
  const label = ".github/workflows/quick.yml";
  const expectedPrefix = [
    "name: Rustia Quick trust fixture",
    "run-name: rustia-quick:${{ inputs.repository_id }}:${{ inputs.admission_id }}:${{ inputs.checkout_sha }}:${{ inputs.quick_intent_digest }}",
    "",
    QUICK_TRIGGERS.trimEnd(),
    "",
    "permissions:",
    "  contents: read",
    "",
    "jobs:",
    "",
  ].join("\n");
  if (!text.startsWith(expectedPrefix)) {
    reject(`${label} must retain the exact reviewed top-level configuration before jobs`);
  }
  checkTriggers(text, QUICK_TRIGGERS, label);
  checkPermissions(text, label);
  checkActionPins(text, label);
  checkNoForbidden(text, label);

  const jobs = jobBlocks(text, label);
  const names = [...jobs.keys()].sort();
  if (JSON.stringify(names) !== JSON.stringify(["integrity", "test"])) {
    reject(`${label} jobs must be exactly integrity and test; found ${names.join(", ")}`);
  }
  for (const [name, block] of jobs) {
    const runners = [...block.matchAll(/^    runs-on: (.+)$/gm)].map((match) => match[1]);
    if (JSON.stringify(runners) !== JSON.stringify(["ubuntu-24.04"])) {
      reject(`${label} job ${name} must run once on ubuntu-24.04`);
    }
    requireText(block, `    name: ${name}\n`, `${label} ${name}`);
    requireText(block, "repository: ${{ inputs.head_repository_full_name }}", `${label} ${name}`);
    requireText(block, "ref: ${{ inputs.checkout_sha }}", `${label} ${name}`);
    requireText(block, "persist-credentials: false", `${label} ${name}`);
  }

  const integrity = jobs.get("integrity");
  requireExactJobHeader(integrity, `  integrity:
    name: integrity
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:`, `${label} integrity`);
  const integritySteps = exactStepBlocks(integrity, [
    "Validate dispatch inputs",
    "Checkout candidate",
    "Verify exact checkout",
  ], `${label} integrity`);
  requireExactStep(integritySteps, "Validate dispatch inputs", `      - name: Validate dispatch inputs
        shell: bash
        env:
          REPOSITORY_ID: \${{ inputs.repository_id }}
          ADMISSION_ID: \${{ inputs.admission_id }}
          PULL_REQUEST: \${{ inputs.pull_request }}
          HEAD_REPOSITORY_ID: \${{ inputs.head_repository_id }}
          HEAD_REPOSITORY_FULL_NAME: \${{ inputs.head_repository_full_name }}
          CHECKOUT_SHA: \${{ inputs.checkout_sha }}
          QUICK_INTENT_DIGEST: \${{ inputs.quick_intent_digest }}
        run: |
          set -euo pipefail
          [[ "$REPOSITORY_ID" =~ ^[1-9][0-9]*$ ]]
          [[ "$ADMISSION_ID" =~ ^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$ ]]
          [[ "$PULL_REQUEST" =~ ^[1-9][0-9]*$ ]]
          [[ "$HEAD_REPOSITORY_ID" =~ ^[1-9][0-9]*$ ]]
          [[ "$HEAD_REPOSITORY_FULL_NAME" =~ ^[^/[:space:][:cntrl:]]+/[^/[:space:][:cntrl:]]+$ ]]
          owner=\${HEAD_REPOSITORY_FULL_NAME%%/*}
          repository=\${HEAD_REPOSITORY_FULL_NAME#*/}
          (( \${#owner} <= 255 && \${#repository} <= 255 ))
          [[ "$CHECKOUT_SHA" =~ ^[0-9a-f]{40}$ ]]
          [[ "$QUICK_INTENT_DIGEST" =~ ^[0-9a-f]{64}$ ]]`, `${label} integrity`);
  requireExactStep(integritySteps, "Checkout candidate", `      - name: Checkout candidate
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          repository: \${{ inputs.head_repository_full_name }}
          ref: \${{ inputs.checkout_sha }}
          fetch-depth: 1
          persist-credentials: false`, `${label} integrity`);
  requireExactStep(integritySteps, "Verify exact checkout", `      - name: Verify exact checkout
        shell: bash
        env:
          EXPECTED_CHECKOUT_SHA: \${{ inputs.checkout_sha }}
        run: |
          set -euo pipefail
          test "$(git rev-parse HEAD)" = "$EXPECTED_CHECKOUT_SHA"`, `${label} integrity`);
  if ((text.match(/      - name: Verify exact checkout\n/g) ?? []).length !== 1) {
    reject(`${label} must contain exactly one Verify exact checkout step`);
  }

  const test = jobs.get("test");
  requireExactJobHeader(test, `  test:
    name: test
    needs: integrity
    if: github.ref == 'refs/heads/main' && needs.integrity.result == 'success'
    runs-on: ubuntu-24.04
    timeout-minutes: 10
    steps:`, `${label} test`);
  const testSteps = exactStepBlocks(test, [
    "Checkout protected policy",
    "Checkout candidate",
    "Verify test checkout",
    "Install pinned Rust toolchain",
    "Install Node",
    "Run protected Quick policy",
  ], `${label} test`);
  requireExactStep(testSteps, "Checkout protected policy", `      - name: Checkout protected policy
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          ref: \${{ github.sha }}
          path: trusted
          fetch-depth: 1
          persist-credentials: false`, `${label} test`);
  requireExactStep(testSteps, "Checkout candidate", `      - name: Checkout candidate
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        with:
          repository: \${{ inputs.head_repository_full_name }}
          ref: \${{ inputs.checkout_sha }}
          path: candidate
          fetch-depth: 0
          persist-credentials: false`, `${label} test`);
  requireExactStep(testSteps, "Verify test checkout", `      - name: Verify test checkout
        shell: bash
        working-directory: candidate
        env:
          EXPECTED_CHECKOUT_SHA: \${{ inputs.checkout_sha }}
        run: |
          set -euo pipefail
          test "$(git rev-parse HEAD)" = "$EXPECTED_CHECKOUT_SHA"`, `${label} test`);
  requireExactStep(testSteps, "Install pinned Rust toolchain", `      - name: Install pinned Rust toolchain
        uses: actions-rust-lang/setup-rust-toolchain@166cdcfd11aee3cb47222f9ddb555ce30ddb9659 # v1
        with:
          toolchain: 1.95.0
          components: rustfmt
          cache: false`, `${label} test`);
  requireExactStep(testSteps, "Install Node", `      - name: Install Node
        uses: actions/setup-node@820762786026740c76f36085b0efc47a31fe5020 # v7.0.0
        with:
          node-version: "22"`, `${label} test`);
  requireExactStep(testSteps, "Run protected Quick policy", `      - name: Run protected Quick policy
        working-directory: candidate
        env:
          RUSTIA_PREFLIGHT_ROOT: \${{ github.workspace }}/candidate
        run: node ../trusted/tools/run-release-preflight.js --stage quick`, `${label} test`);
  if ((text.match(/uses: actions\/checkout@/g) ?? []).length !== 3) {
    reject(`${label} must contain one candidate checkout per job and one protected-policy checkout`);
  }
}

function hostedJobs(expected) {
  return Object.fromEntries(Object.entries(expected).filter(([name]) => name !== "safari-runtime").map(([name]) =>
    [name, `runs-on: ${name === "macos-arm64-runtime" ? "macos-15" : name === "windows-runtime" ? "windows-2025" : "ubuntu-24.04"}`]));
}

function checkCi(text, mode = "private") {
  if (!["private", "public", "rehearsal"].includes(mode)) reject("unknown correctness CI mode");
  const label = ".github/workflows/ci.yml";
  const triggers = mode === "rehearsal" ? REHEARSAL_TRIGGERS : mode === "public"
    ? '"on":\n  public:\n  pull_request:\n  push:\n    branches:\n      - main\n  workflow_dispatch:\n'
    : '"on":\n  pull_request:\n  push:\n    branches:\n      - main\n  workflow_dispatch:\n    inputs:\n      safari:\n        description: Run visible Safari tests on an existing logged-in runner desktop\n        type: boolean\n        default: false\n      safari_version:\n        description: Exact installed Safari version required by the explicit Safari run\n        type: string\n        default: "26.6.2"\n';
  checkTriggers(text, triggers, label);
  checkPermissions(text, label);
  checkJobRunners(text, mode === "private" ? CI_JOBS : hostedJobs(CI_JOBS), label, mode);
  checkActionPins(text, label);
  checkNoForbidden(text, label);
  requireText(text, "stage: [quick, workspace, release]", label);
  requireText(text, 'node tools/run-release-preflight.js --stage "${{ matrix.stage }}"', label);
  requireText(text, "fail-fast: false", label);
  const jobs = jobBlocks(text, label);
  requireText(jobs.get("release-preflight"), [
    "      - name: Run canonical release preflight",
    "        run: node tools/run-release-preflight.js --stage \"${{ matrix.stage }}\"",
  ].join("\n"), `${label} canonical preflight execution`);
  if (mode !== "private") {
    for (const [name, block] of jobs) {
      const budget = name === "release-preflight" ? 180 : name === "release-gate" ? 5 : 60;
      const values = [...block.matchAll(/^    timeout-minutes:([^\n]*)$/gm)].map((match) => match[1].trim());
      if (JSON.stringify(values) !== JSON.stringify([String(budget)])) reject(`${label} ${name} needs its exact hosted budget`);
    }
  }
  requireText(text, "max-parallel: 2", label);
  requireText(jobs.get("release-preflight"), "fetch-depth: 0", `${label} release-preflight`);
  requireText(jobs.get("release-preflight"), [
    "      - name: Check feature-gated executor accounting paths",
    "        if: matrix.stage == 'workspace'",
    "        run: |",
    "          cargo clippy --locked -p rustia-compiler --all-targets --features executor-diagnostics -- -D warnings",
    "          cargo test --locked --release -p rustia-compiler --features executor-diagnostics --test gc_accounting_paths -- --nocapture",
  ].join("\n"), `${label} executor accounting diagnostic`);
  const stepGuards = [...text.matchAll(/^        if:([^\n]*)$/gm)].map((match) => match[1].trim());
  if (JSON.stringify(stepGuards) !== JSON.stringify(["matrix.stage == 'workspace'", "matrix.stage == 'workspace'"])) {
    reject(`${label} must retain exactly the two reviewed workspace step guards`);
  }
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
    "name: " + (mode === "rehearsal" ? "Public rehearsal / " : "") + "Engine build (${{ matrix.target }})",
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
  const firefox = jobs.get("firefox-runtime");
  for (const fragment of [
    "image: mcr.microsoft.com/playwright:v1.64.1-noble@sha256:47b510dd9230bf0b9bfc9b20102241e577ea7a99334ab4c3ca19f19770e9b8ea",
    `timeout-minutes: ${mode === "private" ? 30 : 60}`,
    "cargo install wasm-bindgen-cli --version 0.2.126 --locked",
    "https://archive.mozilla.org/pub/firefox/releases/160.0/linux-x86_64/en-US/firefox-160.0.tar.xz",
    "https://github.com/mozilla/geckodriver/releases/download/v0.37.1/geckodriver-v0.37.1-linux64.tar.gz",
    `          printf '%s  %s\\n' e9c7ed0d4abc6eb3648c2c5652bd325e690423ef49414ef201eac59c0932ea8ec52260cf652510b2c98971c7e7eb36e6be8101d87cae17c6dd28ed763bac32a6 "$browser_tools/firefox.tar.xz" | sha512sum --check --status`,
    `          printf '%s  %s\\n' e815130ea95983e162ae91843b48d3a3ce991735635fce83a647afde21e09f7e "$browser_tools/geckodriver.tar.gz" | sha256sum --check --status`,
    '          browser_home="$browser_tools/firefox-home"',
    '          mkdir "$browser_home"',
    `            printf 'exec env -u XAUTHORITY -u XDG_RUNTIME_DIR HOME=%q %q "$@"\\n' "$browser_home" "$browser_tools/firefox/firefox"`,
    `${safeDirectory}\n          bash tools/run-browser-correctness.sh firefox 160.0 0.37.1`,
  ]) requireText(firefox, fragment, `${label} firefox-runtime`);
  if (mode === "private") {
  const safari = jobs.get("safari-runtime");
  for (const fragment of [
    "timeout-minutes: 30",
    "SAFARI_VERSION: ${{ inputs.safari_version }}",
    "            echo \"::error::Safari needs the runner account's existing logged-in desktop session; no browser tests ran.\"",
    '            exit 1',
    '          test -n "$SAFARI_VERSION"',
    '          test "$actual_safari" = "$SAFARI_VERSION"',
    "rustup toolchain install 1.95.0 --profile minimal --target wasm32-unknown-unknown --no-self-update",
    "https://github.com/wasm-bindgen/wasm-bindgen/releases/download/0.2.126/wasm-bindgen-0.2.126-aarch64-apple-darwin.tar.gz",
    `          printf '%s  %s\\n' 7df536babe345deb68828148dbdc71179118afdab42d83547c7cebfbf1426bd5 "$safari_tools/wasm-tools.tar.gz" | shasum -a 256 --check --status`,
    '          bash tools/run-browser-correctness.sh safari "$SAFARI_VERSION"',
  ]) requireText(safari, fragment, `${label} safari-runtime`);
  requireText(safari, [
    "          runner_owns_console=false",
    "          if [ \"$(stat -f '%Su' /dev/console)\" = \"$(id -un)\" ]; then",
    "            runner_owns_console=true",
    "          fi",
    "          runner_gui_available=false",
    "          if launchctl print \"gui/$(id -u)\" >/dev/null 2>&1; then",
    "            runner_gui_available=true",
    "          fi",
    "          printf 'Safari readiness: runnerOwnsConsole=%s; runnerGuiDomainExists=%s\\n' \"$runner_owns_console\" \"$runner_gui_available\"",
    "          if [ \"$runner_owns_console\" != true ] || [ \"$runner_gui_available\" != true ]; then",
    "            echo \"::error::Safari needs the runner account's existing logged-in desktop session; no browser tests ran.\"",
    "            exit 1",
    "          fi",
  ].join("\n"), `${label} Safari GUI prerequisite`);
  for (const forbidden of ["--enable", "sudo ", "brew ", "defaults write", "chmod -R", "chown ", "uses: actions-rust-lang/setup-rust-toolchain@"]) {
    forbidText(safari, forbidden, `${label} Safari permissions/bootstrap`);
  }
  }
  const windows = jobs.get("windows-runtime");
  requireText(windows, `if: ${mode === "private" ? PRIVATE_MANUAL : mode === "public" ? PUBLIC_MANUAL : `${REHEARSAL} && inputs.extended`}`, `${label} windows-runtime`);
  const macos = jobs.get("macos-arm64-runtime");
  if (mode === "private") {
    requireText(macos, [
      '          cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"',
      '          if [ ! -x "$cargo_bin/rustup" ]; then',
      '            echo "::error::Rustup is missing from this account\'s Cargo tools directory."',
      "            exit 1",
      "          fi",
      '          export PATH="$cargo_bin:$PATH"',
      '          echo "$cargo_bin" >> "$GITHUB_PATH"',
      "          rustup toolchain install 1.95.0 --profile minimal --target aarch64-apple-darwin --no-self-update",
    ].join("\n"), `${label} macOS rustup setup`);
    forbidText(macos, "brew ", `${label} macOS bootstrap`);
    forbidText(macos, "uses: actions-rust-lang/setup-rust-toolchain@", `${label} macOS bootstrap`);
  } else {
    requireText(macos, HOSTED_MAC_SETUP, label + " hosted macOS rustup setup");
  }
  const release = jobs.get("release-preflight");
  if (!release.includes("stage: [quick, workspace, release]")) reject(`${label} must run all canonical preflight stages`);
}

function checkErrorFuzz(text, mode = "private") {
  if (!["private", "public", "rehearsal"].includes(mode)) reject("unknown correctness CI mode");
  const label = ".github/workflows/error-fuzz.yml";
  const ordinaryTriggers = '  pull_request:\n    types: [opened, synchronize, reopened]\n  workflow_dispatch:\n';
  const triggers = mode === "public" ? '"on":\n  public:\n' : '"on":\n';
  checkTriggers(text, mode === "rehearsal" ? REHEARSAL_TRIGGERS : triggers + ordinaryTriggers, label);
  checkPermissions(text, label);
  checkJobRunners(text, mode === "private" ? FUZZ_JOBS : hostedJobs(FUZZ_JOBS), label, mode);
  checkActionPins(text, label);
  checkNoForbidden(text, label);
  requireText(text, "cargo test --locked --manifest-path fuzz/Cargo.toml generated_differential_smoke -- --ignored", label);
  requireText(text, `timeout-minutes: ${mode === "private" ? 20 : 60}`, `${label} differential-smoke`);
  const jobs = jobBlocks(text, label);
  requireText(jobs.get("differential-smoke"), [
    "      - name: Verify release diagnostic parity against pinned Luau",
    "        run: node tools/audit-release-errors.mjs",
  ].join("\n"), label + " diagnostic parity");
  if (mode === "public") {
    requireText(jobs.get("differential-smoke"), "run: cargo test --locked --manifest-path fuzz/Cargo.toml --features oracle --test upstream_compiler", label + " compiler ports");
    requireText(jobs.get("differential-smoke"), "run: cargo test --locked --manifest-path fuzz/Cargo.toml --features oracle --test upstream_parser", label + " parser ports");
  }
  const stepGuards = [...text.matchAll(/^        if:([^\n]*)$/gm)].map((match) => match[1].trim());
  if (JSON.stringify(stepGuards) !== JSON.stringify(["always()"])) reject(`${label} must retain only its evidence-upload step guard`);
  const campaign = jobs.get("campaign");
  const manual = mode === "private" ? PRIVATE_MANUAL : mode === "public" ? PUBLIC_MANUAL : `${REHEARSAL} && inputs.extended`;
  requireText(campaign, `if: ${manual}`, `${label} campaign`);
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
  requireText(campaign, `if: ${manual}`, `${label} campaign`);
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
  const hosted = profile.correctness_ci === "public-hosted";
  if (profile.correctness_ci !== undefined && !["private-self-hosted", "public-hosted"].includes(profile.correctness_ci)) {
    reject("unknown selected correctness CI mode");
  }
  const sortedFiles = files.slice().sort();
  const rehearsalFiles = ["public-ci-rehearsal.yml", "public-error-fuzz-rehearsal.yml"];
  const activeFiles = hosted ? EXPECTED_FILES : CORE_FILES;
  const withRehearsal = [...CORE_FILES, ...rehearsalFiles].sort();
  if (JSON.stringify(sortedFiles) !== JSON.stringify(activeFiles.slice().sort()) &&
      (hosted || JSON.stringify(sortedFiles) !== JSON.stringify(withRehearsal))) {
    reject(`active workflows must be exactly ${activeFiles.join(", ")}; found ${sortedFiles.join(", ")}`);
  }
  for (const file of CORE_FILES) {
    const text = workflows[file];
    if (typeof text !== "string") reject(`missing active workflow ${file}`);
    checkNoForbidden(text, `.github/workflows/${file}`);
  }
  checkCi(workflows["ci.yml"], hosted ? "public" : "private");
  checkErrorFuzz(workflows["error-fuzz.yml"], hosted ? "public" : "private");
  if (hosted) {
    if (typeof workflows[QUICK_FILE] !== "string") reject(`missing active workflow ${QUICK_FILE}`);
    if (typeof workflows[BULK_FILE] !== "string") reject(`missing active workflow ${BULK_FILE}`);
    checkQuick(workflows[QUICK_FILE]);
    checkBulk(workflows[BULK_FILE]);
  }
  if (files.includes(rehearsalFiles[0])) {
    checkCi(workflows[rehearsalFiles[0]], "rehearsal");
    checkErrorFuzz(workflows[rehearsalFiles[1]], "rehearsal");
  }
  return { files: sortedFiles.length, profile: profile.correctness_ci ?? "private-self-hosted" };
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

function bulkSelfTest(root) {
  const assert = require("node:assert/strict");
  const bulk = fs.readFileSync(path.join(root, ".github/workflows/bulk.yml"), "utf8");
  checkBulk(bulk);
  let cases = 0;
  for (const [label, changed] of [
    ["bulk pull-request trigger", bulk.replace("  workflow_dispatch:", "  pull_request:\n  workflow_dispatch:")],
    ["bulk input drift", bulk.replace("      manifest_digest:", "      reported_digest:")],
    ["bulk write permission", bulk.replace("contents: read", "contents: write")],
    ["bulk job flow permission", bulk.replace("    runs-on: ubuntu-24.04", "    permissions: { actions: write, contents: read }\n    runs-on: ubuntu-24.04")],
    ["bulk spaced job flow permission", bulk.replace("    runs-on: ubuntu-24.04", "    permissions : { actions: write, contents: read }\n    runs-on: ubuntu-24.04")],
    ["bulk quoted job flow permission", bulk.replace("    runs-on: ubuntu-24.04", "    \"permissions\": { actions: write, contents: read }\n    runs-on: ubuntu-24.04")],
    ["bulk single-quoted job flow permission", bulk.replace("    runs-on: ubuntu-24.04", "    'permissions': { actions: write, contents: read }\n    runs-on: ubuntu-24.04")],
    ["bulk job environment preload", bulk.replace("    timeout-minutes: 10\n    steps:", "    timeout-minutes: 10\n    env: { NODE_OPTIONS: --require=./candidate/attacker.js }\n    steps:")],
    ["bulk top-level environment preload", bulk.replace('"on":\n', "env:\n  NODE_OPTIONS: --require=./candidate/attacker.js\n\n\"on\":\n")],
    ["bulk top-level run defaults", bulk.replace('"on":\n', "defaults:\n  run:\n    shell: bash -c 'node candidate/attacker.js; bash {0}'\n\n\"on\":\n")],
    ["bulk token exposure", bulk.replace("        shell: bash", "        env:\n          GITHUB_TOKEN: ${{ github.token }}\n        shell: bash")],
    ["bulk indexed token exposure", bulk.replace("        shell: bash", "        env:\n          TOKEN: ${{ github['token'] }}\n        shell: bash")],
    ["bulk spaced indexed token exposure", bulk.replace("        shell: bash", "        env:\n          TOKEN: ${{ github[ 'token' ] }}\n        shell: bash")],
    ["bulk double-quoted indexed token exposure", bulk.replace("        shell: bash", "        env:\n          TOKEN: ${{ github[ \"token\" ] }}\n        shell: bash")],
    ["bulk whole context token exposure", bulk.replace("        shell: bash", "        env:\n          CONTEXT: ${{ toJSON(github) }}\n        shell: bash")],
    ["bulk moving protected policy ref", bulk.replace("ref: ${{ github.sha }}", "ref: main")],
    ["bulk untrusted ref", bulk.replace("ref: ${{ inputs.checkout_sha }}", "ref: main")],
    ["bulk persisted credentials", bulk.replace("persist-credentials: false", "persist-credentials: true")],
    ["bulk self-hosted runner", bulk.replace("runs-on: ubuntu-24.04", "runs-on: self-hosted")],
    ["bulk title drift", bulk.replace("run-name: rustia-ci:", "run-name: bulk:")],
    ["bulk job-name drift", bulk.replace("  integrity:\n", "  verify:\n")],
    ["bulk missing exact-checkout step", bulk.replace("Verify exact checkout", "Trust checkout")],
    ["bulk command before validation", bulk.replace("    steps:\n      - name: Validate dispatch inputs", "    steps:\n      - name: Run early command\n        run: true\n\n      - name: Validate dispatch inputs")],
    ["bulk unnamed command before validation", bulk.replace("    steps:\n      - name: Validate dispatch inputs", "    steps:\n      - run: true\n\n      - name: Validate dispatch inputs")],
    ["bulk command before exact verification", bulk.replace("      - name: Verify exact checkout", "      - name: Run candidate early\n        run: true\n\n      - name: Verify exact checkout")],
    ["bulk in-step command before exact verification", bulk.replace('          test "$(git rev-parse HEAD)" = "$EXPECTED_CHECKOUT_SHA"', '          node candidate/early.js\n          test "$(git rev-parse HEAD)" = "$EXPECTED_CHECKOUT_SHA"')],
    ["bulk test before verification", bulk.replace("      - name: Verify test checkout", "      - name: Check the protected workflow policy\n        run: node tools/check-workflows.js\n\n      - name: Verify test checkout")],
    ["bulk candidate policy checker", bulk.replace("node trusted/tools/check-workflows.js", "node candidate/tools/check-workflows.js")],
    ["bulk comment-spoofed policy provenance", bulk
      .replace("ref: ${{ github.sha }}", "ref: ${{ inputs.checkout_sha }}")
      .replace("node trusted/tools/check-workflows.js", "node candidate/tools/check-workflows.js\n        # ref: ${{ github.sha }}\n        # run: node trusted/tools/check-workflows.js")],
  ]) {
    assert.notEqual(changed, bulk, `${label}: mutation did not apply`);
    assert.throws(() => checkBulk(changed), /bulk\.yml/, label);
    cases += 1;
  }
  return { cases, passed: true };
}

function quickSelfTest(root) {
  const assert = require("node:assert/strict");
  const quick = fs.readFileSync(path.join(root, ".github/workflows/quick.yml"), "utf8");
  checkQuick(quick);
  let cases = 0;
  for (const [label, changed] of [
    ["Quick pull-request trigger", quick.replace("  workflow_dispatch:", "  pull_request:\n  workflow_dispatch:")],
    ["Quick input drift", quick.replace("      quick_intent_digest:", "      reported_digest:")],
    ["Quick write permission", quick.replace("contents: read", "contents: write")],
    ["Quick self-hosted runner", quick.replace("runs-on: ubuntu-24.04", "runs-on: self-hosted")],
    ["Quick title drift", quick.replace("run-name: rustia-quick:", "run-name: quick:")],
    ["Quick job-name drift", quick.replace("  integrity:\n", "  verify:\n")],
    ["Quick untrusted repository", quick.replace("repository: ${{ inputs.head_repository_full_name }}", "repository: slowchop/rustia")],
    ["Quick untrusted ref", quick.replace("ref: ${{ inputs.checkout_sha }}", "ref: main")],
    ["Quick moving policy ref", quick.replace("ref: ${{ github.sha }}", "ref: main")],
    ["Quick shallow validation history", quick.replace("          path: candidate\n          fetch-depth: 0", "          path: candidate\n          fetch-depth: 1")],
    ["Quick persisted credentials", quick.replace("persist-credentials: false", "persist-credentials: true")],
    ["Quick token exposure", quick.replace("        shell: bash", "        env:\n          TOKEN: ${{ github.token }}\n        shell: bash")],
    ["Quick command before validation", quick.replace("    steps:\n      - name: Validate dispatch inputs", "    steps:\n      - name: Run early command\n        run: true\n\n      - name: Validate dispatch inputs")],
    ["Quick candidate before exact verification", quick.replace("      - name: Verify exact checkout", "      - name: Run candidate early\n        run: true\n\n      - name: Verify exact checkout")],
    ["Quick candidate policy runner", quick.replace("node ../trusted/tools/run-release-preflight.js", "node tools/run-release-preflight.js")],
    ["Quick wrong candidate root", quick.replace("${{ github.workspace }}/candidate", "${{ github.workspace }}/trusted")],
    ["Quick skipped compiler partition", quick.replace("--stage quick", "--stage workspace")],
    ["Quick enabled cache", quick.replace("cache: false", "cache: true")],
    ["Quick unpinned toolchain", quick.replace("toolchain: 1.95.0", "toolchain: stable")],
  ]) {
    assert.notEqual(changed, quick, `${label}: mutation did not apply`);
    assert.throws(() => checkQuick(changed), /quick\.yml/, label);
    cases += 1;
  }
  return { cases, passed: true };
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
  const files = CORE_FILES.slice();
  const run = (changedProfile = profile, changedFiles = files, changedWorkflows = workflows) =>
    validatePublicProfile({ profile: changedProfile, files: changedFiles, workflows: changedWorkflows });
  let cases = 0;
  const expectFail = (label, fn, pattern) => {
    assert.throws(fn, pattern, label);
    cases += 1;
  };
  try {
    assert.deepEqual(run(), { files: 2, profile: "private-self-hosted" });
    assert.deepEqual(run({ ...profile, benchmark_bridge: "private-maintainer-cohorts-v1" }), {
      files: 2, profile: "private-self-hosted",
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
      ["hosted fallback", "ci.yml", (s) => s.replace(LINUX_RUNNER, "runs-on: ubuntu-24.04"), /must run on/],
      ["unscoped self-hosted runner", "ci.yml", (s) => s.replace(LINUX_RUNNER, "runs-on: [self-hosted, Linux]"), /must run on/],
      ["benchmark runner used for correctness", "ci.yml", (s) => s.replace("group: general-linux", "group: bench-solo"), /must run on/],
      ["missing private guard", "ci.yml", (s) => s.replace(`    if: ${PRIVATE_PR}\n`, ""), /trusted-event guard/],
      ["public jobs permitted", "ci.yml", (s) => s.replace(PRIVATE_PR, PRIVATE_PR.replace("github.event.repository.private == true && ", "")), /trusted-event guard/],
      ["fork PRs permitted", "ci.yml", (s) => s.replace(PRIVATE_PR, "github.event.repository.private == true"), /trusted-event guard/],
      ["hosted fuzz fallback", "error-fuzz.yml", (s) => s.replace(LINUX_RUNNER, "runs-on: ubuntu-24.04"), /must run on/],
      ["unguarded fuzz job", "error-fuzz.yml", (s) => s.replace(`    if: ${PRIVATE_PR}\n`, ""), /trusted-event guard/],
      ["unpinned macOS setup", "ci.yml", (s) => s.replace("rustup toolchain install 1.95.0 --profile minimal", "rustup toolchain install stable --profile minimal"), /macOS rustup setup/],
      ["system package write on macOS", "ci.yml", (s) => s.replace('          cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"', '          brew install bash\n          cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"'), /must not include/],
      ["untrusted action ref", "ci.yml", (s) => s.replace(/actions\/checkout@([0-9a-f]{40})/, "actions/checkout@v7"), /pin every action/],
      ["write permission", "ci.yml", (s) => s.replace("contents: read", "contents: write"), /contents: write/],
      ["secret use", "error-fuzz.yml", (s) => `${s}\n# secrets.X\n`, /secrets/],
      ["private workflow trigger", "error-fuzz.yml", (s) => s.replace("workflow_dispatch:", "schedule:\n    - cron: '0 0 * * *'\n  workflow_dispatch:"), /schedule/],
      ["benchmark command", "ci.yml", (s) => `${s}\n# node tools/run-dedicated-benchmark.js\n`, /run-dedicated-benchmark/],
      ["unbounded fuzz", "error-fuzz.yml", (s) => s.replace("-max_total_time=7200", "-max_total_time=9000"), /bounded durations/],
      ["campaign on PR label", "error-fuzz.yml", (s) => s.replace(`if: ${PRIVATE_MANUAL}`, `if: ${PRIVATE_MANUAL} || contains(github.event.pull_request.labels.*.name, 'run-error-fuzz')`), /trusted-event guard/],
      ["credentials persisted", "ci.yml", (s) => s.replace("persist-credentials: false", "persist-credentials: true"), /persist-credentials: false/],
      ["enabled build cache", "ci.yml", (s) => s.replace("cache: false", "cache: true"), /cache: false/],
      ["missing preflight aggregate", "ci.yml", (s) => s.replace("run: test \"$PREFLIGHT_RESULT\" = success && test \"$WEBKIT_RESULT\" = success", "run: true"), /run: test.*PREFLIGHT_RESULT/],
      ["missing executor accounting diagnostic", "ci.yml", (s) => s.replace("--test gc_accounting_paths -- --nocapture", "--test missing_diagnostic -- --nocapture"), /executor accounting diagnostic/],
      ["duplicated executor accounting diagnostic", "ci.yml", (s) => s.replace("      - name: Check feature-gated executor accounting paths\n        if: matrix.stage == 'workspace'", "      - name: Check feature-gated executor accounting paths\n        if: always()"), /executor accounting diagnostic/],
      ["missing WebKit safe-directory setup", "ci.yml", (s) => s.replace('          git config --global --add safe.directory "$GITHUB_WORKSPACE"\n', ""), /safe\.directory/],
      ["Safari without opt-in", "ci.yml", (s) => s.replace(`if: ${PRIVATE_SAFARI}`, `if: ${PRIVATE_MANUAL}`), /trusted-event guard/],
      ["Safari enabled by default", "ci.yml", (s) => s.replace("type: boolean\n        default: false", "type: boolean\n        default: true"), /reviewed public workflow triggers/],
      ["Safari GUI ownership omitted", "ci.yml", (s) => s.replace("stat -f '%Su' /dev/console", "printf runner"), /Safari GUI prerequisite/],
      ["Safari GUI failure falls through", "ci.yml", (s) => s.replace('Safari needs the runner account\'s existing logged-in desktop session; no browser tests ran."\n            exit 1', 'Safari needs the runner account\'s existing logged-in desktop session; no browser tests ran."\n            true'), /Safari GUI prerequisite/],
      ["Safari on PRs", "ci.yml", (s) => s.replace(`if: ${PRIVATE_SAFARI}`, `if: ${PRIVATE_PR}`), /trusted-event guard/],
      ["Safari readiness diagnostics omitted", "ci.yml", (s) => s.replace("Safari readiness: runnerOwnsConsole=%s; runnerGuiDomainExists=%s", "readiness unavailable"), /Safari GUI prerequisite/],
      ["Safari checksum omitted", "ci.yml", (s) => s.replace("shasum -a 256 --check --status", "true"), /safari-runtime/],
      ["Safari execution omitted", "ci.yml", (s) => s.replace('bash tools/run-browser-correctness.sh safari "$SAFARI_VERSION"', "true"), /safari-runtime/],
      ["Safari permissions enabled", "ci.yml", (s) => s + "\n          # safaridriver --enable\n", /Safari permissions/],
      ["Firefox on PRs", "ci.yml", (s) => s.replace(`  firefox-runtime:\n    name: Browser runtime (real Firefox, manual)\n    if: ${PRIVATE_MANUAL}`, `  firefox-runtime:\n    name: Browser runtime (real Firefox, manual)\n    if: ${PRIVATE_PR}`), /trusted-event guard/],
      ["Firefox checksum omitted", "ci.yml", (s) => s.replace("sha512sum --check --status", "true"), /firefox-runtime/],
      ["geckodriver checksum omitted", "ci.yml", (s) => s.replace("sha256sum --check --status", "true"), /firefox-runtime/],
      ["Firefox owned home omitted", "ci.yml", (s) => s.replace('browser_home="$browser_tools/firefox-home"', 'browser_home="$HOME"'), /firefox-runtime/],
      ["Firefox runtime omitted", "ci.yml", (s) => s.replace("bash tools/run-browser-correctness.sh firefox 160.0 0.37.1", "true"), /firefox-runtime/],
      ["Firefox uses unpinned container", "ci.yml", (s) => s.replace(/(firefox-runtime:[\s\S]*?image: )[^\n]+/, "$1mcr.microsoft.com/playwright:latest"), /firefox-runtime/],
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
    cases += quickSelfTest(root).cases;
    cases += bulkSelfTest(root).cases;
    return { cases, passed: true };
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

module.exports = { HOSTED_MAC_SETUP, validatePublicProfile, checkCi, checkErrorFuzz, checkQuick, checkBulk, readCheckout, selfTest, quickSelfTest, bulkSelfTest,
  REHEARSAL, REHEARSAL_TRIGGERS, PUBLIC_PR, PUBLIC_MANUAL };
