#!/usr/bin/env node
"use strict";

// Coordinator for the owned benchmark host.
//
// This is the four cloud coordinators minus the parts that only existed to
// manage a rental: no provisioning, no billing ceiling, no destroy gate, no
// resource-ownership journal. What remains is ship, run, collect, promote.
//
// The host is named only by an ssh destination in RUSTIA_BENCH_SSH, which the
// operator keeps in their own ssh config. No hostname, address, key path or
// known-hosts entry belongs in this repository (CLAUDE.md: nothing
// infra-identifying lands here). `provider.host.name` is a provenance label
// that is published in row data; it is NOT an ssh target and must never be
// used as one, and sshDestination() refuses a destination whose host is any
// label this repository publishes.

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/dedicated");
const CONFIG_DIR = path.join(REPO, "bench/cloud");
const BACKFILL_DIR = path.join(REPO, "bench/history/backfills");
const EVIDENCE_DIR = path.join(REPO, "bench/evidence/history-backfill");
// Two runners reach the same host the same way. They differ only in which
// script is shipped, which directory it owns, and which variable names that
// directory -- so the transport is one function and the lane is data, rather
// than a second copy of ship/run/fetch that agrees with the first until one of
// them is edited.
const LANES = Object.freeze({
  history: Object.freeze({
    runner: "tools/run-linux-history-backfill.js",
    root: ".rustia-history-backfill",
    rootEnv: "RUSTIA_BACKFILL_ROOT",
  }),
  matrix: Object.freeze({
    runner: "tools/run-linux-matrix-benchmark.js",
    root: ".rustia-matrix",
    rootEnv: "RUSTIA_MATRIX_ROOT",
  }),
  memory: Object.freeze({
    runner: "tools/run-linux-memory-benchmark.js",
    root: ".rustia-memory",
    rootEnv: "RUSTIA_MEMORY_ROOT",
  }),
});
const REMOTE_RUNNER = LANES.history.runner;
const REMOTE_ROOT = LANES.history.root;
const SSH_ENV = "RUSTIA_BENCH_SSH";
const FULL_COMMIT = /^[0-9a-f]{40}$/;

// Eight legs a side. The calibration measured this host at two and found its
// spread widened by leg variation, so this is the lever that tightens an
// acceptance interval; it is even so that A B B A stays balanced.
const DEFAULT_ACCEPTANCE_PASSES = 8;

// Free space the host must have under $HOME before a run may start. The lanes'
// working roots and the runner's own work directory all live there, and on
// 2026-09-06 a filled disk failed the weekly matrix before this script ran,
// as a runner-level error that named no cause. This is a floor that catches
// that failure, not a measurement of what a build needs; a run that
// legitimately needs more should raise it with the figure it observed.
const MINIMUM_FREE_GIB = 20;

// A non-interactive ssh session does not read the login profile, so neither
// rustup's nor a home-directory node install is on PATH by default. Both are
// user-local by design (nothing on this host needs root), so the coordinator
// names them explicitly rather than depending on the remote shell's setup. An
// earlier backfill lost 25 minutes to exactly this: cargo was absent from a
// non-interactive PATH, every build failed, and the loop discarded the error.
const REMOTE_PATH = 'export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"\n';

function fail(message) {
  console.error(`run-dedicated-benchmark: ${message}`);
  process.exit(1);
}

function sshDestination() {
  const destination = process.env[SSH_ENV];
  if (!destination || !destination.trim()) {
    fail(
      `set ${SSH_ENV} to the benchmark host's ssh destination or ssh_config alias ` +
        `(kept outside this repository; see rustia-ops)`,
    );
  }
  const problem = sshDestinationProblem(destination.trim(), publishedHostLabels(REPO));
  if (problem !== null) fail(problem);
  return destination.trim();
}

// A provenance label is public and an ssh destination is private, so the two
// must never be one name. Saying so in prose did not make it so: an ssh_config
// alias spelt like the published label would quietly make the label an ssh
// target, and host-check and unlock reach the host without loading a config to
// compare against. So every ssh and scp call takes its destination from
// sshDestination(), which checks it against every label the repository
// publishes, read afresh each time rather than restated here.
//
// Every owned-host lane freezes its config into bench/cloud under this prefix
// -- history, matrix and both memory cohorts -- so a config frozen by a lane
// added later is read without a second list.
const DEDICATED_CONFIG = /^dedicated-.*\.json$/;

/// Every provenance label this repository publishes: the host each runner
/// class names in bench/operations-status.json, and provider.host.name of every
/// frozen owned-host config in bench/cloud.
function publishedHostLabels(root) {
  const labels = new Set();
  const status = JSON.parse(fs.readFileSync(path.join(root, "bench/operations-status.json"), "utf8"));
  for (const runner of Object.values(status?.runner_classes ?? {})) {
    if (typeof runner?.host === "string") labels.add(runner.host);
  }
  const configDir = path.join(root, "bench/cloud");
  for (const name of fs.readdirSync(configDir).filter((file) => DEDICATED_CONFIG.test(file))) {
    const label = JSON.parse(fs.readFileSync(path.join(configDir, name), "utf8"))?.provider?.host?.name;
    if (typeof label === "string") labels.add(label);
  }
  return [...labels];
}

