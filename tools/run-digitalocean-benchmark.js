#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { spawn, spawnSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const DEFAULT_CONFIG = path.join(
  REPO,
  "bench/cloud/digitalocean-region-b861ee9.json",
);
const TRIAL_CONFIGS = Object.freeze({
  "basic-2c4g": path.join(REPO, "bench/cloud/digitalocean-trial-basic-2c4g.json"),
  "premium-intel-2c4g": path.join(
    REPO,
    "bench/cloud/digitalocean-trial-premium-intel-2c4g.json",
  ),
});
const REMOTE_RUNNER = path.join(REPO, "tools/run-linux-fixed-benchmark.js");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/digitalocean");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/;
const CONTEXT_HINT = "run `just bench-do-auth` once from an interactive terminal";
const API_TIMEOUT_MS = 30 * 1000;
const FORCE_KILL_DELAY_MS = 10 * 1000;
const CLEANUP_RESERVE_MS = 8 * 60 * 1000;
const DELETE_CONFIRM_TIMEOUT_MS = 4 * 60 * 1000;
const CREATE_RECONCILE_TIMEOUT_MS = 2 * 60 * 1000;
const KEY_RECONCILE_TIMEOUT_MS = 30 * 1000;
const COORDINATOR_LOCK = path.join(STATE_ROOT, "coordinator.lock");

let activeChild = null;
let activeForceKillTimer = null;
let interruptedSignal = null;
let cleanupInProgress = false;

function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode !== null) return;
  child.kill(signal);
  if (!activeForceKillTimer) {
    activeForceKillTimer = setTimeout(() => {
      if (child.exitCode === null) child.kill("SIGKILL");
    }, FORCE_KILL_DELAY_MS);
  }
}

for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    interruptedSignal = interruptedSignal || signal;
    if (!cleanupInProgress && activeChild && activeChild.exitCode === null) {
      terminateChild(activeChild, signal);
    }
  });
}

function fail(message) {
  throw new Error(message);
}

function throwIfInterrupted() {
  if (interruptedSignal && !cleanupInProgress) {
    fail(`cloud benchmark interrupted by ${interruptedSignal}`);
  }
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

function validateConfig(value) {
  const root = requireObject(value, "config");
  requireExactKeys(
    root,
    ["schema", "provider", "source", "measurement", "qualification", "host_gate", "publication"],
    "config",
  );
  if (
    root.schema !== "rustia.digitalocean-fixed-benchmark.v2" &&
    root.schema !== "rustia.digitalocean-fixed-benchmark.v3"
  ) {
    fail("unsupported cloud benchmark config schema");
  }

  const provider = requireObject(root.provider, "config.provider");
  requireExactKeys(
    provider,
    ["context", "region", "image_slug", "size", "max_runtime_minutes"],
    "config.provider",
  );
  requireString(provider.context, "config.provider.context");
  requireString(provider.region, "config.provider.region");
  requireString(provider.image_slug, "config.provider.image_slug");
  requireInteger(provider.max_runtime_minutes, "config.provider.max_runtime_minutes", 1);
  const size = requireObject(provider.size, "config.provider.size");
  requireExactKeys(
    size,
    ["description", "vcpus", "memory_mib", "disk_gib", "max_hourly_price_usd"],
    "config.provider.size",
  );
  requireString(size.description, "config.provider.size.description");
  requireInteger(size.vcpus, "config.provider.size.vcpus", 2);
  requireInteger(size.memory_mib, "config.provider.size.memory_mib", 1);
  requireInteger(size.disk_gib, "config.provider.size.disk_gib", 1);
  requireNumber(size.max_hourly_price_usd, "config.provider.size.max_hourly_price_usd", 0.001);

  const source = requireObject(root.source, "config.source");
  requireExactKeys(
    source,
    ["baseline_commit", "candidate_commit", "harness_commit", "expected_sha256"],
    "config.source",
  );
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    if (!FULL_COMMIT.test(requireString(source[field], `config.source.${field}`))) {
      fail(`config.source.${field} must be a lowercase full commit`);
    }
  }
  if (source.baseline_commit === source.candidate_commit) {
    fail("baseline and candidate commits must differ");
  }
  const expected = requireObject(source.expected_sha256, "config.source.expected_sha256");
  requireExactKeys(
    expected,
    [
      "baseline_original_harness",
      "overlay_harness",
      "workload",
      "cargo_lock",
      "toolchain",
      "analyzer",
      "history_before",
    ],
    "config.source.expected_sha256",
  );
  for (const [field, hash] of Object.entries(expected)) {
    if (!SHA256.test(requireString(hash, `config.source.expected_sha256.${field}`))) {
      fail(`config.source.expected_sha256.${field} must be a lowercase SHA-256`);
    }
  }

  const measurement = requireObject(root.measurement, "config.measurement");
  requireExactKeys(
    measurement,
    [
      "workload",
      "workload_path",
      "fixed_n",
      "repetitions",
      "expected_checksum",
      "run_order",
      "bootstrap_repetitions",
      "bootstrap_seed",
      "material_regression_threshold_percent",
      "expected_legs_per_group",
    ],
    "config.measurement",
  );
  requireString(measurement.workload, "config.measurement.workload");
  const workloadPath = requireString(measurement.workload_path, "config.measurement.workload_path");
  if (path.isAbsolute(workloadPath) || workloadPath.split("/").includes("..")) {
    fail("config.measurement.workload_path must be a repository-relative path");
  }
  requireInteger(measurement.fixed_n, "config.measurement.fixed_n", 1);
  requireInteger(measurement.repetitions, "config.measurement.repetitions", 1);
  requireString(measurement.expected_checksum, "config.measurement.expected_checksum");
  requireInteger(measurement.bootstrap_repetitions, "config.measurement.bootstrap_repetitions", 1000);
  requireInteger(measurement.bootstrap_seed, "config.measurement.bootstrap_seed", 0);
  requireNumber(
    measurement.material_regression_threshold_percent,
    "config.measurement.material_regression_threshold_percent",
    0,
  );
  const expectedLegs = requireInteger(
    measurement.expected_legs_per_group,
    "config.measurement.expected_legs_per_group",
    1,
  );
  if (!Array.isArray(measurement.run_order) || measurement.run_order.length !== expectedLegs * 2) {
    fail("config.measurement.run_order must contain exactly two groups of expected legs");
  }
  const seenFiles = new Set();
  const counts = { baseline: 0, candidate: 0 };
  for (const [index, rawLeg] of measurement.run_order.entries()) {
    const leg = requireObject(rawLeg, `config.measurement.run_order[${index}]`);
    requireExactKeys(leg, ["variant", "file"], `config.measurement.run_order[${index}]`);
    if (!(leg.variant in counts)) {
      fail(`config.measurement.run_order[${index}].variant must be baseline or candidate`);
    }
    if (!SAFE_FILE.test(requireString(leg.file, `config.measurement.run_order[${index}].file`))) {
      fail(`config.measurement.run_order[${index}].file must be a safe JSON basename`);
    }
    if (seenFiles.has(leg.file)) {
      fail(`duplicate run-order file ${leg.file}`);
    }
    seenFiles.add(leg.file);
    counts[leg.variant] += 1;
  }
  if (counts.baseline !== expectedLegs || counts.candidate !== expectedLegs) {
    fail("run order must contain the declared number of baseline and candidate legs");
  }

  const qualification = requireObject(root.qualification, "config.qualification");
  if (root.schema === "rustia.digitalocean-fixed-benchmark.v3") {
    requireExactKeys(
      qualification,
      ["paired_pilot_rounds", "max_ratio_spread_percent"],
      "config.qualification",
    );
    requireInteger(qualification.paired_pilot_rounds, "config.qualification.paired_pilot_rounds", 2);
    requireNumber(
      qualification.max_ratio_spread_percent,
      "config.qualification.max_ratio_spread_percent",
      0,
    );
    for (let index = 0; index < measurement.run_order.length; index += 2) {
      if (measurement.run_order[index].variant === measurement.run_order[index + 1].variant) {
        fail("DigitalOcean v3 run-order pairs must contain one baseline and one candidate leg");
      }
    }
  } else {
    requireExactKeys(
      qualification,
      ["candidate_pilot_legs", "max_median_spread_percent"],
      "config.qualification",
    );
    requireInteger(qualification.candidate_pilot_legs, "config.qualification.candidate_pilot_legs", 2);
    requireNumber(
      qualification.max_median_spread_percent,
      "config.qualification.max_median_spread_percent",
      0,
    );
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
      "wait_timeout_seconds",
    ],
    "config.host_gate",
  );
  requireInteger(gate.benchmark_cpu, "config.host_gate.benchmark_cpu", 0);
  requireInteger(gate.cooldown_seconds, "config.host_gate.cooldown_seconds", 1);
  requireInteger(gate.idle_window_seconds, "config.host_gate.idle_window_seconds", 1);
  requireNumber(gate.minimum_idle_percent, "config.host_gate.minimum_idle_percent", 0);
  if (gate.minimum_idle_percent > 100) {
    fail("config.host_gate.minimum_idle_percent must be <= 100");
  }
  requireNumber(gate.maximum_load_1m, "config.host_gate.maximum_load_1m", 0);
  requireInteger(gate.maximum_steal_ticks, "config.host_gate.maximum_steal_ticks", 0);
  requireInteger(gate.wait_timeout_seconds, "config.host_gate.wait_timeout_seconds", 1);

  const publication = requireObject(root.publication, "config.publication");
  requireExactKeys(
    publication,
    ["mode", "required_verdict", "final_measurements"],
    "config.publication",
  );
  if (!["acceptance", "diagnostic"].includes(publication.mode)) {
    fail("publication.mode must be acceptance or diagnostic");
  }
  if (publication.required_verdict !== "material_regression_excluded") {
    fail("publication.required_verdict must be material_regression_excluded");
  }
  requireInteger(publication.final_measurements, "config.publication.final_measurements", 3);

  return root;
}

