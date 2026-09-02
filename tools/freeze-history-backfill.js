#!/usr/bin/env node

"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const MAIN_SERIES = "bench/history/series/ci-bench-main.json";
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

function loadSeriesProtocol(file) {
  const resolved = path.resolve(REPO, file);
  const relative = path.relative(REPO, resolved);
  if (relative.startsWith("..") || path.isAbsolute(relative)) {
    fail("series protocol path must remain inside the repository");
  }
  let manifest;
  try {
    manifest = JSON.parse(fs.readFileSync(resolved, "utf8"));
  } catch (error) {
    fail(`could not read series protocol ${relative}: ${error.message}`);
  }
  if (manifest.schema !== "rustia.bench.history-series.v1" || manifest.schema_version !== 1) {
    fail(`${relative} must declare rustia.bench.history-series.v1`);
  }
  if (manifest.protocol !== "rustia_main_series_v1") {
    fail(`${relative}.protocol must be rustia_main_series_v1`);
  }
  if (!FULL_COMMIT.test(manifest.harness_commit ?? "")) {
    fail(`${relative}.harness_commit must be a full 40-hex commit`);
  }
  if (manifest.native_target !== "linux-x86_64") {
    fail(`${relative}.native_target must be linux-x86_64`);
  }
  if (JSON.stringify(manifest.workloads) !== JSON.stringify(WORKLOADS)) {
    fail(`${relative}.workloads must match the frozen workload order`);
  }
  if (!Number.isSafeInteger(manifest.repetitions) || manifest.repetitions < 1) {
    fail(`${relative}.repetitions must be a positive integer`);
  }
  if (typeof manifest.host !== "string" || manifest.host === "") {
    fail(`${relative}.host must be a non-empty string`);
  }
  if (!Number.isSafeInteger(manifest.pinned_core) || manifest.pinned_core < 0) {
    fail(`${relative}.pinned_core must be a non-negative integer`);
  }
  const fixedN = {};
  const checksums = {};
  for (const workload of WORKLOADS) {
    const n = manifest.fixed_n?.[workload];
    if (!Number.isSafeInteger(n) || n < 1) {
      fail(`${relative}.fixed_n.${workload} must be a positive integer`);
    }
    const checksum = manifest.expected_checksums?.[workload];
    if (typeof checksum !== "string" || checksum === "") {
      fail(`${relative}.expected_checksums.${workload} must be a non-empty string`);
    }
    fixedN[workload] = n;
    checksums[workload] = checksum;
  }
  return {
    harness_commit: manifest.harness_commit,
    host: manifest.host,
    pinned_core: manifest.pinned_core,
    repetitions: manifest.repetitions,
    fixed_n: fixedN,
    expected_checksums: checksums,
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

/// The first commit that contains an engine at all. Everything before it is
/// design notes, licences and demos with no `crates/` tree, so there is nothing
/// to measure. Used when the operator names no start commit, which is what lets
/// `just bench-history` take no arguments.
function firstEngineCommit() {
  const output = git(["log", "--reverse", "--format=%H", "--", ...ENGINE_PATHS]).trim();
  const first = output ? output.split("\n", 1)[0] : "";
  if (!FULL_COMMIT.test(first)) fail("could not locate the first Rustia engine revision");
  return first;
}

/// Selects and validates the repairs that apply to one run's commits. Pure, so
/// the admissibility rules can be tested without a repository: `resolveParent`
/// maps a commit to its first parent. Throws on the first inadmissible repair.
function selectRepairs(registry, selected, resolveParent) {
  if (registry.schema !== "rustia.bench.history-repairs.v1") {
    throw new Error("bench/history/repairs.json must declare schema rustia.bench.history-repairs.v1");
  }
  const chosen = new Set(selected);
  const repairs = (registry.repairs ?? []).filter((repair) => chosen.has(repair.commit));
  for (const repair of repairs) {
    const { commit, cherry_pick: pick } = repair;
    if (!pick || !FULL_COMMIT.test(pick.from_commit ?? "")) {
      throw new Error(`repair for ${commit} must name a full cherry_pick.from_commit`);
    }
    // The parent relation is the whole admissibility argument: a repair may
    // only take code from the commit immediately following the revision it
    // repairs. Checked here so an inadmissible registry fails at freeze time
    // rather than twenty minutes into a run.
    const parent = resolveParent(pick.from_commit);
    if (parent !== commit) {
      throw new Error(
        `repair for ${commit} is inadmissible: ${pick.from_commit.slice(0, 8)}^ is ` +
          `${String(parent).slice(0, 8)}, not the repaired commit`,
      );
    }
    if (!Array.isArray(pick.paths) || pick.paths.length === 0) {
      throw new Error(`repair for ${commit} must name at least one cherry_pick path`);
    }
    for (const file of pick.paths) {
      if (file.startsWith("bench/")) {
        throw new Error(
          `repair for ${commit} may not touch ${file}: bench/ belongs to the harness overlay`,
        );
      }
    }
  }
  return repairs;
}

/// Source repairs for revisions a fixed harness cannot otherwise drive, filtered
/// to the run's selected commits. Each is re-derived from Git by the runner; the
/// config carries the derivation, never the patch bytes.
function sourceModifications(selected) {
  let registry;
  try {
    registry = JSON.parse(fs.readFileSync(path.join(REPO, "bench/history/repairs.json"), "utf8"));
  } catch (error) {
    fail(`could not read bench/history/repairs.json: ${error.message}`);
  }
  try {
    return selectRepairs(registry, selected, (commit) => git(["rev-parse", `${commit}^`]).trim());
  } catch (error) {
    fail(error.message);
  }
}

function freeze(start, outputFile, only, pair, repetitions, seriesFile) {
  const tracked = git(["status", "--porcelain=v1", "--untracked-files=no"]).trim();
  if (tracked) fail("tracked worktree changes must be committed before freezing a backfill");
  const head = git(["rev-parse", "HEAD"]).trim();
  const series = seriesFile ? loadSeriesProtocol(seriesFile) : null;
  const harness = series
    ? git(["rev-parse", `${series.harness_commit}^{commit}`]).trim()
    : head;
  const resolvedStart = start
    ? git(["rev-parse", `${start}^{commit}`]).trim()
    : firstEngineCommit();
  if (!FULL_COMMIT.test(harness) || !FULL_COMMIT.test(resolvedStart)) fail("invalid Git revision");
  // The owned benchmark host, taken verbatim from the committed dedicated
  // config so host identity has exactly one source in the repository.
  const baseConfig = JSON.parse(
    fs.readFileSync(path.join(REPO, "bench/cloud/dedicated-core2-tape-sparse.json"), "utf8"),
  );
  if (baseConfig.schema !== "rustia.dedicated-fixed-benchmark.v1") {
    fail("dedicated base config must declare schema rustia.dedicated-fixed-benchmark.v1");
  }
  if (series) {
    try {
      execFileSync("git", ["merge-base", "--is-ancestor", harness, head], {
        cwd: REPO,
        stdio: "ignore",
      });
    } catch {
      fail(`${seriesFile}.harness_commit must be an ancestor of HEAD`);
    }
    if (series.host !== baseConfig.provider.host.name) {
      fail(`${seriesFile}.host does not match the dedicated host config`);
    }
    if (series.pinned_core !== baseConfig.host_gate.benchmark_cpu) {
      fail(`${seriesFile}.pinned_core does not match the dedicated host config`);
    }
  }
  const measurements = series ?? loadCurrentRustiaRows();
  const frozenRepetitions = series?.repetitions ?? repetitions;
  // `--only` freezes a single revision. With `--series`, the per-commit lane
  // takes the harness, fixed sizes, checksums and repetitions from the reviewed
  // series manifest rather than whichever acceptance report happens to be
  // current, so the reconstruction and live rows remain one protocol.
  // `--pair` freezes exactly two revisions in the order given. The PR lane runs
  // this twice with the order reversed, so the two sides interleave as A B B A
  // rather than being measured one after the other -- host drift over the run
  // then lands on both sides instead of on whichever went second.
  let commits;
  if (pair) {
    commits = pair.map((revision) => git(["rev-parse", `${revision}^{commit}`]).trim());
    if (!commits.every((commit) => FULL_COMMIT.test(commit))) {
      fail("--pair must name two resolvable commits");
    }
    if (commits[0] === commits[1]) {
      fail("--pair needs two different commits; compare a commit with itself across legs instead");
    }
  } else if (only) {
    commits = [git(["rev-parse", `${only}^{commit}`]).trim()];
    if (!FULL_COMMIT.test(commits[0])) fail("--only must name a resolvable commit");
  } else {
    commits = targetCommits(resolvedStart, harness);
  }
  const config = {
    schema: "rustia.dedicated-history-backfill.v1",
    provider: baseConfig.provider,
    source: {
      harness_commit: harness,
      // A reconstruction measures revisions leading up to its harness. The
      // live series deliberately keeps that reviewed harness fixed while main
      // advances, so its targets are descendants instead.
      target_relation: series ? "harness_is_ancestor" : "targets_are_ancestors",
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
      // Derivations, not patch bytes. The runner re-reads each diff from Git at
      // apply time, so "taken verbatim from the commit that introduced it"
      // cannot drift from what is actually applied.
      source_modifications: sourceModifications(commits),
      expected_sha256: Object.fromEntries(
        Object.entries(HASH_FILES).map(([field, file]) => [field, sha256(gitBlob(harness, file))]),
      ),
    },
    measurement: {
      protocol: "rustia_history_backfill_v1",
      native_target: "linux-x86_64",
      workloads: WORKLOADS,
      fixed_n: measurements.fixed_n,
      // The reconstruction and the per-commit series both use five, and a row
      // that is to join that series has to be measured the same way. The PR lane
      // publishes nothing, so it is free to buy a tighter interval with more
      // repetitions -- which is the only reason this is a parameter.
      repetitions: frozenRepetitions,
      expected_checksums: measurements.expected_checksums,
    },
    // The dedicated host's gate, plus the retry bound the backfill protocol
    // needs and the fixed-benchmark config has no use for. Written out rather
    // than copied so a reader sees the whole gate in one place: `pinned_cpus`
    // and `benchmark_cpu` have to agree, and the steal ceilings are zero
    // because steal is structurally absent on bare metal, which makes any
    // non-zero reading a fault rather than a tolerance.
    host_gate: {
      ...baseConfig.host_gate,
      maximum_timed_attempts_per_group: 3,
    },
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
    "Usage: node tools/freeze-history-backfill.js [--start COMMIT | --only COMMIT |\n" +
      "                                            --pair BASELINE CANDIDATE]\n" +
      "                                           [--series FILE.json]\n" +
      "                                           [--repetitions N] [--output FILE.json]\n" +
      "       node tools/freeze-history-backfill.js --self-test\n" +
      "\n" +
      "Without --start the series begins at the first commit that contains an engine.\n" +
      "Without --output the config is written to stdout.\n",
  );
}

function selfTest() {
  const A = "a".repeat(40);
  const B = "b".repeat(40);
  const parents = { [B]: A };
  const resolve = (commit) => parents[commit] ?? "";
  const registry = {
    schema: "rustia.bench.history-repairs.v1",
    repairs: [{ commit: A, cherry_pick: { from_commit: B, paths: ["Cargo.toml"] } }],
  };
  const check = (name, run, expected) => {
    let message = null;
    try {
      run();
    } catch (error) {
      message = error.message;
    }
    if (expected === null) {
      if (message !== null) fail(`self-test ${name}: unexpected failure ${message}`);
      return;
    }
    if (message === null || !message.includes(expected)) {
      fail(`self-test ${name}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(message)}`);
    }
  };

  check("admissible repair", () => {
    const picked = selectRepairs(registry, [A], resolve);
    if (picked.length !== 1) throw new Error(`expected 1 repair, got ${picked.length}`);
  }, null);

  check("repairs outside the selected commits are dropped", () => {
    const picked = selectRepairs(registry, ["c".repeat(40)], resolve);
    if (picked.length !== 0) throw new Error(`expected 0 repairs, got ${picked.length}`);
  }, null);

  check(
    "repair whose source is not the next commit",
    () => selectRepairs(registry, [A], () => "d".repeat(40)),
    "inadmissible",
  );
  check(
    "repair touching the harness overlay",
    () =>
      selectRepairs(
        { ...registry, repairs: [{ commit: A, cherry_pick: { from_commit: B, paths: ["bench/src/main.rs"] } }] },
        [A],
        resolve,
      ),
    "belongs to the harness overlay",
  );
  check(
    "repair with no paths",
    () =>
      selectRepairs(
        { ...registry, repairs: [{ commit: A, cherry_pick: { from_commit: B, paths: [] } }] },
        [A],
        resolve,
      ),
    "at least one cherry_pick path",
  );
  check("wrong registry schema", () => selectRepairs({ schema: "nope" }, [A], resolve), "must declare schema");

  // The committed registry must itself be admissible against the real history.
  sourceModifications([...new Set((JSON.parse(
    fs.readFileSync(path.join(REPO, "bench/history/repairs.json"), "utf8"),
  ).repairs ?? []).map((repair) => repair.commit))]);

  // The argument surface. --pair is two commits in the order given, is mutually
  // exclusive with the two series selectors, and refuses a commit paired with
  // itself -- that comparison exists, but it is legs of one revision, not a pair.
  const args = (list) => parseArgs(list);
  if (args(["--pair", "aaa", "bbb"])?.pair?.length !== 2) fail("self-test: --pair must take two values");
  if (args(["--pair", "aaa", "bbb", "--only", "ccc"]) !== null) fail("self-test: --pair with --only must be refused");
  if (args(["--pair", "aaa", "bbb", "--start", "ccc"]) !== null) fail("self-test: --pair with --start must be refused");
  if (args(["--pair", "aaa"]) !== null) fail("self-test: --pair with one value must be refused");
  if (args(["--repetitions", "25"])?.repetitions !== 25) fail("self-test: --repetitions must parse");
  if (args(["--repetitions", "0"]) !== null) fail("self-test: --repetitions 0 must be refused");
  if (args(["--repetitions", "-3"]) !== null) fail("self-test: negative --repetitions must be refused");
  if (args(["--repetitions", "1e6"]) !== null) fail("self-test: non-integer --repetitions must be refused");
  if (args(["--repetitions", "10001"]) !== null) fail("self-test: unbounded --repetitions must be refused");
  if (args([])?.repetitions !== undefined) fail("self-test: --repetitions must default to unset");
  if (args(["--only", "HEAD", "--series", MAIN_SERIES])?.series !== MAIN_SERIES) {
    fail("self-test: --series must parse with --only");
  }
  if (args(["--series", MAIN_SERIES]) !== null) fail("self-test: --series requires --only");
  if (args(["--only", "HEAD", "--series", MAIN_SERIES, "--repetitions", "5"]) !== null) {
    fail("self-test: the series manifest owns repetitions");
  }

  process.stdout.write("freeze-history-backfill self-test ok\n");
}

function parseArgs(argv) {
  const parsed = {
    start: undefined,
    output: undefined,
    only: undefined,
    pair: undefined,
    repetitions: undefined,
    series: undefined,
    selfTest: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--self-test") parsed.selfTest = true;
    else if (option === "--start") parsed.start = argv[++index];
    else if (option === "--output") parsed.output = argv[++index];
    else if (option === "--only") parsed.only = argv[++index];
    else if (option === "--pair") parsed.pair = [argv[++index], argv[++index]];
    else if (option === "--repetitions") parsed.repetitions = argv[++index];
    else if (option === "--series") parsed.series = argv[++index];
    else return null;
  }
  if (parsed.selfTest && (parsed.start || parsed.output || parsed.only || parsed.pair || parsed.series)) return null;
  // --only names the whole series; a start commit would be ignored.
  if (parsed.only !== undefined && parsed.start !== undefined) return null;
  for (const value of [parsed.start, parsed.output, parsed.only, parsed.series]) {
    if (value !== undefined && !value) return null;
  }
  if (parsed.pair !== undefined) {
    if (parsed.start !== undefined || parsed.only !== undefined) return null;
    if (parsed.pair.length !== 2 || parsed.pair.some((value) => !value)) return null;
  }
  if (parsed.repetitions !== undefined) {
    if (!/^[1-9][0-9]*$/.test(parsed.repetitions)) return null;
    parsed.repetitions = Number(parsed.repetitions);
    if (parsed.repetitions > 10_000) return null;
  }
  if (parsed.series !== undefined) {
    if (parsed.only === undefined || parsed.start !== undefined || parsed.pair !== undefined) return null;
    if (parsed.repetitions !== undefined) return null;
  }
  return parsed;
}

// The files whose bytes the frozen config pins by hash, plus the one it reads
// its fixed sizes and expected checksums from. Exported so the screening lane
// refuses exactly the changes that would break a comparison, rather than a
// directory that happens to contain them.
module.exports = { HASH_FILES, CURRENT_RESULTS: "bench/current-results.json", MAIN_SERIES };

if (require.main !== module) {
  // Required for the constants above; nothing else runs.
} else try {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed === null) {
    usage();
    process.exitCode = 2;
  } else if (parsed.selfTest) {
    selfTest();
  } else {
    freeze(parsed.start, parsed.output, parsed.only, parsed.pair, parsed.repetitions ?? 5, parsed.series);
  }
} catch (error) {
  process.stderr.write(`freeze-history-backfill: ${error.message}\n`);
  process.exitCode = 1;
}