/// The host an ssh destination names, from `[ssh://][user@]host[:port][/]`. The
/// user ends at the last `@`, which is where ssh splits it. The host ends at a
/// `:` or a `/`, where ssh's URI parser ends it: that parser accepts an empty
/// path, so `ssh://host/` reaches `host`. An IPv6 literal is bracketed so its
/// colons are not read as a port.
function sshHostPart(destination) {
  const authority = destination.replace(/^ssh:\/\//i, "");
  const host = authority.slice(authority.lastIndexOf("@") + 1);
  if (host.startsWith("[")) {
    const close = host.indexOf("]");
    return close === -1 ? host : host.slice(1, close);
  }
  return host.split(/[:/]/)[0];
}

/// Why `destination` must not be used as an ssh target, or null. ssh lowercases
/// a host before matching it against ssh_config, so case is ignored. The message
/// names the published label and nothing else of the destination: the user,
/// port and spelling the operator chose stay out of the log.
function sshDestinationProblem(destination, labels) {
  const host = sshHostPart(destination).toLowerCase();
  const label = labels.find((candidate) => candidate.toLowerCase() === host);
  if (label === undefined) return null;
  return (
    `${SSH_ENV} names ${JSON.stringify(label)}, which is a provenance label published in row data, ` +
    `never an ssh target: use a private ssh_config alias that is not a published label ` +
    `(kept outside this repository; see rustia-ops)`
  );
}

function git(args) {
  return execFileSync("git", args, { cwd: REPO, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
}

function sha256File(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

function writeJson(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, `${JSON.stringify(value, null, 1)}\n`);
}

function local(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: REPO,
    encoding: "utf8",
    stdio: options.capture === false ? "inherit" : ["ignore", "pipe", "pipe"],
    maxBuffer: 256 * 1024 * 1024,
  });
  if (result.status !== 0) {
    const detail = options.capture === false ? "" : `: ${(result.stderr ?? "").trim()}`;
    fail(`${command} ${args.join(" ")} failed${detail}`);
  }
  return result.stdout ?? "";
}

// The attested host is either reached over ssh, or is this machine when the
// coordinator runs on it -- which is what a workflow on the bench runner does.
// Both paths run the same script and, critically, both attest before measuring:
// running on the host does not exempt a run from proving the host is what the
// config says.
let localMode = false;
function useLocalHost(enabled) { localMode = enabled; }

function shellArgv() {
  return localMode
    ? ["bash", ["-s"]]
    : ["ssh", ["-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", sshDestination(), "bash", "-s"]];
}

// spawnSync needs the script on stdin, which `local` does not thread, so these
// drive spawnSync directly rather than growing an option no other caller needs.
function sshCapture(script) {
  const [command, args] = shellArgv();
  const result = spawnSync(command, args, {
    input: script, encoding: "utf8", maxBuffer: 256 * 1024 * 1024,
  });
  if (result.status !== 0) {
    fail(`${command} failed: ${(result.stderr ?? "").trim() || `exit ${result.status}`}`);
  }
  return result.stdout ?? "";
}

function sshStream(script) {
  const [command, args] = shellArgv();
  const result = spawnSync(command, args, { input: script, stdio: ["pipe", "inherit", "inherit"] });
  if (result.status !== 0) fail(`run failed with exit ${result.status}`);
}

// ---------------------------------------------------------------------------
// Pure helpers (covered by --self-test)
// ---------------------------------------------------------------------------

/// Expands a Linux CPU list ("4", "4,5", "4-5", "0-1,4") to sorted numbers.
function parseCpuList(spec) {
  const cpus = new Set();
  for (const part of String(spec).split(",")) {
    const piece = part.trim();
    if (!piece) continue;
    const range = piece.match(/^(\d+)-(\d+)$/);
    if (range) {
      const from = Number(range[1]);
      const to = Number(range[2]);
      if (to < from) throw new Error(`invalid CPU range ${piece}`);
      for (let cpu = from; cpu <= to; cpu += 1) cpus.add(cpu);
      continue;
    }
    if (!/^\d+$/.test(piece)) throw new Error(`invalid CPU list entry ${piece}`);
    cpus.add(Number(piece));
  }
  if (cpus.size === 0) throw new Error(`CPU list ${JSON.stringify(spec)} is empty`);
  return [...cpus].sort((left, right) => left - right);
}

/// Compares the host the config claims against what the machine reports.
/// Returns a list of human-readable mismatches; empty means attested.
function compareAttestation(declared, observed) {
  const problems = [];
  // /proc/cpuinfo appends marketing text the config's model name omits ("AMD
  // Ryzen 5 3600X" against "AMD Ryzen 5 3600X 6-Core Processor"), so requiring
  // equality would reject the very host the config describes. A prefix is the
  // right relation: it still rejects a different chip, which is the thing that
  // would silently change every measurement.
  const declaredModel = String(declared.cpu_model).trim().replace(/\s+/g, " ");
  const observedModel = String(observed.cpu_model ?? "").trim().replace(/\s+/g, " ");
  if (!observedModel.startsWith(declaredModel)) {
    problems.push(`cpu_model: config says ${declaredModel}, host reports ${observedModel}`);
  }
  if (observed.logical_cpus !== declared.logical_cpus) {
    problems.push(
      `logical_cpus: config says ${declared.logical_cpus}, host reports ${observed.logical_cpus}`,
    );
  }
  // "notsupported" means the kernel cannot control SMT, which covers both a CPU
  // that has none and one whose firmware disabled it — the case on this host,
  // where a 6-core/12-thread part reports 6 CPUs. Neither reading proves SMT is
  // off on its own, so the logical/physical equality check in the config and the
  // logical_cpus comparison above are what actually establish it; this only
  // rejects a machine that reports SMT explicitly switched on.
  if (observed.smt !== "off" && observed.smt !== "notsupported") {
    problems.push(`SMT must be off, host reports ${JSON.stringify(observed.smt)}`);
  }
  // Checked here rather than left to the build: a disk that fills mid-run
  // wastes the whole measurement, and the error it produces names the
  // symptom, not the cause.
  const freeKib = Number(observed.free_kib);
  if (!Number.isFinite(freeKib) || freeKib < MINIMUM_FREE_GIB * 1024 * 1024) {
    const reported = Number.isFinite(freeKib) ? `${(freeKib / 1024 / 1024).toFixed(1)} GiB` : "unknown";
    problems.push(
      `free space: at least ${MINIMUM_FREE_GIB} GiB needed under $HOME, host reports ${reported}`,
    );
  }
  let declaredPinned;
  try {
    declaredPinned = parseCpuList(declared.pinned_cpus);
  } catch (error) {
    problems.push(`pinned_cpus: ${error.message}`);
    return problems;
  }
  let isolated = [];
  try {
    isolated = observed.isolated_cpus ? parseCpuList(observed.isolated_cpus) : [];
  } catch {
    isolated = [];
  }
  // Every pinned CPU must actually be held off the scheduler. A pinned core the
  // kernel still schedules on is the failure this whole protocol exists to
  // avoid, and it is invisible in the timings themselves.
  const unisolated = declaredPinned.filter((cpu) => !isolated.includes(cpu));
  if (unisolated.length > 0) {
    problems.push(
      `pinned CPUs ${unisolated.join(",")} are not in the kernel's isolated set ` +
        `(${observed.isolated_cpus || "none"})`,
    );
  }
  return problems;
}

/// Where a promoted run lands in the working tree.
function promotionPaths(harnessCommit) {
  if (!FULL_COMMIT.test(harnessCommit)) throw new Error("harness commit must be a full 40-hex SHA");
  return {
    publication: path.join(BACKFILL_DIR, `${harnessCommit}.json`),
    evidence: path.join(EVIDENCE_DIR, harnessCommit),
  };
}

/// Backfill artifacts already in the tree that this run would sit beside.
/// bench/render-current-results.js charts every file in that directory, so a
/// second artifact is a second line — and two reconstructions measured under
/// different harnesses or on different machines are not comparable to each
/// other. Superseding is therefore the normal outcome of re-running, not an
/// exceptional one, and it must be deliberate rather than silent. Keyed on more
/// than the destination path: a new harness commit lands at a new filename, so
/// a path-only check would quietly leave the old series charted alongside.
function existingBackfills(exceptHarnessCommit) {
  if (!fs.existsSync(BACKFILL_DIR)) return [];
  return fs
    .readdirSync(BACKFILL_DIR)
    .filter((name) => name.endsWith(".json"))
    .map((name) => name.slice(0, -".json".length))
    .filter((commit) => commit !== exceptHarnessCommit);
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  const known = new Set([
    "plan", "history", "main", "pr", "accept", "live-serde", "calibrate", "matrix", "memory", "fetch",
    "unlock", "host-check", "self-test",
  ]);
  if (!known.has(command)) return null;
  const parsed = {
    command,
    replace: false,
    config: undefined,
    cohort: undefined,
    out: undefined,
    previousBinary: undefined,
    local: false,
    plan: false,
    base: undefined,
    json: undefined,
    repetitions: undefined,
    confidence: undefined,
    baseline: undefined,
    candidate: undefined,
    passes: undefined,
  };
  for (let index = 0; index < rest.length; index += 1) {
    const option = rest[index];
    if (option === "--replace") {
      parsed.replace = true;
    } else if (option === "--config") {
      // A trailing `--config` leaves rest[++index] undefined, which must be
      // rejected rather than silently read as "no config given".
      const value = rest[++index];
      if (!value) return null;
      parsed.config = value;
    } else if (option === "--cohort") {
      // Which engines the memory lane measures. Anything but the two named
      // cohorts is refused: a misspelt cohort must not become a rustia-only
      // run by default.
      const value = rest[++index];
      if (!MEMORY_COHORTS.includes(value)) return null;
      parsed.cohort = value;
    } else if (option === "--local") {
      parsed.local = true;
    } else if (option === "--plan") {
      parsed.plan = true;
    } else if (option === "--out") {
      const value = rest[++index];
      if (!value) return null;
      parsed.out = value;
    } else if (option === "--previous-binary") {
      const value = rest[++index];
      if (!value) return null;
      parsed.previousBinary = value;
    } else if (option === "--base") {
      const value = rest[++index];
      if (!value) return null;
      parsed.base = value;
    } else if (option === "--json") {
      const value = rest[++index];
      if (!value) return null;
      parsed.json = value;
    } else if (option === "--baseline") {
      const value = rest[++index];
      if (!value) return null;
      parsed.baseline = value;
    } else if (option === "--candidate") {
      const value = rest[++index];
      if (!value) return null;
      parsed.candidate = value;
    } else if (option === "--passes") {
      // Even only: an odd pass count leaves one side with an extra early
      // position, which is the drift the A B B A arrangement exists to cancel.
      const value = rest[++index];
      if (!value || !/^[1-9][0-9]*$/.test(value)) return null;
      const parsedValue = Number(value);
      if (parsedValue < 2 || parsedValue > 16 || parsedValue % 2 !== 0) return null;
      parsed.passes = parsedValue;
    } else if (option === "--repetitions") {
      const value = rest[++index];
      if (!value || !/^[1-9][0-9]*$/.test(value) || Number(value) > 10_000) return null;
      parsed.repetitions = Number(value);
    } else if (option === "--confidence") {
      const value = rest[++index];
      const parsedValue = Number(value);
      if (!value || !Number.isFinite(parsedValue) || parsedValue <= 0.5 || parsedValue >= 1) return null;
      parsed.confidence = parsedValue;
    } else {
      return null;
    }
  }
  // --out is shared by the four lanes that write a file; --previous-binary is
  // the per-commit lane's alone.
  if (parsed.out !== undefined && !["main", "pr", "accept", "live-serde", "calibrate"].includes(command)) {
    return null;
  }
  if (parsed.previousBinary !== undefined && command !== "main") return null;
  // --config names a frozen file for the lanes that freeze one. The other
  // lanes derive everything from HEAD, so a --config there would be read by
  // nothing, and an option that is accepted and ignored is worse than one
  // that is refused.
  if (parsed.config !== undefined && !["plan", "history", "fetch", "matrix", "memory"].includes(command)) {
    return null;
  }
  // --cohort selects which frozen memory protocol to generate; a --config
  // names one outright, so the two together would be one of them ignored.
  if (parsed.cohort !== undefined && (command !== "memory" || parsed.config !== undefined)) return null;
  // --base and --json describe a comparison; --repetitions and --confidence buy
  // a tighter interval, which only the lanes that publish nothing may do.
  // --plan measures and validates but promotes nothing, which only the lane that
  // promotes into this repository has any use for.
  if (parsed.plan && command !== "matrix" && command !== "memory") return null;
  if (parsed.base !== undefined && command !== "pr") return null;
  if (parsed.json !== undefined && command !== "pr" && command !== "accept") return null;
  for (const option of [parsed.repetitions, parsed.confidence]) {
    if (option !== undefined && command !== "calibrate" && command !== "accept") return null;
  }
  // An acceptance run names both sides explicitly and buys legs; no other
  // lane may, because every other lane derives its sides from the history it
  // is measuring rather than from an argument.
  for (const option of [parsed.baseline, parsed.candidate, parsed.passes]) {
    if (option !== undefined && command !== "accept") return null;
  }
  // `--replace` supersedes an existing artifact, which both the measuring
  // command and the re-promoting one can do; nothing else promotes.
  if (parsed.replace && command !== "history" && command !== "fetch") return null;
  return parsed;
}

// ---------------------------------------------------------------------------
// Host attestation
// ---------------------------------------------------------------------------

/// Reads the machine's own account of itself. Only the three isolation tokens
/// are taken from /proc/cmdline: the full command line carries root=UUID= and
/// other host detail that must not enter this repository.
const ATTEST_SCRIPT = `
set -u
${REMOTE_PATH}printf 'cpu_model=%s\\n' "$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | sed 's/^ *//')"
printf 'logical_cpus=%s\\n' "$(nproc --all)"
printf 'smt=%s\\n' "$(cat /sys/devices/system/cpu/smt/control 2>/dev/null || echo unknown)"
printf 'isolated_cpus=%s\\n' "$(cat /sys/devices/system/cpu/isolated 2>/dev/null || echo '')"
printf 'kernel=%s\\n' "$(uname -r)"
printf 'free_kib=%s\\n' "$(df -Pk "$HOME" | awk 'NR==2{print $4}')"
printf 'isolation=%s\\n' "$(tr ' ' '\\n' < /proc/cmdline | grep -E '^(isolcpus|nohz_full|rcu_nocbs)=' | tr '\\n' ' ')"
printf 'node=%s\\n' "$(command -v node >/dev/null && node --version || echo missing)"
printf 'cargo=%s\\n' "$(command -v cargo >/dev/null && cargo --version || echo missing)"
`;

function attestHost(config) {
  const raw = sshCapture(ATTEST_SCRIPT);
  const observed = {};
  for (const line of raw.split("\n")) {
    const at = line.indexOf("=");
    if (at > 0) observed[line.slice(0, at)] = line.slice(at + 1).trim();
  }
  observed.logical_cpus = Number(observed.logical_cpus);
  observed.free_kib = Number(observed.free_kib);
  const declared = config.provider.host;
  const problems = compareAttestation(declared, observed);
  if (observed.node === "missing") {
    problems.push(
      "node is not installed on the benchmark host, and the measurement runner is a Node script " +
        "that executes there (install it once as a host prerequisite; see rustia-ops)",
    );
  }
  if (observed.cargo === "missing") problems.push("cargo is not on the benchmark host's PATH");
  if (problems.length > 0) {
    fail(`host attestation failed:\n  - ${problems.join("\n  - ")}`);
  }
  return providerRecord(declared, observed);
}

// The provider block written beside every run. Matrix evidence commits it into
// a public-bound repository, so its field names are held to the rule the
// self-test checks: nothing that names an address, an account or an
// identifier. The rented lanes let 16 provider IPs into the repository through
// exactly such a block.
function providerRecord(declared, observed) {
  return {
    schema: "rustia.bench.dedicated-provider.v1",
    host: declared.name,
    class: "dedicated physical machine",
    attested_at: new Date().toISOString(),
    cpu_model: observed.cpu_model,
    logical_cpus: observed.logical_cpus,
    smt: observed.smt,
    isolated_cpus: observed.isolated_cpus,
    kernel: observed.kernel,
    free_gib: Number((observed.free_kib / 1024 / 1024).toFixed(1)),
    cpu_isolation: observed.isolation,
    pinned_cpus: declared.pinned_cpus,
    node_version: observed.node,
    cargo_version: observed.cargo,
  };
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

function freezeConfig(explicitPath) {
  if (explicitPath) {
    const resolved = path.resolve(REPO, explicitPath);
    if (!fs.existsSync(resolved)) fail(`config ${explicitPath} does not exist`);
    return resolved;
  }
  const harness = git(["rev-parse", "HEAD"]).trim();
  const output = path.join(CONFIG_DIR, `dedicated-history-${harness.slice(0, 7)}.json`);
  if (fs.existsSync(output)) {
    process.stdout.write(`reusing frozen config ${path.relative(REPO, output)}\n`);
    return output;
  }
  local("node", ["tools/freeze-history-backfill.js", "--output", path.relative(REPO, output)], {
    capture: false,
  });
  return output;
}

function loadConfig(file) {
  const config = JSON.parse(fs.readFileSync(file, "utf8"));
  if (config.schema !== "rustia.dedicated-history-backfill.v1") {
    fail(`${path.relative(REPO, file)} must declare schema rustia.dedicated-history-backfill.v1`);
  }
  return config;
}

// ---------------------------------------------------------------------------
// Promotion
// ---------------------------------------------------------------------------

function promote(runDirectory, config, replace) {
  const statusFile = path.join(runDirectory, "status.json");
  if (!fs.existsSync(statusFile)) fail("the export contains no status.json");
  const status = JSON.parse(fs.readFileSync(statusFile, "utf8"));
  if (status.status !== "backfill_complete") {
    fail(`remote run did not complete: ${status.status}${status.reason ? ` (${status.reason})` : ""}`);
  }
  const publicationSource = path.join(runDirectory, "publication/bench/history/backfill.json");
  const evidenceSource = path.join(runDirectory, "evidence/history-backfill");
  if (!fs.existsSync(publicationSource)) fail("the export contains no publication");
  const publication = JSON.parse(fs.readFileSync(publicationSource, "utf8"));
  if (publication.harness_commit !== config.source.harness_commit) {
    fail("the export's harness commit does not match the config that was shipped");
  }
  const destination = promotionPaths(publication.harness_commit);

  const superseded = existingBackfills(publication.harness_commit);
  const sameHarness = fs.existsSync(destination.publication);
  if ((superseded.length > 0 || sameHarness) && !replace) {
    const names = [...(sameHarness ? [publication.harness_commit] : []), ...superseded]
      .map((commit) => commit.slice(0, 7))
      .join(", ");
    fail(
      `bench/history/backfills already holds ${names}; pass --replace to supersede. ` +
        `Reconstructions from different harnesses or machines are not comparable, ` +
        `so keeping both would chart them as two lines`,
    );
  }
  fs.mkdirSync(BACKFILL_DIR, { recursive: true });
  for (const commit of superseded) {
    const stale = promotionPaths(commit);
    fs.rmSync(stale.publication, { force: true });
    fs.rmSync(stale.evidence, { recursive: true, force: true });
    process.stdout.write(`superseded ${commit.slice(0, 7)}\n`);
  }
  fs.rmSync(destination.evidence, { recursive: true, force: true });
  fs.copyFileSync(publicationSource, destination.publication);
  fs.cpSync(evidenceSource, destination.evidence, { recursive: true });

  process.stdout.write(
    `promoted ${publication.targets.length} revisions / ${publication.rows.length} rows to ` +
      `${path.relative(REPO, destination.publication)}\n`,
  );
  process.stdout.write("regenerating derived artifacts\n");
  local("node", ["bench/render-current-results.js"], { capture: false });
  local("node", ["tools/check-package-files.js"], { capture: false });
  local("node", ["tools/check-publishability.js"], { capture: false });
  for (const check of [
    ["node", ["tools/check-benchmark-results.js"]],
    ["node", ["tools/check-benchmark-commit-reachability.js"]],
    ["node", ["bench/render-current-results.js", "--check"]],
  ]) {
    local(check[0], check[1], { capture: false });
  }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

function commandPlan(options) {
  const configFile = freezeConfig(options.config);
  const config = loadConfig(configFile);
  const provider = attestHost(config);
  process.stdout.write(
    `config     ${path.relative(REPO, configFile)}\n` +
      `host       ${provider.host} (${provider.cpu_model}, ${provider.logical_cpus} CPUs, SMT ${provider.smt})\n` +
      `isolation  ${provider.cpu_isolation || "none"}\n` +
      `free       ${provider.free_gib} GiB under $HOME (floor ${MINIMUM_FREE_GIB})\n` +
      `pinned     CPU ${config.host_gate.benchmark_cpu} of ${provider.pinned_cpus}\n` +
      `revisions  ${config.source.target_commits.length}\n` +
      `repairs    ${config.source.source_modifications.length}\n` +
      `rows       ${config.publication.rows}\n` +
      `deadline   ${config.provider.host.max_runtime_minutes} minutes\n`,
  );
}

function commandHistory(options) {
  const configFile = freezeConfig(options.config);
  const config = loadConfig(configFile);

  // Checked before measuring rather than only at promotion: 35 minutes of
  // measurement that cannot be promoted is a waste the operator should be
  // spared.
  const destination = promotionPaths(config.source.harness_commit);
  const blocking = existingBackfills(config.source.harness_commit);
  if ((blocking.length > 0 || fs.existsSync(destination.publication)) && !options.replace) {
    fail(
      `bench/history/backfills already holds a reconstruction; pass replace=1 to supersede it. ` +
        `This is the change-of-machine and change-of-harness path: the old numbers are not ` +
        `comparable with the new ones, so the series is re-derived rather than extended`,
    );
  }

  const provider = attestHost(config);
  const runId = `${config.source.harness_commit.slice(0, 12)}-${Date.now().toString(36)}`;
  const runDirectory = path.join(STATE_ROOT, "runs", runId);
  fs.mkdirSync(runDirectory, { recursive: true });
  const providerFile = path.join(runDirectory, "provider.json");
  writeJson(providerFile, provider);
  const bundleFile = path.join(runDirectory, "repo.bundle");
  local("git", ["bundle", "create", bundleFile, "HEAD"], { capture: false });

  process.stdout.write(`shipping ${config.source.target_commits.length} revisions to ${provider.host}\n`);
  process.stdout.write("running; this takes about 35 minutes for 75 revisions\n");
  shipAndRun(configFile, providerFile, bundleFile, options.local);

  // fetchExport extracts into a subdirectory and returns it; promoting the run
  // directory itself finds no status.json and throws away a completed
  // measurement. commandFetch had this right and this did not.
  const extracted = fetchExport(runDirectory);
  promote(extracted, config, options.replace);
  fs.writeFileSync(path.join(STATE_ROOT, "last-run"), `${runId}\n`);
}

function fetchExport(runDirectory, lane = LANES.history) {
  const archive = path.join(runDirectory, "export.tar.gz");
  // The export comes back the same way the run went out. In local mode the host
  // is this machine, so the archive is copied rather than fetched -- reaching
  // for scp here would demand an ssh destination the local lane has no use for,
  // which is exactly how the first per-commit run failed after measuring
  // successfully.
  if (localMode) {
    fs.copyFileSync(path.join(os.homedir(), lane.root, "export.tar.gz"), archive);
  } else {
    local("scp", ["-q", `${sshDestination()}:${lane.root}/export.tar.gz`, archive], {
      capture: false,
    });
  }
  const extracted = path.join(runDirectory, "export");
  fs.rmSync(extracted, { recursive: true, force: true });
  fs.mkdirSync(extracted, { recursive: true });
  local("tar", ["-xzf", archive, "-C", extracted], { capture: false });
  process.stdout.write(`export sha256 ${sha256File(archive)}\n`);
  return extracted;
}

// Measure HEAD alone and emit rows for the live per-commit series.
//
// Skips the measurement entirely when this commit builds the same binary as the
// last measured one. That gate is exact rather than a path heuristic: a
// documentation or tooling commit produces identical bytes, so recording it
// would add a duplicate point claiming to be a new measurement. The binary hash
// is already computed for provenance, so the check is free.
// Puts the runner, the frozen config, the attestation and a bundle of this
// checkout where the host can see them, then runs it. In local mode the host is
// this machine, so the inbox is a directory rather than a destination.
function shipAndRun(configFile, providerFile, bundleFile, isLocal, lane = LANES.history) {
  const inbox = `$HOME/${lane.root}/inbox`;
  sshCapture(`set -eu\nmkdir -p "${inbox}"\n`);
  if (isLocal) {
    const resolved = path.join(os.homedir(), lane.root, "inbox");
    fs.mkdirSync(resolved, { recursive: true });
    for (const file of [path.join(REPO, lane.runner), configFile, providerFile, bundleFile]) {
      fs.copyFileSync(file, path.join(resolved, path.basename(file)));
    }
  } else {
    local(
      "scp",
      ["-q", lane.runner, configFile, providerFile, bundleFile, `${sshDestination()}:${lane.root}/inbox/`],
      { capture: false },
    );
  }
  // Absolute paths, not basenames. The two runners resolve a relative argument
  // against different directories -- the history one against the process's cwd,
  // the matrix one against its own root -- so `repo.bundle` found the bundle for
  // one lane and "does not exist" for the other. Naming the files outright means
  // neither runner's idea of where it is standing can matter.
  sshStream(
    `set -eu\n` +
      REMOTE_PATH +
      `export ${lane.rootEnv}="$HOME/${lane.root}"\n` +
      `cd "${inbox}"\n` +
      `node "${inbox}/${path.basename(lane.runner)}" "${inbox}/${path.basename(configFile)}" ` +
      `"${inbox}/${path.basename(providerFile)}" "${inbox}/${path.basename(bundleFile)}"\n`,
  );
}

function commandMain(options) {
  if (!options.out) fail("main requires --out FILE to write the series rows to");
  const configFile = path.join(STATE_ROOT, `main-${git(["rev-parse", "--short", "HEAD"]).trim()}.json`);
  fs.rmSync(configFile, { force: true });
  local(
    "node",
    [
      "tools/freeze-history-backfill.js",
      "--only",
      "HEAD",
      "--series",
      freezer.MAIN_SERIES,
      "--output",
      path.relative(REPO, configFile),
    ],
    { capture: false },
  );
  const config = loadConfig(configFile);
  const commit = config.source.target_commits[0];
  const provider = attestHost(config);

  const runId = `main-${commit.slice(0, 12)}`;
  const runDirectory = path.join(STATE_ROOT, "runs", runId);
  fs.rmSync(runDirectory, { recursive: true, force: true });
  fs.mkdirSync(runDirectory, { recursive: true });
  const providerFile = path.join(runDirectory, "provider.json");
  writeJson(providerFile, provider);
  const bundleFile = path.join(runDirectory, "repo.bundle");
  local("git", ["bundle", "create", bundleFile, "HEAD"], { capture: false });

  shipAndRun(configFile, providerFile, bundleFile, options.local);

  const extracted = fetchExport(runDirectory);
  const status = JSON.parse(fs.readFileSync(path.join(extracted, "status.json"), "utf8"));
  if (status.status !== "backfill_complete") {
    fail(`remote run did not complete: ${status.status}${status.reason ? ` (${status.reason})` : ""}`);
  }
  const publication = JSON.parse(
    fs.readFileSync(path.join(extracted, "publication/bench/history/backfill.json"), "utf8"),
  );
  const target = publication.targets[0];
  if (!target || target.commit !== commit) fail("the export does not describe the measured commit");

  if (options.previousBinary && options.previousBinary === target.binary_sha256) {
    // Same bytes as the last measured commit, so there is nothing new to record.
    process.stdout.write(`unchanged: ${commit.slice(0, 8)} builds the same binary as the last measured commit\n`);
    fs.writeFileSync(options.out, "");
    return;
  }

  const rows = publication.rows.map((row) => ({ ...row, binary_sha256: target.binary_sha256 }));
  fs.writeFileSync(options.out, rows.map((row) => `${JSON.stringify(row)}\n`).join(""));
  process.stdout.write(
    `measured ${commit.slice(0, 8)}: ${rows.length} rows, binary ${target.binary_sha256.slice(0, 12)}\n`,
  );
}

// ---------------------------------------------------------------------------
// Cross-engine matrix
// ---------------------------------------------------------------------------

// The acceptance rules come from the rented lane rather than a second
// implementation of them. Requiring that coordinator is side-effect free: it
// only runs when invoked directly.
const matrixRules = require("./run-linode-benchmark.js");

const MATRIX_PUBLICATIONS = Object.freeze({
  history: "publication/bench/history/runs.jsonl",
  wasm: "publication/bench/wasm/current-results.json",
  dispatch: "publication/bench/wasm/dispatch-current-results.json",
});

/// Why an owned-lane matrix `host.json` uname must not be committed, or null.
/// The file is committed to a public repository, so the lane records
/// `uname -srvm`, which leaves out the node name. `uname -a` puts the node name
/// second, where `uname -srvm` puts the kernel release, so the second field
/// must be the release the attestation read. That checks the form rather than
/// a list of names, which would pass every name it did not list. The message
/// does not quote the uname, since a wrong one may carry the name.
function matrixUnameProblem(uname, kernel, arch) {
  const fields = String(uname ?? "").trim().split(/\s+/);
  if (fields[0] === "Linux" && typeof kernel === "string" && fields[1] === kernel && fields.at(-1) === arch) {
    return null;
  }
  return (
    "matrix host evidence is not uname -srvm (Linux, the attested kernel release, the version, the machine); " +
    "the owned lane must record uname -srvm, since any other form can carry the node name"
  );
}

// The rented lane's evidence check asserts the host is a KVM guest, which is
// exactly the thing that is no longer true. The rest of it -- manifest schema,
// source hashes, protocol and provider hashes, per-file hashes, result shape --
// transfers unchanged, so this is that check with the virtualisation assertion
// replaced by the one that matters here: the machine must not be virtualised.
function validateDedicatedMatrixEvidence(config, evidenceDirectory, providerFile, publications) {
  const manifest = JSON.parse(fs.readFileSync(path.join(evidenceDirectory, "manifest.json"), "utf8"));
  if (
    manifest.schema !== "rustia.bench.cross-engine-matrix-evidence.v1" ||
    manifest.benchmark_commit !== config.source.benchmark_commit
  ) {
    fail("matrix evidence manifest has the wrong schema or commit");
  }
  const host = JSON.parse(fs.readFileSync(path.join(evidenceDirectory, "host.json"), "utf8"));
  if (host.platform !== "linux" || host.arch !== "x86_64") {
    fail("matrix host evidence is not Linux x86-64");
  }
  // systemd-detect-virt prints "none" on bare metal and exits non-zero doing it,
  // and prints nothing at all when it is not installed. Both mean "no hypervisor
  // was detected"; anything else names one. Checked as a rejection rather than
  // an exact match, so a run is not thrown away after forty minutes over the
  // spelling of an absence.
  const virtualization = String(host.virtualization ?? "").trim();
  if (virtualization !== "" && virtualization !== "none") {
    fail(
      `matrix host evidence reports virtualization ${JSON.stringify(virtualization)}; ` +
        "the owned host is bare metal",
    );
  }
  if (host.benchmark_cpu !== config.host_gate.benchmark_cpu) {
    fail("matrix host evidence pinned a different CPU than the config");
  }
  const attested = JSON.parse(fs.readFileSync(providerFile, "utf8"));
  const unameProblem = matrixUnameProblem(host.uname, attested.kernel, host.arch);
  if (unameProblem !== null) fail(unameProblem);
  if (manifest.protocol_sha256 !== matrixRules.sha256File(path.join(evidenceDirectory, "protocol.json"))) {
    fail("matrix manifest protocol hash is wrong");
  }
  if (manifest.provider_sha256 !== matrixRules.sha256File(path.join(evidenceDirectory, "provider.json"))) {
    fail("matrix manifest provider hash is wrong");
  }
  if (matrixRules.sha256File(path.join(evidenceDirectory, "provider.json")) !== sha256File(providerFile)) {
    fail("matrix provider evidence does not match the attestation this run shipped");
  }
  if (manifest.host_sha256 !== matrixRules.sha256File(path.join(evidenceDirectory, "host.json"))) {
    fail("matrix manifest host hash is wrong");
  }
  const publicationHashes = {
    history: matrixRules.sha256File(publications.history),
    wasm: matrixRules.sha256File(publications.wasm),
    dispatch: matrixRules.sha256File(publications.dispatch),
  };
  for (const [name, wanted] of Object.entries(publicationHashes)) {
    if (manifest.publication_sha256?.[name] !== wanted) {
      fail(`matrix manifest ${name} publication hash is wrong`);
    }
  }
  const shape = {
    native_rows: config.publication.native_rows,
    wasm_rows: config.publication.wasm_rows,
    dispatch_runtimes: config.publication.dispatch_runtimes,
  };
  for (const [key, wanted] of Object.entries(shape)) {
    if (manifest.result_shape?.[key] !== wanted) fail(`matrix manifest result shape ${key} is wrong`);
  }
  for (const [file, hash] of Object.entries(manifest.evidence_sha256 ?? {})) {
    if (matrixRules.sha256File(path.join(evidenceDirectory, file)) !== hash) {
      fail(`matrix evidence hash is wrong for ${file}`);
    }
  }
}

// The matrix and memory lanes freeze one config per commit and name it after
// HEAD. --config names a different frozen file instead -- one the operator
// froze deliberately -- and that file is never generated on the operator's
// behalf: a config asked for by name and missing is a mistake, not a request
// to freeze one.
function laneConfig(options, defaultRelative, generate) {
  if (options.config !== undefined) {
    const file = path.resolve(REPO, options.config);
    if (!fs.existsSync(file)) fail(`config ${options.config} does not exist`);
    return { relative: path.relative(REPO, file), file };
  }
  const file = path.join(REPO, defaultRelative);
  if (!fs.existsSync(file)) generate(defaultRelative);
  return { relative: defaultRelative, file };
}

function commandMatrix(options) {
  const commit = git(["rev-parse", "HEAD"]).trim();
  const short = commit.slice(0, 12);
  const { relative: configRelative, file: configFile } = laneConfig(
    options,
    `bench/cloud/dedicated-cross-engine-matrix-${short}.json`,
    (relative) =>
      local("node", ["tools/create-linode-matrix-config.js", "--dedicated", "--output", relative], {
        capture: false,
      }),
  );
  const config = JSON.parse(fs.readFileSync(configFile, "utf8"));
  if (config.source.benchmark_commit !== commit) {
    fail(`${configRelative} was frozen at a different commit; measure the commit it names or delete it`);
  }
  const provider = attestHost({
    provider: config.provider,
    host_gate: config.host_gate,
  });

  const runId = `matrix-${short}`;
  const runDirectory = path.join(STATE_ROOT, "runs", runId);
  fs.rmSync(runDirectory, { recursive: true, force: true });
  fs.mkdirSync(runDirectory, { recursive: true });
  const providerFile = path.join(runDirectory, "provider.json");
  writeJson(providerFile, provider);
  const bundleFile = path.join(runDirectory, "repo.bundle");
  local("git", ["bundle", "create", bundleFile, "HEAD"], { capture: false });

  process.stdout.write(`measuring 13 engines x 5 workloads on ${provider.host}\n`);
  shipAndRun(configFile, providerFile, bundleFile, options.local, LANES.matrix);
  const extracted = fetchExport(runDirectory, LANES.matrix);

  const status = JSON.parse(fs.readFileSync(path.join(extracted, "status.json"), "utf8"));
  matrixRules.validateMatrixStatus(status);
  const publications = Object.fromEntries(
    Object.entries(MATRIX_PUBLICATIONS).map(([name, relative]) => [name, path.join(extracted, relative)]),
  );
  const historyDestination = path.join(REPO, "bench/history/runs.jsonl");
  // Applies the rented lane's own rules: the export must be this repository's
  // history with exactly the frozen number of rows appended, each belonging to
  // the measured commit, and the wasm and dispatch results must match their
  // declared dimensions.
  matrixRules.validateMatrixNativeRows(config, publications.history, historyDestination, status);
  matrixRules.validateMatrixWasm(config, publications.wasm, status);
  matrixRules.validateMatrixDispatch(config, publications.dispatch, status);
  const evidenceSource = path.join(extracted, "evidence/matrix", commit);
  validateDedicatedMatrixEvidence(config, evidenceSource, providerFile, publications);

  if (options.plan) {
    process.stdout.write("validated; --plan means nothing was promoted\n");
    return;
  }

  const evidenceDestination = path.join(REPO, "bench/evidence/matrix", commit, runId);
  if (fs.existsSync(evidenceDestination)) fail(`refusing to overwrite matrix evidence ${path.relative(REPO, evidenceDestination)}`);
  fs.mkdirSync(path.dirname(evidenceDestination), { recursive: true });
  fs.cpSync(evidenceSource, evidenceDestination, { recursive: true });
  fs.copyFileSync(publications.history, historyDestination);
  fs.copyFileSync(publications.wasm, path.join(REPO, "bench/wasm/current-results.json"));
  fs.copyFileSync(publications.dispatch, path.join(REPO, "bench/wasm/dispatch-current-results.json"));
  local("node", ["tools/register-matrix-cohort.js"], { capture: false });
  process.stdout.write(
    `promoted cohort ${short}: ${config.publication.native_rows} native rows, ` +
      `${config.publication.wasm_rows} wasm rows\n` +
      `next: regenerate the reports and commit ${configRelative}, the publications and the evidence\n`,
  );
}

// ---------------------------------------------------------------------------
// Rustia memory cohort
// ---------------------------------------------------------------------------

// Measures per-VM memory on the owned host and promotes the cohort into this
// checkout. Unlike the matrix, the runner exports evidence rather than a
// finished publication: bench/publish-memory-analysis.js derives the results
// file and the appended history from the analysis in that evidence, which is
// the same tool the rented lane used and the reason this command is short.
//
// Two cohorts share the lane, the entrypoint and the frozen protocol, and
// differ in the engine set alone. rustia-only is the routine per-commit
// cohort, 11 rows; cross-engine measures the thirteen matrix engines, 110
// rows, and is what puts the competitor rows and the mlua/Luau/JIT reference
// on the same machine as the Rustia rows they are compared with.
const MEMORY_COHORTS = Object.freeze(["rustia-only", "cross-engine"]);
// `freeze` is the in-memory config each `generate` command writes, built by the
// same freezer function its CLI calls. The self-test reads it to check the host
// label every cohort's rows will carry, so a cohort added here is covered there
// without a second list.
const MEMORY_COHORT_FREEZERS = Object.freeze({
  "rustia-only": {
    file: (short) => `bench/cloud/dedicated-rustia-ci-memory-${short}.json`,
    generate: (relative) => ["tools/create-rustia-ci-memory-config.js", "--dedicated", "--output", relative],
    freeze: (commit) => {
      const freezer = require("./create-rustia-ci-memory-config.js");
      return freezer.toDedicated(freezer.createConfig(commit));
    },
  },
  "cross-engine": {
    file: (short) => `bench/cloud/dedicated-cross-engine-memory-${short}.json`,
    generate: (relative) => ["tools/create-cross-engine-memory-config.js", "--output", relative],
    freeze: (commit) => require("./create-cross-engine-memory-config.js").createConfig(commit),
  },
});

async function commandMemory(options) {
  const commit = git(["rev-parse", "HEAD"]).trim();
  const short = commit.slice(0, 12);
  const freezer = MEMORY_COHORT_FREEZERS[options.cohort ?? "rustia-only"];
  const { relative: configRelative, file: configFile } = laneConfig(
    options,
    freezer.file(short),
    (relative) => local("node", freezer.generate(relative), { capture: false }),
  );
  const config = JSON.parse(fs.readFileSync(configFile, "utf8"));
  if (config.source.benchmark_commit !== commit) {
    fail(`${configRelative} was frozen at a different commit; measure the commit it names or delete it`);
  }

  // The two files this promotion rewrites must be exactly what the config was
  // frozen against, or the cohort is being appended to a history that moved
  // underneath it.
  const historyDestination = path.join(REPO, "bench/memory/history/runs.jsonl");
  const currentDestination = path.join(REPO, "bench/memory/current-results.json");
  if (
    sha256File(historyDestination) !== config.source.expected_sha256.memory_history_before ||
    sha256File(currentDestination) !== config.source.expected_sha256.memory_current_before
  ) {
    fail("the local memory publication changed after this config was frozen; re-freeze it");
  }

  const provider = attestHost({ provider: config.provider, host_gate: config.host_gate });
  const runId = `memory-${short}`;
  const runDirectory = path.join(STATE_ROOT, "runs", runId);
  fs.rmSync(runDirectory, { recursive: true, force: true });
  fs.mkdirSync(runDirectory, { recursive: true });
  const providerFile = path.join(runDirectory, "provider.json");
  writeJson(providerFile, provider);
  const bundleFile = path.join(runDirectory, "repo.bundle");
  local("git", ["bundle", "create", bundleFile, "HEAD"], { capture: false });

  process.stdout.write(`measuring per-VM memory on ${provider.host}\n`);
  shipAndRun(configFile, providerFile, bundleFile, options.local, LANES.memory);
  const extracted = fetchExport(runDirectory, LANES.memory);

  const status = JSON.parse(fs.readFileSync(path.join(extracted, "status.json"), "utf8"));
  if (status.status !== "memory_complete") {
    fail(`memory run did not complete: ${status.status}${status.detail ? ` (${status.detail})` : ""}`);
  }
  const cohort = config.measurement.cohort_id;
  const evidenceSource = path.join(extracted, "evidence/memory", commit, cohort);
  // The rented lane's own acceptance rules, which know both evidence naming
  // schemes and check the raw sample count against the frozen protocol.
  await matrixRules.validateCrossEngineMemoryEvidence(config, evidenceSource);

  if (options.plan) {
    process.stdout.write("validated; --plan means nothing was promoted\n");
    return;
  }

  const evidenceDestination = path.join(REPO, "bench/memory/raw", commit, cohort);
  if (fs.existsSync(evidenceDestination)) {
    fail(`refusing to overwrite memory evidence ${path.relative(REPO, evidenceDestination)}`);
  }
  // Staged, then renamed into place, with the two publication files restored if
  // anything between here and the render fails. A half-promoted memory cohort
  // is a results file that disagrees with its own history.
  const snapshots = [historyDestination, currentDestination].map((file) => ({
    file,
    bytes: fs.readFileSync(file),
  }));
  const historyStage = `${historyDestination}.incoming`;
  const currentStage = `${currentDestination}.incoming`;
  let promoted = false;
  try {
    fs.mkdirSync(path.dirname(evidenceDestination), { recursive: true });
    fs.cpSync(evidenceSource, evidenceDestination, { recursive: true, errorOnExist: true, force: false });
    fs.copyFileSync(historyDestination, historyStage);
    local("node", [
      path.join(REPO, "bench/publish-memory-analysis.js"),
      "--analysis", path.join(evidenceDestination, "analysis.json"),
      "--cohort-id", cohort,
      "--evidence", path.relative(REPO, evidenceDestination),
      "--history", historyStage,
      "--current", currentStage,
      "--commit", commit,
    ], { capture: false });
    fs.renameSync(historyStage, historyDestination);
    fs.renameSync(currentStage, currentDestination);
    promoted = true;
    local("node", [path.join(REPO, "bench/render-current-results.js")], { capture: false });
    local("node", [path.join(REPO, "bench/render-current-results.js"), "--check"], { capture: false });
  } catch (error) {
    fs.rmSync(historyStage, { force: true });
    fs.rmSync(currentStage, { force: true });
    fs.rmSync(evidenceDestination, { recursive: true, force: true });
    if (promoted) for (const snapshot of snapshots) fs.writeFileSync(snapshot.file, snapshot.bytes);
    throw error;
  }
  process.stdout.write(
    `promoted memory cohort ${short}: ${config.publication.rows} rows, evidence at ` +
      `${path.relative(REPO, evidenceDestination)}\n`,
  );
}

// ---------------------------------------------------------------------------
// PR screening
// ---------------------------------------------------------------------------

// Files whose bytes define the comparison rather than take part in it: the
// workload text, the harness, the toolchain, the locked dependency set, and the
// results file the fixed sizes and expected checksums are read from. A change to
// any of them makes the two sides incomparable under one frozen config, and the
// run would fail on a checksum mismatch deep inside the host rather than saying
// so.
//
// Taken from the freezer rather than restated. The first version listed the
// directory `bench/`, which is where those files live but also where the README
// lives -- so the lane's first real pull request, a documentation change, was
// refused as a protocol change. A prefix that is right about the intent and
// wrong about the set is worse than no rule, because it looks like it worked.
const freezer = require("./freeze-history-backfill.js");
const PROTOCOL_PATHS = Object.freeze([
  ...Object.values(freezer.HASH_FILES),
  freezer.CURRENT_RESULTS,
  freezer.MAIN_SERIES,
]);

function protocolChanges(paths) {
  const protocol = new Set(PROTOCOL_PATHS);
  return paths.filter((file) => protocol.has(file));
}

// The screening thresholds, per workload, measured rather than chosen. The file
// is written by `calibrate` from a run in which nothing changed, so a threshold
// only exists once this host has been observed producing it. There is no
// default: a guessed figure that happens to sit inside the noise reports
// movement on PRs that changed nothing, which is worse than no lane at all.
function loadScreeningThresholds() {
  const file = path.join(REPO, "bench/pr-screening.json");
  if (!fs.existsSync(file)) {
    fail(
      "bench/pr-screening.json is missing; run `run-dedicated-benchmark.js calibrate` on the " +
        "benchmark host and commit its output before screening pull requests",
    );
  }
  const data = JSON.parse(fs.readFileSync(file, "utf8"));
  if (data.schema !== "rustia.bench.pr-screening.v1") {
    fail("bench/pr-screening.json must declare schema rustia.bench.pr-screening.v1");
  }
  if (data.host !== "ci-bench") fail("bench/pr-screening.json was not calibrated on ci-bench");
  const thresholds = data.workloads;
  if (!thresholds || typeof thresholds !== "object") {
    fail("bench/pr-screening.json must map every workload to a threshold");
  }
  for (const workload of Object.keys(thresholds)) {
    const entry = thresholds[workload];
    if (!Number.isFinite(entry?.material_threshold_percent) || entry.material_threshold_percent <= 0) {
      fail(`bench/pr-screening.json ${workload} must carry a positive material_threshold_percent`);
    }
  }
  return data;
}

// One measurement pass over a frozen pair, start to finish, returning the
// extracted export. `history`, `main` and this lane all reach the host through
// shipAndRun and come back through fetchExport; the difference between them is
// the config, not the transport.
function runPass(configFile, label, isLocal) {
  const config = loadConfig(configFile);
  const provider = attestHost(config);
  const runDirectory = path.join(STATE_ROOT, "runs", label);
  fs.rmSync(runDirectory, { recursive: true, force: true });
  fs.mkdirSync(runDirectory, { recursive: true });
  const providerFile = path.join(runDirectory, "provider.json");
  writeJson(providerFile, provider);
  const bundleFile = path.join(runDirectory, "repo.bundle");
  local("git", ["bundle", "create", bundleFile, "HEAD"], { capture: false });
  shipAndRun(configFile, providerFile, bundleFile, isLocal);
  const extracted = fetchExport(runDirectory);
  const status = JSON.parse(fs.readFileSync(path.join(extracted, "status.json"), "utf8"));
  if (status.status !== "backfill_complete") {
    fail(`${label} did not complete: ${status.status}${status.reason ? ` (${status.reason})` : ""}`);
  }
  return extracted;
}

function passRawDirectory(extracted) {
  return path.join(extracted, "evidence/history-backfill/raw");
}

function passBuilds(extracted) {
  const file = path.join(extracted, "evidence/history-backfill/builds.json");
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function binaryOf(builds, commit) {
  const build = builds.find((entry) => entry.commit === commit);
  if (!build || !build.binary_sha256) fail(`the export does not describe a build of ${commit.slice(0, 8)}`);
  return build.binary_sha256;
}

function freezePairConfig(baseline, candidate, repetitions, label) {
  const configFile = path.join(STATE_ROOT, `${label}.json`);
  fs.rmSync(configFile, { force: true });
  local(
    "node",
    [
      "tools/freeze-history-backfill.js",
      "--pair",
      baseline,
      candidate,
      "--repetitions",
      String(repetitions),
      "--output",
      path.relative(REPO, configFile),
    ],
    { capture: false },
  );
  return configFile;
}

// Runs the analyzer over one workload's legs and returns its parsed output. The
// legs are named by side, so a mix-up between them is a missing file rather than
// a silently reversed comparison.
function analyzeWorkload(workload, baselineLegs, candidateLegs, threshold, confidence) {
  const args = [path.join(REPO, "bench/analyze-fixed-samples.js")];
  for (const leg of baselineLegs) args.push("--baseline", leg);
  for (const leg of candidateLegs) args.push("--candidate", leg);
  args.push(
    "--expected-legs",
    String(baselineLegs.length),
    "--require-full-commits",
    "--material-threshold",
    String(threshold),
    "--confidence",
    String(confidence),
  );
  const output = local("node", args);
  const analysis = JSON.parse(output);
  if (analysis.identity.workload !== workload) {
    fail(`analyzer reported ${analysis.identity.workload} for ${workload}`);
  }
  return analysis;
}

function screeningSummary(result) {
  const lines = [];
  lines.push("## Benchmark screening");
  lines.push("");
  if (result.skipped) {
    lines.push(result.reason);
    return `${lines.join("\n")}\n`;
  }
  lines.push(
    `Baseline \`${result.baseline.slice(0, 8)}\` (merge base) against \`${result.candidate.slice(0, 8)}\`, ` +
      `measured on \`${result.host}\` pinned to core ${result.pinned_core}, ` +
      `${result.legs_per_side} legs per side interleaved A B B A, ` +
      `${result.repetitions} repetitions per leg, ` +
      `${(result.confidence * 100).toFixed(0)}% intervals.`,
  );
  lines.push("");
  lines.push("| workload | change | interval | reading |");
  lines.push("| --- | ---: | ---: | --- |");
  for (const entry of result.workloads) {
    const sign = entry.delta_percent > 0 ? "+" : "";
    lines.push(
      `| \`${entry.workload}\` | ${sign}${entry.delta_percent.toFixed(2)}% | ` +
        `[${entry.interval.lower.toFixed(2)}, ${entry.interval.upper.toFixed(2)}] | ${entry.reading} |`,
    );
  }
  lines.push("");
  lines.push(
    "`slower` and `faster` mean the interval clears zero. `no change measured` means it does not -- " +
      "which is not the same as proof that nothing moved, only that this run cannot separate it from " +
      "the host's own spread.",
  );
  const regressions = result.workloads.filter((entry) => entry.verdict === "regression");
  if (regressions.length > 0) {
    lines.push("");
    lines.push(
      `**${regressions.length} workload(s) exceeded the calibrated screening threshold.** ` +
        "The threshold is the spread this host shows when nothing changed, so a reading above it " +
        "is a change rather than noise. This lane reports; it does not block.",
    );
  }
  return `${lines.join("\n")}\n`;
}

function parseLiveSerdeOutput(output) {
  const read = (label) => {
    const match = output.match(new RegExp(`^${label}:\\s+([0-9]+(?:\\.[0-9]+)?)`, "m"));
    const value = match ? Number(match[1]) : Number.NaN;
    if (!Number.isFinite(value) || value <= 0) {
      throw new Error(`live-serde benchmark did not report a positive ${label} value`);
    }
    return value;
  };
  const liveNs = read("live serde");
  const ownedNs = read("owned path");
  return {
    live_ns: liveNs,
    owned_ns: ownedNs,
    owned_over_live: ownedNs / liveNs,
    live_latency_reduction_percent: (1 - liveNs / ownedNs) * 100,
  };
}

function liveSerdeSummary(result) {
  const reduction = result.live_latency_reduction_percent;
  const comparison = reduction >= 0
    ? `live is ${reduction.toFixed(2)}% lower latency`
    : `live is ${Math.abs(reduction).toFixed(2)}% higher latency`;
  return (
    "## Live-value serde microbenchmark\n\n" +
    `Commit \`${result.commit.slice(0, 8)}\`, measured on \`${result.host}\` pinned to core ` +
    `${result.pinned_core}. The candidate compares direct live deserialization with its explicit ` +
    "OwnedValue snapshot path using the example's interleaved A B B A protocol.\n\n" +
    "| path | ns/deserialization |\n" +
    "| --- | ---: |\n" +
    `| live value | ${result.live_ns.toFixed(1)} |\n` +
    `| owned snapshot | ${result.owned_ns.toFixed(1)} |\n\n` +
    `Owned/live: **${result.owned_over_live.toFixed(3)}x** (${comparison}). ` +
    "Equal outputs are asserted before timing. This candidate-only microbenchmark is screening evidence, " +
    "not a published historical performance claim.\n"
  );
}

function commandLiveSerde(options) {
  if (!options.local) {
    fail("live-serde runs only inside the ci-bench workflow with --local");
  }
  if (!options.out) fail("live-serde requires --out FILE to write its summary");

  const commit = git(["rev-parse", "HEAD"]).trim();
  const configFile = path.join(STATE_ROOT, `live-serde-${commit.slice(0, 12)}.json`);
  fs.rmSync(configFile, { force: true });
  local(
    "node",
    [
      "tools/freeze-history-backfill.js",
      "--only",
      commit,
      "--output",
      path.relative(REPO, configFile),
    ],
    { capture: false },
  );
  const config = loadConfig(configFile);
  const provider = attestHost(config);
  if (provider.host !== "ci-bench") fail(`live-serde requires ci-bench, got ${provider.host}`);

  local(
    "cargo",
    ["build", "--locked", "--release", "-p", "rustia", "--example", "live_serde_bench"],
    { capture: false },
  );
  const binary = path.join(REPO, "target/release/examples/live_serde_bench");
  if (!fs.existsSync(binary)) fail(`live-serde benchmark binary is missing at ${binary}`);
  const output = local("taskset", ["-c", String(config.host_gate.benchmark_cpu), binary]);
  const measured = parseLiveSerdeOutput(output);
  const result = {
    commit,
    host: provider.host,
    pinned_core: config.host_gate.benchmark_cpu,
    ...measured,
  };
  const summary = liveSerdeSummary(result);
  fs.writeFileSync(options.out, summary);
  process.stdout.write(summary);
}

// A/A calibration. Measures one revision four times and compares two of those
// passes against the other two, so both sides are the same bytes and every
// difference is the host. Four passes rather than two because the PR lane uses
// two legs a side, and an interval width only transfers to a comparison built
// the same way.
//
// The split is 1,4 against 2,3: the two groups straddle the run symmetrically,
// so a slow drift over the session cancels instead of loading onto whichever
// group went last.
//
// This is deliberately more pessimistic than the lane it calibrates. Here each
// leg gets its own quiet window; in the PR lane the two sides are measured
// inside one window, which can only make them more comparable. A threshold set
// from this is therefore not tighter than the noise it will meet.
const CALIBRATION_PASSES = 4;
// Every way of splitting four passes into two balanced groups. One split is one
// draw, and two calibration runs of the same commit disagreed by a factor of
// eight on generic_iteration -- a two-leg hierarchical bootstrap has two legs to
// resample from, which is barely an estimate. All three splits are measured from
// the same passes at no extra cost, and the threshold is the widest any of them
// produced: a threshold must not be narrower than something this host has
// actually been seen doing.
const CALIBRATION_SPLITS = Object.freeze([
  // The arrangement the screening lane actually uses. Each side's passes
  // straddle the run, so a drift across the session falls on both equally.
  Object.freeze({ label: "1,4 vs 2,3", baseline: [0, 3], candidate: [1, 2], straddles: true }),
  // Deliberately not drift-neutral. 1,2 vs 3,4 is first half against second
  // half; 1,3 vs 2,4 puts every candidate pass immediately after its baseline.
  // Neither is how the lane measures, so neither can fail a calibration -- but
  // both are real comparisons of the same binary, so whatever they show is
  // movement the host can produce, and the threshold must clear it.
  Object.freeze({ label: "1,2 vs 3,4", baseline: [0, 1], candidate: [2, 3], straddles: false }),
  Object.freeze({ label: "1,3 vs 2,4", baseline: [0, 2], candidate: [1, 3], straddles: false }),
]);

function calibrationThreshold(interval) {
  const widest = Math.max(Math.abs(interval.lower), Math.abs(interval.upper));
  // Two decimals, rounded up, so the recorded threshold is never narrower than
  // what was actually observed.
  return Math.ceil(widest * 100) / 100;
}

function commandCalibrate(options) {
  const commit = git(["rev-parse", "HEAD"]).trim();
  const repetitions = options.repetitions ?? 5;
  const confidence = options.confidence ?? 0.99;
  const passes = [];
  for (let index = 0; index < CALIBRATION_PASSES; index += 1) {
    const label = `calibrate-${commit.slice(0, 12)}-${index + 1}`;
    const configFile = path.join(STATE_ROOT, `${label}.json`);
    fs.rmSync(configFile, { force: true });
    local(
      "node",
      [
        "tools/freeze-history-backfill.js",
        "--only",
        commit,
        "--repetitions",
        String(repetitions),
        "--output",
        path.relative(REPO, configFile),
      ],
      { capture: false },
    );
    process.stdout.write(`calibration pass ${index + 1}/${CALIBRATION_PASSES}\n`);
    passes.push(runPass(configFile, label, options.local));
  }

  // Every pass must have built the same bytes, or this is not an A/A run at all.
  const binaries = new Set(passes.map((pass) => binaryOf(passBuilds(pass), commit)));
  if (binaries.size !== 1) {
    fail(`calibration passes produced ${binaries.size} different binaries for one commit`);
  }

  // The workload list comes from the frozen config rather than a constant here,
  // so calibration covers exactly what the lane measures and cannot drift from
  // it.
  const config = loadConfig(path.join(STATE_ROOT, `calibrate-${commit.slice(0, 12)}-1.json`));
  const legFor = (passIndex, workload) =>
    path.join(passRawDirectory(passes[passIndex]), `${commit}-${workload}-samples.json`);
  const workloads = {};
  const evidence = [];
  const broken = [];
  const drift = [];
  for (const workload of config.measurement.workloads) {
    const splits = CALIBRATION_SPLITS.map((split) => {
      const args = [path.join(REPO, "bench/analyze-fixed-samples.js")];
      for (const index of split.baseline) args.push("--baseline", legFor(index, workload));
      for (const index of split.candidate) args.push("--candidate", legFor(index, workload));
      args.push(
        "--expected-legs",
        "2",
        "--allow-identical-commits",
        "--material-threshold",
        "0",
        "--confidence",
        String(confidence),
      );
      const analysis = JSON.parse(local("node", args));
      return { split, analysis, threshold: calibrationThreshold(analysis.comparison.bootstrap_ci_percent) };
    });
    // The widest split decides the threshold; every split is checked against the
    // canary, because a protocol fault that only one arrangement of the same
    // passes reveals is still a protocol fault.
    const widest = splits.reduce((left, right) => (right.threshold > left.threshold ? right : left));
    const analysis = widest.analysis;
    const interval = analysis.comparison.bootstrap_ci_percent;
    const threshold = widest.threshold;
    // The canary, and it applies to the arrangement the lane measures in. The
    // claim being made is "when nothing changed, this lane reports
    // indistinguishable"; the straddling split is that claim, so an interval
    // there that clears zero means the protocol is measuring something other
    // than the code and no threshold should be written.
    //
    // The other two splits are not failures. They are the same binary arranged
    // so that a drift across the session does not cancel, and this host does
    // drift: the run that produced this rule read sparse_tables as 1.2% faster
    // than itself under 1,3 vs 2,4, with an interval that excluded zero. That is
    // a finding about the host, not a fault in the code, and treating it as a
    // failure would leave the lane permanently uncalibratable while telling us
    // nothing new. It is recorded, and it widens the threshold.
    for (const candidate of splits) {
      if (candidate.analysis.comparison.direction.status === "indistinguishable") continue;
      const bounds = candidate.analysis.comparison.bootstrap_ci_percent;
      const note =
        `${workload} split ${candidate.split.label}: A/A interval ` +
        `[${bounds.lower.toFixed(2)}, ${bounds.upper.toFixed(2)}] ` +
        `excludes zero (${candidate.analysis.comparison.direction.status})`;
      if (candidate.split.straddles) broken.push(note);
      else drift.push(note);
    }
    workloads[workload] = {
      material_threshold_percent: threshold,
      chosen_split: widest.split.label,
      // Splits that read the same binary as changed. Empty is the expected
      // state; a non-empty list is this host's session drift, made visible.
      drift_splits: splits
        .filter(
          (entry) =>
            !entry.split.straddles &&
            entry.analysis.comparison.direction.status !== "indistinguishable",
        )
        .map((entry) => entry.split.label),
      // Every split, so a reader can see how much the threshold depends on which
      // arrangement was drawn rather than having to run the calibration twice to
      // find out.
      split_thresholds_percent: Object.fromEntries(
        splits.map((entry) => [entry.split.label, entry.threshold]),
      ),
      observed_aa_interval_percent: interval,
      observed_aa_delta_percent: analysis.comparison.candidate_delta_percent,
      // Both components of the conservative envelope, because which one is
      // wider says what would actually tighten this lane. The pooled interval
      // narrows with more repetitions per leg; the hierarchical one is driven by
      // leg-to-leg variation and does not, so if it dominates then buying more
      // repetitions buys nothing and the answer is more passes.
      observed_aa_pooled_percent: analysis.comparison.pooled_bootstrap_ci_percent,
      observed_aa_hierarchical_percent: analysis.comparison.hierarchical_bootstrap_ci_percent,
      widened_by:
        analysis.comparison.hierarchical_bootstrap_ci_percent.upper -
          analysis.comparison.hierarchical_bootstrap_ci_percent.lower >
        analysis.comparison.pooled_bootstrap_ci_percent.upper -
          analysis.comparison.pooled_bootstrap_ci_percent.lower
          ? "leg_variation"
          : "repetition_variation",
    };
    evidence.push({ workload, interval, delta: analysis.comparison.candidate_delta_percent, threshold });
  }

  for (const line of evidence) {
    process.stdout.write(
      `${line.workload.padEnd(20)} delta ${line.delta.toFixed(3).padStart(8)}%  ` +
        `interval [${line.interval.lower.toFixed(2)}, ${line.interval.upper.toFixed(2)}]  ` +
        `threshold ${line.threshold.toFixed(2)}%\n`,
    );
  }
  if (drift.length > 0) {
    process.stdout.write(
      `\nthis host drifts across a session: ${drift.length} non-straddling split(s) read the same\n` +
        "binary as changed. The lane measures A B B A precisely so that this cancels; the readings\n" +
        "below are why that ordering is load-bearing rather than decorative, and they widen the\n" +
        `thresholds above:\n  ${drift.join("\n  ")}\n\n`,
    );
  }
  if (broken.length > 0) {
    fail(
      "the A/A calibration is not indistinguishable from zero in the arrangement the screening " +
        "lane measures in, so the comparison protocol is measuring something other than the code:" +
        `\n  ${broken.join("\n  ")}`,
    );
  }

  const output = {
    schema: "rustia.bench.pr-screening.v1",
    calibrated_at: new Date().toISOString(),
    calibrated_from_commit: commit,
    host: config.provider.host.name,
    pinned_core: config.host_gate.benchmark_cpu,
    passes: CALIBRATION_PASSES,
    legs_per_side: 2,
    repetitions,
    confidence,
    splits: CALIBRATION_SPLITS.map((split) => split.label),
    straddling_split: CALIBRATION_SPLITS.find((split) => split.straddles).label,
    method:
      "One revision measured in four separate passes, then compared against itself under all three " +
      "balanced splits of those passes through bench/analyze-fixed-samples.js with " +
      "--allow-identical-commits. Both sides of every split are the same binary, so each interval is " +
      "this host's own spread. A workload's threshold is the widest end of the widest split, rounded " +
      "up. All three are recorded because one split is one draw: two runs of this calibration at the " +
      "same commit disagreed by a factor of eight on one workload, which is what a two-leg " +
      "hierarchical bootstrap does. Every workload here is widened by leg variation rather than " +
      "repetition variation, so more repetitions per leg would not tighten this lane -- more passes " +
      "would. Only the straddling split can fail a calibration, because it is the arrangement the " +
      "screening lane measures in and therefore the only one whose indistinguishability is being " +
      "claimed; the other two are the same binary arranged so that session drift does not cancel, " +
      "and a workload whose drift_splits is non-empty is one where that ordering is doing real work.",
    workloads,
  };
  const destination = options.out ?? path.join(REPO, "bench/pr-screening.json");
  writeJson(destination, output);
  // Also to stdout. A calibration that exists only as a file inside a CI
  // runner's temporary directory is a measurement nobody can retrieve; the run
  // log is the durable copy, and it carries full precision rather than the
  // two-decimal table above.
  process.stdout.write(`${fs.readFileSync(destination, "utf8")}\n`);
  process.stdout.write(`wrote ${path.relative(REPO, destination)}\n`);
}

function commandPr(options) {
  if (!options.out) fail("pr requires --out FILE to write the screening summary to");
  const screening = loadScreeningThresholds();
  const candidate = git(["rev-parse", "HEAD"]).trim();
  const baseline = git(["merge-base", options.base ?? "origin/main", "HEAD"]).trim();
  if (!FULL_COMMIT.test(baseline) || !FULL_COMMIT.test(candidate)) {
    fail("could not resolve both the merge base and the candidate to full commits");
  }
  const write = (result) => {
    fs.writeFileSync(options.out, screeningSummary(result));
    if (options.json) writeJson(options.json, result);
  };

  if (baseline === candidate) {
    write({ skipped: true, reason: "The merge base is this commit; there is nothing to compare against." });
    return;
  }

  const changed = git(["diff", "--name-only", `${baseline}..${candidate}`]).trim().split("\n").filter(Boolean);
  const protocol = protocolChanges(changed);
  if (protocol.length > 0) {
    write({
      skipped: true,
      reason:
        "This change edits the measurement protocol itself " +
        `(${protocol.slice(0, 6).map((file) => `\`${file}\``).join(", ")}` +
        `${protocol.length > 6 ? ", …" : ""}), so the two sides cannot be measured under one frozen ` +
        "config. Compare them with a full acceptance run instead.",
    });
    return;
  }

  const repetitions = screening.repetitions;
  const confidence = screening.confidence;
  const first = runPass(
    freezePairConfig(baseline, candidate, repetitions, `pr-${candidate.slice(0, 12)}-ab`),
    `pr-${candidate.slice(0, 12)}-ab`,
    options.local,
  );
  const builds = passBuilds(first);
  if (binaryOf(builds, baseline) === binaryOf(builds, candidate)) {
    write({
      skipped: true,
      reason:
        `This change builds a byte-identical binary to its merge base ` +
        `(\`${binaryOf(builds, baseline).slice(0, 12)}\`), so there is nothing to measure.`,
    });
    return;
  }
  const second = runPass(
    freezePairConfig(candidate, baseline, repetitions, `pr-${candidate.slice(0, 12)}-ba`),
    `pr-${candidate.slice(0, 12)}-ba`,
    options.local,
  );

  // One leg per pass per side. Both sides appear in both passes, in opposite
  // order, so each side's legs sit at positions 1 and 4 or 2 and 3 of the
  // session -- the arrangement the calibration shows is the one that cancels
  // this host's drift.
  const legs = (commit, workload) =>
    [first, second].map((pass) =>
      path.join(passRawDirectory(pass), `${commit}-${workload}-samples.json`),
    );
  const workloads = [];
  for (const workload of Object.keys(screening.workloads)) {
    const baselineLegs = legs(baseline, workload);
    const candidateLegs = legs(candidate, workload);
    for (const file of [...baselineLegs, ...candidateLegs]) {
      if (!fs.existsSync(file)) fail(`missing measured leg ${path.relative(STATE_ROOT, file)}`);
    }
    const threshold = screening.workloads[workload].material_threshold_percent;
    const analysis = analyzeWorkload(workload, baselineLegs, candidateLegs, threshold, confidence);
    const direction = analysis.comparison.direction.status;
    workloads.push({
      workload,
      delta_percent: analysis.comparison.candidate_delta_percent,
      interval: analysis.comparison.bootstrap_ci_percent,
      threshold_percent: threshold,
      direction,
      verdict: analysis.comparison.verdict.status,
      reading:
        direction === "faster" ? "faster" : direction === "slower" ? "slower" : "no change measured",
    });
  }

  const config = loadConfig(path.join(STATE_ROOT, `pr-${candidate.slice(0, 12)}-ab.json`));
  const result = {
    schema: "rustia.bench.pr-screening-result.v1",
    skipped: false,
    baseline,
    candidate,
    host: config.provider.host.name,
    pinned_core: config.host_gate.benchmark_cpu,
    legs_per_side: 2,
    repetitions,
    confidence,
    workloads,
  };
  write(result);
  process.stdout.write(screeningSummary(result));
}

// A full acceptance comparison of two commits that are already in history.
//
// The screening lane answers "did this pull request move anything" in four
// minutes at two legs a side. This answers "did this change move THIS workload,
// and by how much" by buying more legs. The calibration in bench/pr-screening.json
// is explicit that every workload on this host is widened by leg variation
// rather than repetition variation -- "more repetitions per leg would not
// tighten this lane; more passes would" -- so passes are the lever here and
// repetitions stay at the calibrated value. Raising repetitions buys runtime
// and no resolution, which is exactly the mistake the rented-host protocol made
// when it asked for 201 of them.
//
// Both commits must already be ancestors of HEAD: the pair config generator
// requires it, and the bundle shipped to the host is built from HEAD, so an
// unreachable commit would fail on the host rather than here.
function commandAccept(options) {
  if (!options.out) fail("accept requires --out FILE to write the acceptance summary to");
  if (!options.baseline || !options.candidate) {
    fail("accept requires --baseline COMMIT and --candidate COMMIT");
  }
  const screening = loadScreeningThresholds();
  const baseline = git(["rev-parse", options.baseline]).trim();
  const candidate = git(["rev-parse", options.candidate]).trim();
  if (!FULL_COMMIT.test(baseline) || !FULL_COMMIT.test(candidate)) {
    fail("could not resolve both --baseline and --candidate to full commits");
  }
  if (baseline === candidate) fail("--baseline and --candidate name the same commit");
  for (const commit of [baseline, candidate]) {
    if (!isAncestorOfHead(commit)) {
      fail(
        `${commit.slice(0, 12)} is not an ancestor of HEAD; an acceptance run measures commits ` +
          "that are already in this history",
      );
    }
  }

  const passes = options.passes ?? DEFAULT_ACCEPTANCE_PASSES;
  const repetitions = options.repetitions ?? screening.repetitions;
  const confidence = options.confidence ?? screening.confidence;
  const label = `accept-${baseline.slice(0, 12)}-${candidate.slice(0, 12)}`;

  // Pass i measures the pair in AB order when i is even and BA when it is odd,
  // so the session reads A B B A repeated. Each side therefore sits at both an
  // early and a late position within every block of four legs, which is the
  // arrangement the calibration's straddling split is measured in.
  const extracted = [];
  for (let index = 0; index < passes; index += 1) {
    const baselineFirst = index % 2 === 0;
    const order = baselineFirst ? "ab" : "ba";
    const passLabel = `${label}-p${index}${order}`;
    const first = baselineFirst ? baseline : candidate;
    const second = baselineFirst ? candidate : baseline;
    extracted.push(
      runPass(freezePairConfig(first, second, repetitions, passLabel), passLabel, options.local),
    );
  }

  // Both sides are built once per pass. If the two commits produce the same
  // bytes there is no experiment: report that rather than a number, the way the
  // screening lane does.
  const builds = passBuilds(extracted[0]);
  if (binaryOf(builds, baseline) === binaryOf(builds, candidate)) {
    const identical = {
      schema: "rustia.bench.acceptance-result.v1",
      skipped: true,
      baseline,
      candidate,
      reason:
        `\`${baseline.slice(0, 8)}\` and \`${candidate.slice(0, 8)}\` build a byte-identical ` +
        `binary (\`${binaryOf(builds, baseline).slice(0, 12)}\`), so there is nothing to measure.`,
    };
    fs.writeFileSync(options.out, acceptanceSummary(identical));
    if (options.json) writeJson(options.json, identical);
    process.stdout.write(acceptanceSummary(identical));
    return;
  }

  const legs = (commit, workload) =>
    extracted.map((pass) => path.join(passRawDirectory(pass), `${commit}-${workload}-samples.json`));

  const workloads = [];
  for (const workload of Object.keys(screening.workloads)) {
    const baselineLegs = legs(baseline, workload);
    const candidateLegs = legs(candidate, workload);
    for (const file of [...baselineLegs, ...candidateLegs]) {
      if (!fs.existsSync(file)) fail(`missing measured leg ${path.relative(STATE_ROOT, file)}`);
    }
    const threshold = screening.workloads[workload].material_threshold_percent;
    const analysis = analyzeWorkload(workload, baselineLegs, candidateLegs, threshold, confidence);
    const direction = analysis.comparison.direction.status;
    workloads.push({
      workload,
      delta_percent: analysis.comparison.candidate_delta_percent,
      interval: analysis.comparison.bootstrap_ci_percent,
      threshold_percent: threshold,
      direction,
      verdict: analysis.comparison.verdict.status,
      reading:
        direction === "faster" ? "faster" : direction === "slower" ? "slower" : "no change measured",
    });
  }

  const config = loadConfig(path.join(STATE_ROOT, `${label}-p0ab.json`));
  const result = {
    schema: "rustia.bench.acceptance-result.v1",
    skipped: false,
    baseline,
    candidate,
    host: config.provider.host.name,
    pinned_core: config.host_gate.benchmark_cpu,
    passes,
    legs_per_side: passes,
    repetitions,
    confidence,
    // The thresholds were calibrated at the screening lane's leg count. More
    // legs narrow this host's own spread, so carrying that figure here is
    // conservative: it can only make a real movement harder to call material,
    // never easier. The interval is the evidence; the threshold is a guard.
    threshold_calibrated_legs_per_side: screening.legs_per_side,
    workloads,
  };
  fs.writeFileSync(options.out, acceptanceSummary(result));
  if (options.json) writeJson(options.json, result);
  process.stdout.write(acceptanceSummary(result));
}

function acceptanceSummary(result) {
  const lines = [];
  lines.push("## Benchmark acceptance");
  lines.push("");
  if (result.skipped) {
    lines.push(result.reason);
    return `${lines.join("\n")}\n`;
  }
  lines.push(
    `Baseline \`${result.baseline.slice(0, 8)}\` against candidate \`${result.candidate.slice(0, 8)}\`, ` +
      `measured on \`${result.host}\` pinned to core ${result.pinned_core}, ` +
      `${result.legs_per_side} legs per side interleaved A B B A, ` +
      `${result.repetitions} repetitions per leg, ` +
      `${(result.confidence * 100).toFixed(0)}% intervals.`,
  );
  lines.push("");
  lines.push(
    `Thresholds are the screening calibration's, measured at ${result.threshold_calibrated_legs_per_side} ` +
      "legs per side. More legs narrow this host's spread, so a threshold carried from that " +
      "calibration is conservative here: it can only make a movement harder to call material.",
  );
  lines.push("");
  lines.push("| workload | delta | interval | threshold | reading |");
  lines.push("| --- | ---: | --- | ---: | --- |");
  for (const entry of result.workloads) {
    const interval =
      entry.interval && Number.isFinite(entry.interval.lower)
        ? `${entry.interval.lower.toFixed(2)}%..${entry.interval.upper.toFixed(2)}%`
        : "n/a";
    lines.push(
      `| ${entry.workload} | ${entry.delta_percent.toFixed(2)}% | ${interval} | ` +
        `${entry.threshold_percent.toFixed(2)}% | ${entry.reading} |`,
    );
  }
  return `${lines.join("\n")}\n`;
}

// An acceptance run measures history, so both sides must be reachable from the
// bundle that is shipped to the host -- and that bundle is built from HEAD.
function isAncestorOfHead(commit) {
  try {
    execFileSync("git", ["merge-base", "--is-ancestor", commit, "HEAD"], { cwd: REPO, stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function commandFetch(options) {
  const lastRunFile = path.join(STATE_ROOT, "last-run");
  const runId = fs.existsSync(lastRunFile) ? fs.readFileSync(lastRunFile, "utf8").trim() : null;
  const configFile = freezeConfig(options.config);
  const config = loadConfig(configFile);
  const runDirectory = runId
    ? path.join(STATE_ROOT, "runs", runId)
    : path.join(STATE_ROOT, "runs", `fetch-${Date.now().toString(36)}`);
  fs.mkdirSync(runDirectory, { recursive: true });
  const extracted = fetchExport(runDirectory);
  promote(extracted, config, options.replace);
}

// Each lane holds its own lock under its own root, so all three are cleared.
// Clearing the history lane's alone, as this once did, left a stale matrix or
// memory lock to be found by hand.
function commandUnlock() {
  const script = Object.entries(LANES)
    .map(
      ([name, lane]) =>
        `lock="$HOME/${lane.root}/lock"\n` +
        `if [ -f "$lock" ]; then printf '${name}: '; cat "$lock"; rm -f "$lock"; echo " removed"; ` +
        `else echo "${name}: no lock held"; fi\n`,
    )
    .join("");
  const out = sshCapture(`set -u\n${script}`);
  process.stdout.write(out.endsWith("\n") ? out : `${out}\n`);
}

/// A read-only look at the host between runs, limited to what has actually
/// stopped a run here: free space under $HOME (a filled disk stopped the weekly
/// matrix before this script ran on 2026-09-06), the journal's share of it, the
/// load average (a driver fault held it at 43 for a day), and whether that
/// driver is loaded. Nothing printed names the machine.
const HOST_CHECK_SCRIPT = `
set -u
printf 'free_under_home=%s GiB (floor ${MINIMUM_FREE_GIB})\\n' "$(df -Pk "$HOME" | awk 'NR==2{printf "%.1f", $4/1048576}')"
journal="$(journalctl --disk-usage 2>/dev/null | sed 's/^Archived and active journals take up //')"
printf 'journal=%s\\n' "\${journal:-unavailable}"
diag="$(du -sh "$HOME"/actions-runner/_diag 2>/dev/null | cut -f1)"
printf 'runner_diag=%s\\n' "\${diag:-none}"
printf 'loadavg=%s\\n' "$(cut -d' ' -f1-3 /proc/loadavg)"
printf 'nouveau_loaded=%s\\n' "$(lsmod | grep -c '^nouveau ')"
printf 'isolated_cpus=%s\\n' "$(cat /sys/devices/system/cpu/isolated 2>/dev/null || echo '')"
`;

function commandHostCheck() {
  const out = sshCapture(HOST_CHECK_SCRIPT);
  process.stdout.write(out.endsWith("\n") ? out : `${out}\n`);
}

function selfTest() {
  const assert = require("node:assert/strict");

  assert.deepEqual(parseCpuList("4"), [4]);
  assert.deepEqual(parseCpuList("4,5"), [4, 5]);
  assert.deepEqual(parseCpuList("4-5"), [4, 5]);
  assert.deepEqual(parseCpuList("0-1,4"), [0, 1, 4]);
  assert.throws(() => parseCpuList(""), /empty/);
  assert.throws(() => parseCpuList("5-4"), /invalid CPU range/);
  assert.throws(() => parseCpuList("x"), /invalid CPU list entry/);

  const declared = {
    name: "ci-bench",
    cpu_model: "AMD Ryzen 5 3600X",
    logical_cpus: 6,
    physical_cores: 6,
    pinned_cpus: "4",
    max_runtime_minutes: 180,
  };
  const good = {
    cpu_model: "AMD Ryzen 5 3600X",
    logical_cpus: 6,
    smt: "off",
    isolated_cpus: "4-5",
    free_kib: 64 * 1024 * 1024,
  };
  assert.deepEqual(compareAttestation(declared, good), []);
  // The record written beside every run must not carry a field that could
  // hold an address, an account or an identifier. The host field is the
  // provenance label the config declares, never a machine name.
  const record = providerRecord(declared, {
    ...good,
    kernel: "6.8.0-generic",
    isolation: "isolcpus=4,5 nohz_full=4,5",
    node: "v22.0.0",
    cargo: "cargo 1.95.0",
  });
  const forbidden = /ip|addr|account|token|key|user|label|region|instance|id$/i;
  const leaked = Object.keys(record).filter((field) => forbidden.test(field));
  assert.deepEqual(leaked, [], `provider record field(s) may carry infrastructure detail: ${leaked}`);
  assert.equal(record.host, declared.name);
  assert.equal(record.pinned_cpus, declared.pinned_cpus);
  // The floor is inclusive, and a host that cannot report its free space is
  // refused rather than assumed fine: the filled-disk failure this guards
  // against never reached this script, so silence is not evidence.
  assert.deepEqual(
    compareAttestation(declared, { ...good, free_kib: MINIMUM_FREE_GIB * 1024 * 1024 }),
    [],
  );
  assert.match(
    compareAttestation(declared, { ...good, free_kib: MINIMUM_FREE_GIB * 1024 * 1024 - 1 }).join("|"),
    /free space: at least \d+ GiB needed under \$HOME, host reports \d+\.\d GiB/,
  );
  assert.match(
    compareAttestation(declared, { ...good, free_kib: Number.NaN }).join("|"),
    /free space: .* host reports unknown/,
  );
  // /proc/cpuinfo's trailing marketing text must not read as a different chip.
  assert.deepEqual(
    compareAttestation(declared, { ...good, cpu_model: "AMD Ryzen 5 3600X 6-Core Processor" }),
    [],
  );
  // A pinned core the kernel still schedules on is the exact failure this
  // protocol exists to prevent, and it never shows up in the timings.
  assert.match(
    compareAttestation(declared, { ...good, isolated_cpus: "5" }).join("|"),
    /not in the kernel's isolated set/,
  );
  assert.match(compareAttestation(declared, { ...good, smt: "on" }).join("|"), /SMT must be off/);
  assert.match(
    compareAttestation(declared, { ...good, logical_cpus: 12 }).join("|"),
    /logical_cpus/,
  );
  assert.match(
    compareAttestation(declared, { ...good, cpu_model: "Other" }).join("|"),
    /cpu_model/,
  );
  // "notsupported" is the real reading from this host, whose firmware disabled
  // SMT so the kernel cannot control it. Accepted, because logical_cpus ===
  // physical_cores is what establishes SMT is off.
  assert.deepEqual(compareAttestation(declared, { ...good, smt: "notsupported" }), []);

  // The label a row carries must be the machine the coordinator attested.
  // attestHost() reports config.provider.host.name and compares the machine's
  // own /sys and /proc readings against the rest of that block, so a row
  // labelled with the same field is labelled with a machine that was checked.
  // Three separately shipped runners stamp it -- they cannot share a module,
  // because only one file at a time is copied to the host -- so the agreement is
  // asserted here, where every lane is driven from.
  const head = git(["rev-parse", "HEAD"]).trim();
  const matrixFreezer = require("./create-linode-matrix-config.js");
  const dedicatedMatrix = matrixFreezer.toDedicated(matrixFreezer.createConfig(head));
  // Every memory cohort, not only the routine one: the cross-engine cohort's
  // competitor rows share one published table with the rustia-only cohort's
  // Rustia rows, and that table may name exactly one machine -- the timing
  // authority in bench/operations-status.json, which the renderer and the
  // checker both pin it to. A cohort frozen under another label would measure
  // for an hour and then be refused at render, so it is refused here instead.
  const operationsStatus = JSON.parse(fs.readFileSync(path.join(REPO, "bench/operations-status.json"), "utf8"));
  const authorities = Object.values(operationsStatus.runner_classes)
    .filter((runner) => runner?.timing_authority === true);
  assert.equal(authorities.length, 1, "operations-status must name exactly one timing authority");
  const authority = authorities[0].host;
  const lanes = [["matrix", dedicatedMatrix, matrixRules.matrixHostLabel(dedicatedMatrix)]];
  for (const cohort of MEMORY_COHORTS) {
    const config = MEMORY_COHORT_FREEZERS[cohort].freeze(head);
    lanes.push([`memory ${cohort}`, config, matrixRules.memoryHostLabel(config)]);
  }
  for (const [lane, config, label] of lanes) {
    assert.equal(label, config.provider.host.name, `${lane} rows must be labelled with the attested host`);
    assert.equal(label, authority, `${lane} rows must be labelled with the timing authority ${authority}`);
  }
  // An address in the frozen host block is refused before it can label a
  // cross-engine row, as it is for the routine cohort.
  const crossEngine = MEMORY_COHORT_FREEZERS["cross-engine"].freeze(head);
  assert.throws(
    () => matrixRules.memoryHostLabel({
      ...crossEngine,
      provider: { host: { ...crossEngine.provider.host, name: "bench.example.com" } },
    }),
    /not a label/,
  );

  // The label every lane stamps is one sshDestination() refuses, however the
  // destination wraps it: the host part is compared, so a user, an ssh://
  // scheme or a port does not hide the label, and neither does case, which ssh
  // folds before it reads ssh_config. Every spelling yields the one message,
  // so nothing of the destination but the label reaches the log.
  const published = publishedHostLabels(REPO);
  for (const [lane, , label] of lanes) {
    assert(published.includes(label), `${lane} rows' label must be one sshDestination() refuses`);
  }
  const refusal = sshDestinationProblem(authority, published);
  assert.match(
    refusal ?? "",
    new RegExp(
      `^${SSH_ENV} names "${authority}", which is a provenance label published in row data, never an ssh ` +
        "target: use a private ssh_config alias that is not a published label",
    ),
  );
  for (const destination of [
    `user@${authority}`,
    `ssh://${authority}:22`,
    `ssh://user@${authority}:22`,
    // ssh strips the brackets from a URI's host, whatever they enclose.
    `ssh://user@[${authority}]:22`,
    // ssh accepts a URI with an empty path, and a trailing slash still reaches
    // the host.
    `ssh://${authority}/`,
    `ssh://user@${authority}/`,
    `ssh://user@${authority}:22/`,
    authority.toUpperCase(),
    `ssh://user@${authority.toUpperCase()}:22`,
  ]) {
    assert.equal(sshDestinationProblem(destination, published), refusal, `${destination} must be refused`);
  }
  // The refusal is wired into every ssh call, not just defined: host-check and
  // unlock only ever run ssh, and each exits on the label before starting it.
  // A stub ssh and scp ahead of the real ones record any call that gets through.
  const stubDir = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-ssh-stub-"));
  try {
    const called = path.join(stubDir, "called");
    for (const tool of ["ssh", "scp"]) {
      fs.writeFileSync(path.join(stubDir, tool), `#!/bin/sh\necho "${tool}" >> "${called}"\n`, { mode: 0o755 });
    }
    for (const command of ["host-check", "unlock"]) {
      for (const destination of [authority, `user@${authority}:22`, `ssh://${authority}/`]) {
        const run = spawnSync(process.execPath, [__filename, command], {
          encoding: "utf8",
          env: { ...process.env, [SSH_ENV]: destination, PATH: `${stubDir}${path.delimiter}${process.env.PATH}` },
        });
        assert.equal(run.status, 1, `${command} with ${destination} must be refused: ${run.stderr}`);
        assert.equal(run.stderr, `run-dedicated-benchmark: ${refusal}\n`, `${command} with ${destination}`);
        assert(!fs.existsSync(called), `${command} with ${destination} reached ssh`);
      }
    }
  } finally {
    fs.rmSync(stubDir, { recursive: true, force: true });
  }
  // Anything whose host is not a published label is the operator's business.
  for (const destination of ["box", "user@box", "ssh://user@box:22", "ssh://user@[fe80::1]:22", "10.0.0.5"]) {
    assert.equal(sshDestinationProblem(destination, published), null, `${destination} is not a published label`);
  }
  // Each source adds its own labels. A runner class that names no host adds
  // nothing, and a frozen config is read by the dedicated- prefix every lane's
  // file carries, so a rented lane's config is not.
  const labelRoot = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-labels-"));
  try {
    writeJson(path.join(labelRoot, "bench/operations-status.json"), {
      runner_classes: {
        shared: { timing_authority: false },
        owned: { timing_authority: true, host: "owned-class" },
      },
    });
    const frozen = {
      "dedicated-history-abc1234.json": "owned-history",
      "dedicated-cross-engine-matrix-abc.json": "owned-matrix",
      "linode-cross-engine-matrix-abc.json": "rented",
    };
    for (const cohort of MEMORY_COHORTS) {
      frozen[path.basename(MEMORY_COHORT_FREEZERS[cohort].file("abc"))] = `owned-${cohort}`;
    }
    for (const [name, label] of Object.entries(frozen)) {
      writeJson(path.join(labelRoot, "bench/cloud", name), { provider: { host: { name: label } } });
    }
    const fixtureLabels = publishedHostLabels(labelRoot);
    assert.deepEqual(
      [...fixtureLabels].sort(),
      ["owned-class", ...MEMORY_COHORTS.map((cohort) => `owned-${cohort}`), "owned-history", "owned-matrix"].sort(),
    );
    for (const label of fixtureLabels) {
      assert.match(sshDestinationProblem(`user@${label}`, fixtureLabels) ?? "", new RegExp(`names "${label}"`));
    }
  } finally {
    fs.rmSync(labelRoot, { recursive: true, force: true });
  }

  const harness = "a".repeat(40);
  const paths = promotionPaths(harness);
  assert.ok(paths.publication.endsWith(`${harness}.json`));
  assert.ok(paths.evidence.endsWith(harness));
  assert.throws(() => promotionPaths("short"), /full 40-hex/);

  // The committed tree holds exactly one reconstruction, and re-running must
  // supersede it rather than chart a second line beside it.
  const present = existingBackfills("z".repeat(40));
  assert.equal(present.length, 1, `expected one committed backfill, found ${present.length}`);
  assert.deepEqual(existingBackfills(present[0]), [], "a run excludes its own artifact");

  const defaults = {
    replace: false, config: undefined, cohort: undefined, out: undefined, previousBinary: undefined,
    local: false, plan: false, base: undefined, json: undefined, repetitions: undefined,
    confidence: undefined, baseline: undefined, candidate: undefined, passes: undefined,
  };
  assert.deepEqual(parseArgs(["plan"]), { command: "plan", ...defaults });
  assert.deepEqual(parseArgs(["history", "--replace"]), {
    command: "history", ...defaults, replace: true,
  });
  // Local mode still attests: running on the host does not exempt a run from
  // proving the host is what the config says.
  assert.equal(parseArgs(["main", "--local", "--out", "r.jsonl"]).local, true);
  // The per-commit options belong to that lane alone.
  assert.equal(parseArgs(["main", "--out", "rows.jsonl"]).out, "rows.jsonl");
  assert.equal(parseArgs(["history", "--out", "rows.jsonl"]), null);
  assert.equal(parseArgs(["main", "--out"]), null);
  assert.equal(parseArgs(["main", "--previous-binary"]), null);
  assert.equal(parseArgs(["bogus"]), null);
  assert.equal(parseArgs(["plan", "--replace"]), null, "plan promotes nothing");
  assert.equal(parseArgs(["fetch", "--replace"]).replace, true, "fetch promotes, so it may supersede");
  assert.equal(parseArgs(["history", "--config"]), null);
  // --config names a frozen file for the lanes that freeze one; the lanes that
  // derive everything from HEAD have nothing for it to name.
  assert.equal(parseArgs(["matrix", "--config", "bench/cloud/x.json"]).config, "bench/cloud/x.json");
  assert.equal(parseArgs(["memory", "--config", "bench/cloud/x.json"]).config, "bench/cloud/x.json");
  // --cohort picks one of the two memory protocols and nothing else; a name
  // outside the pair is refused rather than defaulted.
  assert.equal(parseArgs(["memory", "--cohort", "cross-engine"]).cohort, "cross-engine");
  assert.equal(parseArgs(["memory", "--cohort", "rustia-only"]).cohort, "rustia-only");
  assert.equal(parseArgs(["memory"]).cohort, undefined);
  assert.equal(parseArgs(["memory", "--cohort", "bogus"]), null);
  assert.equal(parseArgs(["memory", "--cohort"]), null);
  assert.equal(parseArgs(["matrix", "--cohort", "cross-engine"]), null);
  assert.equal(parseArgs(["main", "--out", "r.jsonl", "--cohort", "cross-engine"]), null);
  assert.equal(parseArgs(["memory", "--cohort", "cross-engine", "--config", "x.json"]), null);
  for (const cohort of MEMORY_COHORTS) {
    assert.match(MEMORY_COHORT_FREEZERS[cohort].file("abc"), /^bench\/cloud\/dedicated-.*-abc\.json$/);
    assert.equal(MEMORY_COHORT_FREEZERS[cohort].generate("x.json").at(-1), "x.json");
  }
  assert.equal(parseArgs(["main", "--out", "r.jsonl", "--config", "x.json"]), null);
  assert.equal(parseArgs(["pr", "--out", "s.md", "--config", "x.json"]), null);
  assert.deepEqual(parseArgs(["host-check"]), { command: "host-check", ...defaults });
  assert.equal(parseArgs(["host-check", "--local"]).local, true);

  // The screening lanes. --base and --json describe a comparison, so only `pr`
  // takes them; --repetitions and --confidence buy a tighter interval, which
  // only the lane that publishes nothing is free to do.
  assert.equal(parseArgs(["pr", "--out", "s.md"]).out, "s.md");
  assert.equal(parseArgs(["pr", "--out", "s.md", "--base", "origin/main"]).base, "origin/main");
  assert.equal(parseArgs(["pr", "--out", "s.md", "--json", "s.json"]).json, "s.json");
  assert.equal(parseArgs(["live-serde", "--local", "--out", "s.md"]).out, "s.md");
  assert.equal(parseArgs(["live-serde", "--base", "main"]), null);
  assert.equal(parseArgs(["main", "--out", "r.jsonl", "--base", "x"]), null, "the series has no merge base");
  assert.equal(parseArgs(["pr", "--repetitions", "25"]), null, "screening must not retune itself");
  assert.equal(parseArgs(["calibrate", "--repetitions", "25"]).repetitions, 25);
  assert.equal(parseArgs(["calibrate", "--confidence", "0.99"]).confidence, 0.99);
  assert.equal(parseArgs(["calibrate", "--confidence", "1"]), null);
  assert.equal(parseArgs(["calibrate", "--confidence", "0.5"]), null);
  assert.equal(parseArgs(["calibrate", "--repetitions", "0"]), null);
  assert.equal(parseArgs(["history", "--out", "x"]), null);
  assert.equal(parseArgs(["pr", "--base"]), null);

  // An acceptance run names both sides and buys legs. The pair is explicit
  // because the commits being compared are already in history and neither is
  // derivable from HEAD.
  const accept = parseArgs(["accept", "--baseline", "a1", "--candidate", "b2", "--out", "a.md"]);
  assert.equal(accept.baseline, "a1");
  assert.equal(accept.candidate, "b2");
  assert.equal(accept.passes, undefined, "passes defaults inside the command, not the parser");
  assert.equal(parseArgs(["accept", "--passes", "8", "--out", "a.md"]).passes, 8);
  assert.equal(parseArgs(["accept", "--json", "a.json", "--out", "a.md"]).json, "a.json");
  // Repetitions are permitted here and refused for screening: an acceptance run
  // publishes nothing, so it may retune itself.
  assert.equal(parseArgs(["accept", "--repetitions", "25", "--out", "a.md"]).repetitions, 25);
  // Odd pass counts leave one side an extra early position, which is exactly
  // the drift A B B A cancels.
  assert.equal(parseArgs(["accept", "--passes", "7", "--out", "a.md"]), null);
  assert.equal(parseArgs(["accept", "--passes", "0", "--out", "a.md"]), null);
  assert.equal(parseArgs(["accept", "--passes", "18", "--out", "a.md"]), null);
  assert.equal(parseArgs(["accept", "--passes"]), null);
  assert.equal(parseArgs(["accept", "--baseline"]), null);
  assert.equal(parseArgs(["accept", "--candidate"]), null);
  // The pair belongs to this lane alone; every other lane derives its sides
  // from the history it measures.
  assert.equal(parseArgs(["pr", "--baseline", "a1", "--out", "s.md"]), null);
  assert.equal(parseArgs(["main", "--passes", "8", "--out", "r.jsonl"]), null);
  assert.equal(parseArgs(["history", "--candidate", "b2"]), null);
  assert.equal(parseArgs(["matrix", "--plan"]).plan, true);
  assert.equal(parseArgs(["memory", "--plan"]).plan, true);
  assert.equal(parseArgs(["memory"]).command, "memory");
  assert.equal(parseArgs(["matrix"]).plan, false);
  assert.equal(parseArgs(["history", "--plan"]), null, "only the matrix lane validates without promoting");
  assert.equal(parseArgs(["pr", "--plan"]), null);

  // Two runners, one transport. The lanes must not share a directory, or a
  // matrix run would wipe the history lane's warm cargo cache and vice versa.
  // No two lanes may share a directory, an environment variable or a runner: a
  // matrix run would otherwise wipe another lane's warm cargo cache.
  const laneList = Object.values(LANES);
  for (const key of ["root", "rootEnv", "runner"]) {
    assert.equal(
      new Set(laneList.map((lane) => lane[key])).size,
      laneList.length,
      `every lane must have its own ${key}`,
    );
  }
  for (const lane of Object.values(LANES)) {
    assert.match(lane.root, /^\.[a-z-]+$/, "a lane root must be a dotted name under $HOME");
    assert(fs.existsSync(path.join(REPO, lane.runner)), `${lane.runner} must exist`);
  }

  // shipAndRun copies the runner into the host's inbox on its own, beside the
  // config, the attestation and the bundle; a module required next to the
  // runner does not exist there. Each runner is started from a directory that
  // holds nothing else and must reach its usage line rather than "Cannot find
  // module", which is how the memory lane's third owned-host run ended.
  const inbox = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-inbox-"));
  try {
    for (const lane of Object.values(LANES)) {
      const shipped = path.join(inbox, path.basename(lane.runner));
      fs.copyFileSync(path.join(REPO, lane.runner), shipped);
      const alone = spawnSync(process.execPath, [shipped], { cwd: inbox, encoding: "utf8" });
      assert.equal(alone.status, 2, `${lane.runner} must load from an inbox holding only itself: ${alone.stderr}`);
      assert.match(alone.stderr, /^usage: /, `${lane.runner} must print its usage when started without arguments`);
    }
  } finally {
    fs.rmSync(inbox, { recursive: true, force: true });
  }

  // Protocol files define the comparison; a change to one is not comparable
  // under a single frozen config, and the lane has to say so rather than fail
  // on a checksum deep inside the host.
  assert.deepEqual(protocolChanges(["crates/rustia-vm/src/vm.rs"]), []);
  assert.deepEqual(protocolChanges(["Cargo.lock"]), ["Cargo.lock"]);
  assert.deepEqual(protocolChanges(["rust-toolchain.toml"]), ["rust-toolchain.toml"]);
  assert.deepEqual(protocolChanges(["bench/Cargo.toml"]), ["bench/Cargo.toml"]);
  assert.deepEqual(
    protocolChanges(["bench/workloads/luau/01_dense_tables.luau"]),
    ["bench/workloads/luau/01_dense_tables.luau"],
  );
  assert.deepEqual(protocolChanges(["bench/current-results.json"]), ["bench/current-results.json"]);
  assert.deepEqual(protocolChanges([freezer.MAIN_SERIES]), [freezer.MAIN_SERIES]);
  // Files under bench/ that the frozen config does not pin are ordinary changes.
  // Refusing these was the defect the lane's first pull request found.
  assert.deepEqual(protocolChanges(["bench/README.md", "bench/CLAIMS.md", "bench/RESULTS.md"]), []);
  assert.deepEqual(protocolChanges(["bench/history/runs.jsonl", "bench/cloud/x.json"]), []);
  assert.deepEqual(protocolChanges(["benchmarks/notes.md", "Cargo.lock.bak"]), []);
  // The set has to be the freezer's own, or the two drift and the lane refuses
  // the wrong changes again.
  for (const file of Object.values(freezer.HASH_FILES)) {
    assert.deepEqual(protocolChanges([file]), [file], `${file} is pinned by the config`);
  }
  assert.equal(PROTOCOL_PATHS.length, Object.keys(freezer.HASH_FILES).length + 2);

  // The threshold is the widest end of the A/A interval, rounded up, so it is
  // never narrower than what the host was actually seen doing.
  assert.equal(calibrationThreshold({ lower: -1.234, upper: 0.5 }), 1.24);
  assert.equal(calibrationThreshold({ lower: -0.1, upper: 2.001 }), 2.01);
  assert.equal(calibrationThreshold({ lower: -0.5, upper: 0.5 }), 0.5);

  // The two calibration groups must straddle the run, not sit one after the
  // other, or a drift across the session lands entirely on the second group.
  // Every balanced split of the passes, each using every pass exactly once, and
  // no split repeated -- three is all there are for four passes.
  assert.equal(CALIBRATION_SPLITS.length, 3);
  const seenSplits = new Set();
  for (const split of CALIBRATION_SPLITS) {
    assert.equal(split.baseline.length, split.candidate.length);
    assert.equal(
      new Set([...split.baseline, ...split.candidate]).size,
      CALIBRATION_PASSES,
      `${split.label} must use every calibration pass exactly once`,
    );
    // A split and its mirror image are the same comparison.
    const key = [split.baseline, split.candidate].map((g) => [...g].sort().join("")).sort().join("|");
    assert(!seenSplits.has(key), `${split.label} duplicates another split`);
    seenSplits.add(key);
  }
  // Exactly one split straddles the run, and it is the one the screening lane
  // measures in: pass order A B then B A puts each side at positions 1 and 4 or
  // 2 and 3. Only that split can fail a calibration, so if this ever stops being
  // true the canary is testing an arrangement nothing uses.
  const straddling = CALIBRATION_SPLITS.filter((split) => split.straddles);
  assert.equal(straddling.length, 1, "exactly one split may be the lane's own arrangement");
  assert.deepEqual(straddling[0].baseline, [0, 3]);
  assert.deepEqual(straddling[0].candidate, [1, 2]);
  assert.equal(CALIBRATION_SPLITS[0].straddles, true, "the headline split must be the lane's own");
  for (const split of CALIBRATION_SPLITS) {
    assert.equal(typeof split.straddles, "boolean", `${split.label} must declare whether it straddles`);
  }

  // Absence of a hypervisor, however it is spelled, and rejection of one.
  const hostEvidence = (virtualization) => ({
    platform: "linux", arch: "x86_64", benchmark_cpu: 4, uname: "Linux 6.8.0 x86_64", virtualization,
  });
  for (const absent of ["none", "", "  ", undefined]) {
    const probe = hostEvidence(absent);
    assert.equal(String(probe.virtualization ?? "").trim() === "" ||
      String(probe.virtualization).trim() === "none", true, `${JSON.stringify(absent)} must read as bare metal`);
  }
  for (const present of ["kvm", "vmware", "microsoft", "xen"]) {
    assert.notEqual(String(present).trim(), "none", `${present} must not read as bare metal`);
  }

  // The owned lane's uname is checked for its form, not for names. Every
  // committed owned-host matrix evidence passes. `uname -a` fails whatever the
  // node is called, and so does a uname from a kernel the attestation did not read.
  const matrixEvidenceRoot = path.join(REPO, "bench/evidence/matrix");
  let ownedEvidence = 0;
  for (const commit of fs.readdirSync(matrixEvidenceRoot)) {
    for (const run of fs.readdirSync(path.join(matrixEvidenceRoot, commit))) {
      const dir = path.join(matrixEvidenceRoot, commit, run);
      const providerPath = path.join(dir, "provider.json");
      if (!fs.existsSync(providerPath)) continue;
      const provider = JSON.parse(fs.readFileSync(providerPath, "utf8"));
      if (provider.schema !== "rustia.bench.dedicated-provider.v1") continue;
      const evidence = JSON.parse(fs.readFileSync(path.join(dir, "host.json"), "utf8"));
      assert.equal(matrixUnameProblem(evidence.uname, provider.kernel, evidence.arch), null, `${commit}/${run}`);
      ownedEvidence += 1;
    }
  }
  assert(ownedEvidence > 0, "expected committed owned-host matrix evidence to check the uname form against");
  const kernel = "6.8.0-138-generic";
  const version = "#138-Ubuntu SMP PREEMPT_DYNAMIC Tue Jan 6 00:00:00 UTC 2026";
  assert.equal(matrixUnameProblem(`Linux ${kernel} ${version} x86_64`, kernel, "x86_64"), null);
  for (const node of ["localhost", "box", authority]) {
    assert.match(
      matrixUnameProblem(`Linux ${node} ${kernel} ${version} x86_64 x86_64 x86_64 GNU/Linux`, kernel, "x86_64") ?? "",
      /must record uname -srvm/,
      `uname -a with node ${node} must be refused`,
    );
  }
  for (const [uname, attestedKernel] of [
    [`Linux ${kernel} ${version} x86_64`, "6.8.0-139-generic"],
    [`Linux ${kernel} ${version} x86_64`, undefined],
    [`Linux ${kernel} ${version} aarch64`, kernel],
    [undefined, kernel],
  ]) {
    assert.match(matrixUnameProblem(uname, attestedKernel, "x86_64") ?? "", /must record uname -srvm/, String(uname));
  }

  // A skipped screening still renders a summary a reader can act on.
  const skipped = screeningSummary({ skipped: true, reason: "nothing to compare" });
  assert.match(skipped, /## Benchmark screening/);
  assert.match(skipped, /nothing to compare/);
  const rendered = screeningSummary({
    skipped: false, baseline: "a".repeat(40), candidate: "b".repeat(40), host: "ci-bench",
    pinned_core: 4, legs_per_side: 2, repetitions: 5, confidence: 0.99,
    workloads: [
      { workload: "dense_tables", delta_percent: -3.5, interval: { lower: -4, upper: -3 },
        direction: "faster", verdict: "material_regression_excluded", reading: "faster" },
      { workload: "closures_upvalues", delta_percent: 9, interval: { lower: 8, upper: 10 },
        direction: "slower", verdict: "regression", reading: "slower" },
    ],
  });
  assert.match(rendered, /ci-bench/);
  assert.match(rendered, /-3\.50%/);
  assert.match(rendered, /\+9\.00%/);
  assert.match(rendered, /1 workload\(s\) exceeded the calibrated screening threshold/);

  const liveSerde = parseLiveSerdeOutput(
    "live serde:  1000.0 ns/deserialize\nowned path:  2500.0 ns/deserialize\nowned/live:  2.500x\n",
  );
  assert.equal(liveSerde.live_ns, 1000);
  assert.equal(liveSerde.owned_ns, 2500);
  assert.equal(liveSerde.owned_over_live, 2.5);
  assert.equal(liveSerde.live_latency_reduction_percent, 60);
  assert.match(
    liveSerdeSummary({
      commit: "c".repeat(40), host: "ci-bench", pinned_core: 4, ...liveSerde,
    }),
    /2\.500x.*60\.00% lower latency/,
  );
  assert.throws(() => parseLiveSerdeOutput("live serde:  1.0 ns/deserialize\n"), /owned path/);

  process.stdout.write("run-dedicated-benchmark self-test ok\n");
}

function usage() {
  process.stdout.write(
    "Usage: node tools/run-dedicated-benchmark.js <command> [options]\n" +
      "\n" +
      "  plan      [--config FILE]              freeze, attest the host, print the plan\n" +
      "  history   [--config FILE] [--replace]  measure every revision and promote\n" +
      "  main      --out FILE [--previous-binary SHA]  measure HEAD alone for the live series\n" +
      "  pr        --out FILE [--base REF] [--json FILE]  screen HEAD against its merge base\n" +
      "  accept    --baseline C --candidate C --out FILE [--passes N] [--json FILE]\n" +
      "                                         full acceptance comparison of two commits\n" +
      "  live-serde --local --out FILE             compare live and owned serde on ci-bench\n" +
      "  matrix    [--config FILE] [--plan]     measure 13 engines x 5 workloads and promote\n" +
      "  memory    [--cohort rustia-only|cross-engine] [--config FILE] [--plan]\n" +
      "                                         measure per-VM memory and promote\n" +
      "  calibrate [--out FILE] [--repetitions N] [--confidence P]\n" +
      "                                         measure one revision against itself and write\n" +
      "                                         the per-workload screening thresholds\n" +
      "  fetch     [--config FILE] [--replace]  re-collect and promote the last remote run\n" +
      "  unlock                                 clear stale lane locks on the benchmark host\n" +
      "  host-check                             print free space, journal size and load there\n" +
      "  self-test                              run the pure-function checks\n" +
      "\n" +
      `Set ${SSH_ENV} to the host's ssh destination or ssh_config alias.\n`,
  );
}

try {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed === null) {
    usage();
    process.exitCode = 2;
  } else if (parsed.command === "self-test") {
    selfTest();
  } else {
    fs.mkdirSync(STATE_ROOT, { recursive: true });
    useLocalHost(parsed.local === true);
    if (parsed.command === "plan") commandPlan(parsed);
    else if (parsed.command === "history") commandHistory(parsed);
    else if (parsed.command === "main") commandMain(parsed);
    else if (parsed.command === "pr") commandPr(parsed);
    else if (parsed.command === "accept") commandAccept(parsed);
    else if (parsed.command === "live-serde") commandLiveSerde(parsed);
    else if (parsed.command === "calibrate") commandCalibrate(parsed);
    else if (parsed.command === "matrix") commandMatrix(parsed);
    else if (parsed.command === "memory") {
      commandMemory(parsed).catch((error) => {
        process.stderr.write(`run-dedicated-benchmark: ${error.message}\n`);
        process.exitCode = 1;
      });
    }
    else if (parsed.command === "fetch") commandFetch(parsed);
    else if (parsed.command === "unlock") commandUnlock();
    else if (parsed.command === "host-check") commandHostCheck();
  }
} catch (error) {
  process.stderr.write(`run-dedicated-benchmark: ${error.message}\n`);
  process.exitCode = 1;
}