function loadConfig(file = DEFAULT_CONFIG) {
  let value;
  try {
    value = JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot load ${path.relative(REPO, file)}: ${error.message}`);
  }
  return validateConfig(value);
}

function trialConfigPath(profile) {
  const configFile = TRIAL_CONFIGS[profile];
  if (!configFile) {
    fail(
      `unknown trial profile ${JSON.stringify(profile)}; choose one of: ` +
        Object.keys(TRIAL_CONFIGS).join(", "),
    );
  }
  return configFile;
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function sha256File(file) {
  return sha256(fs.readFileSync(file));
}

function quoteArg(value) {
  return /^[A-Za-z0-9_./:=+,-]+$/.test(value) ? value : JSON.stringify(value);
}

function commandText(command, args) {
  return [command, ...args].map(quoteArg).join(" ");
}

async function run(command, args, options = {}) {
  throwIfInterrupted();
  const {
    cwd = REPO,
    capture = true,
    allowFailure = false,
    input = null,
    timeoutMs = 0,
    announce = false,
  } = options;
  if (announce) {
    process.stderr.write(`+ ${commandText(command, args)}\n`);
  }
  return await new Promise((resolve, reject) => {
    const stdout = [];
    const stderr = [];
    const child = spawn(command, args, {
      cwd,
      env: process.env,
      stdio: [input === null ? "inherit" : "pipe", capture ? "pipe" : "inherit", capture ? "pipe" : "inherit"],
    });
    activeChild = child;
    let timedOut = false;
    let timer = null;
    if (timeoutMs > 0) {
      timer = setTimeout(() => {
        timedOut = true;
        terminateChild(child);
      }, timeoutMs);
    }
    if (capture) {
      child.stdout.on("data", (chunk) => stdout.push(Buffer.from(chunk)));
      child.stderr.on("data", (chunk) => stderr.push(Buffer.from(chunk)));
    }
    child.on("error", (error) => {
      if (timer) clearTimeout(timer);
      if (activeForceKillTimer) clearTimeout(activeForceKillTimer);
      activeForceKillTimer = null;
      if (activeChild === child) activeChild = null;
      reject(new Error(`cannot run ${command}: ${error.message}`));
    });
    child.on("close", (status, signal) => {
      if (timer) clearTimeout(timer);
      if (activeForceKillTimer) clearTimeout(activeForceKillTimer);
      activeForceKillTimer = null;
      if (activeChild === child) activeChild = null;
      const result = {
        status: status === null ? 1 : status,
        signal,
        stdout: Buffer.concat(stdout),
        stderr: Buffer.concat(stderr),
        timedOut,
      };
      if (result.status !== 0 && !allowFailure) {
        const detail = result.stderr.toString("utf8").trim();
        const suffix = timedOut
          ? " (timed out)"
          : signal
            ? ` (signal ${signal})`
            : "";
        reject(
          new Error(
            `${commandText(command, args)} failed with status ${result.status}${suffix}${detail ? `: ${detail}` : ""}`,
          ),
        );
        return;
      }
      resolve(result);
    });
    if (input !== null) {
      child.stdin.end(input);
    }
  });
}

function outputText(result) {
  return result.stdout.toString("utf8").trim();
}

function parseJsonOutput(result, description) {
  const text = outputText(result);
  try {
    return JSON.parse(text);
  } catch (error) {
    fail(`${description} returned invalid JSON: ${error.message}`);
  }
}

function findExecutable(name) {
  const pathValue = process.env.PATH || "";
  for (const directory of pathValue.split(path.delimiter)) {
    const candidate = path.join(directory || ".", name);
    try {
      fs.accessSync(candidate, fs.constants.X_OK);
      return candidate;
    } catch {
      // Keep searching.
    }
  }
  return null;
}

function requireExecutables(names) {
  const missing = names.filter((name) => !findExecutable(name));
  if (missing.length > 0) {
    fail(`missing required commands: ${missing.join(", ")}`);
  }
}

function normalizeList(value, description) {
  if (!Array.isArray(value)) {
    fail(`${description} must return a JSON array`);
  }
  return value;
}

function selectSize(sizes, config) {
  const wanted = config.provider.size;
  const matches = normalizeList(sizes, "DigitalOcean size catalog").filter(
    (entry) =>
      entry &&
      entry.description === wanted.description &&
      entry.vcpus === wanted.vcpus &&
      entry.memory === wanted.memory_mib &&
      entry.disk === wanted.disk_gib &&
      entry.available === true &&
      Array.isArray(entry.regions) &&
      entry.regions.includes(config.provider.region) &&
      typeof entry.price_hourly === "number" &&
      entry.price_hourly <= wanted.max_hourly_price_usd,
  );
  if (matches.length !== 1) {
    fail(
      `expected exactly one available ${wanted.vcpus}-vCPU/${wanted.memory_mib}-MiB ` +
        `${wanted.description} size in ${config.provider.region} at or below ` +
        `$${wanted.max_hourly_price_usd}/h; found ${matches.length}`,
    );
  }
  return matches[0];
}

function selectImage(images, config) {
  const matches = normalizeList(images, "DigitalOcean image catalog").filter(
    (entry) => entry && entry.slug === config.provider.image_slug && entry.public === true,
  );
  if (matches.length !== 1) {
    fail(`expected exactly one public image with slug ${config.provider.image_slug}; found ${matches.length}`);
  }
  if (!Number.isSafeInteger(matches[0].id)) {
    fail(`image ${config.provider.image_slug} did not have a numeric id`);
  }
  return matches[0];
}

async function gitBlob(commit, file) {
  const result = await run("git", ["show", `${commit}:${file}`]);
  return result.stdout;
}

async function verifySource(config, configFile = DEFAULT_CONFIG) {
  const relativeConfig = path.relative(REPO, configFile);
  if (relativeConfig.startsWith("..") || path.isAbsolute(relativeConfig)) {
    fail("cloud benchmark config must be inside the repository");
  }
  const trackedStatus = outputText(
    await run("git", ["status", "--porcelain=v1", "--untracked-files=no"]),
  );
  if (trackedStatus.length > 0) {
    fail("tracked worktree changes must be committed before a cloud benchmark");
  }
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    await run("git", ["cat-file", "-e", `${config.source[field]}^{commit}`]);
  }
  for (const file of [
    "tools/run-digitalocean-benchmark.js",
    "tools/run-linux-fixed-benchmark.js",
    relativeConfig,
  ]) {
    await run("git", ["ls-files", "--error-unmatch", file]);
  }

  const source = config.source;
  const expected = source.expected_sha256;
  const checks = [
    [
      "baseline original harness",
      sha256(await gitBlob(source.baseline_commit, "bench/src/main.rs")),
      expected.baseline_original_harness,
    ],
    [
      "candidate overlay harness",
      sha256(await gitBlob(source.harness_commit, "bench/src/main.rs")),
      expected.overlay_harness,
    ],
    [
      "baseline workload",
      sha256(await gitBlob(source.baseline_commit, config.measurement.workload_path)),
      expected.workload,
    ],
    [
      "candidate workload",
      sha256(await gitBlob(source.candidate_commit, config.measurement.workload_path)),
      expected.workload,
    ],
    ["baseline Cargo.lock", sha256(await gitBlob(source.baseline_commit, "Cargo.lock")), expected.cargo_lock],
    ["candidate Cargo.lock", sha256(await gitBlob(source.candidate_commit, "Cargo.lock")), expected.cargo_lock],
    [
      "baseline toolchain",
      sha256(await gitBlob(source.baseline_commit, "rust-toolchain.toml")),
      expected.toolchain,
    ],
    [
      "candidate toolchain",
      sha256(await gitBlob(source.candidate_commit, "rust-toolchain.toml")),
      expected.toolchain,
    ],
    [
      "analyzer",
      sha256(await gitBlob(source.harness_commit, "bench/analyze-fixed-samples.js")),
      expected.analyzer,
    ],
    ["current history", sha256File(path.join(REPO, "bench/history/runs.jsonl")), expected.history_before],
  ];
  for (const [name, actual, wanted] of checks) {
    if (actual !== wanted) {
      fail(`${name} SHA-256 is ${actual}, expected ${wanted}`);
    }
  }

  const head = outputText(await run("git", ["rev-parse", "HEAD"]));
  return {
    orchestration_commit: head,
    config_path: relativeConfig,
    config_sha256: sha256File(configFile),
    coordinator_sha256: sha256File(__filename),
    remote_runner_sha256: sha256File(REMOTE_RUNNER),
    verified_source_hashes: Object.fromEntries(checks.map(([name, actual]) => [name, actual])),
  };
}

async function doctlJson(args, config, options = {}) {
  const result = await run(
    "doctl",
    [...args, "--context", config.provider.context, "--output", "json"],
    { timeoutMs: API_TIMEOUT_MS, ...options },
  );
  if (result.status !== 0) return null;
  return parseJsonOutput(result, `doctl ${args.join(" ")}`);
}

async function resolveProviderPlan(config) {
  const sizesResult = await run(
    "doctl",
    ["compute", "size", "list", "--context", config.provider.context, "--output", "json"],
    { allowFailure: true, timeoutMs: API_TIMEOUT_MS },
  );
  if (sizesResult.status !== 0) {
    const detail = sizesResult.stderr.toString("utf8").trim();
    fail(`DigitalOcean authentication failed; ${CONTEXT_HINT}${detail ? `: ${detail}` : ""}`);
  }
  const size = selectSize(parseJsonOutput(sizesResult, "DigitalOcean size catalog"), config);
  const images = await doctlJson(["compute", "image", "list-distribution"], config);
  const image = selectImage(images, config);
  const regions = normalizeList(
    await doctlJson(["compute", "region", "list"], config),
    "DigitalOcean region catalog",
  );
  const region = regions.find((entry) => entry && entry.slug === config.provider.region);
  if (!region || region.available !== true) {
    fail(`DigitalOcean region ${config.provider.region} is not currently available`);
  }
  return { size, image, region };
}

function sanitizedProviderPlan(config, provider, source) {
  return {
    schema: config.schema,
    provider: "digitalocean",
    protocol_mode: config.publication.mode,
    context: config.provider.context,
    region: provider.region.slug,
    image: {
      id: provider.image.id,
      slug: provider.image.slug,
      distribution: provider.image.distribution,
      name: provider.image.name,
      created_at: provider.image.created_at,
    },
    size: {
      slug: provider.size.slug,
      description: provider.size.description,
      vcpus: provider.size.vcpus,
      memory_mib: provider.size.memory,
      disk_gib: provider.size.disk,
      price_hourly_usd: provider.size.price_hourly,
      price_monthly_usd: provider.size.price_monthly,
    },
    billable_lifetime_limit_minutes: config.provider.max_runtime_minutes,
    planned_compute_cost_ceiling_usd: Number(
      (provider.size.price_hourly * config.provider.max_runtime_minutes / 60).toFixed(6),
    ),
    cleanup_reserve_minutes: CLEANUP_RESERVE_MS / 60_000,
    billing_caveat:
      "the coordinator enforces this lifecycle deadline and reserves cleanup time, but a provider/API outage can delay confirmed deletion",
    source,
    comparison: {
      baseline_commit: config.source.baseline_commit,
      candidate_commit: config.source.candidate_commit,
      harness_overlay_commit: config.source.harness_commit,
      workload: config.measurement.workload,
      fixed_n: config.measurement.fixed_n,
      repetitions: config.measurement.repetitions,
      order: config.measurement.run_order,
    },
  };
}

async function buildPlan(config, { print = true, configFile = DEFAULT_CONFIG } = {}) {
  requireExecutables(["doctl", "git", "node", "scp", "ssh", "ssh-keygen", "tar"]);
  const source = await verifySource(config, configFile);
  const provider = await resolveProviderPlan(config);
  const plan = sanitizedProviderPlan(config, provider, source);
  if (print) {
    process.stdout.write(`${JSON.stringify(plan, null, 2)}\n`);
  }
  return { plan, provider, source };
}

async function verifyDeleteAuthority(config) {
  const sentinel = 2_147_483_647;
  if ((await listDroplets(config)).some((entry) => entry && String(entry.id) === String(sentinel))) {
    fail(`refusing to use delete-authority sentinel ${sentinel}: the account owns that Droplet ID`);
  }
  if ((await listSshKeys(config)).some((entry) => entry && String(entry.id) === String(sentinel))) {
    fail(`refusing to use delete-authority sentinel ${sentinel}: the account owns that SSH-key ID`);
  }
  const probes = [
    { label: "Droplet", args: ["compute", "droplet", "delete", String(sentinel), "--force"] },
    { label: "SSH key", args: ["compute", "ssh-key", "delete", String(sentinel), "--force"] },
  ];
  for (const probe of probes) {
    const result = await run(
      "doctl",
      [...probe.args, "--context", config.provider.context, "--http-retry-max", "0"],
      { allowFailure: true, timeoutMs: API_TIMEOUT_MS },
    );
    const detail = `${result.stdout.toString("utf8")}\n${result.stderr.toString("utf8")}`.trim();
    if (result.status !== 0 && !/(?:\b404\b|not found|does not exist)/i.test(detail)) {
      fail(
        `${probe.label} delete authority could not be proven before billing; verify the custom ` +
          `token scopes (${detail || `doctl status ${result.status}`})`,
      );
    }
  }
  process.stdout.write("Pre-billing absent-ID probes confirmed Droplet and SSH-key delete authority.\n");
}

function safeTimestamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function writeJsonAtomic(file, value, mode = 0o600) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const temporary = `${file}.tmp-${process.pid}`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode });
  fs.renameSync(temporary, file);
}

function writeBytesAtomic(file, bytes, mode = 0o644) {
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomBytes(4).toString("hex")}`;
  fs.writeFileSync(temporary, bytes, { mode });
  fs.renameSync(temporary, file);
}

function readState(directory) {
  const file = path.join(directory, "state.json");
  const state = JSON.parse(fs.readFileSync(file, "utf8"));
  state.__directory = directory;
  return state;
}

function processExists(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return null;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error && error.code === "ESRCH") return false;
    return true;
  }
}

