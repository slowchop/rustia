#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

// The retired Linode rental was destroyed after every run, so its ROOT was a
// throwaway path under /opt with no ownership concerns. ci-bench is a machine
// this project owns outright and reaches over SSH as an unprivileged user
// (see run-dedicated-benchmark.js's REMOTE_ROOT, which already ships
// ".rustia-history-backfill" to the same default), so ROOT lives under that
// user's home directory instead, with an env var escape hatch for a
// non-default disk.
const ROOT = process.env.RUSTIA_BACKFILL_ROOT ?? path.join(os.homedir(), ".rustia-history-backfill");
const REPOSITORY = path.join(ROOT, "repo.git");
const HARNESS = path.join(ROOT, "harness");
const SOURCES = path.join(ROOT, "sources");
// The single build tree every revision is checked out into. See buildTargets
// for why this is not one directory per commit.
const SOURCE_TREE = path.join(SOURCES, "tree");
// The cargo target dir is deliberately outside WIPE_ON_START below: a warm
// target dir is what makes each historical revision build in ~26s instead of
// minutes, and unlike repo.git/sources/binaries it holds nothing keyed to a
// specific run, so a previous run's target dir is never stale.
const CACHE = path.join(ROOT, "cache");
const TARGET = path.join(CACHE, "target");
const BINARIES = path.join(ROOT, "binaries");
const ATTEMPTS = path.join(ROOT, "attempts");
const EXPORT = path.join(ROOT, "export");
const EXPORT_ARCHIVE = path.join(ROOT, "export.tar.gz");
const RUN_LOG = path.join(ROOT, "remote-run.log");
const EVIDENCE = path.join(ROOT, "evidence");
const LOCK_FILE = path.join(ROOT, "lock");
// rustup installs here; the old runner assumed it ran as root and hardcoded
// /root/.cargo/bin, so cargo/rustc/rustup are resolved through PATH instead.
const CARGO_BIN = path.join(os.homedir(), ".cargo", "bin");
const PROTOCOL = "rustia_history_backfill_v1";
const TARGET_RELATIONS = new Set(["targets_are_ancestors", "harness_is_ancestor"]);
const WORKLOADS = Object.freeze([
  "dense_tables",
  "sparse_tables",
  "closures_upvalues",
  "generic_iteration",
  "stdlib_math_table",
]);
const WORKLOAD_FILES = Object.freeze({
  dense_tables: "bench/workloads/luau/01_dense_tables.luau",
  sparse_tables: "bench/workloads/luau/02_sparse_tables.luau",
  closures_upvalues: "bench/workloads/luau/04_closures_upvalues.luau",
  generic_iteration: "bench/workloads/luau/09_generic_iteration.luau",
  stdlib_math_table: "bench/workloads/luau/10_stdlib_math_table.luau",
});
const OVERLAY_FILES = Object.freeze({
  bench_manifest: "bench/Cargo.toml",
  native_harness: "bench/src/main.rs",
  cargo_lock: "Cargo.lock",
  toolchain: "rust-toolchain.toml",
  ...Object.fromEntries(
    WORKLOADS.map((workload) => [`workload_${workload}`, WORKLOAD_FILES[workload]]),
  ),
});
const CORE_TREES = Object.freeze([
  "crates/rustia-compiler",
  "crates/rustia-lexer",
  "crates/rustia-parser",
  "crates/rustia-vm",
]);
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const MAX_TIMED_ATTEMPTS = 3;
// A second run must not inherit a previous run's clone, worktrees, binaries,
// timed-attempt scratch, or last export/evidence -- see resetWorkingState.
// CACHE (the warm cargo target dir) and LOCK_FILE (this run's own lock,
// already held by the time resetWorkingState runs) are deliberately absent.
const WIPE_ON_START = Object.freeze([REPOSITORY, HARNESS, SOURCES, BINARIES, ATTEMPTS, EXPORT, EVIDENCE]);

let logReady = false;

function fail(message) {
  throw new Error(message);
}

function log(message) {
  const line = `[${new Date().toISOString()}] ${message}`;
  process.stdout.write(`${line}\n`);
  if (logReady) fs.appendFileSync(RUN_LOG, `${line}\n`);
}

function requireObject(value, location) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${location} must be an object`);
  }
  return value;
}

function requireString(value, location) {
  if (typeof value !== "string" || value.length === 0) {
    fail(`${location} must be a non-empty string`);
  }
  return value;
}

function requireInteger(value, location, minimum = 0) {
  if (!Number.isSafeInteger(value) || value < minimum) {
    fail(`${location} must be an integer >= ${minimum}`);
  }
  return value;
}

function requireNumber(value, location, minimum = 0) {
  if (typeof value !== "number" || !Number.isFinite(value) || value < minimum) {
    fail(`${location} must be a finite number >= ${minimum}`);
  }
  return value;
}

function requireExactKeys(value, expected, location) {
  const actual = Object.keys(value).sort();
  const wanted = expected.slice().sort();
  if (JSON.stringify(actual) !== JSON.stringify(wanted)) {
    fail(`${location} keys must be ${wanted.join(", ")}; got ${actual.join(", ")}`);
  }
}

function requireFrozenArray(value, frozen, location) {
  if (!Array.isArray(value) || JSON.stringify(value) !== JSON.stringify(frozen)) {
    fail(`${location} must be exactly ${frozen.join(", ")}`);
  }
}

function loadJson(file, location) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot load ${location}: ${error.message}`);
  }
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  return sha256(fs.readFileSync(file));
}

// Mirrors expandCpuList in run-linux-fixed-benchmark.js: provider.host.pinned_cpus
// is a taskset-style CPU-list string ("4", "4,5", "4-5"). This file's
// host_gate.benchmark_cpu cross-check (below) is what the fixed runner has no
// equivalent of, since it names its pinned core directly rather than through
// a separate host_gate section.
function expandCpuList(text, location) {
  const cpus = new Set();
  for (const token of String(text).trim().split(",")) {
    const match = token.match(/^(\d+)(?:-(\d+))?$/);
    if (!match) fail(`${location} has invalid CPU-list syntax`);
    const start = Number(match[1]);
    const end = match[2] === undefined ? start : Number(match[2]);
    if (!Number.isSafeInteger(start) || !Number.isSafeInteger(end) || end < start || end - start > 4096) {
      fail(`${location} has an invalid CPU range`);
    }
    for (let cpu = start; cpu <= end; cpu += 1) cpus.add(cpu);
  }
  return [...cpus].sort((left, right) => left - right);
}

