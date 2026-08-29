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
// used as one.

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/dedicated");
const CONFIG_DIR = path.join(REPO, "bench/cloud");
const BACKFILL_DIR = path.join(REPO, "bench/history/backfills");
const EVIDENCE_DIR = path.join(REPO, "bench/evidence/history-backfill");
const REMOTE_RUNNER = "tools/run-linux-history-backfill.js";
const REMOTE_ROOT = ".rustia-history-backfill";
const SSH_ENV = "RUSTIA_BENCH_SSH";
const FULL_COMMIT = /^[0-9a-f]{40}$/;

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
  return destination.trim();
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

// spawnSync needs the remote script on stdin, which `local` does not thread, so
// the ssh helpers drive spawnSync directly rather than growing an option no
// other caller needs.
function sshCapture(script) {
  const result = spawnSync(
    "ssh",
    ["-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", sshDestination(), "bash", "-s"],
    { input: script, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  if (result.status !== 0) {
    fail(`ssh failed: ${(result.stderr ?? "").trim() || `exit ${result.status}`}`);
  }
  return result.stdout ?? "";
}

function sshStream(script) {
  const result = spawnSync(
    "ssh",
    ["-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", sshDestination(), "bash", "-s"],
    { input: script, stdio: ["pipe", "inherit", "inherit"] },
  );
  if (result.status !== 0) fail(`remote run failed with exit ${result.status}`);
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
  if (observed.smt !== "off" && observed.smt !== "notsupported") {
    problems.push(`SMT must be off, host reports ${JSON.stringify(observed.smt)}`);
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
  const known = new Set(["plan", "history", "fetch", "unlock", "self-test"]);
  if (!known.has(command)) return null;
  const parsed = { command, replace: false, config: undefined };
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
    } else {
      return null;
    }
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
printf 'cpu_model=%s\\n' "$(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | sed 's/^ *//')"
printf 'logical_cpus=%s\\n' "$(nproc --all)"
printf 'smt=%s\\n' "$(cat /sys/devices/system/cpu/smt/control 2>/dev/null || echo unknown)"
printf 'isolated_cpus=%s\\n' "$(cat /sys/devices/system/cpu/isolated 2>/dev/null || echo '')"
printf 'kernel=%s\\n' "$(uname -r)"
printf 'isolation=%s\\n' "$(tr ' ' '\\n' < /proc/cmdline | grep -E '^(isolcpus|nohz_full|rcu_nocbs)=' | tr '\\n' ' ')"
printf 'node=%s\\n' "$(command -v node >/dev/null && node --version || echo missing)"
printf 'cargo=%s\\n' "$(PATH="$HOME/.cargo/bin:$PATH" command -v cargo >/dev/null && PATH="$HOME/.cargo/bin:$PATH" cargo --version || echo missing)"
`;

function attestHost(config) {
  const raw = sshCapture(ATTEST_SCRIPT);
  const observed = {};
  for (const line of raw.split("\n")) {
    const at = line.indexOf("=");
    if (at > 0) observed[line.slice(0, at)] = line.slice(at + 1).trim();
  }
  observed.logical_cpus = Number(observed.logical_cpus);
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
  const remote = `$HOME/${REMOTE_ROOT}`;
  sshCapture(`set -eu\nmkdir -p "${remote}/inbox"\n`);
  local(
    "scp",
    ["-q", REMOTE_RUNNER, configFile, providerFile, bundleFile, `${sshDestination()}:${REMOTE_ROOT}/inbox/`],
    { capture: false },
  );

  process.stdout.write("running; this takes about 35 minutes for 75 revisions\n");
  sshStream(
    `set -eu\n` +
      `export RUSTIA_BACKFILL_ROOT="$HOME/${REMOTE_ROOT}"\n` +
      `cd "$HOME/${REMOTE_ROOT}/inbox"\n` +
      `export PATH="$HOME/.cargo/bin:$PATH"\n` +
      `node ${path.basename(REMOTE_RUNNER)} ` +
      `${path.basename(configFile)} provider.json repo.bundle\n`,
  );

  fetchExport(runDirectory);
  promote(runDirectory, config, options.replace);
  fs.writeFileSync(path.join(STATE_ROOT, "last-run"), `${runId}\n`);
}

function fetchExport(runDirectory) {
  const archive = path.join(runDirectory, "export.tar.gz");
  local("scp", ["-q", `${sshDestination()}:${REMOTE_ROOT}/export.tar.gz`, archive], {
    capture: false,
  });
  const extracted = path.join(runDirectory, "export");
  fs.rmSync(extracted, { recursive: true, force: true });
  fs.mkdirSync(extracted, { recursive: true });
  local("tar", ["-xzf", archive, "-C", extracted], { capture: false });
  process.stdout.write(`export sha256 ${sha256File(archive)}\n`);
  return extracted;
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

function commandUnlock() {
  const out = sshCapture(
    `set -u\n` +
      `lock="$HOME/${REMOTE_ROOT}/lock"\n` +
      `if [ -f "$lock" ]; then cat "$lock"; rm -f "$lock"; echo "removed"; else echo "no lock held"; fi\n`,
  );
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
  };
  assert.deepEqual(compareAttestation(declared, good), []);
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
  // An unsupported SMT control is not a failure: a CPU without SMT reports
  // "notsupported" and is exactly as good as one with SMT switched off.
  assert.deepEqual(compareAttestation(declared, { ...good, smt: "notsupported" }), []);

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

  assert.deepEqual(parseArgs(["plan"]), { command: "plan", replace: false, config: undefined });
  assert.deepEqual(parseArgs(["history", "--replace"]), {
    command: "history",
    replace: true,
    config: undefined,
  });
  assert.equal(parseArgs(["bogus"]), null);
  assert.equal(parseArgs(["plan", "--replace"]), null, "plan promotes nothing");
  assert.equal(parseArgs(["fetch", "--replace"]).replace, true, "fetch promotes, so it may supersede");
  assert.equal(parseArgs(["history", "--config"]), null);

  process.stdout.write("run-dedicated-benchmark self-test ok\n");
}

function usage() {
  process.stdout.write(
    "Usage: node tools/run-dedicated-benchmark.js <command> [options]\n" +
      "\n" +
      "  plan      [--config FILE]              freeze, attest the host, print the plan\n" +
      "  history   [--config FILE] [--replace]  measure every revision and promote\n" +
      "  fetch     [--config FILE] [--replace]  re-collect and promote the last remote run\n" +
      "  unlock                                 clear a stale lock on the benchmark host\n" +
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
    if (parsed.command === "plan") commandPlan(parsed);
    else if (parsed.command === "history") commandHistory(parsed);
    else if (parsed.command === "fetch") commandFetch(parsed);
    else if (parsed.command === "unlock") commandUnlock();
  }
} catch (error) {
  process.stderr.write(`run-dedicated-benchmark: ${error.message}\n`);
  process.exitCode = 1;
}
