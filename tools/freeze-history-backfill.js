#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const WORKLOADS = Object.freeze([
  "dense_tables",
  "sparse_tables",
  "closures_upvalues",
  "generic_iteration",
  "stdlib_math_table",
]);
const ENGINE_PATHS = Object.freeze([
  "crates/rustia-compiler/src",
  "crates/rustia-compiler/Cargo.toml",
  "crates/rustia-lexer/src",
  "crates/rustia-lexer/Cargo.toml",
  "crates/rustia-parser/src",
  "crates/rustia-parser/Cargo.toml",
  "crates/rustia-vm/src",
  "crates/rustia-vm/Cargo.toml",
]);
const HASH_FILES = Object.freeze({
  bench_manifest: "bench/Cargo.toml",
  native_harness: "bench/src/main.rs",
  cargo_lock: "Cargo.lock",
  toolchain: "rust-toolchain.toml",
  workload_dense_tables: "bench/workloads/luau/01_dense_tables.luau",
  workload_sparse_tables: "bench/workloads/luau/02_sparse_tables.luau",
  workload_closures_upvalues: "bench/workloads/luau/04_closures_upvalues.luau",
  workload_generic_iteration: "bench/workloads/luau/09_generic_iteration.luau",
  workload_stdlib_math_table: "bench/workloads/luau/10_stdlib_math_table.luau",
});

function fail(message) {
  throw new Error(message);
}