function validateConfig(value) {
  const root = requireObject(value, "config");
  requireExactKeys(
    root,
    ["schema", "provider", "source", "measurement", "host_gate", "publication"],
    "config",
  );
  if (root.schema !== "rustia.dedicated-history-backfill.v1") {
    fail("unsupported history backfill config schema");
  }

  // An owned host has no provider catalog to validate against -- no region,
  // image, size, or per-hour price. Mirrors validateDedicatedProvider in
  // run-linux-fixed-benchmark.js: the provider section instead names what
  // the operator guarantees -- the machine, the cores held off the
  // scheduler, and the one core a timed leg must run on.
  requireExactKeys(root.provider, ["host"], "config.provider");
  const host = requireObject(root.provider.host, "config.provider.host");
  requireExactKeys(
    host,
    ["name", "cpu_model", "logical_cpus", "physical_cores", "pinned_cpus", "max_runtime_minutes"],
    "config.provider.host",
  );
  requireString(host.name, "config.provider.host.name");
  requireString(host.cpu_model, "config.provider.host.cpu_model");
  requireString(host.pinned_cpus, "config.provider.host.pinned_cpus");
  requireInteger(host.logical_cpus, "config.provider.host.logical_cpus", 1);
  requireInteger(host.physical_cores, "config.provider.host.physical_cores", 1);
  requireInteger(host.max_runtime_minutes, "config.provider.host.max_runtime_minutes", 1);
  // SMT is required off, so logical CPUs and physical cores must agree. A
  // config that disagrees with itself would otherwise pass here and fail
  // only on the host, after a build.
  if (host.logical_cpus !== host.physical_cores) {
    fail(
      `config.provider.host declares ${host.logical_cpus} logical CPUs across ${host.physical_cores} ` +
        `physical cores; the dedicated protocol requires SMT off, so these must be equal`,
    );
  }

  const source = requireObject(root.source, "config.source");
  const sourceKeys = [
    "harness_commit",
    "revision_selection",
    "target_commits",
    "source_modifications",
    "expected_sha256",
  ];
  if (Object.prototype.hasOwnProperty.call(source, "target_relation")) {
    sourceKeys.push("target_relation");
  }
  requireExactKeys(
    source,
    sourceKeys,
    "config.source",
  );
  if (!FULL_COMMIT.test(requireString(source.harness_commit, "config.source.harness_commit"))) {
    fail("config.source.harness_commit must be a full lowercase commit");
  }
  const targetRelation = source.target_relation ?? "targets_are_ancestors";
  if (!TARGET_RELATIONS.has(targetRelation)) {
    fail(
      "config.source.target_relation must be targets_are_ancestors or harness_is_ancestor",
    );
  }
  const revisionSelection = requireObject(source.revision_selection, "config.source.revision_selection");
  requireExactKeys(
    revisionSelection,
    ["walk", "includes_non_first_parent_commits", "ordering_is_presentational", "note"],
    "config.source.revision_selection",
  );
  requireString(revisionSelection.walk, "config.source.revision_selection.walk");
  requireString(revisionSelection.note, "config.source.revision_selection.note");
  if (typeof revisionSelection.includes_non_first_parent_commits !== "boolean") {
    fail("config.source.revision_selection.includes_non_first_parent_commits must be a boolean");
  }
  if (typeof revisionSelection.ordering_is_presentational !== "boolean") {
    fail("config.source.revision_selection.ordering_is_presentational must be a boolean");
  }
  // Unlike the retired Linode cap (target_commits.length <= 64), which bounded
  // a rental bill rather than correctness, an owned host has no per-run cost
  // to cap: only non-empty, unique, full commits are required.
  if (!Array.isArray(source.target_commits) || source.target_commits.length === 0) {
    fail("config.source.target_commits must contain at least one commit");
  }
  const seen = new Set();
  for (const [index, commit] of source.target_commits.entries()) {
    if (!FULL_COMMIT.test(commit)) {
      fail(`config.source.target_commits[${index}] must be a full lowercase commit`);
    }
    if (seen.has(commit)) fail(`config.source.target_commits duplicates ${commit}`);
    seen.add(commit);
  }
  if (!Array.isArray(source.source_modifications)) {
    fail("config.source.source_modifications must be an array");
  }
  for (const [index, repair] of source.source_modifications.entries()) {
    const location = `config.source.source_modifications[${index}]`;
    requireObject(repair, location);
    requireExactKeys(
      repair,
      [
        "commit",
        "subject",
        "cherry_pick",
        "why_it_could_not_be_measured",
        "changes_applied",
        "why_the_timing_is_still_this_revision",
        "why_it_is_derived_rather_than_stored",
        "verification",
      ],
      location,
    );
    if (!seen.has(repair.commit)) {
      fail(`${location}.commit ${repair.commit} is not one of config.source.target_commits`);
    }
    requireString(repair.subject, `${location}.subject`);
    const cherryPick = requireObject(repair.cherry_pick, `${location}.cherry_pick`);
    requireExactKeys(cherryPick, ["from_commit", "from_subject", "paths"], `${location}.cherry_pick`);
    if (!FULL_COMMIT.test(requireString(cherryPick.from_commit, `${location}.cherry_pick.from_commit`))) {
      fail(`${location}.cherry_pick.from_commit must be a full lowercase commit`);
    }
    requireString(cherryPick.from_subject, `${location}.cherry_pick.from_subject`);
    if (!Array.isArray(cherryPick.paths) || cherryPick.paths.length === 0) {
      fail(`${location}.cherry_pick.paths must be a non-empty array`);
    }
    for (const [pathIndex, relativePath] of cherryPick.paths.entries()) {
      requireString(relativePath, `${location}.cherry_pick.paths[${pathIndex}]`);
      // A repair patches source that ships inside the historical binary;
      // bench/ is the harness overlay applied separately by overlayHarness,
      // and a repair reaching into it would silently change what the harness
      // measures instead of just repairing why an old revision cannot build.
      if (relativePath.startsWith("bench/")) {
        fail(`${location}.cherry_pick.paths[${pathIndex}] ${relativePath} must not be under bench/`);
      }
    }
    for (const field of [
      "why_it_could_not_be_measured",
      "why_the_timing_is_still_this_revision",
      "why_it_is_derived_rather_than_stored",
      "verification",
    ]) {
      requireString(repair[field], `${location}.${field}`);
    }
    if (!Array.isArray(repair.changes_applied) || repair.changes_applied.length === 0) {
      fail(`${location}.changes_applied must be a non-empty array`);
    }
    for (const [changeIndex, change] of repair.changes_applied.entries()) {
      requireString(change, `${location}.changes_applied[${changeIndex}]`);
    }
  }
  const expectedHashes = requireObject(source.expected_sha256, "config.source.expected_sha256");
  requireExactKeys(expectedHashes, Object.keys(OVERLAY_FILES), "config.source.expected_sha256");
  for (const [field, hash] of Object.entries(expectedHashes)) {
    if (!SHA256.test(hash)) fail(`config.source.expected_sha256.${field} must be a lowercase SHA-256`);
  }

  const measurement = requireObject(root.measurement, "config.measurement");
  requireExactKeys(
    measurement,
    ["protocol", "native_target", "workloads", "fixed_n", "repetitions", "expected_checksums"],
    "config.measurement",
  );
  if (measurement.protocol !== PROTOCOL) fail(`config.measurement.protocol must be ${PROTOCOL}`);
  if (measurement.native_target !== "linux-x86_64") {
    fail("config.measurement.native_target must be linux-x86_64");
  }
  requireFrozenArray(measurement.workloads, WORKLOADS, "config.measurement.workloads");
  if (requireInteger(measurement.repetitions, "config.measurement.repetitions", 1) !== 5) {
    fail("config.measurement.repetitions must remain frozen at 5");
  }
  const fixedN = requireObject(measurement.fixed_n, "config.measurement.fixed_n");
  const checksums = requireObject(
    measurement.expected_checksums,
    "config.measurement.expected_checksums",
  );
  requireExactKeys(fixedN, WORKLOADS, "config.measurement.fixed_n");
  requireExactKeys(checksums, WORKLOADS, "config.measurement.expected_checksums");
  for (const workload of WORKLOADS) {
    requireInteger(fixedN[workload], `config.measurement.fixed_n.${workload}`, 1);
    requireString(checksums[workload], `config.measurement.expected_checksums.${workload}`);
  }

  const gate = requireObject(root.host_gate, "config.host_gate");
  requireExactKeys(
    gate,
    [
      "benchmark_cpu",
      "cooldown_seconds",
      "idle_window_seconds",
      "minimum_idle_percent",
      "maximum_load_1m",
      "maximum_steal_ticks",
      "maximum_timed_steal_percent",
      "wait_timeout_seconds",
      "maximum_timed_attempts_per_group",
    ],
    "config.host_gate",
  );
  requireInteger(gate.benchmark_cpu, "config.host_gate.benchmark_cpu", 0);
  requireInteger(gate.cooldown_seconds, "config.host_gate.cooldown_seconds", 1);
  requireInteger(gate.idle_window_seconds, "config.host_gate.idle_window_seconds", 1);
  requireNumber(gate.minimum_idle_percent, "config.host_gate.minimum_idle_percent", 0);
  if (gate.minimum_idle_percent > 100) fail("config.host_gate.minimum_idle_percent must be <= 100");
  requireNumber(gate.maximum_load_1m, "config.host_gate.maximum_load_1m", 0);
  if (requireInteger(gate.maximum_steal_ticks, "config.host_gate.maximum_steal_ticks", 0) !== 0) {
    fail("history backfill idle steal budget must remain zero");
  }
  // An owned, isolcpus-pinned core has no hypervisor neighbor to steal cycles
  // from in the first place, so the timed leg gets zero tolerance too --
  // unlike the old Linode leg's 0.1% (and the maximum_timed_steal_ticks
  // fallback that came with it), which budgeted for a shared host this
  // project did not control. There is nothing left for a tick-based fallback
  // to forgive, so that key is gone rather than frozen at some other value.
  if (
    requireNumber(gate.maximum_timed_steal_percent, "config.host_gate.maximum_timed_steal_percent", 0) !== 0
  ) {
    fail("history backfill timed steal budget must remain zero on a dedicated host");
  }
  const maximumAttempts = requireInteger(
    gate.maximum_timed_attempts_per_group,
    "config.host_gate.maximum_timed_attempts_per_group",
    1,
  );
  if (maximumAttempts > MAX_TIMED_ATTEMPTS) {
    fail(`history backfill allows at most ${MAX_TIMED_ATTEMPTS} timed attempts`);
  }
  // 900s (was 300s): an unmetered box can afford to wait longer for quiet
  // than a per-minute rental could.
  if (requireInteger(gate.wait_timeout_seconds, "config.host_gate.wait_timeout_seconds", 1) !== 900) {
    fail("history backfill quiet-host timeout must remain 900 seconds");
  }
  // The fixed paired-leg runner has no equivalent check: it names the pinned
  // core directly in provider metadata rather than cross-referencing a
  // separate host_gate section against it. Here the two sections are
  // independent parts of the same config, so a benchmark_cpu the operator
  // never isolated is a config bug this can catch before anything is built.
  const pinnedCpus = expandCpuList(host.pinned_cpus, "config.provider.host.pinned_cpus");
  if (!pinnedCpus.includes(gate.benchmark_cpu)) {
    fail(
      `config.host_gate.benchmark_cpu ${gate.benchmark_cpu} is not among config.provider.host.pinned_cpus ` +
        `(${JSON.stringify(host.pinned_cpus)} = ${pinnedCpus.join(",")})`,
    );
  }

  const publication = requireObject(root.publication, "config.publication");
  requireExactKeys(publication, ["mode", "rows"], "config.publication");
  if (publication.mode !== "history_backfill") {
    fail("config.publication.mode must be history_backfill");
  }
  const wantedRows = source.target_commits.length * WORKLOADS.length;
  if (requireInteger(publication.rows, "config.publication.rows", 1) !== wantedRows) {
    fail(`config.publication.rows must be ${wantedRows}`);
  }
  return root;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: options.cwd,
    env: options.env || process.env,
    encoding: "utf8",
    input: options.input,
    stdio:
      options.capture === false ? "inherit" : [options.input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !options.allowFailure) {
    const stderr = String(result.stderr || "").trim();
    fail(`${command} ${args.join(" ")} failed with status ${result.status}${stderr ? `: ${stderr}` : ""}`);
  }
  return {
    status: result.status,
    stdout: String(result.stdout || ""),
    stderr: String(result.stderr || ""),
  };
}

