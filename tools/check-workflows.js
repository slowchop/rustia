#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const workflowDir = path.join(root, ".github", "workflows");
const ciPath = path.join(workflowDir, "ci.yml");
const rustiaMemoryPath = path.join(workflowDir, "rustia-memory.yml");
const benchmarkPublicationPath = path.join(workflowDir, "publish-benchmark-data.yml");
const weeklyMatrixPath = path.join(workflowDir, "weekly-matrix-bench.yml");
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

function requireBefore(text, first, second, label) {
  const firstAt = text.indexOf(first);
  const secondAt = text.indexOf(second);
  if (firstAt === -1 || secondAt === -1 || firstAt > secondAt) {
    fail(`${label} must include ${JSON.stringify(first)} before ${JSON.stringify(second)}`);
  }
}

// GitHub's markers for skipping push and pull-request workflows. Case is
// ignored because GitHub documents none, so a variant it might honour still
// counts here.
const SKIP_MARKER = /\[(?:skip ci|ci skip|no ci|skip actions|actions skip)\]|skip-checks:\s*true/gi;

// What the weekly matrix dispatches on main once its cohort has landed, exactly.
const WEEKLY_MATRIX_DISPATCHES = [
  '          failed=""',
  "          curl -sSf -X POST \\",
  '            -H "Authorization: Bearer $GITHUB_TOKEN" \\',
  '            -H "Accept: application/vnd.github+json" \\',
  '            "$GITHUB_API_URL/repos/$GITHUB_REPOSITORY/actions/workflows/publish-benchmark-data.yml/dispatches" \\',
  "            -d '{\"ref\":\"main\"}' || failed=\"publish-benchmark-data.yml\"",
  "          curl -sSf -X POST \\",
  '            -H "Authorization: Bearer $GITHUB_TOKEN" \\',
  '            -H "Accept: application/vnd.github+json" \\',
  '            "$GITHUB_API_URL/repos/$GITHUB_REPOSITORY/actions/workflows/ci.yml/dispatches" \\',
  "            -d '{\"ref\":\"main\"}' || failed=\"${failed:+$failed and }ci.yml\"",
  '          if [ -n "$failed" ]; then',
  '            echo "::error::The cohort has landed, but dispatching $failed on main failed. Dispatch each one named here on main by hand; re-running this job would measure and land another cohort." >&2',
  "            exit 1",
  "          fi",
  "",
].join("\n");

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
    // Only this success-dependent job-level local call is permitted. Do not
    // turn the exception into acceptance of arbitrary local actions/workflows.
    const publicationCall = "uses: ./.github/workflows/publish-benchmark-data.yml";
    const caller = label === ".github/workflows/rustia-memory.yml"
      ? `\n  publish:\n    needs: benchmark\n    ${publicationCall}\n`
      : label === ".github/workflows/main-bench.yml"
        ? `\n  publish:\n    needs: measure\n    if: always() && needs.measure.outputs.data_published == 'true'\n    ${publicationCall}\n`
        : null;
    const reusablePublication = caller !== null
      && line === publicationCall
      && text.includes(caller)
      && usesLines(text).filter(use => use === publicationCall).length === 1;
    if (!official && !rustToolchain && !reusablePublication) {
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
  [
    "general-linux-docker",
    {
      block: [
        "    runs-on:",
        "      group: general-linux",
        "      labels: [self-hosted, Linux, X64, general-linux, docker]",
      ].join("\n"),
      timingAuthority: false,
    },
  ],
  [
    "general-macos",
    {
      block: [
        "    runs-on:",
        "      group: general-macos",
        "      labels: [self-hosted, macOS, ARM64, general-macos]",
      ].join("\n"),
      timingAuthority: false,
    },
  ],
  [
    "general-windows",
    {
      block: [
        "    runs-on:",
        "      group: general-windows",
        "      labels: [self-hosted, Windows, X64, general-windows]",
      ].join("\n"),
      timingAuthority: false,
    },
  ],
  [
    // The owned benchmark host's runner group. Targeted by group alone: the
    // group has exactly one runner, so naming labels as well would add a
    // second thing to keep in sync with the machine for no extra guarantee.
    "bench-solo",
    {
      block: ["    runs-on:", "      group: bench-solo"].join("\n"),
      timingAuthority: true,
    },
  ],
]);

// Every workflow must appear here, so adding one is a deliberate act rather than
// a file that quietly inherits whatever the loop happens to allow. `null` means
// the workflow may only use GitHub-hosted runners.
const WORKFLOW_RUNNER_CLASS = new Map([
  ["error-fuzz.yml", "general-linux"],
  ["ci.yml", "general-linux"],
  ["browser-correctness.yml", "general-linux"],
  ["weekly-matrix-bench.yml", "bench-solo"],
  ["publish-benchmark-data.yml", "general-linux"],
  ["main-bench.yml", "bench-solo"],
  ["pr-bench.yml", "bench-solo"],
  ["acceptance-bench.yml", "bench-solo"],
  ["rustia-memory.yml", "bench-solo"],
]);

// Explicit correctness jobs select the platform and capabilities they need.
const JOB_RUNNER_CLASS = new Map([
  ["ci.yml:macos-arm64-runtime", "general-macos"],
  ["ci.yml:webkit-runtime", "general-linux-docker"],
  ["ci.yml:windows-runtime", "general-windows"],
  ["browser-correctness.yml:safari-runtime", "general-macos"],
]);