function git(args, options = {}) {
  return execFileSync("git", args, {
    cwd: REPO,
    encoding: options.encoding || "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function gitBlob(commit, file) {
  return git(["show", `${commit}:${file}`], { encoding: "buffer" });
}

function loadCurrentRustiaRows() {
  const current = JSON.parse(fs.readFileSync(path.join(REPO, "bench/current-results.json"), "utf8"));
  if (!Array.isArray(current.rows)) fail("bench/current-results.json has no rows array");
  const rows = current.rows.filter(
    (row) =>
      row.engine === "rustia" &&
      row.target === "linux-x86_64" &&
      WORKLOADS.includes(row.workload),
  );
  if (rows.length !== WORKLOADS.length) {
    fail(`current results contain ${rows.length} Rustia Linux rows, expected ${WORKLOADS.length}`);
  }
  const byWorkload = new Map(rows.map((row) => [row.workload, row]));
  return {
    fixed_n: Object.fromEntries(
      WORKLOADS.map((workload) => {
        const n = byWorkload.get(workload).n;
        if (!Number.isSafeInteger(n) || n <= 0) fail(`current ${workload} N is invalid`);
        return [workload, n];
      }),
    ),
    expected_checksums: Object.fromEntries(
      WORKLOADS.map((workload) => {
        const checksum = byWorkload.get(workload).checksum;
        if (typeof checksum !== "string" || checksum.length === 0) {
          fail(`current ${workload} checksum is invalid`);
        }
        return [workload, checksum];
      }),
    ),
  };
}

/// Seconds per revision measured end to end on the dedicated ci-bench host:
/// a 17s incremental rebuild plus a 19s five-workload measurement at the
/// protocol's fixed `n`. This is a FLOOR, not an average - it was taken with a
/// warm cargo cache and a warm target directory, and a revision with a large
/// diff rebuilds more. It exists to project a wall-clock estimate, never to
/// gate on one.
const MEASURED_SECONDS_PER_REVISION = 36;

/// Revision count past which the projected wall-clock is worth announcing.
/// This was previously a hard failure at 64, inherited from the hourly-billed
/// ephemeral-Linode era where a bound on revisions was a bound on the bill and
/// on the rental window. Backfills now run on an owned host with no meter, so
/// the operator wants the projection, not a refusal.
const REVISION_COUNT_ADVISORY = 64;

function targetCommits(start, harness) {
  const ancestor = execFileSync(
    "git",
    ["merge-base", "--is-ancestor", start, harness],
    { cwd: REPO, stdio: "ignore" },
  );
  void ancestor;
  const startParent = `${start}^`;
  // Every engine revision, not just the first-parent chain. A first-parent walk
  // represents work that landed through a merge by the merge commit alone, so
  // the individual commits inside it are never measured and a movement in the
  // series localises to a merge rather than to a change. Measured on
  // 2026-08-28: first-parent selected 43 of the 74 engine revisions, dropping
  // commits including the guarded numeric table loop quickening, the table
  // fast-path broadening and the PUC 5.1.5 table backend port. All 36 of the
  // then-dropped revisions were built and run standalone on ci-bench: 36 built,
  // 36 ran, and all produced the same workload checksum as the first-parent
  // set, so measuring them is safe.
  //
  // `--topo-order` keeps each branch's revisions contiguous instead of
  // interleaving parallel branches by commit date. This is a PRESENTATION
  // choice, not a correctness one: date order is an arbitrary but entirely
  // valid linearization here, and both orders respect every real ancestry
  // relation in the selected set.
  const output = git([
    "log",
    "--topo-order",
    "--reverse",
    "--format=%H",
    `${startParent}..${harness}`,
    "--",
    ...ENGINE_PATHS,
  ]).trim();
  const commits = output ? output.split("\n") : [];
  if (commits.length === 0) fail("the selected range contains no Rustia engine revisions");
  if (commits.some((commit) => !FULL_COMMIT.test(commit))) fail("git returned an invalid commit");
  if (commits.length > REVISION_COUNT_ADVISORY) {
    const minutes = Math.ceil((commits.length * MEASURED_SECONDS_PER_REVISION) / 60);
    process.stderr.write(
      `freeze-history-backfill: ${commits.length} revisions selected; projected wall-clock at least ` +
        `${minutes} minutes (${MEASURED_SECONDS_PER_REVISION}s/revision measured warm, treat as a floor)\n`,
    );
  }
  return commits;
}

function freeze(start, outputFile) {
  const tracked = git(["status", "--porcelain=v1", "--untracked-files=no"]).trim();
  if (tracked) fail("tracked worktree changes must be committed before freezing a backfill");
  const harness = git(["rev-parse", "HEAD"]).trim();
  const resolvedStart = git(["rev-parse", `${start}^{commit}`]).trim();
  if (!FULL_COMMIT.test(harness) || !FULL_COMMIT.test(resolvedStart)) fail("invalid Git revision");
  const baseConfig = JSON.parse(
    fs.readFileSync(path.join(REPO, "bench/cloud/linode-cross-engine-matrix-83bc2f3.json"), "utf8"),
  );
  const measurements = loadCurrentRustiaRows();
  const commits = targetCommits(resolvedStart, harness);
  const config = {
    schema: "rustia.linode-fixed-benchmark.v2",
    provider: baseConfig.provider,
    source: {
      harness_commit: harness,
      // What this series is, recorded where a future reader will actually look.
      // Selecting every engine revision rather than the first-parent chain means
      // the list contains commits from parallel branches, so there is no moment
      // in the project's history at which `main` passed through them in this or
      // any other order. The series is therefore a chosen linearization over
      // parallel work, not a replay of how `main` evolved, and a chart of it
      // will invite the stronger reading unless this says otherwise.
      revision_selection: {
        walk: "git log --topo-order --reverse over every engine path",
        includes_non_first_parent_commits: true,
        ordering_is_presentational: true,
        note:
          "Revisions come from parallel branches as well as the mainline, so " +
          "this ordering is one valid linearization among several rather than " +
          "the sequence main actually took. Movements attribute to a revision, " +
          "not to a moment.",
      },
      target_commits: commits,
      expected_sha256: Object.fromEntries(
        Object.entries(HASH_FILES).map(([field, file]) => [field, sha256(gitBlob(harness, file))]),
      ),
    },
    measurement: {
      protocol: "rustia_history_backfill_v1",
      native_target: "linux-x86_64",
      workloads: WORKLOADS,
      fixed_n: measurements.fixed_n,
      repetitions: 5,
      expected_checksums: measurements.expected_checksums,
    },
    host_gate: baseConfig.host_gate,
    publication: {
      mode: "history_backfill",
      rows: commits.length * WORKLOADS.length,
    },
  };
  const bytes = `${JSON.stringify(config, null, 2)}\n`;
  if (outputFile) {
    const resolved = path.resolve(REPO, outputFile);
    const relative = path.relative(REPO, resolved);
    if (relative.startsWith("..") || path.isAbsolute(relative)) {
      fail("output path must remain inside the repository");
    }
    fs.mkdirSync(path.dirname(resolved), { recursive: true });
    fs.writeFileSync(resolved, bytes, { flag: "wx" });
    process.stdout.write(
      `froze ${commits.length} Rustia revisions / ${config.publication.rows} rows at ${relative}\n`,
    );
  } else {
    process.stdout.write(bytes);
  }
}

function usage() {
  process.stdout.write(
    "Usage: node tools/freeze-history-backfill.js START_COMMIT [OUTPUT.json]\n",
  );
}

try {
  if (process.argv.length < 3 || process.argv.length > 4) {
    usage();
    process.exitCode = 2;
  } else {
    freeze(process.argv[2], process.argv[3]);
  }
} catch (error) {
  process.stderr.write(`freeze-history-backfill: ${error.message}\n`);
  process.exitCode = 1;
}