function runText(command, args, options = {}) {
  return run(command, args, options).stdout.trim();
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
}

function appendJsonLine(file, value) {
  fs.appendFileSync(file, `${JSON.stringify(value)}\n`);
}

function verifySource(config, bundle) {
  run("git", ["clone", "--bare", bundle, REPOSITORY], { capture: false });
  run("git", ["--git-dir", REPOSITORY, "bundle", "verify", bundle], { capture: false });
  run(
    "git",
    ["--git-dir", REPOSITORY, "worktree", "add", "--detach", HARNESS, config.source.harness_commit],
    { capture: false },
  );
  assert.equal(runText("git", ["-C", HARNESS, "rev-parse", "HEAD"]), config.source.harness_commit);
  const targetRelation = config.source.target_relation ?? "targets_are_ancestors";
  for (const commit of config.source.target_commits) {
    run("git", ["--git-dir", REPOSITORY, "cat-file", "-e", `${commit}^{commit}`]);
    const ancestry = targetRelation === "harness_is_ancestor"
      ? [config.source.harness_commit, commit]
      : [commit, config.source.harness_commit];
    const ancestor = run(
      "git",
      ["--git-dir", REPOSITORY, "merge-base", "--is-ancestor", ...ancestry],
      { allowFailure: true },
    );
    if (ancestor.status !== 0) {
      if (targetRelation === "harness_is_ancestor") {
        fail(`harness commit is not an ancestor of target ${commit}`);
      }
      fail(`target ${commit} is not an ancestor of the harness commit`);
    }
  }
  for (const [field, relative] of Object.entries(OVERLAY_FILES)) {
    const actual = sha256File(path.join(HARNESS, relative));
    if (actual !== config.source.expected_sha256[field]) {
      fail(`${relative} SHA-256 is ${actual}, expected ${config.source.expected_sha256[field]}`);
    }
  }
}

// The old runner ran as root and hardcoded /root/.cargo/bin/<tool>; this one
// runs as whatever unprivileged user owns ci-bench, so every cargo/rustc/
// rustup invocation below resolves the bare command name through PATH, with
// CARGO_BIN prepended so it works whether or not the invoking shell already
// sourced rustup's own profile snippet.
function cargoEnvironment(extra = {}) {
  return {
    ...process.env,
    ...extra,
    PATH: `${CARGO_BIN}${path.delimiter}${process.env.PATH || ""}`,
  };
}

function installRust() {
  if (!fs.existsSync(path.join(CARGO_BIN, "rustup"))) {
    const installer = path.join(ROOT, "rustup-init.sh");
    run("curl", [
      "--proto",
      "=https",
      "--tlsv1.2",
      "--fail",
      "--silent",
      "--show-error",
      "https://sh.rustup.rs",
      "-o",
      installer,
    ]);
    run("sh", [installer, "-y", "--profile", "minimal", "--default-toolchain", "none"], {
      capture: false,
    });
  }
  run("rustup", ["toolchain", "install", "1.95.0", "--profile", "minimal"], {
    env: cargoEnvironment(),
    capture: false,
  });
}

function overlayHarness(target) {
  fs.copyFileSync(path.join(HARNESS, "bench/Cargo.toml"), path.join(target, "bench/Cargo.toml"));
  fs.rmSync(path.join(target, "bench/src"), { recursive: true, force: true });
  fs.cpSync(path.join(HARNESS, "bench/src"), path.join(target, "bench/src"), { recursive: true });
  fs.rmSync(path.join(target, "bench/workloads"), { recursive: true, force: true });
  fs.cpSync(path.join(HARNESS, "bench/workloads"), path.join(target, "bench/workloads"), {
    recursive: true,
  });
  fs.copyFileSync(path.join(HARNESS, "Cargo.lock"), path.join(target, "Cargo.lock"));
  fs.copyFileSync(
    path.join(HARNESS, "rust-toolchain.toml"),
    path.join(target, "rust-toolchain.toml"),
  );
}