function processFingerprint(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) return null;
  const result = spawnSync(
    "ps",
    ["-p", String(pid), "-o", "lstart=", "-o", "command="],
    { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
  );
  if (result.status !== 0) return null;
  const fingerprint = String(result.stdout || "").trim();
  return fingerprint.length > 0 ? fingerprint : null;
}

function lockOwnerIsLive(owner) {
  if (processExists(owner.pid) === false) return false;
  const currentFingerprint = processFingerprint(owner.pid);
  if (owner.process_fingerprint && currentFingerprint) {
    return owner.process_fingerprint === currentFingerprint;
  }
  return true;
}

function acquireCoordinatorLock() {
  fs.mkdirSync(STATE_ROOT, { recursive: true, mode: 0o700 });
  const token = crypto.randomBytes(16).toString("hex");
  const pending = `${COORDINATOR_LOCK}.pending-${token}`;
  fs.mkdirSync(pending, { mode: 0o700 });
  writeJsonAtomic(path.join(pending, "owner.json"), {
    schema: "rustia.digitalocean-benchmark-lock.v1",
    pid: process.pid,
    token,
    process_fingerprint: processFingerprint(process.pid),
    acquired_at: new Date().toISOString(),
  });
  try {
    fs.renameSync(pending, COORDINATOR_LOCK);
  } catch (error) {
    fs.rmSync(pending, { recursive: true, force: true });
    if (!error || !["EEXIST", "ENOTEMPTY"].includes(error.code)) throw error;
    let owner;
    try {
      owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
    } catch {
      fail(`DigitalOcean coordinator lock exists without readable ownership at ${COORDINATOR_LOCK}`);
    }
    if (!lockOwnerIsLive(owner)) {
      fail(`stale DigitalOcean coordinator lock for PID ${owner.pid}; run \`just bench-do-unlock\``);
    }
    fail(`DigitalOcean benchmark coordinator PID ${owner.pid} is already active`);
  }
  return token;
}

function releaseCoordinatorLock(token) {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot verify DigitalOcean coordinator lock ownership: ${error.message}`);
  }
  if (owner.pid !== process.pid || owner.token !== token) {
    fail("refusing to remove a DigitalOcean coordinator lock owned by another process");
  }
  fs.rmSync(COORDINATOR_LOCK, { recursive: true });
}

function unlockStaleCoordinator() {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot read DigitalOcean coordinator lock ownership: ${error.message}`);
  }
  if (lockOwnerIsLive(owner)) {
    fail(`refusing to unlock live DigitalOcean coordinator PID ${owner.pid}`);
  }
  const quarantine = `${COORDINATOR_LOCK}.stale-${crypto.randomBytes(8).toString("hex")}`;
  fs.renameSync(COORDINATOR_LOCK, quarantine);
  const movedOwner = JSON.parse(fs.readFileSync(path.join(quarantine, "owner.json"), "utf8"));
  if (movedOwner.pid !== owner.pid || movedOwner.token !== owner.token) {
    fail(`stale lock ownership changed during quarantine; retained at ${quarantine}`);
  }
  fs.rmSync(quarantine, { recursive: true });
  process.stdout.write(`Removed stale DigitalOcean coordinator lock for PID ${owner.pid}.\n`);
}

function listStates() {
  if (!fs.existsSync(STATE_ROOT)) return [];
  return fs
    .readdirSync(STATE_ROOT, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && fs.existsSync(path.join(STATE_ROOT, entry.name, "state.json")))
    .map((entry) => readState(path.join(STATE_ROOT, entry.name)))
    .sort((left, right) => String(left.created_at).localeCompare(String(right.created_at)));
}

function incompleteStates() {
  return listStates().filter(
    (state) => !state.droplet_destroyed_at || !state.ssh_key_deleted_at,
  );
}

function stateForRunId(runId) {
  const states = incompleteStates();
  if (runId) {
    const matches = states.filter((state) => state.run_id === runId);
    if (matches.length !== 1) fail(`no active owned run has id ${runId}`);
    return matches[0];
  }
  if (states.length !== 1) {
    fail(
      states.length === 0
        ? "no active owned DigitalOcean benchmark run exists"
        : `more than one active run exists; pass one of: ${states.map((state) => state.run_id).join(", ")}`,
    );
  }
  return states[0];
}

function configForState(state) {
  const context = requireString(state.context, `state ${state.run_id}.context`);
  return {
    provider: { context },
  };
}