// Commands that take a timing. Until now only the first three were forbidden,
// and the four on-host runners were forbidden nowhere -- a workflow on the
// shared pool could have invoked any of them and no rule would have stopped it.
// That was the real hole; the raw-harness ban was the visible part of it.
const RAW_HARNESS = "cargo run -p script-bench-harness";
const HARNESS_REFRESH_ONLY = `${RAW_HARNESS} -- --refresh-results`;
const MEASUREMENT_COMMANDS = Object.freeze([
  RAW_HARNESS,
  "node tools/run-wasm-gate.js",
  "node tools/run-wasm-dispatch-gate.js",
  // Deleted runner. The entry stays so the name cannot return through a
  // workflow; a forbid needs no file to exist.
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
// measure there. No workflow invokes one any more -- the matrix moved to the
// owned host -- and weekly-matrix-bench.yml forbids the string by name, so the
// remaining question is when the coordinators themselves are deleted rather
// than whether a workflow can reach one.

// contents: write can create a commit on main. Forbidden everywhere except the
// workflows named here, so a third holder cannot appear by omission.
const CONTENTS_WRITE_WORKFLOWS = new Set(["weekly-matrix-bench.yml"]);

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
    let job = "";
    let allowedMentions = 0;
    for (let index = 0; index < lines.length; index += 1) {
      const jobHeader = lines[index].match(/^  ([a-zA-Z0-9_-]+):$/);
      if (jobHeader) job = jobHeader[1];
      const inline = lines[index].match(RUNS_ON_PATTERN);
      if (!inline) continue;
      const value = inline[1].trim();
      if (value.startsWith("[")) {
        fail(`${label} must use expanded runs-on form so runner labels are reviewable`);
      }
      if (value === "") {
        const jobClassName = JOB_RUNNER_CLASS.get(`${file}:${job}`) ?? className;
        const jobClass = jobClassName === null ? null : RUNNER_CLASSES.get(jobClassName);
        if (jobClass === null) {
          fail(`${label} may only use GitHub-hosted runners, but declares an expanded runs-on block`);
        }
        const blockLines = jobClass.block.split("\n").length;
        const runnerBlock = lines.slice(index, index + blockLines).join("\n");
        if (runnerBlock !== jobClass.block) {
          fail(`${label} job ${job} expanded runs-on block must match the reviewed ${jobClassName} class exactly`);
        }
        allowedMentions += runnerBlock.split("self-hosted").length - 1;
        continue;
      }
      if (!hostedRunners.has(value)) {
        fail(`${label} hosted runs-on value must be one of ${[...hostedRunners].join(", ")}, got ${value}`);
      }
    }

    // A workflow may name exactly the runners its class allows, and no others.
    // Counting occurrences catches a bench workflow that also names the shared
    // pool, which a plain allowlist cannot express.
    // How many times the file says "self-hosted", against how many times its own
    // runner block accounts for. A group-only block contains none, so the
    // permitted count is zero rather than one per block.
    const selfHostedMentions = text.split("self-hosted").length - 1;
    if (selfHostedMentions !== allowedMentions) {
      fail(
        `${label} mentions "self-hosted" ${selfHostedMentions} time(s); its ${className ?? "hosted-only"} ` +
          `runner block accounts for ${allowedMentions}. A workflow may name only its own class.`,
      );
    }

    for (const command of MEASUREMENT_COMMANDS) {
      // The harness has a mode that takes no timing: --refresh-results returns
      // before any measurement and re-derives the report tables from committed
      // history. The ban exists to stop a workflow taking a timing, so it has to
      // name that rather than the binary -- otherwise the rule is worked around
      // by invoking the same command through `just`, which is what the matrix
      // lane did for months without any rule noticing.
      if (command === RAW_HARNESS) {
        const uses = text.split(RAW_HARNESS).length - 1;
        const refreshes = text.split(HARNESS_REFRESH_ONLY).length - 1;
        if (uses > refreshes) {
          fail(`${label} must not include ${JSON.stringify(RAW_HARNESS)} except as --refresh-results`);
        }
        continue;
      }
      forbidIncludes(text, command, label);
    }
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

// The lanes that hold the bench host for a full measurement wait for it in one
// concurrency group, in the order they began waiting. queue: max is the part
// that makes it a queue: a group without it keeps a single pending run, and a
// newly queued run cancels that one -- cancel-in-progress: false protects only
// the run already going. That cancelled the per-commit run for f49646f on
// 2026-09-15. Exclusivity is not this group's job; bench-solo holding one
// runner is. Screening stays out because it cancels its own superseded runs,
// which GitHub rejects alongside queue: max, and the publisher stays out
// because the memory lane calls it while holding this group.
const BENCH_HOST_QUEUE_GROUP = "rustia-bench-host-serial";
const BENCH_HOST_QUEUE_BLOCK = [
  "concurrency:",
  `  group: ${BENCH_HOST_QUEUE_GROUP}`,
  "  cancel-in-progress: false",
  "  queue: max",
  "",
].join("\n");
const BENCH_HOST_QUEUE_WORKFLOWS = Object.freeze([
  "acceptance-bench.yml",
  "main-bench.yml",
  "rustia-memory.yml",
  "weekly-matrix-bench.yml",
]);

function checkBenchHostQueue(files) {
  for (const required of BENCH_HOST_QUEUE_WORKFLOWS) {
    if (!files.includes(required)) fail(`missing .github/workflows/${required}`);
  }
  for (const file of files) {
    const text = readWorkflow(path.join(workflowDir, file));
    const label = `.github/workflows/${file}`;
    if (!BENCH_HOST_QUEUE_WORKFLOWS.includes(file)) {
      forbidIncludes(text, BENCH_HOST_QUEUE_GROUP, label);
      continue;
    }
    requireIncludes(text, `\n${BENCH_HOST_QUEUE_BLOCK}`, label);
    // One block, at workflow level. A job-level group beside it would put the
    // measuring job back in a queue that holds one pending run.
    if (text.split("concurrency:").length - 1 !== 1) {
      fail(`${label} must declare exactly one concurrency block, the bench host queue`);
    }
  }
}

function checkAcceptanceEvidence(text) {
  const label = ".github/workflows/acceptance-bench.yml";
  const retention = text.split("      - name: Retain diagnostic evidence, including failed runs\n")[1];
  if (!retention) fail(`${label} must retain acceptance evidence`);
  requireIncludes(retention, "        if: ${{ always() }}\n", label);
  requireIncludes(retention, "            .rustia-benchmark-state/dedicated/runs/accept-*/export.tar.gz\n", label);
  requireIncludes(retention, "            ${{ runner.temp }}/rustia-acceptance.*/acceptance.*\n", label);
  requireIncludes(retention, "          include-hidden-files: true\n", label);
  requireIncludes(retention, "          retention-days: 30\n", label);
}

function checkCi(text) {
  // GitHub evaluates job env before runner/steps/job contexts exist.
  for (const block of text.matchAll(/^    env:\n((?:^      [^\n]*\n)+)/gm)) {
    if (/\$\{\{[^}]*\b(?:runner|steps|job)\./.test(block[1])) {
      fail("ci.yml job env uses a context that is only available inside steps");
    }
  }
  const label = ".github/workflows/ci.yml";
  const rustChannel = pinnedRustChannel();
  const releasePreflightJob = text.split("\n  macos-arm64-runtime:\n", 1)[0];
  requireIncludes(text, "name: CI\n", label);
  // workflow_dispatch is how the weekly matrix gets CI onto main's tip after a
  // landing. Its app-token merge does start push workflows, so the dispatch may
  // now duplicate that run; the concurrency group below collapses the pair.
  requireIncludes(
    text,
    "\"on\":\n  pull_request:\n  push:\n    branches:\n      - main\n  workflow_dispatch:\n",
    label,
  );
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  // Anyone who can dispatch this can already run it through a same-repository
  // pull request, which is what makes the manual trigger harmless -- as long as
  // it holds a read-only token and no secret. The token is pinned read-only
  // above; these reject a write scope or write-all added at any level, and the
  // secrets context in any form, including toJSON(secrets) and bracket access.
  if (/\bsecrets\b/i.test(text)) fail(`${label} must not reference the secrets context`);
  if (/:\s*["']?write/.test(text)) fail(`${label} must not grant a write permission`);
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
  requireIncludes(text, 'node tools/run-release-preflight.js --stage "${{ matrix.stage }}"', label);
  requireIncludes(releasePreflightJob, 'stage: [quick, workspace, release]', label);
  requireIncludes(text, '    name: Release preflight\n    needs: [release-preflight, webkit-runtime]\n    if: always()', label);
  requireIncludes(text, 'PREFLIGHT_RESULT: ${{ needs.release-preflight.result }}', label);
  requireIncludes(text, 'run: test "$PREFLIGHT_RESULT" = success', label);
  requireIncludes(text, 'WEBKIT_RESULT: ${{ needs.webkit-runtime.result }}', label);
  requireIncludes(text, '&& test "$WEBKIT_RESULT" = success', label);
  requireIncludes(text, 'bash tools/run-browser-correctness.sh webkit 1.64.1', label);
  requireIncludes(text, 'npm ci --prefix tools/browser --ignore-scripts', label);
  requireIncludes(releasePreflightJob, "fetch-depth: 0", `${label} release-preflight job`);
  // Preflight lints the benchmark adapter for wasm32. The action's `rustup
  // override` bypasses rust-toolchain.toml's target list, so the job names it.
  requireIncludes(
    releasePreflightJob,
    "target: wasm32-unknown-unknown",
    `${label} release-preflight job`,
  );
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
  const macosJob = text.split("\n  macos-arm64-runtime:\n")[1]?.split("\n  portable-engine-build:\n")[0];
  if (!macosJob) fail(`${label} must execute macOS arm64 correctness, separately from cross-compilation`);
  requireIncludes(macosJob, RUNNER_CLASSES.get("general-macos").block, `${label} macos job`);
  for (const check of [
    `RUSTUP_TOOLCHAIN: ${rustChannel}`,
    "RUSTFLAGS: -D warnings",
    'cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"',
    'if [ ! -x "$cargo_bin/rustup" ]; then',
    'export PATH="$cargo_bin:$PATH"',
    'echo "$cargo_bin" >> "$GITHUB_PATH"',
    `rustup toolchain install ${rustChannel} --profile minimal --target aarch64-apple-darwin --no-self-update`,
    'test "$(uname -s)" = Darwin',
    'test "$(uname -m)" = arm64',
    'test "$(rustc -vV | sed -n \'s/^host: //p\')" = aarch64-apple-darwin',
    "bash tools/run-private-platform-golden.sh native default aarch64-apple-darwin",
    "bash tools/run-private-platform-golden.sh native bare aarch64-apple-darwin",
    "cargo test --locked -p rustia --target aarch64-apple-darwin --test host_functions --test typed_objects --test live_value_contract --test module_resolution --test yieldable_require --test callable_continuations",
    "cargo test --locked -p rustia --target aarch64-apple-darwin --no-default-features --test host_functions --test typed_objects --test live_value_contract --test module_resolution --test yieldable_require --test callable_continuations",
  ]) requireIncludes(macosJob, check, `${label} macos job`);
  for (const forbidden of ["uses: actions-rust-lang/setup-rust-toolchain@", "brew ", "sudo ", "chmod ", "chown "]) {
    forbidIncludes(macosJob, forbidden, `${label} macos job`);
  }
  for (const command of [
    "bash tools/run-private-platform-golden.sh native default x86_64-unknown-linux-gnu",
    "bash tools/run-private-platform-golden.sh native bare x86_64-unknown-linux-gnu",
  ]) requireIncludes(releasePreflightJob, command, `${label} Linux golden job`);
  for (const command of [
    "bash tools/run-private-platform-golden.sh wasm bare",
    "bash tools/run-private-platform-golden.sh wasm default",
  ]) requireIncludes(text, command, label);
  requireIncludes(text, "name: Wasm runtime (Node/V8)", label);
  requireIncludes(
    text,
    "cargo install wasm-bindgen-cli --version 0.2.126 --locked",
    label,
  );
  // The install is guarded by a version comparison so it is idempotent on a
  // runner that already has the binary. That puts the pinned version in the
  // file twice, and the two copies must agree or the guard silently stops
  // matching and every run reinstalls. Pin both literals to the same version.
  requireIncludes(text, '!= "wasm-bindgen 0.2.126"', label);
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

function checkMainBench(text) {
  const label = ".github/workflows/main-bench.yml";
  requireIncludes(text, "\"on\":\n  push:\n    branches:\n      - main\n", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "if: github.ref == 'refs/heads/main'", label);
  requireIncludes(text, "persist-credentials: false", label);
  requireIncludes(text, "timeout-minutes:", label);
  requireIncludes(text, "data_published: ${{ steps.record.outputs.data_published }}", label);
  requireIncludes(text, "      - name: Measure and publish the series row\n        id: record\n", label);
  requireIncludes(text, "  publish:\n    needs: measure\n    if: always() && needs.measure.outputs.data_published == 'true'\n    uses: ./.github/workflows/publish-benchmark-data.yml", label);
  requireBefore(text,
    'node tools/push-benchmark-data-publication.js --repo-dir "$data_dir" --kind series',
    'echo "data_published=true" >> "$GITHUB_OUTPUT"', label);
  requireBefore(text, 'echo "data_published=true" >> "$GITHUB_OUTPUT"', 'if [ "$verdict" = moved ]; then', label);
  requireIncludes(text, 'echo "host_moved=true" >> "$GITHUB_OUTPUT"', label);
  requireIncludes(text, "      - name: Report flagged host control\n        if: steps.record.outputs.host_moved == 'true'", label);
  requireIncludes(text, "flagged_history_controls", label);
  requireIncludes(text, "node tools/run-dedicated-benchmark.js main --local", label);
  // Rows are validated against the committed protocol before they are pushed,
  // so a row that does not belong to this series never reaches published data.
  requireIncludes(text, "node tools/check-benchmark-series.js --rows-root", label);
  requireIncludes(
    text,
    "node tools/push-benchmark-data-publication.js --repo-dir \"$data_dir\" --kind series",
    label,
  );
  // The data repository's main is written only by the helper, which owns the
  // fast-forward push and the bounded fetch-rebase-retry; a workflow must not
  // grow a push of its own.
  forbidIncludes(text, "git -C \"$data_dir\" push origin HEAD:main", label);
  requireIncludes(text, "trap cleanup EXIT", label);
  // The rows live in the data repository; this lane must never write history
  // into this one, and must not become a second contents-write holder.
  forbidIncludes(text, "bench/history/runs.jsonl", label);
  forbidIncludes(text, "actions/upload-artifact", label);
  forbidIncludes(text, "LINODE_TOKEN", label);
}

function checkPrBench(text) {
  const label = ".github/workflows/pr-bench.yml";
  // pull_request, never pull_request_target: the latter would run this with a
  // writable token in the base repository's context. The global ban below covers
  // it too; this is the file where it would actually be tempting.
  requireIncludes(text, '"on":\n  pull_request:\n', label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "cancel-in-progress: true", label);
  requireIncludes(text, "persist-credentials: false", label);
  requireIncludes(text, "timeout-minutes:", label);
  requireIncludes(text, "node tools/run-dedicated-benchmark.js pr", label);
  requireIncludes(text, "node tools/run-dedicated-benchmark.js calibrate", label);

  // The fork guard. The repository is private today, so this changes nothing
  // now; it is pinned because the day it starts to matter is the day the
  // repository goes public, and by then nobody will be reading this file.
  // Without it, any stranger's pull request would execute their code on the
  // machine that produces every published number.
  requireIncludes(
    text,
    "github.event.pull_request.head.repo.full_name == github.repository",
    label,
  );

  // Screening reports; it does not record. Three independent statements of that:
  // no write to either repository, nothing that reaches published history, and
  // no secret to write with.
  forbidIncludes(text, "git push", label);
  // Checkout drops this job's credentials, so any Git command that talks to the
  // remote fails on a private repository. fetch-depth 0 has already brought the
  // history down; nothing here needs the network.
  forbidIncludes(text, "git fetch", label);
  forbidIncludes(text, "bench/history", label);
  forbidIncludes(text, "--history", label);
  forbidIncludes(text, "secrets.", label);
  forbidIncludes(text, "contents: write", label);
  forbidIncludes(text, "actions/upload-artifact", label);
  // A screening run must not be able to supersede the reconstruction or promote
  // an artifact, both of which --replace does.
  forbidIncludes(text, "--replace", label);
}

function checkReleasePreflight(text) {
  requireIncludes(text, 'node("tools/check-external-facade.js");', "release preflight");
  const label = "tools/run-release-preflight.js";
  for (const needle of [
    '"git", ["diff", "--quiet", "--", "Cargo.lock"]',
    '"cargo", ["metadata", "--locked"',
    // Every committed lock file, found by listing, fetched with --locked. The
    // wasm adapter's own lock has gone stale more than once (smallvec, then
    // serde_path_to_error); the second time it stopped the weekly matrix.
    '"git", ["ls-files", "-z"]',
    "/(^|\\/)Cargo\\.lock$/",
    "for (const manifest of lockedManifests())",
    '"cargo", ["fetch", "--locked", "--manifest-path", manifest]',
    '"cargo", ["fmt", "--all"',
    '"cargo", ["clippy", "--locked", "--workspace", "--all-targets"',
    // The builds only the owned benchmark host compiles. Without these, a
    // break under their cfgs is found by the weekly matrix or a memory run,
    // after merge. The host denies warnings through the toolchain action's
    // RUSTFLAGS, so each check denies them too.
    '"clippy",\n  "--locked",\n  "--manifest-path",\n  "bench/Cargo.toml",\n  "--bin",\n  "script-bench",\n' +
      '  "--no-default-features",\n  "--features",\n  "all-engines",\n  "--",\n  "-D",\n  "warnings",\n',
    '"clippy",\n  "--locked",\n  "--manifest-path",\n  "bench/Cargo.toml",\n  "--bin",\n  "script-bench",\n' +
      '  "--no-default-features",\n  "--features",\n  "mlua-lua54",\n  "--",\n  "-D",\n  "warnings",\n',
    '"clippy",\n  "--locked",\n  "--manifest-path",\n  "bench/wasm/adapter-runner/Cargo.toml",\n' +
      '  "--target",\n  "wasm32-unknown-unknown",\n  "--",\n  "-D",\n  "warnings",\n',
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
    '"tools/run-linode-benchmark.js"',
    '"tools/run-linux-matrix-benchmark.js"',
    // The weekly matrix's landing helper. Nothing short of a real cohort
    // reaches its merge path, so its self-test is the only run it gets before
    // a forty-minute measurement depends on it.
    '"tools/land-matrix-publication.js", "self-test"',
    // The owned-host lane. Its self-tests carry the rules that keep a source
    // repair honest and a host attested, so preflight must keep running them.
    '"tools/run-dedicated-benchmark.js"',
    '"tools/check-benchmark-series.js"',
    // Reads history only; it is not a measurement. Preflight runs its self-test
    // because the rule it carries -- that a per-unit ratio needs both sides to
    // mean the same unit -- is what keeps a host comparison honest.
    '"tools/record-host-bridge.js"',
    // Its self-test is the only thing standing between a destructive edit to a
    // published dataset and a silently changed dashboard.
    '"tools/retire-cloud-history.js"',
    '"tools/run-linux-history-backfill.js"',
    '"tools/freeze-history-backfill.js"',
    // The frozen owned-host config: exact provider keys, SMT off, and the
    // timed core inside the pinned set. Every lane copies that block.
    '"tools/check-dedicated-config.js"',
    '"tools/push-benchmark-data-publication.js"',
    '"bench/run-memory-supervisor.js"',
    '"bench/analyze-memory-samples.js"',
    '"bench/run-memory-cohort.js"',
    '"bench/publish-memory-analysis.js"',
    '"tools/run-linux-memory-benchmark.js"',
    // The cross-engine memory freezer and the two tools the memory workflow
    // runs after measuring; preflight is where CI runs their self-tests.
    '"tools/create-cross-engine-memory-config.js"',
    '"tools/run-wasm-gate.js"',
    '"tools/run-wasm-dispatch-gate.js"',
    '"tools/merge-rustia-ci-memory-data.js"',
    '"tools/summarize-rustia-ci-memory.js"',
    '"tools/check-benchmark-results.js"',
    '"tools/check-wasm-gate-results.js"',
    '"tools/render-benchmark-operations-status.js", "--check"',
    '"tools/check-current-report-summary.js"',
    '"bench/render-current-results.js"',
    '"tools/check-grabby-build-results.js"',
    '"bench/render-claims.js"',
    '"cargo", ["fmt", "--all", "--manifest-path", "fuzz/Cargo.toml"',
    '"clippy",\n  "--locked",\n  "--manifest-path",\n  "fuzz/Cargo.toml"',
    '"cargo", ["test", "--locked", "--manifest-path", "fuzz/Cargo.toml"]',
    '"cargo", ["test", "--locked", "--workspace"]',
  ]) {
    requireIncludes(text, needle, label);
  }
}

// The memory lane measures a commit that may be older than main, with that
// commit's own tools. What it writes into the data repository must not pass
// through them: the merge is where a row's machine is checked, and a merge tool
// from before host labels checks nothing about it. So the merge and the push
// run from copies of main's tools, taken while the tree is still main, and a
// commit whose analyzer lacks the host-label line is refused right after the
// detach, before the measurement takes the host.
//
// These rules report rather than exit, so the tripwire in
// checkTrustedMemoryPublication can break each one and watch it fire.
const TRUSTED_TOOLS_DIR = '          trusted_tools="$RUNNER_TEMP/rustia-trusted-tools"';
const TRUSTED_TOOLS_MKDIR = '          install -d -m 700 "$trusted_tools"';
const TRUSTED_MEMORY_TOOLS = Object.freeze([
  {
    name: "merge-rustia-ci-memory-data.js",
    invocation: [
      '          node "$trusted_tools/merge-rustia-ci-memory-data.js" \\',
      "            --generated-current bench/memory/current-results.json \\",
      "            --generated-history bench/memory/history/runs.jsonl \\",
      '            --data-root "$data_dir"',
    ],
  },
  {
    name: "push-benchmark-data-publication.js",
    invocation: ['            node "$trusted_tools/push-benchmark-data-publication.js" --repo-dir "$data_dir" --kind memory'],
  },
]);
const MEMORY_DETACH = '          git checkout --detach "$BENCHMARK_COMMIT"';
const MEMORY_MEASURE = "            node tools/run-dedicated-benchmark.js memory --local";
// Carried verbatim by every file that emits or validates a host label, and
// asserted in bench/analyze-memory-samples.js by tools/check-benchmark-results.js
// since the commit that confined labels to this character set. A commit whose
// copy of that file lacks the line predates that rule, so the hosts its rows
// carry need not be labels: the two commits before it that stamped a host
// allowed dots.
const HOST_LABEL_LINE = "const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;";
const HOST_LABEL_GATE = `          if ! grep -qxF '${HOST_LABEL_LINE}' bench/analyze-memory-samples.js; then`;

function trustedCopy(tool) {
  return `          install -m 644 tools/${tool.name} "$trusted_tools/${tool.name}"`;
}

function memoryPublicationProblems(text) {
  const lines = text.split("\n");
  const indexes = (line) => lines.flatMap((candidate, index) => (candidate === line ? [index] : []));
  const only = (line) => (indexes(line).length === 1 ? indexes(line)[0] : -1);
  const detach = only(MEMORY_DETACH);
  const measure = lines.indexOf(MEMORY_MEASURE);
  const publishStep = only("      - name: Publish measured memory data only");
  if (
    detach === -1 ||
    lines[detach + 1] !== '          test "$(git rev-parse HEAD)" = "$BENCHMARK_COMMIT"' ||
    !(detach < measure && measure < publishStep)
  ) {
    return ["must detach at the measured commit once, check HEAD on the next line, then measure, then publish"];
  }
  const problems = [];
  // Each step is its own shell, so the directory is named twice: once where
  // the copies are taken and once where they run. A third assignment could
  // point the second at something else.
  const assignments = lines.flatMap((line, index) => (/\btrusted_tools=/.test(line) ? [index] : []));
  if (
    assignments.length !== 2 ||
    assignments.some((index) => lines[index] !== TRUSTED_TOOLS_DIR) ||
    !(assignments[0] < detach && publishStep < assignments[1])
  ) {
    problems.push(
      `must set ${JSON.stringify(TRUSTED_TOOLS_DIR.trim())} once before the detach and once in the publish step, ` +
        "and nowhere else",
    );
  }
  const mkdir = only(TRUSTED_TOOLS_MKDIR);
  // Per tool, the rule naming the likeliest mistake comes first, since the
  // first problem is the one a failing run prints.
  for (const tool of TRUSTED_MEMORY_TOOLS) {
    const historical = `node tools/${tool.name}`;
    if (text.includes(historical)) {
      problems.push(
        `must not include ${JSON.stringify(historical)}: after the detach that runs the measured commit's copy, ` +
          "not main's",
      );
    }
    // Any other spelling of the same thing -- ./tools/, the workspace path, a
    // cd first -- runs the measured commit's copy too.
    lines.forEach((line, index) => {
      if (!line.includes(tool.name) || line.trim().startsWith("#")) return;
      if (line === trustedCopy(tool) || line === tool.invocation[0]) return;
      if (line.includes(historical)) return;
      problems.push(`line ${index + 1} names ${tool.name} outside main's copy and its trusted invocation: ${line.trim()}`);
    });
    // Exactly once: a second copy after the detach would replace main's tool
    // with the measured commit's, and the invocation would never know.
    const copy = only(trustedCopy(tool));
    if (!(assignments[0] < mkdir && mkdir < copy && copy < detach)) {
      problems.push(
        `must copy main's ${tool.name} exactly once, before the detach, while the tree is still main: ` +
          JSON.stringify(trustedCopy(tool).trim()),
      );
    }
    const run = lines.findIndex((line, index) => index > publishStep && line === tool.invocation[0]);
    const block = lines.slice(run, run + tool.invocation.length).join("\n");
    if (!(assignments[1] < run) || block !== tool.invocation.join("\n")) {
      problems.push(
        `must run main's copy of ${tool.name} in the publish step, with the same flags: ` +
          JSON.stringify(tool.invocation.map((line) => line.trim()).join(" ")),
      );
    }
  }
  // The copies are main's only while the pinned detach is the one thing that
  // moves the tree. A switch, reset or rebase before them, or a second checkout
  // step, would put the measured commit in place first, and they would copy its
  // tools. This is a lint over single lines against the ways a workflow edit
  // would plausibly move the tree, not a proof that nothing can: the workflow
  // is reviewed code, and a line built to slip past a pattern is caught there.
  lines.forEach((line, index) => {
    if (line === MEMORY_DETACH || line.trim().startsWith("#")) return;
    if (/\bgit\b.*\b(?:checkout|switch|reset|restore|read-tree|worktree|stash|rebase|pull|cherry-pick|bisect)\b/.test(line)) {
      problems.push(`line ${index + 1} moves the working tree outside the pinned detach: ${line.trim()}`);
    }
  });
  if ((text.match(/\buses: actions\/checkout@/g) ?? []).length !== 1) {
    problems.push("must check the repository out once, at main, with a single actions/checkout step");
  }
  // The trusted directory is written by the pinned copies alone and read by the
  // pinned invocations alone. Any other line naming it -- a bulk copy, an rsync,
  // a symlink to the checkout -- could fill it with the measured commit's tools.
  const trustedLines = new Set([
    TRUSTED_TOOLS_DIR,
    TRUSTED_TOOLS_MKDIR,
    ...TRUSTED_MEMORY_TOOLS.flatMap((tool) => [trustedCopy(tool), tool.invocation[0]]),
  ]);
  lines.forEach((line, index) => {
    if (trustedLines.has(line) || line.trim().startsWith("#")) return;
    if (/rustia-trusted|\$\{?trusted_tools\b/.test(line)) {
      problems.push(`line ${index + 1} names the trusted tools directory outside the pinned copies and runs: ${line.trim()}`);
    }
  });
  // After the detach, because before it the tree is main and the gate passes
  // every commit; before the measurement, because that is what it spares.
  const gate = only(HOST_LABEL_GATE);
  if (
    !(detach < gate && gate < measure) ||
    !lines[gate + 1]?.startsWith('            echo "::error::') ||
    lines[gate + 2] !== "            exit 1" ||
    lines[gate + 3] !== "          fi"
  ) {
    problems.push(
      "must refuse, between the detach and the measurement, a commit whose analyzer lacks the host-label line: " +
        `${JSON.stringify(HOST_LABEL_GATE.trim())}, then an ::error:: and exit 1`,
    );
  }
  return problems;
}

// A copy runs from $RUNNER_TEMP with nothing beside it, so a relative require,
// or a path built from the file's own location, would resolve there and fail at
// the merge -- after the measurement has taken the host. Each tool the memory
// lane copies may therefore load only node: builtins and locate nothing
// relative to itself.
function selfContainedProblems(source, file) {
  const problems = [];
  const loads = source.match(/\b(?:require|import)\s*\(/g) ?? [];
  const builtins = source.match(/\brequire\("node:[a-z_\/]+"\)/g) ?? [];
  if (loads.length !== builtins.length) {
    problems.push(
      `${file} must load only node: builtins; the memory lane runs a copy of it from $RUNNER_TEMP, ` +
        "where nothing else resolves",
    );
  }
  for (const name of ["__dirname", "__filename"]) {
    if (source.includes(name)) {
      problems.push(
        `${file} must not use ${name}; the memory lane runs a copy of it from $RUNNER_TEMP, ` +
          "away from the repository",
      );
    }
  }
  return problems;
}

// The gate greps main's descendants for a whole line, so main must keep it
// whole: indented, or with a trailing comment, it would refuse every commit.
function hostLabelLineProblems(source) {
  if (source.split(/\r?\n/).includes(HOST_LABEL_LINE)) return [];
  return [
    `bench/analyze-memory-samples.js must carry ${JSON.stringify(HOST_LABEL_LINE)} as a whole line; ` +
      "the memory lane's gate greps for exactly that line and would refuse every commit without it",
  ];
}

function checkTrustedMemoryPublication(text, label) {
  const tools = TRUSTED_MEMORY_TOOLS.map((tool) => [tool.name, readWorkflow(path.join(root, "tools", tool.name))]);
  const analyzer = readWorkflow(path.join(root, "bench", "analyze-memory-samples.js"));
  const problemsOf = ({ workflow = text, sources = tools, analyzerSource = analyzer }) => [
    ...memoryPublicationProblems(workflow).map((problem) => `${label} ${problem}`),
    ...sources.flatMap(([name, source]) => selfContainedProblems(source, `tools/${name}`)),
    ...hostLabelLineProblems(analyzerSource),
  ];
  const problems = problemsOf({});
  if (problems.length !== 0) fail(problems[0]);

  // Tripwire: break the reviewed files one way at a time and require each
  // break to be refused by the rule written for it, so a rule that has quietly
  // stopped matching fails here rather than on the dispatch it existed for.
  const lines = text.split("\n");
  // A missing line must stop the tripwire, not become splice(-1), which would
  // quietly break the last line of the file instead.
  const at = (copy, line) => {
    const index = copy.indexOf(line);
    if (index === -1) fail(`tripwire could not find ${JSON.stringify(line.trim())} to break`);
    return index;
  };
  const edit = (change) => {
    const copy = [...lines];
    change(copy);
    return copy.join("\n");
  };
  const takeGate = (copy) => copy.splice(at(copy, HOST_LABEL_GATE), 4);
  const [merge, push] = TRUSTED_MEMORY_TOOLS;
  const replaceLine = (from, to) => edit((copy) => copy.splice(at(copy, from), 1, to));
  const withSource = (name, change) => tools.map(([file, source]) => [file, file === name ? change(source) : source]);
  const cases = [
    ["a workflow that never detaches", { workflow: edit((copy) => copy.splice(at(copy, MEMORY_DETACH), 1)) },
      "must detach at the measured commit once"],
    ["no host-label gate", { workflow: edit(takeGate) }, "lacks the host-label line"],
    ["the gate before the detach, where it reads main's file", {
      workflow: edit((copy) => {
        const gate = takeGate(copy);
        copy.splice(at(copy, MEMORY_DETACH), 0, ...gate);
      }),
    }, "lacks the host-label line"],
    ["the gate after the measurement it exists to spare", {
      workflow: edit((copy) => {
        const gate = takeGate(copy);
        copy.splice(at(copy, MEMORY_MEASURE) + 1, 0, ...gate);
      }),
    }, "lacks the host-label line"],
    ["a gate that reports but does not stop", {
      workflow: edit((copy) => copy.splice(at(copy, HOST_LABEL_GATE) + 2, 1)),
    }, "lacks the host-label line"],
    ["the merge tool copied after the detach", {
      workflow: edit((copy) => {
        copy.splice(at(copy, trustedCopy(merge)), 1);
        copy.splice(at(copy, MEMORY_DETACH) + 2, 0, trustedCopy(merge));
      }),
    }, `must copy main's ${merge.name} exactly once`],
    ["a second copy of the merge tool after the detach", {
      workflow: edit((copy) => copy.splice(at(copy, MEMORY_DETACH) + 2, 0, trustedCopy(merge))),
    }, `must copy main's ${merge.name} exactly once`],
    ["the push helper never copied", { workflow: edit((copy) => copy.splice(at(copy, trustedCopy(push)), 1)) },
      `must copy main's ${push.name} exactly once`],
    ["the trusted directory pointed elsewhere", {
      workflow: edit((copy) => copy.splice(copy.lastIndexOf(TRUSTED_TOOLS_DIR) + 1, 0, "          trusted_tools=tools")),
    }, "must set"],
    ["the measured commit's merge tool", {
      workflow: replaceLine(merge.invocation[0], `          node tools/${merge.name} \\`),
    }, `must not include "node tools/${merge.name}"`],
    ["the measured commit's push helper", {
      workflow: replaceLine(push.invocation[0], `            node tools/${push.name} --repo-dir "$data_dir" --kind memory`),
    }, `must not include "node tools/${push.name}"`],
    ["the measured commit's merge tool under another spelling", {
      workflow: replaceLine(merge.invocation[0], `          node ./tools/${merge.name} \\`),
    }, `names ${merge.name} outside main's copy`],
    ["the merge run without its data root", {
      workflow: replaceLine(merge.invocation[3], "            --data-root ."),
    }, `must run main's copy of ${merge.name} in the publish step, with the same flags`],
    // Each way the review found to fill the trusted directory with the measured
    // commit's tools while every pinned line stayed where it was.
    ...[
      '          git switch --detach "$BENCHMARK_COMMIT"',
      '          git checkout -q "$BENCHMARK_COMMIT"',
      '          git rebase --onto "$BENCHMARK_COMMIT" HEAD',
      '          git pull --ff-only origin "$BENCHMARK_COMMIT"',
    ].map((line) => [`${line.trim()} before the copies`, {
      workflow: edit((copy) => copy.splice(at(copy, TRUSTED_TOOLS_DIR), 0, line)),
    }, "moves the working tree outside the pinned detach"]),
    ["a second checkout step at the measured commit", {
      workflow: edit((copy) => copy.splice(at(copy, "      - name: Validate and select benchmark commit"), 0,
        "      - name: Checkout measured commit",
        "        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
        "        with:",
        "          ref: ${{ inputs.commit_sha }}",
        "")),
    }, "with a single actions/checkout step"],
    ...[
      ["a bulk copy after the detach", MEMORY_DETACH, 2, '          cp tools/*.js "$trusted_tools/"'],
      ["an rsync in the measure step", MEMORY_MEASURE, 0, '            rsync -a tools/ "$RUNNER_TEMP/rustia-trusted-tools/"'],
      ["a symlink to the checkout after the detach", MEMORY_DETACH, 2,
        '          ln -sfn "$PWD/tools" "$RUNNER_TEMP/rustia-trusted-tools"'],
      ["a copy into the directory by a glob after the detach", MEMORY_DETACH, 2,
        '          cp tools/*.js "$RUNNER_TEMP"/rustia-trusted-*/'],
    ].map(([description, anchor, offset, line]) => [description, {
      workflow: edit((copy) => copy.splice(at(copy, anchor) + offset, 0, line)),
    }, "names the trusted tools directory outside the pinned copies and runs"]),
    ["a copied tool with a relative require", {
      sources: withSource(merge.name, (source) => `${source}\nrequire("./lib");\n`),
    }, `tools/${merge.name} must load only node: builtins`],
    ["a copied tool that locates files beside itself", {
      sources: withSource(push.name, (source) => `${source}\nconst here = __dirname;\n`),
    }, `tools/${push.name} must not use __dirname`],
    ["the label line indented in the analyzer the gate reads", {
      analyzerSource: analyzer.replace(HOST_LABEL_LINE, () => `  ${HOST_LABEL_LINE}`),
    }, "as a whole line"],
  ];
  for (const [description, change, reason] of cases) {
    if (
      (change.workflow ?? text) === text &&
      (change.sources ?? tools) === tools &&
      (change.analyzerSource ?? analyzer) === analyzer
    ) {
      fail(`tripwire could not build ${description}: the reviewed shape it breaks is gone`);
    }
    const refusals = problemsOf(change);
    if (!refusals.some((problem) => problem.includes(reason))) {
      fail(
        `tripwire: ${description} was not refused for that reason` +
          (refusals.length === 0 ? "; it was accepted" : `; refused instead with: ${refusals[0]}`),
      );
    }
  }
}

function checkRustiaMemory(text) {
  const label = ".github/workflows/rustia-memory.yml";
  requireIncludes(text, "name: Rustia Memory Cohort\n", label);
  requireIncludes(text, "\"on\":\n  workflow_dispatch:\n", label);
  requireIncludes(text, "commit_sha:\n", label);
  requireIncludes(text, "required: true\n", label);
  requireIncludes(text, "type: string\n", label);
  // The cohort is a closed choice with the routine cohort first and as the
  // default, so a dispatch that says nothing measures 11 rows, not 110.
  requireIncludes(text, "cohort:\n", label);
  requireIncludes(text, "default: rustia-only\n", label);
  requireIncludes(text, "type: choice\n", label);
  requireIncludes(text, "options:\n          - rustia-only\n          - cross-engine\n", label);
  requireIncludes(text, "COHORT: ${{ inputs.cohort }}", label);
  requireIncludes(text, "permissions:\n  contents: read\n", label);
  requireIncludes(text, "if: github.ref == 'refs/heads/main'", label);
  requireIncludes(text, "ref: main", label);
  requireIncludes(text, "BENCHMARK_COMMIT: ${{ inputs.commit_sha }}", label);
  requireIncludes(text, "^[0-9a-f]{40}$", label);
  requireIncludes(text, 'git cat-file -e "${BENCHMARK_COMMIT}^{commit}"', label);
  requireIncludes(text, 'git merge-base --is-ancestor "$BENCHMARK_COMMIT" origin/main', label);
  requireIncludes(text, 'test "$(git rev-parse HEAD)" = "$BENCHMARK_COMMIT"', label);
  // Measurement only through the driver that attests the machine first. The
  // measured commit's own tools run it, so the routine cohort keeps the bare
  // line for the labelled commits that predate the --cohort flag; commits from
  // before a row's host was confined to a label are refused before it. An empty
  // cohort input is the routine cohort.
  requireIncludes(text, 'if [ "${COHORT:-rustia-only}" = rustia-only ]; then\n', label);
  requireIncludes(text, "node tools/run-dedicated-benchmark.js memory --local\n", label);
  requireIncludes(text, 'node tools/run-dedicated-benchmark.js memory --local --cohort "$COHORT"\n', label);
  requireIncludes(text, "node tools/summarize-rustia-ci-memory.js", label);
  requireIncludes(text, "BENCH_DATA_DEPLOY_KEY: ${{ secrets.BENCH_DATA_DEPLOY_KEY }}", label);
  requireIncludes(text, "git@github.com:slowchop/rustia-bench-data.git", label);
  // Merging into the data repository and pushing there run main's copies of
  // those tools, never the measured commit's, and a commit from before a row's
  // host was confined to a label is refused before it takes the host.
  checkTrustedMemoryPublication(text, label);
  // The data repository's main is written only by the helper, which owns the
  // fast-forward push and the bounded fetch-rebase-retry; a workflow must not
  // grow a push of its own.
  forbidIncludes(text, "git -C \"$data_dir\" push origin HEAD:main", label);
  requireIncludes(text, 'git -C "$data_dir" add bench/memory\n', label);
  requireIncludes(text, "  publish:\n    needs: benchmark\n    uses: ./.github/workflows/publish-benchmark-data.yml\n    permissions:\n      contents: read\n    secrets:\n      BENCH_DATA_DEPLOY_KEY: ${{ secrets.BENCH_DATA_DEPLOY_KEY }}", label);
  forbidIncludes(text, "gh-pages", label);
  forbidIncludes(text, "render-current-results", label);
  forbidIncludes(text, "bench/current-results.html", label);
  forbidIncludes(text, "actions: write", label);
  // This was the last lane that could create a billed instance. These are the
  // strings that would bring that back.
  forbidIncludes(text, "LINODE_TOKEN", label);
  forbidIncludes(text, "run-linode-benchmark.js", label);
  forbidIncludes(text, ".rustia-benchmark-state/linode", label);
  forbidIncludes(text, "actions/upload-artifact", label);
  forbidIncludes(text, "retention-days:", label);
  forbidIncludes(text, "pull_request_target", label);
  forbidIncludes(text, "workflow_run", label);
  forbidIncludes(text, "contents: write", label);
  checkActionPins(text, label);
}

function checkWeeklyMatrix(text) {
  const label = ".github/workflows/weekly-matrix-bench.yml";
  requireIncludes(text, "name: Cross-Engine Matrix (weekly)\n", label);
  // Weekly, on the owned host, at an hour it is idle; dispatch stays for an
  // out-of-cycle cohort. The schedule was withheld until the first owned-host
  // cohort had landed and been inspected. It is pinned to one weekly slot so a
  // faster cadence cannot slip in: every run occupies the bench host for forty
  // minutes and lands a cohort in this repository's history.
  requireIncludes(text, "\"on\":\n  schedule:\n    - cron: \"0 15 * * 0\"\n  workflow_dispatch:\n", label);
  // The single sanctioned contents-write grant: this workflow may land accepted
  // matrix cohorts on main and nothing else may. The per-commit series writes to
  // the data repository precisely so that it does not need a second one. main
  // accepts changes only through pull requests, which this token cannot bypass,
  // so the cohort goes to a branch and is merged by the landing helper; a direct
  // push to main is the shape the branch rules reject. pull-requests: write is
  // kept while the app identity below is still unproven on a real landing.
  requireIncludes(text, "permissions:\n  contents: write\n  actions: write\n  pull-requests: write\n", label);
  requireIncludes(text, "node tools/land-matrix-publication.js", label);
  // The helper merges only the commit this run pushed, and only while main is
  // still the commit the publication was rendered on. It refuses to start
  // without both, which would otherwise surface only after a full cohort.
  requireIncludes(text, "--base \"$base_commit\"", label);
  requireIncludes(text, "--head \"$publication_commit\"", label);
  requireIncludes(text, "git push --force origin \"HEAD:refs/heads/$branch\"", label);
  forbidIncludes(text, "HEAD:main", label);
  forbidIncludes(text, ":refs/heads/main", label);
  // A comment names a command without running it, so the checks that say what
  // the step runs, and where, read the file without its comment lines.
  const code = text.split("\n").filter((line) => !/^\s*#/.test(line)).join("\n");
  // The landing merge is the thing a ruleset bypass can be granted to, so it
  // must be made as the slowchop-ops-controller app and not with the workflow
  // token: github-actions[bot] cannot be a bypass actor, and without the app
  // the CI-skip marker above would leave any required check pending for good.
  requireIncludes(text, "app-id: ${{ secrets.OPS_CONTROLLER_APP_ID }}", label);
  requireIncludes(text, "private-key: ${{ secrets.OPS_CONTROLLER_PRIVATE_KEY }}", label);
  requireIncludes(text, "RUSTIA_LANDING_TOKEN: ${{ steps.landing-token.outputs.token }}", label);
  // Scoped to the one command. The dispatch curls keep the workflow token,
  // because the app holds actions: read and dispatching a workflow needs write.
  requireIncludes(code, "GITHUB_TOKEN=\"$RUSTIA_LANDING_TOKEN\" \\\n            node tools/land-matrix-publication.js", label);
  // An installation token lives one hour and a cohort takes about forty
  // minutes, so it is minted after the measurement, never at job start.
  requireBefore(code, "node tools/run-dedicated-benchmark.js matrix --local", "create-github-app-token", label);
  // The publication commit carries the skip marker, and it is the only thing in
  // the file that may, comments included: the title becomes the body of main's
  // merge commit, so main's tip stays unmarked. The marker is why the app needs
  // a ruleset bypass -- a check that never reports would block the merge for
  // good -- and it dates from a landing made with the workflow token, whose
  // pull-request runs were held for an approval the merge never gave. A person
  // merging by hand would push that commit, and a marker on main would skip CI,
  // the series and the dashboard publication.
  requireIncludes(code, 'git commit -m "Publish cross-engine matrix ${RUSTIA_MATRIX_COMMIT} [skip ci]"\n', label);
  requireIncludes(code, '--title "Publish cross-engine matrix ${RUSTIA_MATRIX_COMMIT}" || landing=$?\n', label);
  const skipMarkers = text.match(SKIP_MARKER) || [];
  if (skipMarkers.length !== 1) {
    fail(`${label} must carry exactly one CI-skip marker, on the publication commit; found ${skipMarkers.length}`);
  }
  // The dashboard publication and CI on main's new tip are dispatched once the
  // cohort has landed. An app-token merge starts push workflows, so each may
  // duplicate a run the merge already created; both tolerate it, and they stay
  // until a landing has shown which runs arrive by themselves. Each is
  // attempted whatever happens to the other, and a failure fails the job, so
  // main's new tip is never left without CI behind a green run. The per-commit
  // series is still not dispatched: the push already reaches it, and its
  // byte-identical gate records nothing for a publication.
  requireIncludes(code, WEEKLY_MATRIX_DISPATCHES, label);
  requireBefore(code, "nothing was landed\" >&2\n            exit 1\n          fi\n", WEEKLY_MATRIX_DISPATCHES, label);
  forbidIncludes(text, "main-bench.yml/dispatches", label);
  requireIncludes(text, "publish-benchmark-data.yml/dispatches", label);
  requireIncludes(text, "if: github.ref == 'refs/heads/main'", label);
  requireIncludes(text, "ref: main", label);
  requireIncludes(text, "toolchain: 1.95.0", label);
  requireIncludes(text, "target: wasm32-unknown-unknown", label);
  requireIncludes(text, "cache: false", label);
  // Measurement is reachable only through the driver that attests the machine.
  requireIncludes(text, "node tools/run-dedicated-benchmark.js matrix --local", label);
  requireIncludes(text, "node tools/register-matrix-cohort.js", label);
  requireIncludes(text, "node tools/check-benchmark-results.js", label);
  requireIncludes(text, "node tools/check-package-files.js", label);
  requireIncludes(text, "node tools/check-publishability.js", label);
  // The preflight checkers whose result a cohort can change and that the step
  // did not otherwise run. They run inside the retry loop, after the last render
  // and just before the publication is staged and committed, so they check the
  // tree that lands and a cohort that would turn main red is never landed.
  requireIncludes(
    code,
    "            node tools/check-benchmark-results.js\n" +
      "            node tools/check-wasm-gate-results.js\n" +
      "            node tools/check-project-status.js\n" +
      "            git add bench design tools/check-benchmark-results.js\n" +
      '            git commit -m "Publish cross-engine matrix ${RUSTIA_MATRIX_COMMIT} [skip ci]"\n',
    label,
  );
  // Nothing here may start, pay for, or authenticate to a cloud instance. The
  // lane that did is gone, and these are the strings that would bring it back.
  forbidIncludes(text, "LINODE_TOKEN", label);
  forbidIncludes(text, "run-linode-benchmark.js", label);
  forbidIncludes(text, ".rustia-benchmark-state/linode", label);
  // The progress gate asked the Rustia question, which the per-commit series
  // now answers. Running unconditionally is also what makes host drift visible.
  forbidIncludes(text, "pending_relevant_commit_count", label);
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
  requireIncludes(text, "workflow_call:\n    secrets:\n      BENCH_DATA_DEPLOY_KEY:\n        required: true", label);
  requireIncludes(text, "group: rustia-benchmark-data-publication", label);
  requireIncludes(text, "bench/grabby-builds/**", label);
  requireIncludes(text, "node tools/check-grabby-build-results.js", label);
  requireIncludes(text, "bench/grabby-builds/results.json \\", label);
  requireIncludes(text, 'install -m 644 bench/grabby-builds/results.json "$pages_dir/grabby-builds.json"', label);
  requireIncludes(text, 'git -C "$pages_dir" add index.html grabby-builds.json', label);
  requireIncludes(text, 'install -m 644 "$data_dir/bench/history/series/ci-bench-main.jsonl"', label);
  requireIncludes(text, 'install -m 644 "$data_dir/bench/history/series/ci-bench-host-controls.jsonl"', label);
  requireIncludes(text, 'install -m 644 "$data_dir/bench/memory/current-results.json" bench/memory/current-results.json', label);
  requireIncludes(text, 'install -m 644 "$data_dir/bench/memory/history/runs.jsonl" bench/memory/history/runs.jsonl', label);
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
  requireIncludes(
    text,
    "node tools/push-benchmark-data-publication.js --repo-dir \"$data_dir\" --kind artifacts",
    label,
  );
  // The data repository's main is written only by the helper, which owns the
  // fast-forward push and the bounded fetch-rebase-retry; a workflow must not
  // grow a push of its own.
  forbidIncludes(text, "git -C \"$data_dir\" push origin HEAD:main", label);
  requireIncludes(text, "git -C \"$pages_dir\" push origin HEAD:gh-pages", label);
  for (const forbidden of [
    "LINODE_TOKEN",
    "run-linode-benchmark.js bench",
    "workflow_run",
    "pull_request_target",
    "contents: write",
  ]) forbidIncludes(text, forbidden, label);
  checkActionPins(text, label);  // Validating the rows is not the same as drawing them: the renderer reads
  // them from the checkout, so they must be installed before it runs.
  requireIncludes(text, "install -m 644 \"$data_dir/bench/history/series/ci-bench-main.jsonl\"", label);

}

// Embedded policy checks may supply a minimal process object.
if (process.argv?.includes("--self-test")) {
  const os = require("node:os");
  const assert = require("node:assert/strict");
  const { spawnSync } = require("node:child_process");
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-workflow-policy-"));
  try {
    fs.cpSync(path.join(root, "tools"), path.join(temporary, "tools"), { recursive: true, filter: source => path.basename(source) !== "node_modules" });
    fs.cpSync(workflowDir, path.join(temporary, ".github", "workflows"), { recursive: true });
    fs.mkdirSync(path.join(temporary, "bench"));
    fs.copyFileSync(path.join(root, "bench/analyze-memory-samples.js"), path.join(temporary, "bench/analyze-memory-samples.js"));
    fs.copyFileSync(rustToolchainPath, path.join(temporary, "rust-toolchain.toml"));
    const canonical = readWorkflow(ciPath);
    const macosBlock = RUNNER_CLASSES.get("general-macos").block;
    const linuxBlock = RUNNER_CLASSES.get("general-linux").block;
    const run = (text) => {
      fs.writeFileSync(path.join(temporary, ".github/workflows/ci.yml"), text);
      return spawnSync(process.execPath, [path.join(temporary, "tools/check-workflows.js")], { encoding: "utf8" });
    };
    const baseline = run(canonical);
    assert.equal(baseline.status, 0, baseline.stderr);
    for (const [label, changed, message] of [
      ["runner context in job env", canonical.replace(/^    env:\n/m, "    env:\n      INVALID_PATH: ${{ runner.temp }}\n"), /job env uses a context/],
      ["Mac job in Linux pool", canonical.replace(macosBlock, linuxBlock), /job macos-arm64-runtime.*general-macos/],
      ["Linux job in Mac pool", canonical.replace(linuxBlock, macosBlock), /job release-preflight.*general-linux/],
      ["unregistered Mac job", canonical.replace("  macos-arm64-runtime:", "  another-macos-job:"), /job another-macos-job.*general-linux/],
      ["missing architecture check", canonical.replace('test "$(uname -m)" = arm64', "true"), /must include.*uname/],
      ["unpinned Mac toolchain", canonical.replace(`RUSTUP_TOOLCHAIN: ${pinnedRustChannel()}`, "RUSTUP_TOOLCHAIN: stable"), /must include.*RUSTUP_TOOLCHAIN/],
      ["Mac package manager mutation", canonical.replace("      - name: Verify native arm64", "      - run: brew install bash\n      - name: Verify native arm64"), /must not include.*brew/],
      ["missing user Cargo path", canonical.replace('export PATH="$cargo_bin:$PATH"', "true"), /must include.*export PATH/],
      ["missing Mac golden target", canonical.replace("native default aarch64-apple-darwin", "native default"), /must include.*aarch64-apple-darwin/],
      ["missing Mac suite target", canonical.replace("--target aarch64-apple-darwin --test host_functions", "--test host_functions"), /must include.*aarch64-apple-darwin/],
      ["missing Linux golden target", canonical.replace("native default x86_64-unknown-linux-gnu", "native default"), /must include.*x86_64-unknown-linux-gnu/],
      ["missing shared Wasm golden", canonical.replace("bash tools/run-private-platform-golden.sh wasm default", "true"), /must include.*private-platform-golden/],
      ["timing on shared executor", canonical + "\n# node tools/run-dedicated-benchmark.js\n", /must not include.*run-dedicated-benchmark/],
    ]) {
      assert.notEqual(changed, canonical, `${label}: mutation did not apply`);
      const result = run(changed);
      assert.notEqual(result.status, 0, `${label}: invalid workflow passed`);
      assert.match(result.stderr, message, `${label}: failed for the wrong reason`);
    }
    fs.writeFileSync(path.join(temporary, ".github/workflows/ci.yml"), canonical);
    const mainPath = path.join(temporary, ".github/workflows/main-bench.yml");
    const publicationPath = path.join(temporary, ".github/workflows/publish-benchmark-data.yml");
    const mainWorkflow = fs.readFileSync(mainPath, "utf8");
    const publicationWorkflow = fs.readFileSync(publicationPath, "utf8");
    const acceptancePath = path.join(temporary, ".github/workflows/acceptance-bench.yml");
    const acceptanceWorkflow = fs.readFileSync(acceptancePath, "utf8");
    for (const [label, file, original, changed] of [
      ["publication before saved rows", mainPath, mainWorkflow, mainWorkflow.replace('            echo "data_published=true" >> "$GITHUB_OUTPUT"\n', '').replace('            node tools/push-benchmark-data-publication.js --repo-dir "$data_dir" --kind series', '            echo "data_published=true" >> "$GITHUB_OUTPUT"\n            node tools/push-benchmark-data-publication.js --repo-dir "$data_dir" --kind series')],
      ["drift failure hides publication", mainPath, mainWorkflow, mainWorkflow.replace("if: always() && needs.measure.outputs.data_published == 'true'", "if: success() && needs.measure.outputs.data_published == 'true'")],
      ["publication without push confirmation", mainPath, mainWorkflow, mainWorkflow.replace("if: always() && needs.measure.outputs.data_published == 'true'", "if: always()")],
      ["missing control ledger", publicationPath, publicationWorkflow, publicationWorkflow.replace('install -m 644 "$data_dir/bench/history/series/ci-bench-host-controls.jsonl"', 'true # omitted controls')],
      ["failed acceptance loses evidence", acceptancePath, acceptanceWorkflow, acceptanceWorkflow.replace("if: ${{ always() }}", "if: ${{ success() }}")],
      ["acceptance loses raw exports", acceptancePath, acceptanceWorkflow, acceptanceWorkflow.replace("            .rustia-benchmark-state/dedicated/runs/accept-*/export.tar.gz\n", "")],
    ]) {
      assert.notEqual(changed, original, `${label}: mutation did not apply`);
      fs.writeFileSync(file, changed);
      const result = spawnSync(process.execPath, [path.join(temporary, "tools/check-workflows.js")], { encoding: "utf8" });
      fs.writeFileSync(file, original);
      assert.notEqual(result.status, 0, `${label}: invalid workflow passed`);
      assert.match(result.stderr, /must include|must be an approved action/, `${label}: failed for the wrong reason`);
    }
    const fakeBin = path.join(temporary, "fake-bin");
    fs.mkdirSync(fakeBin);
    fs.writeFileSync(path.join(fakeBin, "cargo"), [
      "#!/usr/bin/env bash",
      '[[ " $* " = *" --target aarch64-apple-darwin "* ]] || exit 23',
      'case "$RUSTIA_GOLDEN_TEST_CASE" in',
      "  zero) exit 0 ;;",
      "  failure) exit 19 ;;",
      "esac",
      "echo 'RUSTIA_PRIVATE_PLATFORM_GOLDEN fixture'",
      'if [[ "$RUSTIA_GOLDEN_TEST_CASE" = duplicate ]]; then',
      "  echo 'RUSTIA_PRIVATE_PLATFORM_GOLDEN fixture'",
      "fi", "",
    ].join("\n"), { mode: 0o755 });
    for (const scenario of ["one", "zero", "duplicate", "failure"]) {
      const result = spawnSync("bash", [path.join(temporary, "tools/run-private-platform-golden.sh"), "native", "default", "aarch64-apple-darwin"], {
        encoding: "utf8",
        env: { ...process.env, PATH: `${fakeBin}${path.delimiter}${process.env.PATH}`, RUSTIA_GOLDEN_TEST_CASE: scenario },
      });
      if (scenario === "one") assert.equal(result.status, 0, result.stderr);
      else assert.notEqual(result.status, 0, `golden runner accepted ${scenario}`);
    }
    console.log("workflow policy self-tests: canonical, nineteen policy rejections, and four explicit-target golden-execution cases pass");
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
  process.exit(0);
}

const files = workflowFiles();
for (const required of [
  "ci.yml",
  "main-bench.yml",
  "pr-bench.yml",
  "rustia-memory.yml",
  "weekly-matrix-bench.yml",
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
checkBenchHostQueue(files);
checkCi(readWorkflow(ciPath));
checkRustiaMemory(readWorkflow(rustiaMemoryPath));
checkWeeklyMatrix(readWorkflow(weeklyMatrixPath));
checkBenchmarkPublication(readWorkflow(benchmarkPublicationPath));
checkMainBench(readWorkflow(path.join(workflowDir, "main-bench.yml")));
checkPrBench(readWorkflow(path.join(workflowDir, "pr-bench.yml")));
checkAcceptanceEvidence(readWorkflow(path.join(workflowDir, "acceptance-bench.yml")));
checkReleasePreflight(readWorkflow(releasePreflightPath));

console.log(`workflow policy ok for ${files.length} workflow file(s)`);