// Some historical revisions cannot be measured as committed -- each entry in
// config.source.source_modifications explains why, and carries a real diff
// taken from a later commit (already in target_commits) rather than a
// hand-authored patch. Applying it here, right after the harness overlay, is
// what makes buildTargets' checkout match what expected_checksums assumes.
function applySourceModifications(config, commit, worktree) {
  const repairs = config.source.source_modifications.filter((entry) => entry.commit === commit);
  if (repairs.length > 1) {
    fail(`${commit} has ${repairs.length} source_modifications entries; only one repair per commit is supported`);
  }
  if (repairs.length === 0) return null;
  const [repair] = repairs;
  const { from_commit: fromCommit, paths } = repair.cherry_pick;
  // validateConfig already checked shape; re-derive the diff from git itself
  // (rather than trust changes_applied's prose) so "verbatim from the commit
  // that introduced it" is true by construction, not merely asserted.
  const parent = runText("git", ["--git-dir", REPOSITORY, "rev-parse", `${fromCommit}^`]);
  if (parent !== commit) {
    fail(
      `source_modifications repair for ${commit} names from_commit ${fromCommit}, whose parent is ` +
        `${parent}, not ${commit}; a repair must be the immediate next commit's diff`,
    );
  }
  for (const relativePath of paths) {
    if (relativePath.startsWith("bench/")) {
      fail(`source_modifications repair for ${commit} touches ${relativePath}, which is under bench/`);
    }
  }
  const diff = run("git", ["--git-dir", REPOSITORY, "diff", `${fromCommit}^`, fromCommit, "--", ...paths]).stdout;
  const applied = run("git", ["-C", worktree, "apply", "--whitespace=nowarn"], {
    input: diff,
    allowFailure: true,
  });
  if (applied.status !== 0) {
    fail(
      `source_modifications repair for ${commit} failed to apply: ` +
        `${applied.stderr.trim() || `git apply exited ${applied.status}`}`,
    );
  }
  return sha256(Buffer.from(diff, "utf8"));
}

function buildEnvironment() {
  return cargoEnvironment({
    CARGO_INCREMENTAL: "0",
    CARGO_TARGET_DIR: TARGET,
    RUSTC_WRAPPER: "",
  });
}

function targetMetadata(commit) {
  const committedAt = Number(
    runText("git", ["--git-dir", REPOSITORY, "show", "-s", "--format=%ct", commit]),
  );
  if (!Number.isSafeInteger(committedAt) || committedAt <= 0) {
    fail(`target ${commit} has an invalid commit timestamp`);
  }
  const subject = runText("git", ["--git-dir", REPOSITORY, "show", "-s", "--format=%s", commit]);
  const trees = Object.fromEntries(
    CORE_TREES.map((relative) => [
      relative,
      runText("git", ["--git-dir", REPOSITORY, "rev-parse", `${commit}:${relative}`]),
    ]),
  );
  return { commit, committed_at_unix: committedAt, subject, core_trees: trees };
}

// provider.host.max_runtime_minutes has no billing meaning on an unmetered,
// owned host. It exists as a hang detector: if a historical build or
// measurement gets stuck, the run fails fast and names the limit instead of
// running until someone notices.
function checkRuntimeBudget(config, startedAtMs, where) {
  const limitMinutes = config.provider.host.max_runtime_minutes;
  const elapsedMinutes = (Date.now() - startedAtMs) / 60000;
  if (elapsedMinutes > limitMinutes) {
    fail(
      `history backfill exceeded provider.host.max_runtime_minutes (${limitMinutes} minutes) ` +
        `${where}: ${elapsedMinutes.toFixed(1)} minutes elapsed`,
    );
  }
}

function buildTargets(config, startedAtMs) {
  const cargo = "cargo";
  fs.mkdirSync(SOURCES, { recursive: true });
  fs.mkdirSync(BINARIES, { recursive: true });
  const builds = [];
  for (const [index, commit] of config.source.target_commits.entries()) {
    checkRuntimeBudget(
      config,
      startedAtMs,
      `before building revision ${index + 1}/${config.source.target_commits.length}`,
    );
    // Every revision is built in ONE directory rather than a per-commit
    // worktree. rustc embeds the manifest path in the binary, so a per-commit
    // path makes every binary unique whatever its source, which destroys two
    // properties worth more than the isolation it buys: two revisions whose
    // engine source compiles to the same code no longer produce the same bytes,
    // so the identical-binary repeatability check can never fire; and a rebuild
    // of the same series can no longer be compared byte-for-byte against the
    // last one. Observed directly — the first run under per-commit worktrees
    // found zero identical pairs where the operator script that used a single
    // directory found three.
    //
    // A shared directory needs the previous revision cleaned out of it, which is
    // what reset --hard plus clean -fd do; the overlay and any repair are then
    // reapplied from scratch below.
    const source = SOURCE_TREE;
    log(`building ${index + 1}/${config.source.target_commits.length} ${commit.slice(0, 8)}`);
    if (!fs.existsSync(path.join(source, ".git"))) {
      run("git", ["--git-dir", REPOSITORY, "worktree", "add", "--detach", source, commit], {
        capture: false,
      });
    } else {
      run("git", ["-C", source, "reset", "--hard", "--quiet"], { capture: false });
      run("git", ["-C", source, "clean", "-qfd"], { capture: false });
      run("git", ["-C", source, "checkout", "--detach", "--quiet", commit], { capture: false });
    }
    overlayHarness(source);
    const sourceModificationSha256 = applySourceModifications(config, commit, source);
    const cargoArgs = [
        "+1.95.0",
        "build",
        "--manifest-path",
        path.join(source, "bench/Cargo.toml"),
        "--bin",
        "script-bench",
        "--release",
        "--no-default-features",
        "--features",
        "rustia-engine",
      ];
    run(cargo, cargoArgs, { env: buildEnvironment(), capture: false });
    const builtBinary = path.join(TARGET, "release/script-bench");
    fs.accessSync(builtBinary, fs.constants.X_OK);
    const binary = path.join(BINARIES, commit);
    fs.copyFileSync(builtBinary, binary);
    fs.chmodSync(binary, 0o700);
    builds.push({
      ...targetMetadata(commit),
      binary,
      binary_sha256: sha256File(binary),
      resolved_lock_sha256: sha256File(path.join(source, "Cargo.lock")),
      source_modification_sha256: sourceModificationSha256,
    });
  }
  return builds;
}

function readCpu(cpu) {
  const line = fs
    .readFileSync("/proc/stat", "utf8")
    .split("\n")
    .find((entry) => entry.startsWith(`cpu${cpu} `));
  if (!line) fail(`cannot read benchmark CPU ${cpu} from /proc/stat`);
  const fields = line.trim().split(/\s+/).slice(1).map(Number);
  if (fields.length < 8 || fields.some((value) => !Number.isFinite(value))) {
    fail("malformed /proc/stat CPU row");
  }
  return { fields };
}

function cpuDelta(before, after) {
  const deltas = after.fields.map((value, index) => value - before.fields[index]);
  if (deltas.some((value) => !Number.isFinite(value) || value < 0)) {
    fail("invalid /proc/stat CPU delta");
  }
  const total = deltas.reduce((sum, value) => sum + value, 0);
  if (total <= 0) fail("empty /proc/stat CPU window");
  const idle = deltas[3] + (deltas[4] || 0);
  const steal = deltas[7] || 0;
  return {
    total_ticks: total,
    idle_ticks: idle,
    steal_ticks: steal,
    idle_percent: (idle / total) * 100,
    steal_percent: (steal / total) * 100,
  };
}