function saveState(state) {
  const directory = state.__directory;
  const copy = { ...state };
  delete copy.__directory;
  writeJsonAtomic(path.join(directory, "state.json"), copy);
}

function startBillingWindow(state, config) {
  const started = Date.now();
  state.billing_window_started_at = new Date(started).toISOString();
  state.billing_deadline_at = new Date(
    started + config.provider.max_runtime_minutes * 60 * 1000,
  ).toISOString();
  saveState(state);
}

function remainingWorkTimeout(state, requestedMs, label) {
  const deadline = Date.parse(state.billing_deadline_at || "");
  if (!Number.isFinite(deadline)) fail("owned run has no valid billing deadline");
  const remaining = deadline - Date.now() - CLEANUP_RESERVE_MS;
  if (remaining <= 0) {
    fail(`${label}: billable lifetime reached the reserved cleanup window`);
  }
  return Math.max(1, Math.min(requestedMs, remaining));
}

function numericId(text, description) {
  const first = text.trim().split(/\s+/)[0];
  if (!/^[1-9][0-9]*$/.test(first)) {
    fail(`${description} did not return a numeric id: ${JSON.stringify(text.trim())}`);
  }
  return Number(first);
}

function objectFromGet(value) {
  if (Array.isArray(value)) return value.length === 1 ? value[0] : null;
  return value && typeof value === "object" ? value : null;
}

async function listDroplets(config) {
  return normalizeList(
    await doctlJson(["compute", "droplet", "list"], config),
    "DigitalOcean droplet list",
  );
}