function sleep(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function waitForQuiet(config, gateFile, timedAttempt, deadline) {
  const gate = config.host_gate;
  let quietAttempt = 0;
  while (Date.now() < deadline) {
    quietAttempt += 1;
    await sleep(gate.cooldown_seconds * 1000);
    const before = readCpu(gate.benchmark_cpu);
    await sleep(gate.idle_window_seconds * 1000);
    const delta = cpuDelta(before, readCpu(gate.benchmark_cpu));
    const load1m = Number(fs.readFileSync("/proc/loadavg", "utf8").trim().split(/\s+/)[0]);
    const accepted =
      delta.idle_percent >= gate.minimum_idle_percent &&
      delta.steal_ticks <= gate.maximum_steal_ticks &&
      load1m <= gate.maximum_load_1m;
    appendJsonLine(gateFile, {
      phase: "pre",
      label: "rustia-history-backfill",
      timed_attempt: timedAttempt,
      quiet_attempt: quietAttempt,
      captured_at: new Date().toISOString(),
      load_1m: load1m,
      ...delta,
      accepted,
    });
    if (accepted) return;
  }
  fail(`history backfill timed attempt ${timedAttempt} could not obtain a quiet host window`);
}

function timedGatePasses(gate, delta) {
  return delta.steal_percent <= gate.maximum_timed_steal_percent;
}

function measurementArgs(config, build, historyFile, samplesFile) {
  return [
    "-c",
    String(config.host_gate.benchmark_cpu),
    build.binary,
    "--engines",
    "rustia",
    "--workloads",
    WORKLOADS.join(","),
    "--reps",
    String(config.measurement.repetitions),
    "--history",
    historyFile,
    "--samples-json",
    samplesFile,
    "--commit",
    build.commit,
    "--host",
    config.provider.host.name,
    "--no-results",
    "--no-latest-json",
  ];
}

function measureOne(config, build, attemptDirectory) {
  const historyFile = path.join(attemptDirectory, `${build.commit}.jsonl`);
  const samplesFile = path.join(attemptDirectory, `${build.commit}-samples.json`);
  for (const workload of WORKLOADS) {
    const workloadHistory = path.join(attemptDirectory, `${build.commit}-${workload}.jsonl`);
    const workloadSamples = path.join(attemptDirectory, `${build.commit}-${workload}-samples.json`);
    const args = measurementArgs(config, build, workloadHistory, workloadSamples);
    args.push(
      "--workloads",
      workload,
      "--fixed-n",
      String(config.measurement.fixed_n[workload]),
    );
    // Measure from the harness worktree, not from a revision's build tree. The
    // harness commit is what pins the workload files, and every revision was
    // overlaid with exactly those bytes, so the harness tree is the honest
    // place to read them from — and unlike a build tree it does not change as
    // revisions are checked out through it.
    //
    // This previously pointed at a per-commit source directory that no longer
    // exists. Node reports a missing cwd as ENOENT against the *command*, so
    // the failure read as "taskset is not installed" when taskset was present
    // the whole time; the assertion below says what is actually wrong.
    if (!fs.existsSync(HARNESS)) {
      fail(`harness worktree ${HARNESS} is missing; cannot measure`);
    }
    run("taskset", args, {
      cwd: HARNESS,
      capture: false,
    });
    fs.appendFileSync(historyFile, fs.readFileSync(workloadHistory));
    const sample = loadJson(workloadSamples, `${build.commit}/${workload} samples`);
    const combined = fs.existsSync(samplesFile)
      ? loadJson(samplesFile, `${build.commit} combined samples`)
      : { schema: "rustia.bench.history-backfill-samples.v1", commit: build.commit, measurements: [] };
    const measurements = Array.isArray(sample.measurements) ? sample.measurements : [];
    combined.measurements.push(...measurements);
    writeJson(samplesFile, combined);
  }
  return { historyFile, samplesFile };
}

function parseHistory(file) {
  const text = fs.readFileSync(file, "utf8").trim();
  if (!text) return [];
  return text.split("\n").map((line, index) => {
    try {
      return JSON.parse(line);
    } catch (error) {
      fail(`${file} line ${index + 1} is invalid JSON: ${error.message}`);
    }
  });
}

function validateRows(config, builds, rows) {
  if (rows.length !== config.publication.rows) {
    fail(`history backfill produced ${rows.length} rows, expected ${config.publication.rows}`);
  }
  const positions = new Map(builds.map((build, index) => [build.commit, index]));
  const slots = new Set();
  for (const [index, row] of rows.entries()) {
    requireObject(row, `history row ${index + 1}`);
    if (!positions.has(row.commit)) fail(`history row ${index + 1} has an unexpected commit`);
    if (row.engine !== "rustia" || row.dialect !== "luau" || row.target !== "linux-x86_64") {
      fail(`history row ${index + 1} has the wrong engine identity`);
    }
    if (row.host !== config.provider.host.name) {
      fail(
        `history row ${index + 1} has host ${JSON.stringify(row.host)}, expected ` +
          `${JSON.stringify(config.provider.host.name)}`,
      );
    }
    if (!WORKLOADS.includes(row.workload)) fail(`history row ${index + 1} has an unknown workload`);
    if (row.n !== config.measurement.fixed_n[row.workload]) {
      fail(`history row ${index + 1} has the wrong fixed N`);
    }
    if (row.reps !== config.measurement.repetitions) {
      fail(`history row ${index + 1} has the wrong repetition count`);
    }
    if (row.checksum !== config.measurement.expected_checksums[row.workload]) {
      fail(
        `history row ${index + 1} checksum ${row.checksum} does not match ` +
          config.measurement.expected_checksums[row.workload],
      );
    }
    for (const field of ["ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
      requireNumber(row[field], `history row ${index + 1}.${field}`, Number.MIN_VALUE);
    }
    const slot = `${row.commit}\0${row.workload}`;
    if (slots.has(slot)) fail(`history backfill duplicates ${row.commit}/${row.workload}`);
    slots.add(slot);
  }
  for (const build of builds) {
    for (const workload of WORKLOADS) {
      if (!slots.has(`${build.commit}\0${workload}`)) {
        fail(`history backfill is missing ${build.commit}/${workload}`);
      }
    }
  }
}

function median(sortedValues) {
  const count = sortedValues.length;
  if (count === 0) return null;
  const middle = Math.floor(count / 2);
  return count % 2 === 0 ? (sortedValues[middle - 1] + sortedValues[middle]) / 2 : sortedValues[middle];
}

// Free repeatability evidence (see bench/README.md): some historical commits
// compile to byte-identical binaries -- a change outside the crates that end
// up in script-bench leaves binary_sha256 unchanged. Every such duplicate was
// still built and measured independently, at a different point in the run,
// so the spread between their ns_per_unit_min readings is a real, no-extra-
// measurement check on how quiet the host actually was. It is evidence, not
// a gate: there is no threshold yet to justify failing a run over.
function repeatabilityCheck(builds, rows) {
  const byHash = new Map();
  for (const build of builds) {
    if (!byHash.has(build.binary_sha256)) byHash.set(build.binary_sha256, []);
    byHash.get(build.binary_sha256).push(build.commit);
  }
  const groups = [...byHash.entries()]
    .filter(([, commits]) => commits.length > 1)
    .map(([binarySha256, commits]) => ({ binary_sha256: binarySha256, commits }));
  if (groups.length === 0) {
    return {
      schema: "rustia.bench.history-backfill-repeatability.v1",
      available: false,
      reason: "no two target commits produced byte-identical binaries in this run",
      groups: [],
      comparisons: [],
      median_spread_percent: null,
      worst_spread_percent: null,
    };
  }
  const rowByCommitWorkload = new Map(rows.map((row) => [`${row.commit}\0${row.workload}`, row]));
  const comparisons = [];
  for (const group of groups) {
    for (const workload of WORKLOADS) {
      const values = group.commits.map((commit) => {
        const row = rowByCommitWorkload.get(`${commit}\0${workload}`);
        if (!row) fail(`repeatabilityCheck: missing ${commit}/${workload} row for a duplicate binary`);
        return row.ns_per_unit_min;
      });
      const lowest = Math.min(...values);
      const highest = Math.max(...values);
      comparisons.push({
        binary_sha256: group.binary_sha256,
        commits: group.commits,
        workload,
        ns_per_unit_min_values: values,
        spread_percent: lowest > 0 ? ((highest - lowest) / lowest) * 100 : 0,
      });
    }
  }
  const spreads = comparisons.map((entry) => entry.spread_percent).sort((left, right) => left - right);
  return {
    schema: "rustia.bench.history-backfill-repeatability.v1",
    available: true,
    groups,
    comparisons,
    median_spread_percent: median(spreads),
    worst_spread_percent: spreads[spreads.length - 1],
  };
}

async function measure(config, builds, evidenceDirectory, startedAtMs) {
  const gateFile = path.join(evidenceDirectory, "gate.jsonl");
  fs.writeFileSync(gateFile, "");
  const quietDeadline = Date.now() + config.host_gate.wait_timeout_seconds * 1000;
  for (let attempt = 1; attempt <= config.host_gate.maximum_timed_attempts_per_group; attempt += 1) {
    await waitForQuiet(config, gateFile, attempt, quietDeadline);
    const attemptDirectory = path.join(ATTEMPTS, String(attempt));
    fs.rmSync(attemptDirectory, { recursive: true, force: true });
    fs.mkdirSync(attemptDirectory, { recursive: true });
    log(`history backfill timed attempt ${attempt}/${config.host_gate.maximum_timed_attempts_per_group} starting`);
    const before = readCpu(config.host_gate.benchmark_cpu);
    const startedAt = new Date().toISOString();
    const outputs = [];
    for (const [index, build] of builds.entries()) {
      checkRuntimeBudget(config, startedAtMs, `before measuring ${index + 1}/${builds.length}`);
      log(`measuring ${index + 1}/${builds.length} ${build.commit.slice(0, 8)}`);
      outputs.push(measureOne(config, build, attemptDirectory));
    }
    const finishedAt = new Date().toISOString();
    const delta = cpuDelta(before, readCpu(config.host_gate.benchmark_cpu));
    const accepted = timedGatePasses(config.host_gate, delta);
    appendJsonLine(gateFile, {
      phase: "timed",
      label: "rustia-history-backfill",
      timed_attempt: attempt,
      started_at: startedAt,
      finished_at: finishedAt,
      ...delta,
      accepted,
    });
    if (!accepted) {
      log(
        `history backfill timed attempt ${attempt} rejected at ` +
          `${delta.steal_percent.toFixed(6)}% / ${delta.steal_ticks} steal tick(s)`,
      );
      continue;
    }
    const rows = outputs.flatMap((output) => parseHistory(output.historyFile));
    validateRows(config, builds, rows);
    log(`history backfill timed attempt ${attempt} accepted`);
    return { gateFile, attemptDirectory, rows, startedAt, finishedAt };
  }
  fail("history backfill exhausted its timed attempts without meeting the steal gate");
}

function collectHost(config, builds) {
  const cpuInfo = fs.readFileSync("/proc/cpuinfo", "utf8");
  const model = cpuInfo.match(/^model name\s*:\s*(.+)$/m);
  return {
    schema: "rustia.bench.history-backfill-host.v1",
    platform: process.platform,
    architecture: process.arch,
    release: os.release(),
    cpu_model: model ? model[1].trim() : "unknown",
    logical_cpus: os.cpus().length,
    benchmark_cpu: config.host_gate.benchmark_cpu,
    rustc_version: runText("rustc", ["+1.95.0", "--version", "--verbose"], { env: cargoEnvironment() }),
    cargo_version: runText("cargo", ["+1.95.0", "--version", "--verbose"], { env: cargoEnvironment() }),
    target_count: builds.length,
  };
}

function evidenceInventory(directory) {
  const files = [];
  function visit(current, prefix) {
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (entry.isDirectory()) visit(path.join(current, entry.name), relative);
      else if (entry.isFile()) files.push(relative);
      else fail(`evidence contains unsupported entry ${relative}`);
    }
  }
  visit(directory, "");
  return files.sort();
}

function publish(config, configFile, providerFile, builds, measurement, evidenceDirectory, repeatability) {
  const measuredAt = Math.floor(Date.parse(measurement.finishedAt) / 1000);
  const publication = {
    schema: "rustia.bench.history-backfill.v1",
    harness_commit: config.source.harness_commit,
    host: config.provider.host.name,
    pinned_core: config.host_gate.benchmark_cpu,
    measured_at_unix: measuredAt,
    native_target: config.measurement.native_target,
    workloads: WORKLOADS,
    fixed_n: config.measurement.fixed_n,
    repetitions: config.measurement.repetitions,
    revision_selection: config.source.revision_selection,
    source_modifications: config.source.source_modifications,
    repeatability,
    targets: builds.map(({ binary, ...build }) => build),
    rows: measurement.rows,
  };
  const publicationFile = path.join(
    EXPORT,
    "publication/bench/history/backfill.json",
  );
  writeJson(publicationFile, publication);
  fs.copyFileSync(configFile, path.join(evidenceDirectory, "protocol.json"));
  fs.copyFileSync(providerFile, path.join(evidenceDirectory, "provider.json"));
  fs.copyFileSync(RUN_LOG, path.join(evidenceDirectory, "remote-run.log"));
  writeJson(path.join(evidenceDirectory, "host.json"), collectHost(config, builds));
  writeJson(
    path.join(evidenceDirectory, "builds.json"),
    builds.map(({ binary, ...build }) => build),
  );
  writeJson(path.join(evidenceDirectory, "repeatability.json"), repeatability);
  const rawDirectory = path.join(evidenceDirectory, "raw");
  fs.mkdirSync(rawDirectory, { recursive: true });
  for (const build of builds) {
    const suffixes = [".jsonl", "-samples.json"];
    // The per-workload sidecars as the harness wrote them. The combined file
    // above is this runner's own envelope; these are rustia.bench.raw-samples.v1
    // and are what bench/analyze-fixed-samples.js reads directly. Exporting them
    // is what lets the PR lane compare two revisions without anything having to
    // rebuild an analyzer input out of parts, which would mean asserting
    // protocol metadata rather than carrying the harness's own.
    for (const workload of WORKLOADS) suffixes.push(`-${workload}-samples.json`);
    for (const suffix of suffixes) {
      const source = path.join(measurement.attemptDirectory, `${build.commit}${suffix}`);
      fs.copyFileSync(source, path.join(rawDirectory, `${build.commit}${suffix}`));
    }
  }
  writeJson(path.join(evidenceDirectory, "manifest.json"), {
    schema: "rustia.bench.history-backfill-evidence.v1",
    harness_commit: config.source.harness_commit,
    target_commits: config.source.target_commits,
    created_at: new Date().toISOString(),
    protocol_sha256: sha256File(configFile),
    provider_sha256: sha256File(providerFile),
    publication_sha256: sha256File(publicationFile),
    evidence_files: evidenceInventory(evidenceDirectory).filter((file) => file !== "manifest.json"),
  });
}

function exportRun(status, evidenceDirectory) {
  fs.rmSync(EXPORT, { recursive: true, force: true });
  fs.mkdirSync(EXPORT, { recursive: true });
  writeJson(path.join(EXPORT, "status.json"), status);
  if (fs.existsSync(evidenceDirectory)) {
    const destination = path.join(EXPORT, "evidence/history-backfill");
    fs.mkdirSync(path.dirname(destination), { recursive: true });
    fs.cpSync(evidenceDirectory, destination, { recursive: true });
  }
  if (status.status === "backfill_complete") {
    // publish() populated this before exportRun reset EXPORT; recover the staged copy.
    const staged = path.join(ROOT, "backfill-publication.json");
    fs.mkdirSync(path.join(EXPORT, "publication/bench/history"), { recursive: true });
    fs.copyFileSync(staged, path.join(EXPORT, "publication/bench/history/backfill.json"));
  }
  run("tar", ["--format=ustar", "-czf", EXPORT_ARCHIVE, "-C", EXPORT, "."], {
    capture: false,
  });
}

// Two runs sharing ROOT -- the same repo clone, the same worktrees, the same
// cargo target dir -- is not a hypothetical: a second run's resetWorkingState
// or `git worktree add` can blow away the first run's in-progress checkout
// mid-build, and both runs end up with zero usable rows. O_EXCL makes the
// lock file's creation itself atomic, so there is no window where two
// processes both believe they hold it.
function acquireLock() {
  fs.mkdirSync(ROOT, { recursive: true });
  for (;;) {
    try {
      const descriptor = fs.openSync(LOCK_FILE, "wx");
      try {
        fs.writeSync(descriptor, JSON.stringify({ pid: process.pid, started_at: new Date().toISOString() }));
      } finally {
        fs.closeSync(descriptor);
      }
      return;
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
    }
    const holder = loadJson(LOCK_FILE, "lock file");
    if (!Number.isSafeInteger(holder.pid)) fail(`${LOCK_FILE} does not name a pid`);
    let holderAlive = true;
    try {
      process.kill(holder.pid, 0);
    } catch (error) {
      if (error.code === "ESRCH") holderAlive = false;
      else if (error.code !== "EPERM") throw error;
    }
    if (holderAlive) {
      fail(
        `history backfill is already running as pid ${holder.pid} (started ${holder.started_at}); ` +
          `refusing to share ${ROOT} with a second, concurrent run`,
      );
    }
    // The recorded pid is dead: a previous run crashed or was killed without
    // reaching the finally block below. Its lock is stale, not a live
    // conflict, so take it over rather than fail forever on an abandoned run.
    log(`lock held by dead pid ${holder.pid} (started ${holder.started_at}); taking it over`);
    fs.rmSync(LOCK_FILE, { force: true });
  }
}

function releaseLock() {
  fs.rmSync(LOCK_FILE, { force: true });
}

// verifySource does `git clone --bare` into REPOSITORY and buildTargets does
// `git worktree add` into SOURCES/<commit>; only EVIDENCE was ever cleared
// between runs, so a second run used to fail on the clone. Every directory
// keyed to one run's state is removed here; CACHE (the warm cargo target
// dir) is not -- see the comment on TARGET above.
function resetWorkingState() {
  // WIPE_ON_START is a fixed list of names joined onto ROOT, so a ROOT of "/"
  // or the bare home directory would recursively delete real directories that
  // happen to share those names. ROOT is operator-supplied through an
  // environment variable, which is exactly the kind of input that is right
  // until one day it is not.
  const resolved = path.resolve(ROOT);
  if (resolved === path.parse(resolved).root || resolved === path.resolve(os.homedir())) {
    fail(
      `RUSTIA_BACKFILL_ROOT resolves to ${resolved}; it must be a directory dedicated to this ` +
        `runner, because starting a run erases the working state beneath it`,
    );
  }
  for (const directory of WIPE_ON_START) {
    fs.rmSync(directory, { recursive: true, force: true });
  }
  fs.rmSync(EXPORT_ARCHIVE, { force: true });
}

async function execute(configFile, providerFile, bundle) {
  if (process.platform !== "linux") {
    fail("history backfill runner must execute on Linux");
  }
  const startedAtMs = Date.now();
  acquireLock();
  try {
    resetWorkingState();
    fs.mkdirSync(EVIDENCE, { recursive: true });
    fs.writeFileSync(RUN_LOG, "", { mode: 0o600 });
    logReady = true;
    const config = validateConfig(loadJson(configFile, "protocol config"));
    // The file is still shipped and archived as evidence (see publish()
    // below); only its old role gating benchmark_cpu against an attested
    // provider.vcpus is gone -- see the comment in the try block below.
    requireObject(loadJson(providerFile, "provider metadata"), "provider metadata");
    let status = {
      schema: "rustia.bench.cloud-run-status.v1",
      status: "failed",
      detail: "history backfill runner did not reach its terminal state",
      started_at: new Date().toISOString(),
    };
    try {
      // The Linode-era check here compared host_gate.benchmark_cpu against an
      // attested provider.vcpus from a freshly-provisioned instance. An owned
      // host is not freshly provisioned each run, so validateConfig's
      // pinned_cpus cross-check now catches an unisolated benchmark_cpu at
      // config-load time, before anything is cloned or built.
      log("verifying fixed harness, historical targets, and source hashes");
      verifySource(config, bundle);
      installRust();
      log(`building ${config.source.target_commits.length} historical Rustia revisions`);
      const builds = buildTargets(config, startedAtMs);
      // The old runner also stopped and masked apt's periodic timers here.
      // That is an unrequested, root-only side effect on a machine this
      // project does not own exclusively for benchmarking; the host_gate
      // idle/steal checks already reject a run a timer contaminates.
      log("measuring fixed current workloads across the historical binaries");
      const measurement = await measure(config, builds, EVIDENCE, startedAtMs);
      const repeatability = repeatabilityCheck(builds, measurement.rows);
      publish(config, configFile, providerFile, builds, measurement, EVIDENCE, repeatability);
      fs.copyFileSync(
        path.join(EXPORT, "publication/bench/history/backfill.json"),
        path.join(ROOT, "backfill-publication.json"),
      );
      status = {
        ...status,
        status: "backfill_complete",
        detail: "fixed-harness Rustia history backfill completed",
        finished_at: new Date().toISOString(),
      };
    } catch (error) {
      status = {
        ...status,
        status: "failed",
        detail: error.message,
        finished_at: new Date().toISOString(),
      };
      log(`failed: ${error.message}`);
    } finally {
      exportRun(status, EVIDENCE);
    }
    return status.status === "backfill_complete" ? 0 : 2;
  } finally {
    releaseLock();
  }
}

function fakeCommits(count, seed) {
  return Array.from({ length: count }, (_, index) =>
    crypto.createHash("sha1").update(`${seed}-${index}`).digest("hex"),
  );
}

function repairFixture(overrides) {
  return {
    commit: overrides.commit,
    subject: "test repair",
    cherry_pick: {
      from_commit: overrides.fromCommit,
      from_subject: "test source commit",
      paths: overrides.paths,
    },
    why_it_could_not_be_measured: "x",
    changes_applied: ["x"],
    why_the_timing_is_still_this_revision: "x",
    why_it_is_derived_rather_than_stored: "x",
    verification: "x",
  };
}

async function selfTest() {
  const hashes = Object.fromEntries(
    Object.keys(OVERLAY_FILES).map((field) => [field, "a".repeat(64)]),
  );
  const commitA = "2".repeat(40);
  const commitB = "3".repeat(40);
  const config = {
    schema: "rustia.dedicated-history-backfill.v1",
    provider: {
      host: {
        name: "ci-bench",
        cpu_model: "AMD Ryzen 5 3600X",
        logical_cpus: 6,
        physical_cores: 6,
        pinned_cpus: "4",
        max_runtime_minutes: 180,
      },
    },
    source: {
      harness_commit: "1".repeat(40),
      revision_selection: {
        walk: "git log --topo-order --reverse over every engine path",
        includes_non_first_parent_commits: true,
        ordering_is_presentational: true,
        note: "test fixture",
      },
      target_commits: [commitA, commitB],
      source_modifications: [],
      expected_sha256: hashes,
    },
    measurement: {
      protocol: PROTOCOL,
      native_target: "linux-x86_64",
      workloads: WORKLOADS,
      fixed_n: Object.fromEntries(WORKLOADS.map((workload) => [workload, 10])),
      repetitions: 5,
      expected_checksums: Object.fromEntries(
        WORKLOADS.map((workload) => [workload, `number:${workload.length}`]),
      ),
    },
    host_gate: {
      benchmark_cpu: 4,
      cooldown_seconds: 30,
      idle_window_seconds: 10,
      minimum_idle_percent: 99,
      maximum_load_1m: 0.1,
      maximum_steal_ticks: 0,
      maximum_timed_steal_percent: 0,
      wait_timeout_seconds: 900,
      maximum_timed_attempts_per_group: 3,
    },
    publication: { mode: "history_backfill", rows: 10 },
  };
  assert.equal(validateConfig(config), config);

  // Existing reconstruction configs omit target_relation and retain their
  // original target-before-harness interpretation. New configs make the
  // direction explicit, and the live series uses the opposite direction.
  for (const target_relation of ["targets_are_ancestors", "harness_is_ancestor"]) {
    assert.equal(
      validateConfig({ ...config, source: { ...config.source, target_relation } }).source.target_relation,
      target_relation,
    );
  }
  assert.throws(() =>
    validateConfig({
      ...config,
      source: { ...config.source, target_relation: "unrelated" },
    }),
  );

  // The old Linode schema string must now be rejected outright.
  assert.throws(() => validateConfig({ ...config, schema: "rustia.linode-fixed-benchmark.v2" }));

  // Unlike the retired Linode cap (target_commits.length <= 64, which bounded
  // a rental bill rather than correctness), an owned host must accept far
  // more than that -- 75 is the actual size of the frozen ci-bench contract.
  const manyCommits = fakeCommits(75, "history-backfill-selftest");
  assert.equal(
    validateConfig({
      ...config,
      source: { ...config.source, target_commits: manyCommits, source_modifications: [] },
      publication: { mode: "history_backfill", rows: manyCommits.length * WORKLOADS.length },
    }).source.target_commits.length,
    75,
  );

  assert.throws(() =>
    validateConfig({
      ...config,
      source: { ...config.source, target_commits: [commitA, commitA] },
    }),
  );
  assert.throws(() =>
    validateConfig({ ...config, publication: { mode: "history_backfill", rows: 9 } }),
  );

  // config.source.revision_selection is required.
  assert.throws(() => {
    const { revision_selection, ...sourceWithoutSelection } = config.source;
    validateConfig({ ...config, source: sourceWithoutSelection });
  });

  // A repair's commit must be one of target_commits.
  assert.throws(() =>
    validateConfig({
      ...config,
      source: {
        ...config.source,
        source_modifications: [repairFixture({ commit: "9".repeat(40), fromCommit: commitB, paths: ["Cargo.toml"] })],
      },
    }),
  );

  // A repair must not touch bench/ -- that is the harness overlay's territory.
  assert.throws(() =>
    validateConfig({
      ...config,
      source: {
        ...config.source,
        source_modifications: [repairFixture({ commit: commitA, fromCommit: commitB, paths: ["bench/Cargo.toml"] })],
      },
    }),
  );

  // benchmark_cpu must be one of the pinned CPUs, not merely a valid integer.
  assert.throws(() => validateConfig({ ...config, host_gate: { ...config.host_gate, benchmark_cpu: 5 } }));

  assert.deepEqual(
    cpuDelta(
      { fields: [10, 0, 5, 100, 2, 1, 1, 0] },
      { fields: [11, 0, 6, 198, 2, 1, 1, 0] },
    ),
    { total_ticks: 100, idle_ticks: 98, steal_ticks: 0, idle_percent: 98, steal_percent: 0 },
  );

  // Zero tolerance for timed-leg steal on a dedicated, isolcpus-pinned host --
  // there is no maximum_timed_steal_ticks fallback left to forgive a tick.
  const gate = { maximum_timed_steal_percent: 0 };
  assert.equal(timedGatePasses(gate, { steal_percent: 0 }), true);
  assert.equal(timedGatePasses(gate, { steal_percent: 0.0001 }), false);

  assert.deepEqual(expandCpuList("4", "test"), [4]);
  assert.deepEqual(expandCpuList("4,6", "test"), [4, 6]);
  assert.deepEqual(expandCpuList("4-6", "test"), [4, 5, 6]);

  // repeatabilityCheck: two builds share a binary; the spread between their
  // independently measured ns_per_unit_min is computed, not thresholded.
  {
    const builds = [
      { commit: commitA, binary_sha256: "same" },
      { commit: commitB, binary_sha256: "same" },
    ];
    const rows = WORKLOADS.map((workload, index) => ({ commit: commitA, workload, ns_per_unit_min: 100 + index })).concat(
      WORKLOADS.map((workload, index) => ({ commit: commitB, workload, ns_per_unit_min: 110 + index })),
    );
    const repeatability = repeatabilityCheck(builds, rows);
    assert.equal(repeatability.available, true);
    assert.equal(repeatability.groups.length, 1);
    assert.equal(repeatability.comparisons.length, WORKLOADS.length);
    const expectedSpread = ((110 - 100) / 100) * 100;
    assert.ok(Math.abs(repeatability.worst_spread_percent - expectedSpread) < 1e-9);
    const none = repeatabilityCheck(
      [{ commit: commitA, binary_sha256: "one" }, { commit: commitB, binary_sha256: "two" }],
      rows,
    );
    assert.equal(none.available, false);
  }

  // validateRows must reject a row measured on the wrong host -- it would
  // silently mix in numbers from a different machine's cache, clock, kernel.
  {
    const builds = [commitA, commitB].map((commit) => ({ commit, binary_sha256: "x" }));
    const goodRows = builds.flatMap((build) =>
      WORKLOADS.map((workload) => ({
        commit: build.commit,
        engine: "rustia",
        dialect: "luau",
        target: "linux-x86_64",
        host: config.provider.host.name,
        workload,
        n: config.measurement.fixed_n[workload],
        reps: config.measurement.repetitions,
        checksum: config.measurement.expected_checksums[workload],
        ns_per_unit_min: 1,
        ns_per_unit_median: 1,
        units_per_sec: 1,
      })),
    );
    const rowsConfig = { ...config, publication: { mode: "history_backfill", rows: goodRows.length } };
    validateRows(rowsConfig, builds, goodRows);
    const badRows = goodRows.map((row, index) => (index === 0 ? { ...row, host: "some-other-box" } : row));
    assert.throws(() => validateRows(rowsConfig, builds, badRows));
  }

  // A runtime budget started in the distant past must trip; one started now must not.
  assert.throws(() => checkRuntimeBudget(config, Date.now() - 999 * 24 * 60 * 60 * 1000, "self-test"));
  checkRuntimeBudget(config, Date.now(), "self-test");

  process.stdout.write("history backfill runner self-test passed\n");
}

async function main() {
  if (process.argv[2] === "self-test") {
    if (process.argv.length !== 3) fail("self-test takes no additional arguments");
    await selfTest();
    return;
  }
  if (process.argv.length !== 5) {
    process.stderr.write(
      "usage: run-linux-history-backfill.js CONFIG PROVIDER BUNDLE\n",
    );
    process.exitCode = 2;
    return;
  }
  process.exitCode = await execute(process.argv[2], process.argv[3], process.argv[4]);
}

main().catch((error) => {
  process.stderr.write(`run-linux-history-backfill: ${error.message}\n`);
  process.exitCode = 1;
});