async function reconcileDropletByName(state, config) {
  const matches = (await listDroplets(config)).filter((entry) => entry && entry.name === state.droplet_name);
  if (matches.length > 1) {
    fail(`refusing to adopt or delete ${matches.length} droplets named ${state.droplet_name}`);
  }
  if (matches.length === 1) {
    state.droplet_id = matches[0].id;
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileDropletAfterCreateAttempt(state, config) {
  if (state.droplet_absence_confirmed_at) return null;
  const attemptedAt = Date.parse(state.droplet_create_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile a Droplet without a create-attempt timestamp");
  const finishedAt = Date.parse(state.droplet_create_finished_at || "");
  const requestDeadline = Date.parse(state.droplet_create_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = Math.max(Date.now(), uncertaintyEndedAt + CREATE_RECONCILE_TIMEOUT_MS);
  while (Date.now() < deadline) {
    try {
      const droplet = await reconcileDropletByName(state, config);
      if (droplet) return droplet;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      // Only a successful list after the grace window can prove absence.
    }
    const delay = Math.min(5000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  const droplet = await reconcileDropletByName(state, config);
  if (droplet) return droplet;
  state.droplet_absence_confirmed_at = new Date().toISOString();
  saveState(state);
  return null;
}

async function getDroplet(state, config, { allowMissing = false } = {}) {
  if (!state.droplet_id) return null;
  const result = await run(
    "doctl",
    [
      "compute",
      "droplet",
      "get",
      String(state.droplet_id),
      "--context",
      config.provider.context,
      "--output",
      "json",
    ],
    { allowFailure: true, timeoutMs: API_TIMEOUT_MS },
  );
  if (result.status !== 0) {
    if (allowMissing) return null;
    fail(`cannot verify owned droplet ${state.droplet_id}: ${result.stderr.toString("utf8").trim()}`);
  }
  const droplet = objectFromGet(parseJsonOutput(result, "DigitalOcean droplet get"));
  if (!droplet || droplet.id !== state.droplet_id || droplet.name !== state.droplet_name) {
    fail(`droplet ${state.droplet_id} does not match owned name ${state.droplet_name}`);
  }
  return droplet;
}

async function listSshKeys(config) {
  return normalizeList(
    await doctlJson(["compute", "ssh-key", "list"], config),
    "DigitalOcean SSH key list",
  );
}

async function reconcileSshKeyByName(state, config) {
  const matches = (await listSshKeys(config)).filter(
    (entry) => entry && entry.name === state.ssh_key_name,
  );
  if (matches.length > 1) {
    fail(`refusing to adopt or delete ${matches.length} SSH keys named ${state.ssh_key_name}`);
  }
  if (matches.length === 1) {
    state.ssh_key_id = matches[0].id;
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileSshKeyAfterImportAttempt(state, config) {
  if (state.ssh_key_absence_confirmed_at) return null;
  const attemptedAt = Date.parse(state.ssh_key_import_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile an SSH key without an import-attempt timestamp");
  const finishedAt = Date.parse(state.ssh_key_import_finished_at || "");
  const requestDeadline = Date.parse(state.ssh_key_import_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = Math.max(Date.now(), uncertaintyEndedAt + KEY_RECONCILE_TIMEOUT_MS);
  while (Date.now() < deadline) {
    try {
      const key = await reconcileSshKeyByName(state, config);
      if (key) return key;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      // Only a successful list after the grace window can prove absence.
    }
    const delay = Math.min(3000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  const key = await reconcileSshKeyByName(state, config);
  if (key) return key;
  state.ssh_key_absence_confirmed_at = new Date().toISOString();
  saveState(state);
  return null;
}

async function deleteSshKey(state, config) {
  if (state.ssh_key_deleted_at) return;
  if (!state.ssh_key_id) {
    const key = state.ssh_key_import_attempted_at
      ? await reconcileSshKeyAfterImportAttempt(state, config)
      : await reconcileSshKeyByName(state, config);
    if (!key) {
      state.ssh_key_deleted_at = new Date().toISOString();
      saveState(state);
      return;
    }
  }
  await run(
    "doctl",
    [
      "compute",
      "ssh-key",
      "delete",
      String(state.ssh_key_id),
      "--force",
      "--context",
      config.provider.context,
    ],
    { allowFailure: true, announce: true, timeoutMs: API_TIMEOUT_MS },
  );

  // DigitalOcean's key list can briefly lag a successful delete. Reconcile it
  // like Droplet deletion instead of treating the first stale list response as
  // a cleanup failure. A successful list that omits the owned id is the proof
  // of absence; failed reads and DELETE 404s are only retry signals.
  const confirmationDeadline = Date.now() + DELETE_CONFIRM_TIMEOUT_MS;
  while (Date.now() < confirmationDeadline) {
    try {
      const present = (await listSshKeys(config)).some(
        (entry) => entry && entry.id === state.ssh_key_id,
      );
      if (!present) {
        state.ssh_key_deleted_at = new Date().toISOString();
        saveState(state);
        return;
      }
      await run(
        "doctl",
        [
          "compute",
          "ssh-key",
          "delete",
          String(state.ssh_key_id),
          "--force",
          "--context",
          config.provider.context,
        ],
        { allowFailure: true, timeoutMs: API_TIMEOUT_MS },
      );
    } catch {
      // A failed read is not proof of deletion; retry until a successful list confirms absence.
    }
    const delay = Math.min(3000, Math.max(0, confirmationDeadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  fail(
    `could not confirm deletion of owned SSH key ${state.ssh_key_id}; run ` +
      `\`just bench-do-destroy ${state.run_id}\` and do not assume cleanup completed`,
  );
}

async function deleteDroplet(state, config) {
  if (state.droplet_destroyed_at) return;
  if (!state.droplet_id) {
    const reconciled = state.droplet_create_attempted_at
      ? await reconcileDropletAfterCreateAttempt(state, config)
      : await reconcileDropletByName(state, config);
    if (!reconciled) {
      state.droplet_destroyed_at = new Date().toISOString();
      state.cleanup_note = state.droplet_create_attempted_at
        ? "post-attempt grace window and final successful list confirmed the unique Droplet name absent"
        : "no Droplet create was attempted and a successful list confirmed the unique name absent";
      saveState(state);
      return;
    }
  }

  let existing = await getDroplet(state, config, { allowMissing: true });
  if (!existing) {
    const stillListed = (await listDroplets(config)).find((entry) => entry.id === state.droplet_id);
    if (!stillListed) {
      state.droplet_destroyed_at = new Date().toISOString();
      saveState(state);
      return;
    }
    if (stillListed.name !== state.droplet_name) {
      fail(`refusing to delete droplet ${state.droplet_id}: owned name no longer matches`);
    }
    existing = stillListed;
  }

  await run(
    "doctl",
    [
      "compute",
      "droplet",
      "delete",
      String(existing.id),
      "--force",
      "--context",
      config.provider.context,
    ],
    { allowFailure: true, announce: true, timeoutMs: API_TIMEOUT_MS },
  );

  const confirmationDeadline = Date.now() + DELETE_CONFIRM_TIMEOUT_MS;
  while (Date.now() < confirmationDeadline) {
    try {
      const present = (await listDroplets(config)).some((entry) => entry.id === existing.id);
      if (!present) {
        state.droplet_destroyed_at = new Date().toISOString();
        saveState(state);
        return;
      }
      await run(
        "doctl",
        [
          "compute",
          "droplet",
          "delete",
          String(existing.id),
          "--force",
          "--context",
          config.provider.context,
        ],
        { allowFailure: true, timeoutMs: API_TIMEOUT_MS },
      );
    } catch {
      // A failed read is not proof of deletion; retry until a successful list confirms absence.
    }
    const delay = Math.min(4000, Math.max(0, confirmationDeadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  fail(
    `could not confirm destruction of owned droplet ${existing.id}; run ` +
      `\`just bench-do-destroy ${state.run_id}\` and do not assume billing stopped`,
  );
}

async function cleanupOwned(state, config) {
  const errors = [];
  try {
    await deleteDroplet(state, config);
  } catch (error) {
    errors.push(`Droplet cleanup: ${error.message}`);
  }
  try {
    await deleteSshKey(state, config);
  } catch (error) {
    errors.push(`SSH key cleanup: ${error.message}`);
  }
  if (state.droplet_destroyed_at && state.ssh_key_deleted_at) {
    for (const file of [state.private_key_path, state.public_key_path]) {
      if (file && fs.existsSync(file)) fs.rmSync(file);
    }
  }
  if (errors.length > 0) fail(errors.join("; "));
}

function sshOptions(state) {
  return [
    "-i",
    state.private_key_path,
    "-o",
    "BatchMode=yes",
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-o",
    `UserKnownHostsFile=${state.known_hosts_path}`,
  ];
}

function scpOptions(state) {
  return [
    "-i",
    state.private_key_path,
    "-o",
    "BatchMode=yes",
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "StrictHostKeyChecking=accept-new",
    "-o",
    `UserKnownHostsFile=${state.known_hosts_path}`,
  ];
}

function publicIpv4(droplet) {
  const networks = droplet && droplet.networks && droplet.networks.v4;
  if (!Array.isArray(networks)) return null;
  const entry = networks.find((network) => network.type === "public");
  return entry && typeof entry.ip_address === "string" ? entry.ip_address : null;
}

async function waitForDroplet(state, config) {
  const deadline = Date.now() + remainingWorkTimeout(state, 5 * 60 * 1000, "droplet activation");
  while (Date.now() < deadline) {
    const droplet = await getDroplet(state, config, { allowMissing: true });
    const ip = publicIpv4(droplet);
    if (droplet && droplet.status === "active" && ip) {
      state.public_ipv4 = ip;
      state.droplet_created_at = droplet.created_at || new Date().toISOString();
      saveState(state);
      return droplet;
    }
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`droplet ${state.droplet_id} did not become active with a public IPv4 address`);
}

async function waitForSsh(state) {
  const deadline = Date.now() + remainingWorkTimeout(state, 5 * 60 * 1000, "SSH readiness");
  while (Date.now() < deadline) {
    const result = await run(
      "ssh",
      [...sshOptions(state), `root@${state.public_ipv4}`, "true"],
      { allowFailure: true, timeoutMs: 15 * 1000 },
    );
    if (result.status === 0) return;
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`SSH did not become ready on owned droplet ${state.droplet_id}`);
}

function verifyDropletMatchesPlan(droplet, plan) {
  const checks = [
    ["size slug", droplet.size_slug, plan.size.slug],
    ["region", droplet.region && droplet.region.slug, plan.region],
    ["image ID", droplet.image && droplet.image.id, plan.image.id],
    ["vCPU count", droplet.vcpus, plan.size.vcpus],
    ["memory MiB", droplet.memory, plan.size.memory_mib],
    ["disk GiB", droplet.disk, plan.size.disk_gib],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(
        `activated Droplet ${field} was ${JSON.stringify(actual)}, ` +
          `expected planned ${JSON.stringify(expected)}`,
      );
    }
  }
}

function safeDropletMetadata(droplet, plan, state) {
  return {
    provider: "digitalocean",
    droplet_id: droplet.id,
    droplet_name: droplet.name,
    created_at: droplet.created_at,
    status: droplet.status,
    region: droplet.region && droplet.region.slug,
    size_slug: droplet.size_slug,
    image: droplet.image
      ? {
          id: droplet.image.id,
          slug: droplet.image.slug,
          distribution: droplet.image.distribution,
          name: droplet.image.name,
          created_at: droplet.image.created_at,
        }
      : null,
    vcpus: droplet.vcpus,
    memory_mib: droplet.memory,
    disk_gib: droplet.disk,
    public_ipv4: state.public_ipv4,
    catalog_plan: plan,
  };
}

function extractArchive(archive, destination) {
  fs.mkdirSync(destination, { recursive: true });
  return run("tar", ["-xzf", archive, "-C", destination], { capture: false, announce: true });
}

function readJsonIfPresent(file) {
  if (!fs.existsSync(file)) return null;
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot read downloaded diagnostic JSON ${file}: ${error.message}`);
  }
}

function diagnosticSummary(config, state, downloadDirectory) {
  const statusFile = path.join(downloadDirectory, "status.json");
  if (!fs.existsSync(statusFile)) fail("diagnostic export did not contain status.json");
  const status = JSON.parse(fs.readFileSync(statusFile, "utf8"));
  state.remote_status = status.status;
  state.remote_status_detail = status.detail || null;

  const started = Date.parse(state.billing_window_started_at || "");
  const destroyed = Date.parse(state.droplet_destroyed_at || "");
  const billableSecondsUpperBound =
    Number.isFinite(started) && Number.isFinite(destroyed)
      ? Math.max(0, (destroyed - started) / 1000)
      : null;
  const hourlyPrice = state.plan.size.price_hourly_usd;
  const estimatedCharge =
    billableSecondsUpperBound === null
      ? null
      : Math.max(0.01, (hourlyPrice * Math.max(60, billableSecondsUpperBound)) / 3600);
  const diagnostic = status.diagnostic || null;
  const evidenceDirectory = path.join(
    downloadDirectory,
    "evidence",
    config.source.candidate_commit,
  );
  const qualificationDirectory = path.join(evidenceDirectory, "qualification-sidecars");
  const qualification =
    (diagnostic && diagnostic.qualification) ||
    readJsonIfPresent(path.join(evidenceDirectory, "qualification.json")) ||
    readJsonIfPresent(path.join(qualificationDirectory, "qualification.json"));
  const host =
    (diagnostic && diagnostic.host) || readJsonIfPresent(path.join(evidenceDirectory, "host.json"));
  let pilotLegs = qualification && Array.isArray(qualification.pilot_legs)
    ? qualification.pilot_legs
    : [];
  const pilotPairs = qualification && Array.isArray(qualification.pilot_pairs)
    ? qualification.pilot_pairs
    : [];
  if (pilotLegs.length === 0 && pilotPairs.length > 0) {
    pilotLegs = pilotPairs.flatMap((pair) => [pair.baseline, pair.candidate].filter(Boolean));
  }
  if (pilotLegs.length === 0 && fs.existsSync(qualificationDirectory)) {
    pilotLegs = fs
      .readdirSync(qualificationDirectory)
      .filter((file) => /^candidate-pilot-[0-9]+\.json$/.test(file))
      .sort()
      .flatMap((file) => {
        const sidecar = readJsonIfPresent(path.join(qualificationDirectory, file));
        const measurements = sidecar && sidecar.measurements;
        return Array.isArray(measurements) && measurements.length === 1
          ? [{ file, harness_median: measurements[0].ns_per_unit_median }]
          : [];
      });
  }
  const buildSeconds =
    diagnostic && diagnostic.timings
      ? diagnostic.timings.build_seconds
      : status.timings?.build_seconds || null;
  const qualificationSeconds =
    diagnostic && diagnostic.timings
      ? diagnostic.timings.qualification_seconds
      : status.timings?.qualification_seconds || null;
  const qualificationPassed =
    status.status === "diagnostic_complete" && qualification?.passed === true;
  const extraAcceptanceLegs =
    config.measurement.run_order.length + config.publication.final_measurements;
  const qualificationLegs = config.schema === "rustia.digitalocean-fixed-benchmark.v3"
    ? config.qualification.paired_pilot_rounds * 2
    : config.qualification.candidate_pilot_legs;
  const projectedAcceptanceSeconds =
    qualificationPassed && billableSecondsUpperBound !== null && Number.isFinite(qualificationSeconds)
      ? billableSecondsUpperBound +
        (qualificationSeconds / qualificationLegs) * extraAcceptanceLegs
      : null;
  const acceptanceConfig = loadConfig(DEFAULT_CONFIG);
  const acceptanceWorkBudgetSeconds =
    acceptanceConfig.provider.max_runtime_minutes * 60 - CLEANUP_RESERVE_MS / 1000;
  const projectionWithinAcceptanceBudget =
    projectedAcceptanceSeconds !== null &&
    projectedAcceptanceSeconds <= acceptanceWorkBudgetSeconds;
  const eligible = qualificationPassed && projectionWithinAcceptanceBudget;
  const projectedAcceptanceCharge =
    projectedAcceptanceSeconds === null
      ? null
      : Math.max(0.01, (hourlyPrice * Math.max(60, projectedAcceptanceSeconds)) / 3600);
  const summary = {
    profile: state.trial_profile,
    run_id: state.run_id,
    status: status.status,
    detail: status.detail || null,
    size_slug: state.plan.size.slug,
    hourly_price_usd: hourlyPrice,
    planned_compute_cost_ceiling_usd: state.plan.planned_compute_cost_ceiling_usd,
    billable_seconds_upper_bound: billableSecondsUpperBound,
    estimated_compute_charge_upper_bound_usd:
      estimatedCharge === null ? null : Number(estimatedCharge.toFixed(6)),
    projected_acceptance_seconds:
      projectedAcceptanceSeconds === null ? null : Number(projectedAcceptanceSeconds.toFixed(3)),
    projected_acceptance_compute_charge_usd:
      projectedAcceptanceCharge === null ? null : Number(projectedAcceptanceCharge.toFixed(6)),
    acceptance_work_budget_seconds: acceptanceWorkBudgetSeconds,
    cpu_model: host ? host.cpu_model : null,
    build_seconds: buildSeconds,
    qualification_seconds: qualificationSeconds,
    pilot_medians_ns_per_unit: Array.isArray(pilotLegs)
      ? pilotLegs.map((leg) => leg.harness_median)
      : [],
    pilot_median_spread_percent:
      qualification && Number.isFinite(qualification.median_spread_percent)
        ? qualification.median_spread_percent
        : null,
    pilot_pair_candidate_over_baseline_ratios: pilotPairs.map(
      (pair) => pair.candidate_over_baseline_ratio,
    ),
    pilot_ratio_spread_percent:
      qualification && Number.isFinite(qualification.ratio_spread_percent)
        ? qualification.ratio_spread_percent
        : null,
    qualification_passed: qualificationPassed,
    projection_within_acceptance_budget: projectionWithinAcceptanceBudget,
    eligible_for_acceptance_profile: eligible,
    remote_exit_status: state.remote_exit_status,
    export_directory: path.relative(REPO, downloadDirectory),
    publication_promoted: false,
  };
  state.trial_summary = summary;
  saveState(state);
  process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
  return summary;
}

async function verifyPromotionCheckout(config, state) {
  const current = await verifySource(config, state.source_config_path || DEFAULT_CONFIG);
  const planned = state.plan && state.plan.source;
  if (!planned || JSON.stringify(current) !== JSON.stringify(planned)) {
    fail("local orchestration checkout changed while the cloud run was in flight; retaining export only");
  }
}

async function promoteAcceptedResult(config, state, downloadDirectory) {
  throwIfInterrupted();
  const statusFile = path.join(downloadDirectory, "status.json");
  if (!fs.existsSync(statusFile)) fail("remote export did not contain status.json");
  const status = JSON.parse(fs.readFileSync(statusFile, "utf8"));
  state.remote_status = status.status;
  state.remote_status_detail = status.detail || null;
  saveState(state);
  if (status.status !== "accepted_and_published") {
    process.stdout.write(
      `Cloud run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
        `(${status.status}${status.detail ? `: ${status.detail}` : ""}).\n`,
    );
    return false;
  }
  await verifyPromotionCheckout(config, state);

  const candidate = config.source.candidate_commit;
  const evidenceSource = path.join(downloadDirectory, "evidence", candidate);
  const evidenceDestination = path.join(REPO, "bench/history/evidence", candidate);
  const historySource = path.join(downloadDirectory, "publication", "bench/history/runs.jsonl");
  const historyDestination = path.join(REPO, "bench/history/runs.jsonl");
  for (const required of [
    path.join(evidenceSource, "manifest.json"),
    path.join(evidenceSource, "analysis.json"),
    historySource,
  ]) {
    if (!fs.existsSync(required)) fail(`remote accepted export is missing ${required}`);
  }
  if (fs.existsSync(evidenceDestination)) {
    fail(`refusing to overwrite existing evidence directory ${evidenceDestination}`);
  }
  const localHistory = fs.readFileSync(historyDestination);
  if (sha256(localHistory) !== config.source.expected_sha256.history_before) {
    fail("local benchmark history changed while the cloud run was in flight");
  }
  const remoteHistory = fs.readFileSync(historySource);
  if (
    remoteHistory.length <= localHistory.length ||
    !remoteHistory.subarray(0, localHistory.length).equals(localHistory)
  ) {
    fail("remote history is not an append-only extension of the exact local history");
  }
  const oldLines = localHistory.toString("utf8").trimEnd().split("\n").length;
  const newLines = remoteHistory.toString("utf8").trimEnd().split("\n").length;
  if (newLines - oldLines !== config.publication.final_measurements) {
    fail(
      `remote history appended ${newLines - oldLines} rows, expected ${config.publication.final_measurements}`,
    );
  }

  const evidenceStage = `${evidenceDestination}.incoming-${state.run_id}`;
  const historyStage = `${historyDestination}.incoming-${state.run_id}`;
  const generatedFiles = [
    path.join(REPO, "bench/RESULTS.md"),
    path.join(REPO, "bench/current-results.json"),
    path.join(REPO, "bench/current-results.html"),
    path.join(REPO, "bench/CLAIMS.md"),
  ];
  const snapshots = [historyDestination, ...generatedFiles].map((file) => {
    if (!fs.existsSync(file)) return { file, existed: false };
    return {
      file,
      existed: true,
      bytes: fs.readFileSync(file),
      mode: fs.statSync(file).mode & 0o777,
    };
  });
  let evidenceCommitted = false;
  let historyCommitted = false;
  try {
    fs.cpSync(evidenceSource, evidenceStage, { recursive: true, errorOnExist: true });
    fs.copyFileSync(historySource, historyStage, fs.constants.COPYFILE_EXCL);
    throwIfInterrupted();
    state.promotion_started_at = new Date().toISOString();
    saveState(state);
    fs.renameSync(evidenceStage, evidenceDestination);
    evidenceCommitted = true;
    fs.renameSync(historyStage, historyDestination);
    historyCommitted = true;
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "gen-current-results"], {
      capture: false,
      announce: true,
    });
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "check-benchmark-results"], {
      capture: false,
      announce: true,
    });
  } catch (error) {
    fs.rmSync(evidenceStage, { recursive: true, force: true });
    fs.rmSync(historyStage, { force: true });
    if (evidenceCommitted) {
      fs.rmSync(evidenceDestination, { recursive: true, force: true });
    }
    if (historyCommitted) {
      for (const snapshot of snapshots) {
        if (snapshot.existed) {
          writeBytesAtomic(snapshot.file, snapshot.bytes, snapshot.mode);
        } else {
          fs.rmSync(snapshot.file, { force: true });
        }
      }
    }
    state.promotion_rolled_back_at = new Date().toISOString();
    state.promotion_rollback_reason = error.message;
    saveState(state);
    throw error;
  }
  state.promoted_at = new Date().toISOString();
  saveState(state);
  process.stdout.write(
    `Accepted evidence promoted to ${path.relative(REPO, evidenceDestination)} and ` +
      `${config.publication.final_measurements} actual history rows appended.\n`,
  );
  return true;
}

async function benchLocked(config, configFile = DEFAULT_CONFIG, { trialProfile = null } = {}) {
  const { plan, provider } = await buildPlan(config, { print: true, configFile });
  await verifyDeleteAuthority(config);
  fs.mkdirSync(STATE_ROOT, { recursive: true, mode: 0o700 });
  const runId = `${safeTimestamp()}-${crypto.randomBytes(4).toString("hex")}`;
  const directory = path.join(STATE_ROOT, runId);
  fs.mkdirSync(directory, { recursive: false, mode: 0o700 });
  const shortCandidate = config.source.candidate_commit.slice(0, 8);
  const dropletName = `rustia-bench-${shortCandidate}-${crypto.randomBytes(4).toString("hex")}`;
  const state = {
    __directory: directory,
    schema: "rustia.digitalocean-benchmark-state.v1",
    run_id: runId,
    created_at: new Date().toISOString(),
    context: config.provider.context,
    protocol_mode: config.publication.mode,
    trial_profile: trialProfile,
    droplet_name: dropletName,
    droplet_id: null,
    droplet_destroyed_at: null,
    ssh_key_name: `${dropletName}-key`,
    ssh_key_id: null,
    ssh_key_deleted_at: null,
    private_key_path: path.join(directory, "id_ed25519"),
    public_key_path: path.join(directory, "id_ed25519.pub"),
    known_hosts_path: path.join(directory, "known_hosts"),
    bundle_path: path.join(directory, "rustia.bundle"),
    source_config_path: configFile,
    config_path: path.join(directory, "protocol.json"),
    runner_path: path.join(directory, "run-linux-fixed-benchmark.js"),
    plan,
  };
  saveState(state);

  let primaryError = null;
  let cleanupError = null;
  let downloadDirectory = null;
  try {
    fs.copyFileSync(configFile, state.config_path);
    fs.copyFileSync(REMOTE_RUNNER, state.runner_path);
    if (sha256File(state.config_path) !== plan.source.config_sha256) {
      fail("protocol config changed after planning and before snapshotting");
    }
    if (sha256File(state.runner_path) !== plan.source.remote_runner_sha256) {
      fail("remote runner changed after planning and before snapshotting");
    }
    await run(
      "ssh-keygen",
      ["-q", "-t", "ed25519", "-N", "", "-C", state.ssh_key_name, "-f", state.private_key_path],
      { capture: false, announce: true },
    );
    fs.chmodSync(state.private_key_path, 0o600);
    await run("git", ["bundle", "create", state.bundle_path, "HEAD"], { announce: true });
    await run("git", ["bundle", "verify", state.bundle_path], { capture: false, announce: true });
    state.bundle_sha256 = sha256File(state.bundle_path);
    saveState(state);

    const keyImportStarted = Date.now();
    state.ssh_key_import_attempted_at = new Date(keyImportStarted).toISOString();
    state.ssh_key_import_request_deadline_at = new Date(
      keyImportStarted + API_TIMEOUT_MS,
    ).toISOString();
    saveState(state);
    const importResult = await run(
      "doctl",
      [
        "compute",
        "ssh-key",
        "import",
        state.ssh_key_name,
        "--public-key-file",
        state.public_key_path,
        "--format",
        "ID",
        "--no-header",
        "--context",
        config.provider.context,
        "--http-retry-max",
        "0",
      ],
      { allowFailure: true, announce: true, timeoutMs: API_TIMEOUT_MS },
    );
    state.ssh_key_import_finished_at = new Date().toISOString();
    saveState(state);
    if (importResult.status === 0) {
      state.ssh_key_id = numericId(outputText(importResult), "DigitalOcean SSH key import");
      saveState(state);
    } else {
      const reconciled = await reconcileSshKeyAfterImportAttempt(state, config);
      if (!reconciled) {
        fail(`SSH key import failed: ${importResult.stderr.toString("utf8").trim()}`);
      }
    }

    startBillingWindow(state, config);
    const createTimeout = remainingWorkTimeout(state, 2 * 60 * 1000, "droplet creation");
    const createStarted = Date.now();
    state.droplet_create_attempted_at = new Date(createStarted).toISOString();
    state.droplet_create_request_deadline_at = new Date(
      createStarted + createTimeout,
    ).toISOString();
    saveState(state);
    const createResult = await run(
      "doctl",
      [
        "compute",
        "droplet",
        "create",
        state.droplet_name,
        "--region",
        config.provider.region,
        "--size",
        provider.size.slug,
        "--image",
        String(provider.image.id),
        "--ssh-keys",
        String(state.ssh_key_id),
        "--droplet-agent=false",
        "--format",
        "ID",
        "--no-header",
        "--context",
        config.provider.context,
        "--http-retry-max",
        "0",
      ],
      {
        allowFailure: true,
        announce: true,
        timeoutMs: createTimeout,
      },
    );
    state.droplet_create_finished_at = new Date().toISOString();
    saveState(state);
    if (createResult.status === 0) {
      state.droplet_id = numericId(outputText(createResult), "DigitalOcean droplet create");
      saveState(state);
    } else {
      const reconciled = await reconcileDropletAfterCreateAttempt(state, config);
      if (!reconciled) {
        fail(`Droplet creation failed: ${createResult.stderr.toString("utf8").trim()}`);
      }
    }

    const droplet = await waitForDroplet(state, config);
    verifyDropletMatchesPlan(droplet, plan);
    await deleteSshKey(state, config);
    await waitForSsh(state);
    await run(
      "ssh",
      [...sshOptions(state), `root@${state.public_ipv4}`, "cloud-init", "status", "--wait"],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "cloud-init"),
      },
    );
    await run(
      "ssh",
      [
        ...sshOptions(state),
        `root@${state.public_ipv4}`,
        "env",
        "DEBIAN_FRONTEND=noninteractive",
        "apt-get",
        "update",
        "-qq",
      ],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "apt metadata refresh"),
      },
    );
    await run(
      "ssh",
      [
        ...sshOptions(state),
        `root@${state.public_ipv4}`,
        "env",
        "DEBIAN_FRONTEND=noninteractive",
        "apt-get",
        "install",
        "-y",
        "--no-install-recommends",
        "build-essential",
        "ca-certificates",
        "curl",
        "git",
        "nodejs",
      ],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 15 * 60 * 1000, "prerequisite install"),
      },
    );
    await run(
      "ssh",
      [...sshOptions(state), `root@${state.public_ipv4}`, "mkdir", "-p", "/opt/rustia/incoming"],
      {
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 60 * 1000, "remote directory setup"),
      },
    );

    const providerMetadata = path.join(directory, "provider.json");
    writeJsonAtomic(providerMetadata, safeDropletMetadata(droplet, plan, state));
    await run(
      "scp",
      [
        ...scpOptions(state),
        state.bundle_path,
        state.config_path,
        state.runner_path,
        providerMetadata,
        `root@${state.public_ipv4}:/opt/rustia/incoming/`,
      ],
      {
        capture: false,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "source transfer"),
      },
    );

    const remoteResult = await run(
      "ssh",
      [
        ...sshOptions(state),
        `root@${state.public_ipv4}`,
        "node",
        "/opt/rustia/incoming/run-linux-fixed-benchmark.js",
        "/opt/rustia/incoming/protocol.json",
        "/opt/rustia/incoming/provider.json",
        "/opt/rustia/incoming/rustia.bundle",
      ],
      {
        capture: false,
        allowFailure: true,
        announce: true,
        timeoutMs: remainingWorkTimeout(
          state,
          config.provider.max_runtime_minutes * 60 * 1000,
          "remote benchmark",
        ),
      },
    );
    state.remote_exit_status = remoteResult.status;
    state.remote_timed_out = remoteResult.timedOut;
    saveState(state);

    const archive = path.join(directory, "export.tar.gz");
    const fetchResult = await run(
      "scp",
      [
        ...scpOptions(state),
        `root@${state.public_ipv4}:/opt/rustia/export.tar.gz`,
        archive,
      ],
      {
        capture: false,
        allowFailure: true,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 10 * 60 * 1000, "result retrieval"),
      },
    );
    if (fetchResult.status !== 0 || !fs.existsSync(archive)) {
      fail("remote benchmark did not produce a retrievable export archive");
    }
    state.export_sha256 = sha256File(archive);
    state.export_path = archive;
    saveState(state);
  } catch (error) {
    primaryError = error;
  } finally {
    cleanupInProgress = true;
    try {
      await cleanupOwned(state, configForState(state));
    } catch (error) {
      cleanupError = error;
    } finally {
      cleanupInProgress = false;
    }
  }

  if (cleanupError) {
    const prefix = primaryError ? `${primaryError.message}; additionally, ` : "";
    fail(`${prefix}${cleanupError.message}`);
  }
  if (primaryError) throw primaryError;
  if (!state.droplet_destroyed_at) fail("Droplet destruction was not confirmed");
  throwIfInterrupted();
  downloadDirectory = path.join(directory, "download");
  await extractArchive(state.export_path, downloadDirectory);
  if (config.publication.mode === "diagnostic") {
    const summary = diagnosticSummary(config, state, downloadDirectory);
    if (summary.status === "accepted_and_published") {
      fail("diagnostic protocol unexpectedly reported publication; export retained without promotion");
    }
    if (!["diagnostic_complete", "host_qualification_failed"].includes(summary.status)) {
      fail(`diagnostic trial ended in unexpected status ${summary.status}; export retained`);
    }
    return summary;
  }
  const promoted = await promoteAcceptedResult(config, state, downloadDirectory);
  if (!promoted && state.remote_exit_status !== 0) {
    fail(`remote benchmark ended with status ${state.remote_exit_status}; diagnostics were retained`);
  }
}

async function bench(config, configFile = DEFAULT_CONFIG) {
  if (config.publication.mode !== "acceptance") {
    fail("bench requires an acceptance-mode protocol");
  }
  const lockToken = acquireCoordinatorLock();
  try {
    const incomplete = incompleteStates();
    if (incomplete.length > 0) {
      fail(
        "incomplete DigitalOcean benchmark cleanup exists; run `just bench-do-status` and " +
          `clean up: ${incomplete.map((state) => state.run_id).join(", ")}`,
      );
    }
    throwIfInterrupted();
    return await benchLocked(config, configFile);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function trial(profile) {
  const configFile = trialConfigPath(profile);
  const config = loadConfig(configFile);
  if (config.publication.mode !== "diagnostic") {
    fail(`trial profile ${profile} is not diagnostic-only`);
  }
  const lockToken = acquireCoordinatorLock();
  try {
    const incomplete = incompleteStates();
    if (incomplete.length > 0) {
      fail(
        "incomplete DigitalOcean benchmark cleanup exists; run `just bench-do-status` and " +
          `clean up: ${incomplete.map((state) => state.run_id).join(", ")}`,
      );
    }
    return await benchLocked(config, configFile, { trialProfile: profile });
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function trialMatrix() {
  const lockToken = acquireCoordinatorLock();
  try {
    const summaries = [];
    for (const profile of Object.keys(TRIAL_CONFIGS)) {
      const incomplete = incompleteStates();
      if (incomplete.length > 0) {
        fail(
          "incomplete DigitalOcean benchmark cleanup exists; run `just bench-do-status` and " +
            `clean up: ${incomplete.map((state) => state.run_id).join(", ")}`,
        );
      }
      const configFile = trialConfigPath(profile);
      const config = loadConfig(configFile);
      if (config.publication.mode !== "diagnostic") {
        fail(`trial profile ${profile} is not diagnostic-only`);
      }
      summaries.push(await benchLocked(config, configFile, { trialProfile: profile }));
    }
    const eligible = summaries
      .filter(
        (summary) =>
          summary.eligible_for_acceptance_profile &&
          Number.isFinite(summary.projected_acceptance_compute_charge_usd),
      )
      .sort(
        (left, right) =>
          left.projected_acceptance_compute_charge_usd -
            right.projected_acceptance_compute_charge_usd ||
          left.hourly_price_usd - right.hourly_price_usd,
      );
    const result = {
      trial_matrix: summaries,
      planned_compute_cost_ceiling_usd: Number(
        summaries
          .reduce((sum, summary) => sum + summary.planned_compute_cost_ceiling_usd, 0)
          .toFixed(6),
      ),
      estimated_compute_charge_upper_bound_usd: Number(
        summaries
          .reduce(
            (sum, summary) =>
              sum + (summary.estimated_compute_charge_upper_bound_usd || 0),
            0,
          )
          .toFixed(6),
      ),
      recommendation:
        eligible.length > 0
          ? {
              profile: eligible[0].profile,
              basis: "lowest projected full-acceptance compute charge among qualified trials",
              projected_acceptance_compute_charge_usd:
                eligible[0].projected_acceptance_compute_charge_usd,
              caveat: "acceptance provisions a new shared host and repeats qualification",
            }
          : null,
    };
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return result;
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function authenticate(config) {
  requireExecutables(["doctl"]);
  const lockToken = acquireCoordinatorLock();
  try {
    const contexts = normalizeList(
      parseJsonOutput(
        await run("doctl", ["auth", "list", "--output", "json"]),
        "doctl authentication context list",
      ),
      "doctl authentication context list",
    );
    if (contexts.some((entry) => entry && entry.name === config.provider.context)) {
      process.stdout.write(
        `Removing stored doctl context ${config.provider.context} so a replacement token is prompted.\n`,
      );
      await run("doctl", ["auth", "remove", "--context", config.provider.context], {
        capture: false,
        announce: true,
      });
    }
    process.stdout.write(
      `DigitalOcean token entry for doctl context ${config.provider.context}. ` +
        "The token is read by doctl's prompt and is not a recipe argument.\n",
    );
    await run("doctl", ["auth", "init", "--context", config.provider.context], {
      capture: false,
      announce: true,
    });
    await run(
      "doctl",
      [
        "compute",
        "region",
        "list",
        "--context",
        config.provider.context,
        "--format",
        "Slug",
        "--no-header",
      ],
      { announce: false, timeoutMs: API_TIMEOUT_MS },
    );
    process.stdout.write(`doctl context ${config.provider.context} is authenticated.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function status() {
  const states = listStates();
  if (states.length === 0) {
    process.stdout.write("No locally owned DigitalOcean benchmark runs.\n");
    return;
  }
  const rows = [];
  for (const state of states) {
    let remote = null;
    if (state.droplet_id && !state.droplet_destroyed_at) {
      try {
        const droplet = await getDroplet(state, configForState(state), { allowMissing: true });
        remote = droplet
          ? { id: droplet.id, name: droplet.name, status: droplet.status, public_ipv4: publicIpv4(droplet) }
          : { status: "not returned by get; run destroy to reconcile safely" };
      } catch (error) {
        remote = { error: error.message };
      }
    }
    rows.push({
      run_id: state.run_id,
      created_at: state.created_at,
      protocol_mode: state.protocol_mode || null,
      trial_profile: state.trial_profile || null,
      size_slug: state.plan && state.plan.size ? state.plan.size.slug : null,
      hourly_price_usd:
        state.plan && state.plan.size ? state.plan.size.price_hourly_usd : null,
      droplet_id: state.droplet_id,
      droplet_name: state.droplet_name,
      droplet_create_attempted_at: state.droplet_create_attempted_at || null,
      droplet_absence_confirmed_at: state.droplet_absence_confirmed_at || null,
      destroyed_at: state.droplet_destroyed_at,
      ssh_key_id: state.ssh_key_id,
      ssh_key_name: state.ssh_key_name,
      ssh_key_deleted_at: state.ssh_key_deleted_at,
      cleanup_complete: Boolean(state.droplet_destroyed_at && state.ssh_key_deleted_at),
      billing_deadline_at: state.billing_deadline_at || null,
      remote_status: state.remote_status || null,
      trial_summary: state.trial_summary || null,
      remote,
    });
  }
  process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
}

async function destroy(runId) {
  requireExecutables(["doctl"]);
  const lockToken = acquireCoordinatorLock();
  try {
    const state = stateForRunId(runId);
    cleanupInProgress = true;
    try {
      await cleanupOwned(state, configForState(state));
    } finally {
      cleanupInProgress = false;
    }
    process.stdout.write(`Confirmed cleanup for owned run ${state.run_id}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function selfTest(config) {
  assert.equal(config.publication.mode, "acceptance");
  assert.equal(config.measurement.run_order.length, 8);
  assert.deepEqual(
    config.measurement.run_order.map((entry) => entry.variant),
    ["baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate"],
  );
  const fixture = [
    {
      slug: "c-2",
      description: "CPU-Optimized",
      vcpus: 2,
      memory: 4096,
      disk: 25,
      available: true,
      regions: ["nyc2"],
      price_hourly: 0.0625,
    },
    {
      slug: "s-2vcpu-4gb",
      description: "Basic",
      vcpus: 2,
      memory: 4096,
      disk: 80,
      available: true,
      regions: ["nyc2"],
      price_hourly: 0.0357,
    },
    {
      slug: "s-2vcpu-4gb-intel",
      description: "Basic Intel",
      vcpus: 2,
      memory: 4096,
      disk: 80,
      available: true,
      regions: ["nyc2"],
      price_hourly: 0.04167,
    },
    {
      slug: "s-2vcpu-8gb-160gb-intel",
      description: "Basic Intel",
      vcpus: 2,
      memory: 8192,
      disk: 160,
      available: true,
      regions: ["nyc2"],
      price_hourly: 0.07143,
    },
    {
      slug: "s-4vcpu-8gb-intel",
      description: "Basic Intel",
      vcpus: 4,
      memory: 8192,
      disk: 160,
      available: true,
      regions: ["nyc2"],
      price_hourly: 0.08333,
    },
  ];
  assert.equal(selectSize(fixture, config).slug, "s-4vcpu-8gb-intel");
  const basicTrial = loadConfig(TRIAL_CONFIGS["basic-2c4g"]);
  const premiumTrial = loadConfig(TRIAL_CONFIGS["premium-intel-2c4g"]);
  assert.deepEqual(premiumTrial.provider, config.provider);
  for (const trialConfig of [basicTrial, premiumTrial]) {
    assert.equal(trialConfig.publication.mode, "diagnostic");
    for (const field of ["source", "measurement", "qualification", "host_gate"]) {
      assert.deepEqual(trialConfig[field], config[field]);
    }
    assert.equal(trialConfig.provider.context, config.provider.context);
    assert.equal(trialConfig.provider.region, config.provider.region);
    assert.equal(trialConfig.provider.image_slug, config.provider.image_slug);
    assert.equal(trialConfig.publication.required_verdict, config.publication.required_verdict);
    assert.equal(trialConfig.publication.final_measurements, config.publication.final_measurements);
  }
  assert.equal(selectSize(fixture, basicTrial).slug, "s-2vcpu-4gb");
  assert.equal(selectSize(fixture, premiumTrial).slug, "s-4vcpu-8gb-intel");
  const plannedShape = {
    region: "nyc2",
    image: { id: 123 },
    size: { slug: "s-4vcpu-8gb-intel", vcpus: 4, memory_mib: 8192, disk_gib: 160 },
  };
  const actualShape = {
    region: { slug: "nyc2" },
    image: { id: 123 },
    size_slug: "s-4vcpu-8gb-intel",
    vcpus: 4,
    memory: 8192,
    disk: 160,
  };
  verifyDropletMatchesPlan(actualShape, plannedShape);
  assert.throws(() => verifyDropletMatchesPlan({ ...actualShape, disk: 81 }, plannedShape));
  assert.throws(() => selectSize([...fixture, { ...fixture[4], slug: "ambiguous" }], config));
  assert.throws(() =>
    validateConfig({
      ...config,
      source: { ...config.source, candidate_commit: config.source.baseline_commit },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      publication: { ...config.publication, mode: "unsafe" },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      measurement: {
        ...config.measurement,
        run_order: config.measurement.run_order.map((leg, index) =>
          index === 1 ? { ...leg, variant: "baseline" } : leg,
        ),
      },
    }),
  );
  assert.equal(
    loadConfig(path.join(REPO, "bench/cloud/digitalocean-dense-ed65077.json")).schema,
    "rustia.digitalocean-fixed-benchmark.v2",
  );
  const runner = await run("node", [REMOTE_RUNNER, "self-test"]);
  assert.match(outputText(runner), /self-test passed/);
  process.stdout.write("digitalocean benchmark coordinator self-test passed\n");
}

function usage() {
  process.stdout.write(`Usage:
  node tools/run-digitalocean-benchmark.js auth
  node tools/run-digitalocean-benchmark.js plan
  node tools/run-digitalocean-benchmark.js bench
  node tools/run-digitalocean-benchmark.js trial-plan PROFILE
  node tools/run-digitalocean-benchmark.js trial PROFILE
  node tools/run-digitalocean-benchmark.js trial-matrix
  node tools/run-digitalocean-benchmark.js status
  node tools/run-digitalocean-benchmark.js destroy [RUN-ID]
  node tools/run-digitalocean-benchmark.js unlock
  node tools/run-digitalocean-benchmark.js self-test
`);
}

async function main() {
  const command = process.argv[2];
  if (command === "unlock") {
    unlockStaleCoordinator();
    return;
  }
  if (command === "status") {
    await status();
    return;
  }
  if (command === "destroy") {
    await destroy(process.argv[3] || "");
    return;
  }
  if (command === "trial-plan") {
    const configFile = trialConfigPath(process.argv[3] || "");
    const config = loadConfig(configFile);
    await buildPlan(config, { configFile });
    return;
  }
  if (command === "trial") {
    const summary = await trial(process.argv[3] || "");
    if (!summary.eligible_for_acceptance_profile) process.exitCode = 2;
    return;
  }
  if (command === "trial-matrix") {
    const result = await trialMatrix();
    if (!result.recommendation) process.exitCode = 2;
    return;
  }
  const config = loadConfig();
  switch (command) {
    case "auth":
      await authenticate(config);
      break;
    case "plan":
      await buildPlan(config);
      break;
    case "bench":
      await bench(config);
      break;
    case "self-test":
      await selfTest(config);
      break;
    case "--help":
    case "-h":
    case "help":
      usage();
      break;
    default:
      usage();
      process.exitCode = 2;
  }
}

main().catch((error) => {
  process.stderr.write(`run-digitalocean-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
