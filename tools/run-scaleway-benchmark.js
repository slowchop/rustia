#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const zlib = require("node:zlib");
const { spawn, spawnSync } = require("node:child_process");

const REPO = path.resolve(__dirname, "..");
const DEFAULT_CONFIG = path.join(REPO, "bench/cloud/scaleway-dense-ed65077.json");
const REMOTE_RUNNER = path.join(REPO, "tools/run-linux-fixed-benchmark.js");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/scaleway");
const RUNS_ROOT = path.join(STATE_ROOT, "runs");
const CREDENTIALS_FILE = path.join(STATE_ROOT, "credentials.json");
const COORDINATOR_LOCK = path.join(STATE_ROOT, "coordinator.lock");
const API_HOST = "api.scaleway.com";
const API_TIMEOUT_MS = 30 * 1000;
const CREATE_RECONCILE_TIMEOUT_MS = 2 * 60 * 1000;
const DELETE_CONFIRM_TIMEOUT_MS = 4 * 60 * 1000;
const CLEANUP_RESERVE_MS = 8 * 60 * 1000;
const ACTIVATION_TIMEOUT_MS = 60 * 60 * 1000;
const FORCE_KILL_DELAY_MS = 10 * 1000;
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/;
const FROZEN_CONFIG_SHA256 = "708164ebcb7075fc115bc490355ff3d276b472cb7b0c666c6485e81f2db9ec6a";
const SCALEWAY_ZONES = Object.freeze([
  "nl-ams-1",
  "nl-ams-2",
  "fr-par-1",
  "fr-par-2",
  "pl-waw-2",
  "pl-waw-3",
]);
const MAX_EXPORT_ARCHIVE_BYTES = 256 * 1024 * 1024;
const MAX_EXPORT_UNCOMPRESSED_BYTES = 512 * 1024 * 1024;
const MAX_EXPORT_ENTRIES = 10_000;

let activeChild = null;
let activeRequest = null;
let activeForceKillTimer = null;
let interruptedSignal = null;
let cleanupInProgress = false;

class ScalewayApiError extends Error {
  constructor(message, status = null, response = null) {
    super(message);
    this.name = "ScalewayApiError";
    this.status = status;
    this.response = response;
  }
}

function fail(message) {
  throw new Error(message);
}

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
    if (cleanupInProgress) return;
    if (activeRequest) activeRequest.destroy(new Error(`interrupted by ${signal}`));
    if (activeChild && activeChild.exitCode === null) terminateChild(activeChild, signal);
  });
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

function canonicalJson(value) {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${canonicalJson(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

function validateConfig(value) {
  const root = requireObject(value, "config");
  requireExactKeys(
    root,
    ["schema", "provider", "source", "measurement", "qualification", "host_gate", "publication"],
    "config",
  );
  if (root.schema !== "rustia.scaleway-fixed-benchmark.v1") {
    fail("unsupported Scaleway benchmark config schema");
  }

  const provider = requireObject(root.provider, "config.provider");
  requireExactKeys(
    provider,
    ["zones", "allowed_stock", "os", "size", "max_runtime_minutes"],
    "config.provider",
  );
  if (!Array.isArray(provider.zones) || JSON.stringify(provider.zones) !== JSON.stringify(SCALEWAY_ZONES)) {
    fail(`config.provider.zones must remain ${SCALEWAY_ZONES.join(", ")}`);
  }
  if (!Array.isArray(provider.allowed_stock) || JSON.stringify(provider.allowed_stock) !== '["available","low"]') {
    fail("config.provider.allowed_stock must remain available,low in that priority order");
  }
  requireInteger(provider.max_runtime_minutes, "config.provider.max_runtime_minutes", 1);
  if (provider.max_runtime_minutes !== 110) {
    fail("Scaleway benchmark provider is pinned to a 110-minute lifecycle");
  }
  const operatingSystem = requireObject(provider.os, "config.provider.os");
  requireExactKeys(
    operatingSystem,
    ["name", "version", "cloud_init_required", "license_required"],
    "config.provider.os",
  );
  requireString(operatingSystem.name, "config.provider.os.name");
  requireString(operatingSystem.version, "config.provider.os.version");
  if (
    operatingSystem.name !== "Ubuntu" ||
    operatingSystem.version !== "24.04 LTS (Noble Numbat)" ||
    operatingSystem.cloud_init_required !== true ||
    operatingSystem.license_required !== false
  ) {
    fail("Scaleway benchmark OS must remain Ubuntu 24.04 with cloud-init and no license");
  }
  const size = requireObject(provider.size, "config.provider.size");
  requireExactKeys(
    size,
    [
      "name",
      "physical_cpus",
      "cpu_name",
      "host_cpu_model",
      "cpu_cores",
      "cpu_threads",
      "cpu_frequency_mhz",
      "memory_bytes",
      "memory_type",
      "memory_frequency_mhz",
      "memory_ecc",
      "disk_count",
      "minimum_disk_bytes",
      "disk_type",
      "subscription_period",
      "max_hourly_price_eur",
      "max_setup_fee_eur",
    ],
    "config.provider.size",
  );
  requireString(size.name, "config.provider.size.name");
  requireInteger(size.physical_cpus, "config.provider.size.physical_cpus", 1);
  requireString(size.cpu_name, "config.provider.size.cpu_name");
  requireString(size.host_cpu_model, "config.provider.size.host_cpu_model");
  requireInteger(size.cpu_cores, "config.provider.size.cpu_cores", 1);
  requireInteger(size.cpu_threads, "config.provider.size.cpu_threads", 1);
  requireInteger(size.cpu_frequency_mhz, "config.provider.size.cpu_frequency_mhz", 1);
  requireInteger(size.memory_bytes, "config.provider.size.memory_bytes", 1);
  requireString(size.memory_type, "config.provider.size.memory_type");
  requireInteger(size.memory_frequency_mhz, "config.provider.size.memory_frequency_mhz", 1);
  if (size.memory_ecc !== true) fail("config.provider.size.memory_ecc must be true");
  requireInteger(size.disk_count, "config.provider.size.disk_count", 1);
  requireInteger(size.minimum_disk_bytes, "config.provider.size.minimum_disk_bytes", 1);
  requireString(size.disk_type, "config.provider.size.disk_type");
  requireString(size.subscription_period, "config.provider.size.subscription_period");
  requireNumber(size.max_hourly_price_eur, "config.provider.size.max_hourly_price_eur", 0.001);
  requireNumber(size.max_setup_fee_eur, "config.provider.size.max_setup_fee_eur", 0);
  if (
    size.name !== "EM-A610R-NVMe" ||
    size.physical_cpus !== 1 ||
    size.cpu_name !== "AMD Ryzen PRO 3600" ||
    size.host_cpu_model !== "AMD Ryzen 5 PRO 3600" ||
    size.cpu_cores !== 6 ||
    size.cpu_threads !== 12 ||
    size.cpu_frequency_mhz !== 3600 ||
    size.memory_bytes !== 32 * 1024 ** 3 ||
    size.memory_type !== "DDR4" ||
    size.memory_frequency_mhz !== 3200 ||
    size.disk_count !== 2 ||
    size.minimum_disk_bytes !== 1_000_000_000_000 ||
    size.disk_type !== "NVMe" ||
    size.subscription_period !== "hourly" ||
    size.cpu_threads % size.cpu_cores !== 0 ||
    size.max_hourly_price_eur !== 0.11 ||
    size.max_setup_fee_eur !== 0
  ) {
    fail("Scaleway benchmark size must remain the exact EM-A610R-NVMe profile at <= EUR 0.11/h");
  }
  if (provider.max_runtime_minutes >= 120) {
    fail("config.provider.max_runtime_minutes must remain below Scaleway's third whole billing hour");
  }

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
    fail("config.measurement.workload_path must be repository-relative");
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
    if (seenFiles.has(leg.file)) fail(`duplicate run-order file ${leg.file}`);
    seenFiles.add(leg.file);
    counts[leg.variant] += 1;
  }
  if (counts.baseline !== expectedLegs || counts.candidate !== expectedLegs) {
    fail("run order must contain the declared number of baseline and candidate legs");
  }

  const qualification = requireObject(root.qualification, "config.qualification");
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
  if (gate.minimum_idle_percent > 100) fail("config.host_gate.minimum_idle_percent must be <= 100");
  requireNumber(gate.maximum_load_1m, "config.host_gate.maximum_load_1m", 0);
  requireInteger(gate.maximum_steal_ticks, "config.host_gate.maximum_steal_ticks", 0);
  requireInteger(gate.wait_timeout_seconds, "config.host_gate.wait_timeout_seconds", 1);

  const publication = requireObject(root.publication, "config.publication");
  requireExactKeys(publication, ["mode", "required_verdict", "final_measurements"], "config.publication");
  if (publication.mode !== "acceptance") fail("Scaleway coordinator is acceptance-only");
  if (publication.required_verdict !== "material_regression_excluded") {
    fail("config.publication.required_verdict must be material_regression_excluded");
  }
  requireInteger(publication.final_measurements, "config.publication.final_measurements", 3);
  const frozenHash = sha256(Buffer.from(canonicalJson(root), "utf8"));
  if (frozenHash !== FROZEN_CONFIG_SHA256) {
    fail(
      `Scaleway acceptance config changed from the frozen protocol: ${frozenHash}, ` +
        `expected ${FROZEN_CONFIG_SHA256}`,
    );
  }
  return root;
}

function loadConfig(file = DEFAULT_CONFIG) {
  try {
    return validateConfig(JSON.parse(fs.readFileSync(file, "utf8")));
  } catch (error) {
    fail(`cannot load ${path.relative(REPO, file)}: ${error.message}`);
  }
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
  if (announce) process.stderr.write(`+ ${commandText(command, args)}\n`);
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
        const suffix = timedOut ? " (timed out)" : signal ? ` (signal ${signal})` : "";
        reject(new Error(`${commandText(command, args)} failed with status ${result.status}${suffix}${detail ? `: ${detail}` : ""}`));
        return;
      }
      resolve(result);
    });
    if (input !== null) child.stdin.end(input);
  });
}

function outputText(result) {
  return result.stdout.toString("utf8").trim();
}

function findExecutable(name) {
  for (const directory of (process.env.PATH || "").split(path.delimiter)) {
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
  if (missing.length > 0) fail(`missing required commands: ${missing.join(", ")}`);
}

function apiErrorDetail(response) {
  if (!response) return "no response body";
  if (typeof response.error === "string" && response.error.length > 0) {
    return response.error.slice(0, 2000);
  }
  if (typeof response.message === "string" && response.message.length > 0) {
    return response.message.slice(0, 2000);
  }
  return "unrecognized API error response";
}

function createOutcomeForError(error) {
  const definitivelyRejectedStatuses = new Set([
    400, 401, 402, 403, 404, 405, 406, 410, 411, 412, 413, 414, 415, 416, 417, 422,
  ]);
  return error && definitivelyRejectedStatuses.has(error.status) ? "definitively_rejected" : "ambiguous";
}

function bareMetalFromCreateResponse(response) {
  return requireApiObject(response, "Scaleway create response");
}

async function apiRequest(secretKey, method, apiPath, options = {}) {
  throwIfInterrupted();
  if (!/^\/(?:baremetal\/v1\/zones\/[a-z0-9-]+|iam\/v1alpha1\/ssh-keys)(?:[/?]|$)/.test(apiPath)) {
    fail("Scaleway API path must stay under the Elastic Metal or IAM SSH-key endpoints");
  }
  const { body = null, acceptedStatuses = null, timeoutMs = API_TIMEOUT_MS } = options;
  if (typeof secretKey !== "string" || secretKey.length === 0 || /\s/.test(secretKey)) {
    fail("Scaleway secret key is unavailable or malformed");
  }
  const requestBody = body === null ? null : Buffer.from(JSON.stringify(body));
  return await new Promise((resolve, reject) => {
    let settled = false;
    const finish = (callback, value) => {
      if (settled) return;
      settled = true;
      if (activeRequest === request) activeRequest = null;
      callback(value);
    };
    const request = https.request(
      {
        hostname: API_HOST,
        port: 443,
        method,
        path: apiPath,
        headers: {
          Accept: "application/json",
          "X-Auth-Token": secretKey,
          "User-Agent": "rustia-scaleway-benchmark/1",
          ...(requestBody === null
            ? {}
            : { "Content-Type": "application/json", "Content-Length": requestBody.length }),
        },
      },
      (response) => {
        const chunks = [];
        let size = 0;
        response.on("data", (chunk) => {
          size += chunk.length;
          if (size > 32 * 1024 * 1024) {
            response.destroy(new Error("Scaleway API response exceeded 32 MiB"));
            return;
          }
          chunks.push(Buffer.from(chunk));
        });
        response.on("error", (error) => finish(reject, new ScalewayApiError(`Scaleway API response failed: ${error.message}`)));
        response.on("end", () => {
          const bytes = Buffer.concat(chunks);
          let data = null;
          if (bytes.length > 0) {
            try {
              data = JSON.parse(bytes.toString("utf8"));
            } catch (error) {
              finish(reject, new ScalewayApiError(`Scaleway API returned invalid JSON: ${error.message}`, response.statusCode));
              return;
            }
          }
          const status = response.statusCode || 0;
          const accepted = acceptedStatuses
            ? acceptedStatuses.includes(status)
            : status >= 200 && status < 300;
          if (!accepted) {
            const detail =
              method === "POST" && /\/baremetal\/v1\/zones\/[^/]+\/servers$/.test(apiPath)
                ? "response detail suppressed because the request contains private cloud-init material"
                : apiErrorDetail(data);
            finish(reject, new ScalewayApiError(`Scaleway API ${method} ${apiPath} returned ${status}: ${detail}`, status, data));
            return;
          }
          finish(resolve, { status, data, headers: response.headers });
        });
      },
    );
    activeRequest = request;
    request.setTimeout(timeoutMs, () => request.destroy(new Error("Scaleway API request timed out")));
    request.on("error", (error) => {
      finish(reject, new ScalewayApiError(`Scaleway API ${method} ${apiPath} failed: ${error.message}`));
    });
    if (requestBody !== null) request.write(requestBody);
    request.end();
  });
}

function writeBytesAtomic(file, bytes, mode = 0o600) {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomBytes(6).toString("hex")}`;
  fs.writeFileSync(temporary, bytes, { flag: "wx", mode });
  fs.chmodSync(temporary, mode);
  fs.renameSync(temporary, file);
}

function writeJsonAtomic(file, value, mode = 0o600) {
  writeBytesAtomic(file, Buffer.from(`${JSON.stringify(value, null, 2)}\n`), mode);
}

function validateCredentials(value) {
  const credentials = requireObject(value, "Scaleway credentials");
  requireExactKeys(credentials, ["schema", "project_id", "secret_key"], "Scaleway credentials");
  if (credentials.schema !== "rustia.scaleway-credentials.v1") {
    fail("unsupported Scaleway credentials schema");
  }
  if (!UUID.test(requireString(credentials.project_id, "Scaleway credentials.project_id"))) {
    fail("Scaleway project ID must be a lowercase UUID");
  }
  const secretKey = requireString(credentials.secret_key, "Scaleway credentials.secret_key");
  if (/\s/.test(secretKey)) fail("Scaleway secret key cannot contain whitespace");
  return credentials;
}

function readStoredCredentials({ allowMissing = false } = {}) {
  let stat;
  try {
    stat = fs.lstatSync(CREDENTIALS_FILE);
  } catch (error) {
    if (allowMissing && error && error.code === "ENOENT") return null;
    fail("Scaleway credentials are not configured; run the auth command from an interactive terminal");
  }
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`refusing non-regular credentials file ${CREDENTIALS_FILE}`);
  if ((stat.mode & 0o077) !== 0) {
    fail(`Scaleway credentials file permissions must be 0600: ${CREDENTIALS_FILE}`);
  }
  try {
    return validateCredentials(JSON.parse(fs.readFileSync(CREDENTIALS_FILE, "utf8")));
  } catch (error) {
    fail(`Scaleway credentials file is malformed: ${error.message}`);
  }
}

async function readTtyLine(prompt, { hidden = false } = {}) {
  if (!process.stdin.isTTY || !process.stderr.isTTY || typeof process.stdin.setRawMode !== "function") {
    fail("auth requires an interactive TTY; credentials are never accepted in argv or environment variables");
  }
  process.stderr.write(prompt);
  const input = process.stdin;
  const wasRaw = Boolean(input.isRaw);
  input.setEncoding("utf8");
  input.setRawMode(true);
  input.resume();
  return await new Promise((resolve, reject) => {
    let value = "";
    const signals = ["SIGINT", "SIGTERM", "SIGHUP"];
    const signalHandlers = new Map();
    const cleanup = () => {
      input.off("data", onData);
      input.off("end", onEnd);
      input.off("error", onError);
      for (const signal of signals) process.off(signal, signalHandlers.get(signal));
      try {
        input.setRawMode(wasRaw);
      } catch {
        // The terminal may already have disappeared.
      }
      input.pause();
      process.stderr.write("\n");
    };
    const abort = (message) => {
      cleanup();
      reject(new Error(message));
    };
    const onEnd = () => abort("credential entry ended before a value was submitted");
    const onError = (error) => abort(`credential entry failed: ${error.message}`);
    for (const signal of signals) {
      signalHandlers.set(signal, () => abort(`credential entry interrupted by ${signal}`));
    }
    const onData = (chunk) => {
      for (const character of chunk) {
        if (character === "\r" || character === "\n") {
          cleanup();
          if (!value || /\s/.test(value)) reject(new Error("credential cannot be empty or contain whitespace"));
          else resolve(value);
          return;
        }
        if (character === "\u0003") {
          abort("credential entry interrupted");
          return;
        }
        if (character === "\u007f" || character === "\b") {
          if (value.length > 0) {
            value = value.slice(0, -1);
            if (!hidden) process.stderr.write("\b \b");
          }
        } else if (character >= " ") {
          value += character;
          if (!hidden) process.stderr.write(character);
        }
      }
    };
    input.on("data", onData);
    input.once("end", onEnd);
    input.once("error", onError);
    for (const signal of signals) process.once(signal, signalHandlers.get(signal));
  });
}

function safeTimestamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z");
}

function roundedHourlyCharge(hourlyPrice, seconds) {
  requireNumber(hourlyPrice, "hourly price", 0);
  requireNumber(seconds, "billable seconds", 0);
  if (seconds === 0) return 0;
  return Number((Math.ceil(seconds / 3600) * hourlyPrice).toFixed(6));
}

function requireApiObject(response, description) {
  return requireObject(response && response.data, description);
}


function integerFromApi(value, description, minimum = 0) {
  const parsed = typeof value === "string" && /^\d+$/.test(value) ? Number(value) : value;
  return requireInteger(parsed, description, minimum);
}

function queryPath(basePath, entries) {
  const query = new URLSearchParams();
  for (const [name, rawValue] of entries) {
    const values = Array.isArray(rawValue) ? rawValue : [rawValue];
    for (const value of values) {
      if (value !== undefined && value !== null && value !== "") query.append(name, String(value));
    }
  }
  return `${basePath}?${query.toString()}`;
}

async function listPaginated(secretKey, basePath, collection, filters, request = apiRequest) {
  const all = [];
  const seenPages = new Set();
  let page = 1;
  let declaredTotal = null;
  while (true) {
    if (seenPages.has(page)) fail(`Scaleway ${collection} pagination repeated page ${page}`);
    seenPages.add(page);
    const apiPath = queryPath(basePath, [["page", page], ["page_size", 100], ...filters]);
    const response = await request(secretKey, "GET", apiPath, { acceptedStatuses: [200] });
    const envelope = requireApiObject(response, `Scaleway ${collection} list response`);
    const rows = envelope[collection];
    if (!Array.isArray(rows)) fail(`Scaleway ${collection} list response.${collection} must be an array`);
    const total = integerFromApi(envelope.total_count, `Scaleway ${collection} total_count`, 0);
    if (declaredTotal === null) declaredTotal = total;
    if (total !== declaredTotal) fail(`Scaleway ${collection} total_count changed during pagination`);
    all.push(...rows);
    if (all.length > declaredTotal) fail(`Scaleway ${collection} pagination exceeded its declared total`);
    if (all.length === declaredTotal) break;
    if (rows.length === 0) fail(`Scaleway ${collection} pagination ended before its declared total`);
    page += 1;
  }
  return all;
}

async function listScaleways(credentials, request = apiRequest, { zones = SCALEWAY_ZONES } = {}) {
  const all = [];
  for (const zone of zones) {
    const rows = await listPaginated(
      credentials.secret_key,
      `/baremetal/v1/zones/${zone}/servers`,
      "servers",
      [["order_by", "created_at_desc"], ["project_id", credentials.project_id]],
      request,
    );
    for (const [index, row] of rows.entries()) {
      const server = requireObject(row, `Scaleway ${zone} server row ${index}`);
      if (!UUID.test(server.id)) fail(`Scaleway ${zone} server row ${index} has a malformed ID`);
      if (server.zone !== zone || server.project_id !== credentials.project_id) {
        fail(`Scaleway ${zone} server list escaped the requested zone or project`);
      }
      all.push(server);
    }
  }
  const ids = new Set();
  for (const server of all) {
    if (ids.has(server.id)) fail(`Scaleway server list repeated ID ${server.id}`);
    ids.add(server.id);
  }
  return all;
}

async function listSshKeys(credentials, request = apiRequest) {
  const rows = await listPaginated(
    credentials.secret_key,
    "/iam/v1alpha1/ssh-keys",
    "ssh_keys",
    [["order_by", "created_at_desc"], ["project_id", credentials.project_id]],
    request,
  );
  const ids = new Set();
  for (const [index, row] of rows.entries()) {
    const key = requireObject(row, `Scaleway SSH-key row ${index}`);
    if (!UUID.test(key.id)) fail(`Scaleway SSH-key row ${index} has a malformed ID`);
    if (key.project_id !== credentials.project_id) fail(`Scaleway SSH-key row ${index} escaped the requested project`);
    if (ids.has(key.id)) fail(`Scaleway SSH-key list repeated ID ${key.id}`);
    ids.add(key.id);
  }
  return rows;
}

function eurMoney(value, description) {
  const money = requireObject(value, description);
  if (money.currency_code !== "EUR") fail(`${description} must be denominated in EUR`);
  const units = integerFromApi(money.units, `${description}.units`, 0);
  const nanos = integerFromApi(money.nanos, `${description}.nanos`, 0);
  if (nanos >= 1_000_000_000) fail(`${description}.nanos must be below one EUR`);
  return units + nanos / 1_000_000_000;
}

function catalogCpuNameMatches(actual, wanted) {
  if (typeof actual !== "string") return false;
  return new Set([
    wanted.cpu_name,
    `${wanted.cpu_name} 6-Core Processor`,
    wanted.host_cpu_model,
    `${wanted.host_cpu_model} 6-Core Processor`,
  ]).has(actual);
}

function validateOffer(config, zone, rawOffer) {
  const offer = requireObject(rawOffer, `Scaleway ${zone} offer`);
  const wanted = config.provider.size;
  if (!UUID.test(offer.id)) fail(`Scaleway ${zone} offer has a malformed ID`);
  const checks = [
    ["name", offer.name, wanted.name],
    ["zone", offer.zone, zone],
    ["subscription period", offer.subscription_period, wanted.subscription_period],
    ["enabled", offer.enable, true],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`Scaleway ${zone} offer ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if (!["empty", "low", "available"].includes(offer.stock)) {
    fail(`Scaleway ${zone} offer returned unknown stock ${JSON.stringify(offer.stock)}`);
  }
  if (!Array.isArray(offer.cpus) || offer.cpus.length !== wanted.physical_cpus) {
    fail(`Scaleway ${zone} offer must expose exactly ${wanted.physical_cpus} physical CPU`);
  }
  const cpu = requireObject(offer.cpus[0], `Scaleway ${zone} offer CPU`);
  if (
    !catalogCpuNameMatches(requireString(cpu.name, `Scaleway ${zone} CPU name`), wanted) ||
    integerFromApi(cpu.core_count, `Scaleway ${zone} CPU core count`, 1) !== wanted.cpu_cores ||
    integerFromApi(cpu.thread_count, `Scaleway ${zone} CPU thread count`, 1) !== wanted.cpu_threads ||
    integerFromApi(cpu.frequency, `Scaleway ${zone} CPU frequency`, 1) !== wanted.cpu_frequency_mhz
  ) {
    fail(`Scaleway ${zone} offer CPU no longer exactly describes ${wanted.cpu_name}`);
  }
  if (!Array.isArray(offer.memories) || offer.memories.length === 0) {
    fail(`Scaleway ${zone} offer has no memory specification`);
  }
  let memoryBytes = 0;
  for (const [index, rawMemory] of offer.memories.entries()) {
    const memory = requireObject(rawMemory, `Scaleway ${zone} memory ${index}`);
    memoryBytes += integerFromApi(memory.capacity, `Scaleway ${zone} memory ${index} capacity`, 1);
    if (
      !requireString(memory.type, `Scaleway ${zone} memory ${index} type`).toUpperCase().includes(wanted.memory_type) ||
      integerFromApi(memory.frequency, `Scaleway ${zone} memory ${index} frequency`, 1) !== wanted.memory_frequency_mhz ||
      memory.is_ecc !== wanted.memory_ecc
    ) {
      fail(`Scaleway ${zone} offer memory ${index} no longer matches the frozen profile`);
    }
  }
  if (memoryBytes !== wanted.memory_bytes) {
    fail(`Scaleway ${zone} offer memory totals ${memoryBytes} bytes, expected ${wanted.memory_bytes}`);
  }
  if (!Array.isArray(offer.disks) || offer.disks.length !== wanted.disk_count) {
    fail(`Scaleway ${zone} offer must expose exactly ${wanted.disk_count} disks`);
  }
  for (const [index, rawDisk] of offer.disks.entries()) {
    const disk = requireObject(rawDisk, `Scaleway ${zone} disk ${index}`);
    if (
      integerFromApi(disk.capacity, `Scaleway ${zone} disk ${index} capacity`, 1) < wanted.minimum_disk_bytes ||
      requireString(disk.type, `Scaleway ${zone} disk ${index} type`).toLowerCase() !== wanted.disk_type.toLowerCase()
    ) {
      fail(`Scaleway ${zone} offer disk ${index} no longer matches the frozen profile`);
    }
  }
  const hourlyPrice = eurMoney(offer.price_per_hour, `Scaleway ${zone} hourly price`);
  const setupFee = eurMoney(offer.fee, `Scaleway ${zone} setup fee`);
  if (hourlyPrice > wanted.max_hourly_price_eur || setupFee > wanted.max_setup_fee_eur) {
    fail(`Scaleway ${zone} offer costs EUR ${hourlyPrice}/h plus EUR ${setupFee}, above the frozen ceiling`);
  }
  if (!Array.isArray(offer.gpus) || offer.gpus.length !== 0) {
    fail(`Scaleway ${zone} benchmark offer unexpectedly includes a GPU`);
  }
  return { offer, hourlyPrice, setupFee };
}

function validateOperatingSystem(config, zone, offer, rawOperatingSystem) {
  const operatingSystem = requireObject(rawOperatingSystem, `Scaleway ${zone} OS`);
  if (!UUID.test(operatingSystem.id)) fail(`Scaleway ${zone} OS has a malformed ID`);
  const wanted = config.provider.os;
  const checks = [
    ["name", operatingSystem.name, wanted.name],
    ["version", operatingSystem.version, wanted.version],
    ["zone", operatingSystem.zone, zone],
    ["enabled", operatingSystem.enabled, true],
    ["allowed", operatingSystem.allowed, true],
    ["license requirement", operatingSystem.license_required, wanted.license_required],
    ["cloud-init support", operatingSystem.cloud_init_supported, wanted.cloud_init_required],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`Scaleway ${zone} OS ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if (!Array.isArray(offer.incompatible_os_ids) || offer.incompatible_os_ids.includes(operatingSystem.id)) {
    fail(`Scaleway ${zone} offer marks the selected Ubuntu OS as incompatible`);
  }
  return operatingSystem;
}

async function resolveProviderPlan(credentials, config, request = apiRequest) {
  const candidates = [];
  for (const zone of config.provider.zones) {
    const offers = await listPaginated(
      credentials.secret_key,
      `/baremetal/v1/zones/${zone}/offers`,
      "offers",
      [["subscription_period", "hourly"], ["name", config.provider.size.name]],
      request,
    );
    const matches = offers.filter((offer) => offer && offer.name === config.provider.size.name);
    if (matches.length > 1) fail(`Scaleway ${zone} returned ${matches.length} exact benchmark offers`);
    if (matches.length === 0) continue;
    const validated = validateOffer(config, zone, matches[0]);
    if (!config.provider.allowed_stock.includes(validated.offer.stock)) continue;
    const operatingSystems = await listPaginated(
      credentials.secret_key,
      `/baremetal/v1/zones/${zone}/os`,
      "os",
      [["offer_id", validated.offer.id]],
      request,
    );
    const osMatches = operatingSystems.filter(
      (entry) => entry && entry.name === config.provider.os.name && entry.version === config.provider.os.version,
    );
    if (osMatches.length !== 1) {
      fail(`Scaleway ${zone} returned ${osMatches.length} exact Ubuntu 24.04 OS rows`);
    }
    candidates.push({
      zone,
      offer: validated.offer,
      operatingSystem: validateOperatingSystem(config, zone, validated.offer, osMatches[0]),
      hourlyPrice: validated.hourlyPrice,
      setupFee: validated.setupFee,
    });
  }
  for (const stock of config.provider.allowed_stock) {
    const selected = candidates.find((candidate) => candidate.offer.stock === stock);
    if (selected) return selected;
  }
  fail(`no exact ${config.provider.size.name} host has allowed stock in the pinned Scaleway zones`);
}

async function verifyDeleteAuthority(credentials, provider, request = apiRequest, { announce = true } = {}) {
  const sentinel = "ffffffff-ffff-4fff-bfff-ffffffffffff";
  if ((await listScaleways(credentials, request)).some((entry) => entry && entry.id === sentinel)) {
    fail(`refusing absent-ID probe because Scaleway ${sentinel} exists on the project`);
  }
  if ((await listSshKeys(credentials, request)).some((entry) => entry && entry.id === sentinel)) {
    fail(`refusing absent-ID probe because Scaleway SSH key ${sentinel} exists on the project`);
  }
  let serverResponse;
  try {
    serverResponse = await request(
      credentials.secret_key,
      "DELETE",
      `/baremetal/v1/zones/${provider.zone}/servers/${sentinel}`,
      { acceptedStatuses: [404] },
    );
  } catch (error) {
    fail(`Scaleway absent-server delete probe failed; require ElasticMetalFullAccess (${error.message})`);
  }
  if (serverResponse.status !== 404) fail("absent-ID Scaleway server delete probe did not return 404");
  let keyResponse;
  try {
    keyResponse = await request(credentials.secret_key, "DELETE", `/iam/v1alpha1/ssh-keys/${sentinel}`, {
      acceptedStatuses: [404],
    });
  } catch (error) {
    fail(`Scaleway absent-SSH-key delete probe failed; require SSHKeysFullAccess (${error.message})`);
  }
  if (keyResponse.status !== 404) fail("absent-ID Scaleway SSH-key delete probe did not return 404");
  if (announce) {
    process.stdout.write("Pre-billing absent-ID Elastic Metal and IAM SSH-key delete probes returned 404.\n");
  }
}

async function inspectIamPermissions(credentials, request = apiRequest, { announce = true } = {}) {
  await listScaleways(credentials, request);
  await listSshKeys(credentials, request);
  if (announce) {
    process.stdout.write(
      "Scaleway Elastic Metal and SSH-key reads passed; the application must retain " +
        "ElasticMetalFullAccess and SSHKeysFullAccess.\n",
    );
  }
}

async function gitBlob(commit, file) {
  return (await run("git", ["show", `${commit}:${file}`])).stdout;
}

async function verifySource(config, configFile = DEFAULT_CONFIG) {
  const relativeConfig = path.relative(REPO, configFile);
  if (relativeConfig.startsWith("..") || path.isAbsolute(relativeConfig)) {
    fail("Scaleway benchmark config must be inside the repository");
  }
  const trackedStatus = outputText(
    await run("git", ["status", "--porcelain=v1", "--untracked-files=no"]),
  );
  if (trackedStatus.length > 0) fail("tracked worktree changes must be committed before a cloud benchmark");
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    await run("git", ["cat-file", "-e", `${config.source[field]}^{commit}`]);
  }
  for (const file of [
    "tools/run-scaleway-benchmark.js",
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
    if (actual !== wanted) fail(`${name} SHA-256 is ${actual}, expected ${wanted}`);
  }
  return {
    orchestration_commit: outputText(await run("git", ["rev-parse", "HEAD"])),
    config_path: relativeConfig,
    config_sha256: sha256File(configFile),
    coordinator_sha256: sha256File(__filename),
    remote_runner_sha256: sha256File(REMOTE_RUNNER),
    verified_source_hashes: Object.fromEntries(checks.map(([name, actual]) => [name, actual])),
  };
}

function sanitizedProviderPlan(config, provider, source) {
  const cpu = provider.offer.cpus[0];
  const memoryBytes = provider.offer.memories.reduce(
    (total, memory) => total + integerFromApi(memory.capacity, "Scaleway catalog memory capacity", 1),
    0,
  );
  return {
    schema: config.schema,
    provider: "scaleway",
    protocol_mode: "acceptance",
    zone: provider.zone,
    os: {
      id: provider.operatingSystem.id,
      name: provider.operatingSystem.name,
      version: provider.operatingSystem.version,
      cloud_init_supported: provider.operatingSystem.cloud_init_supported,
      cloud_init_version: provider.operatingSystem.cloud_init_version,
    },
    size: {
      id: provider.offer.id,
      name: provider.offer.name,
      resource_type: "elastic_metal",
      stock: provider.offer.stock,
      physical_cpus: provider.offer.cpus.length,
      vcpus: integerFromApi(cpu.thread_count, "Scaleway catalog CPU threads", 1),
      cpu_cores: integerFromApi(cpu.core_count, "Scaleway catalog CPU cores", 1),
      cpu_threads: integerFromApi(cpu.thread_count, "Scaleway catalog CPU threads", 1),
      cpu_model: cpu.name,
      cpu_mhz: integerFromApi(cpu.frequency, "Scaleway catalog CPU frequency", 1),
      memory_bytes: memoryBytes,
      disks: provider.offer.disks.map((disk) => ({
        capacity_bytes: integerFromApi(disk.capacity, "Scaleway catalog disk capacity", 1),
        type: disk.type,
      })),
      price_hourly_eur: provider.hourlyPrice,
      setup_fee_eur: provider.setupFee,
      subscription_period: provider.offer.subscription_period,
    },
    billable_lifetime_limit_minutes: config.provider.max_runtime_minutes,
    billing_increment_minutes: 60,
    planned_compute_cost_ceiling_eur: roundedHourlyCharge(
      provider.hourlyPrice,
      config.provider.max_runtime_minutes * 60,
    ),
    cleanup_reserve_minutes: CLEANUP_RESERVE_MS / 60_000,
    billing_caveat:
      "Scaleway hourly Elastic Metal is conservatively rounded to each started hour; deletion, not shutdown, stops accrual, and an API outage can delay confirmed deletion",
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
  requireExecutables(["git", "node", "scp", "ssh", "ssh-keygen", "tar"]);
  const credentials = readStoredCredentials();
  await inspectIamPermissions(credentials);
  const source = await verifySource(config, configFile);
  const provider = await resolveProviderPlan(credentials, config);
  const plan = sanitizedProviderPlan(config, provider, source);
  if (print) process.stdout.write(`${JSON.stringify(plan, null, 2)}\n`);
  return { credentials, plan, provider, source };
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
  const result = spawnSync("ps", ["-p", String(pid), "-o", "lstart=", "-o", "command="], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  });
  if (result.status !== 0) return null;
  const fingerprint = String(result.stdout || "").trim();
  return fingerprint || null;
}

function lockOwnerIsLive(owner) {
  if (processExists(owner.pid) === false) return false;
  const current = processFingerprint(owner.pid);
  if (owner.process_fingerprint && current) return owner.process_fingerprint === current;
  return true;
}

function acquireCoordinatorLock() {
  fs.mkdirSync(STATE_ROOT, { recursive: true, mode: 0o700 });
  fs.chmodSync(STATE_ROOT, 0o700);
  const token = crypto.randomBytes(16).toString("hex");
  const pending = `${COORDINATOR_LOCK}.pending-${token}`;
  fs.mkdirSync(pending, { mode: 0o700 });
  writeJsonAtomic(path.join(pending, "owner.json"), {
    schema: "rustia.scaleway-benchmark-lock.v1",
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
      fail(`Scaleway coordinator lock exists without readable ownership at ${COORDINATOR_LOCK}`);
    }
    if (!lockOwnerIsLive(owner)) {
      fail(`stale Scaleway coordinator lock for PID ${owner.pid}; run the unlock command`);
    }
    fail(`Scaleway benchmark coordinator PID ${owner.pid} is already active`);
  }
  return token;
}

function releaseCoordinatorLock(token) {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot verify Scaleway coordinator lock ownership: ${error.message}`);
  }
  if (owner.pid !== process.pid || owner.token !== token) {
    fail("refusing to remove a Scaleway coordinator lock owned by another process");
  }
  fs.rmSync(COORDINATOR_LOCK, { recursive: true });
}

function unlockStaleCoordinator() {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot read Scaleway coordinator lock ownership: ${error.message}`);
  }
  if (lockOwnerIsLive(owner)) fail(`refusing to unlock live Scaleway coordinator PID ${owner.pid}`);
  const quarantine = `${COORDINATOR_LOCK}.stale-${crypto.randomBytes(8).toString("hex")}`;
  fs.renameSync(COORDINATOR_LOCK, quarantine);
  const moved = JSON.parse(fs.readFileSync(path.join(quarantine, "owner.json"), "utf8"));
  if (moved.pid !== owner.pid || moved.token !== owner.token) {
    fail(`stale lock ownership changed during quarantine; retained at ${quarantine}`);
  }
  fs.rmSync(quarantine, { recursive: true });
  process.stdout.write(`Removed stale Scaleway coordinator lock for PID ${owner.pid}.\n`);
}

function readState(directory) {
  const state = JSON.parse(fs.readFileSync(path.join(directory, "state.json"), "utf8"));
  state.__directory = directory;
  return state;
}

function saveState(state) {
  const copy = { ...state };
  delete copy.__directory;
  writeJsonAtomic(path.join(state.__directory, "state.json"), copy);
}

function listStates() {
  if (!fs.existsSync(RUNS_ROOT)) return [];
  return fs
    .readdirSync(RUNS_ROOT, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && fs.existsSync(path.join(RUNS_ROOT, entry.name, "state.json")))
    .map((entry) => readState(path.join(RUNS_ROOT, entry.name)))
    .sort((left, right) => String(left.created_at).localeCompare(String(right.created_at)));
}

function incompleteStates() {
  return listStates().filter((state) => !state.scaleway_destroyed_at || !state.ssh_key_destroyed_at);
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
        ? "no active owned Scaleway benchmark run exists"
        : `more than one active run exists; pass one of: ${states.map((state) => state.run_id).join(", ")}`,
    );
  }
  return states[0];
}

function startBillingWindow(state, config) {
  const started = Date.now();
  state.billing_window_started_at = new Date(started).toISOString();
  state.billing_deadline_at = new Date(started + config.provider.max_runtime_minutes * 60 * 1000).toISOString();
  saveState(state);
}

function remainingWorkTimeout(state, requestedMs, label) {
  const deadline = Date.parse(state.billing_deadline_at || "");
  if (!Number.isFinite(deadline)) fail("owned run has no valid billing deadline");
  const remaining = deadline - Date.now() - CLEANUP_RESERVE_MS;
  if (remaining <= 0) fail(`${label}: billable lifetime reached the reserved cleanup window`);
  return Math.max(1, Math.min(requestedMs, remaining));
}

function requireCredentialsForState(state, credentials) {
  const stateProject = requireString(state && state.project_id, "owned run project ID");
  const credentialsProject = requireString(credentials && credentials.project_id, "Scaleway credentials project ID");
  if (!UUID.test(stateProject) || !UUID.test(credentialsProject) || stateProject !== credentialsProject) {
    fail(
      `stored Scaleway credentials project ${JSON.stringify(credentialsProject)} does not match ` +
        `owned run project ${JSON.stringify(stateProject)}; refusing remote reconciliation`,
    );
  }
}


function creationTimeMatches(value, attemptedAt, finishedAt, requestDeadlineAt) {
  const attempted = Date.parse(attemptedAt || "");
  const created = Date.parse(value || "");
  const finished = Date.parse(finishedAt || "");
  const requestDeadline = Date.parse(requestDeadlineAt || "");
  const uncertaintyEnd = Math.max(
    attempted,
    Number.isFinite(finished) ? finished : attempted,
    Number.isFinite(requestDeadline) ? requestDeadline : attempted,
  );
  return (
    Number.isFinite(attempted) &&
    Number.isFinite(created) &&
    created >= attempted - 60_000 &&
    created <= uncertaintyEnd + CREATE_RECONCILE_TIMEOUT_MS
  );
}

function scalewayCreationTimeMatches(server, state) {
  return creationTimeMatches(
    server && server.created_at,
    state.scaleway_create_attempted_at,
    state.scaleway_create_finished_at,
    state.scaleway_create_request_deadline_at,
  );
}

function scalewayHasOwnershipTag(server, state) {
  return (
    Array.isArray(server && server.tags) &&
    server.tags.filter((tag) => tag === state.scaleway_ownership_tag).length === 1
  );
}

function ownedScalewayShapeMatches(server, state) {
  return Boolean(
    server &&
      UUID.test(server.id) &&
      server.project_id === state.project_id &&
      server.name === state.scaleway_label &&
      server.description === state.scaleway_description &&
      server.zone === state.plan.zone &&
      server.offer_id === state.plan.size.id &&
      scalewayCreationTimeMatches(server, state),
  );
}

function exactOwnedScaleway(server, state) {
  return ownedScalewayShapeMatches(server, state) && scalewayHasOwnershipTag(server, state);
}

function directCreateScalewayMatches(server, state, { requireJournaledId = true } = {}) {
  return Boolean(
    server &&
      UUID.test(server.id) &&
      (!requireJournaledId || server.id === state.scaleway_id) &&
      server.project_id === state.project_id &&
      server.name === state.scaleway_label &&
      server.zone === state.plan.zone &&
      server.description === state.scaleway_description &&
      server.offer_id === state.plan.size.id &&
      scalewayHasOwnershipTag(server, state),
  );
}

function cleanupOwnedScalewayMatches(server, state) {
  if (!server || server.id !== state.scaleway_id || server.name !== state.scaleway_label) return false;
  if (exactOwnedScaleway(server, state)) return true;
  return Boolean(
    state.scaleway_id_source === "create_response" &&
      server.project_id === state.project_id &&
      server.zone === state.plan.zone,
  );
}

function ownershipRelatedScaleways(servers, state) {
  if (!Array.isArray(servers)) fail("Scaleway server list must be an array");
  return servers.filter(
    (server) =>
      server &&
      (server.name === state.scaleway_label ||
        (Array.isArray(server.tags) && server.tags.includes(state.scaleway_ownership_tag))),
  );
}

async function reconcileScalewayByLabel(state, credentials, request = apiRequest) {
  requireCredentialsForState(state, credentials);
  const related = ownershipRelatedScaleways(await listScaleways(credentials, request), state);
  const matches = related.filter((server) => exactOwnedScaleway(server, state));
  if (related.length !== matches.length || matches.length > 1) {
    fail(
      `refusing to adopt or delete Scaleway ownership set: ${related.length} name/tag related, ` +
        `${matches.length} exact`,
    );
  }
  if (matches.length === 1) {
    state.scaleway_id = matches[0].id;
    state.scaleway_id_source = "list_reconciliation";
    state.scaleway_positive_observed_at = state.scaleway_positive_observed_at || new Date().toISOString();
    if (state.scaleway_create_attempted_at) state.scaleway_create_outcome = "resource_identified";
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileScalewayAfterCreateAttempt(
  state,
  credentials,
  request = apiRequest,
  {
    reconcileTimeoutMs = CREATE_RECONCILE_TIMEOUT_MS,
    now = () => Date.now(),
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  } = {},
) {
  if (state.scaleway_absence_confirmed_at) {
    if (state.scaleway_create_outcome !== "definitively_rejected") {
      fail("Scaleway journal claims absence without a definitively rejected create response");
    }
    return null;
  }
  const attemptedAt = Date.parse(state.scaleway_create_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile a Scaleway server without a create timestamp");
  const finishedAt = Date.parse(state.scaleway_create_finished_at || "");
  const requestDeadline = Date.parse(state.scaleway_create_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = state.scaleway_create_outcome === "definitively_rejected"
    ? now()
    : Math.max(now(), uncertaintyEndedAt + reconcileTimeoutMs);
  while (now() < deadline) {
    try {
      const server = await reconcileScalewayByLabel(state, credentials, request);
      if (server) return server;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      if (/refusing to adopt or delete/.test(error.message)) throw error;
    }
    const delay = Math.min(5000, Math.max(0, deadline - now()));
    if (delay > 0) await sleep(delay);
  }
  const server = await reconcileScalewayByLabel(state, credentials, request);
  if (server) return server;
  if (state.scaleway_create_outcome !== "definitively_rejected") {
    fail(
      "Scaleway create outcome remains ambiguous with no visible owned server; cleanup is unresolved and future creates remain blocked",
    );
  }
  state.scaleway_absence_confirmed_at = new Date(now()).toISOString();
  saveState(state);
  return null;
}

async function getOwnedScaleway(
  state,
  credentials,
  { allowMissing = false, forCleanup = false, request = apiRequest } = {},
) {
  requireCredentialsForState(state, credentials);
  if (!state.scaleway_id) return null;
  if (!UUID.test(state.scaleway_id)) fail("owned Scaleway ID is not a lowercase UUID");
  const response = await request(
    credentials.secret_key,
    "GET",
    `/baremetal/v1/zones/${state.plan.zone}/servers/${state.scaleway_id}`,
    { acceptedStatuses: [200, 404] },
  );
  if (response.status === 404) {
    if (allowMissing) return null;
    fail(`owned Scaleway server ${state.scaleway_id} was not found`);
  }
  const server = requireApiObject(response, `Scaleway server ${state.scaleway_id}`);
  const matches = forCleanup
    ? cleanupOwnedScalewayMatches(server, state)
    : exactOwnedScaleway(server, state) ||
      (state.scaleway_id_source === "create_response" && directCreateScalewayMatches(server, state));
  if (server.id !== state.scaleway_id || !matches) {
    fail(`Scaleway server ${state.scaleway_id} does not match the journaled ownership tuple`);
  }
  return server;
}

function verifyOwnedScalewayIdentity(server, state, { forCleanup = false } = {}) {
  const directCreateAuthority = forCleanup && state.scaleway_id_source === "create_response";
  const checks = [
    ["ID", server.id, state.scaleway_id],
    ["project", server.project_id, state.project_id],
    ["name", server.name, state.scaleway_label],
    ["zone", server.zone, state.plan.zone],
  ];
  if (!directCreateAuthority) {
    checks.push(
      ["creation window", scalewayCreationTimeMatches(server, state), true],
      ["ownership tag", scalewayHasOwnershipTag(server, state), true],
      ["description", server.description, state.scaleway_description],
      ["offer", server.offer_id, state.plan.size.id],
    );
  }
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`refusing to delete owned Scaleway server: ${field} is ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
}

function recordCleanupEvidence(state, field, now, persist) {
  if (state[field]) {
    requireIsoTimestamp(state[field], `owned run ${field}`);
    return false;
  }
  const timestamp = now();
  if (!Number.isFinite(timestamp)) fail(`cannot record ${field} with an invalid clock`);
  state[field] = new Date(timestamp).toISOString();
  persist(state);
  return true;
}

function hasCleanupEvidence(state, field) {
  if (!state[field]) return false;
  requireIsoTimestamp(state[field], `owned run ${field}`);
  return true;
}

async function observeScalewayForCleanup(state, credentials, request) {
  const byGet = await getOwnedScaleway(state, credentials, {
    allowMissing: true,
    forCleanup: true,
    request,
  });
  const servers = await listScaleways(credentials, request);
  const byId = servers.find((server) => server && server.id === state.scaleway_id) || null;
  const related = ownershipRelatedScaleways(servers, state);
  if (!byGet && !byId && related.length === 0) return null;
  if (related.length > 1 || related.some((server) => server.id !== state.scaleway_id)) {
    fail("refusing cleanup: Scaleway server ownership tuple is no longer uniquely exact");
  }
  for (const observed of [byGet, byId]) {
    if (!observed) continue;
    if (!cleanupOwnedScalewayMatches(observed, state)) {
      fail("refusing cleanup: Scaleway server ownership tuple is no longer uniquely exact");
    }
    verifyOwnedScalewayIdentity(observed, state, { forCleanup: true });
  }
  return byGet || byId;
}

async function deleteScaleway(
  state,
  credentials,
  request = apiRequest,
  {
    timeoutMs = DELETE_CONFIRM_TIMEOUT_MS,
    pollIntervalMs = 4000,
    absenceSpacingMs = 4000,
    now = () => Date.now(),
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
    persist = saveState,
  } = {},
) {
  requireCredentialsForState(state, credentials);
  if (state.scaleway_destroyed_at) return;
  if (!state.scaleway_id) {
    const reconciled = state.scaleway_create_attempted_at
      ? await reconcileScalewayAfterCreateAttempt(state, credentials, request)
      : await reconcileScalewayByLabel(state, credentials, request);
    if (!reconciled) {
      state.scaleway_destroyed_at = new Date().toISOString();
      state.cleanup_note = state.scaleway_create_attempted_at
        ? "definitively rejected create response and complete listing confirmed the unique server ownership tuple absent"
        : "no server create was attempted and complete listing confirmed the unique server ownership tuple absent";
      persist(state);
      return;
    }
  }
  if (!UUID.test(state.scaleway_id)) fail("owned Scaleway ID is not a lowercase UUID");
  if (state.scaleway_id_source === "list_reconciliation") {
    recordCleanupEvidence(state, "scaleway_positive_observed_at", now, persist);
  }
  const deadline = now() + timeoutMs;
  let absenceConfirmations = 0;
  let lastAbsenceConfirmedAt = null;
  while (now() < deadline) {
    try {
      const deleteResponse = await request(
        credentials.secret_key,
        "DELETE",
        `/baremetal/v1/zones/${state.plan.zone}/servers/${state.scaleway_id}`,
        { acceptedStatuses: [200, 404] },
      );
      if (deleteResponse.status === 200) {
        recordCleanupEvidence(state, "scaleway_delete_accepted_at", now, persist);
      }
    } catch (error) {
      if (!(error instanceof ScalewayApiError)) throw error;
      // A later authoritative response or positive read plus spaced absence scans is still required.
    }
    try {
      const observed = await observeScalewayForCleanup(state, credentials, request);
      if (observed) {
        recordCleanupEvidence(state, "scaleway_positive_observed_at", now, persist);
        absenceConfirmations = 0;
        lastAbsenceConfirmedAt = null;
      } else {
        const authoritative =
          hasCleanupEvidence(state, "scaleway_delete_accepted_at") ||
          hasCleanupEvidence(state, "scaleway_positive_observed_at");
        const observedAt = now();
        if (
          authoritative &&
          (lastAbsenceConfirmedAt === null || observedAt - lastAbsenceConfirmedAt >= absenceSpacingMs)
        ) {
          absenceConfirmations += 1;
          lastAbsenceConfirmedAt = observedAt;
          if (absenceConfirmations >= 2) {
            state.scaleway_destroyed_at = new Date(observedAt).toISOString();
            state.cleanup_note =
              "authoritative delete/observation followed by two spaced GET-and-complete-list absence confirmations";
            persist(state);
            return;
          }
        }
      }
    } catch (error) {
      if (!(error instanceof ScalewayApiError)) throw error;
    }
    const delay = Math.min(pollIntervalMs, Math.max(0, deadline - now()));
    if (delay > 0) await sleep(delay);
  }
  fail(
    `could not confirm deletion of owned Scaleway server ${state.scaleway_id}; run ` +
      `\`node tools/run-scaleway-benchmark.js destroy ${state.run_id}\` and do not assume billing stopped`,
  );
}

function sshKeyCreationTimeMatches(key, state) {
  return creationTimeMatches(
    key && key.created_at,
    state.ssh_key_create_attempted_at,
    state.ssh_key_create_finished_at,
    state.ssh_key_create_request_deadline_at,
  );
}

function sameEd25519PublicKey(left, right) {
  try {
    return (
      parseEd25519PublicKey(left, "Scaleway SSH public key").base64 ===
      parseEd25519PublicKey(right, "journaled SSH public key").base64
    );
  } catch {
    return false;
  }
}

function exactOwnedSshKey(key, state) {
  return Boolean(
    key &&
      UUID.test(key.id) &&
      key.project_id === state.project_id &&
      key.name === state.ssh_key_name &&
      sameEd25519PublicKey(key.public_key, state.ssh_public_key) &&
      key.disabled === false &&
      sshKeyCreationTimeMatches(key, state),
  );
}

function directCreateSshKeyMatches(key, state, { requireJournaledId = true } = {}) {
  return Boolean(
    key &&
      UUID.test(key.id) &&
      (!requireJournaledId || key.id === state.ssh_key_id) &&
      key.project_id === state.project_id &&
      key.name === state.ssh_key_name &&
      (key.public_key === null || key.public_key === undefined || sameEd25519PublicKey(key.public_key, state.ssh_public_key)) &&
      (key.disabled === null || key.disabled === undefined || key.disabled === false),
  );
}

function cleanupOwnedSshKeyMatches(key, state) {
  if (!key || key.id !== state.ssh_key_id || key.name !== state.ssh_key_name) return false;
  if (exactOwnedSshKey(key, state)) return true;
  return state.ssh_key_id_source === "create_response" && directCreateSshKeyMatches(key, state);
}

function ownershipRelatedSshKeys(keys, state) {
  if (!Array.isArray(keys)) fail("Scaleway SSH-key list must be an array");
  return keys.filter((key) => key && key.name === state.ssh_key_name);
}

async function reconcileSshKeyByName(state, credentials, request = apiRequest) {
  requireCredentialsForState(state, credentials);
  const related = ownershipRelatedSshKeys(await listSshKeys(credentials, request), state);
  const matches = related.filter((key) => exactOwnedSshKey(key, state));
  if (related.length !== matches.length || matches.length > 1) {
    fail(`refusing to adopt or delete Scaleway SSH-key ownership set: ${related.length} name related, ${matches.length} exact`);
  }
  if (matches.length === 1) {
    state.ssh_key_id = matches[0].id;
    state.ssh_key_id_source = "list_reconciliation";
    state.ssh_key_positive_observed_at = state.ssh_key_positive_observed_at || new Date().toISOString();
    if (state.ssh_key_create_attempted_at) state.ssh_key_create_outcome = "resource_identified";
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileSshKeyAfterCreateAttempt(
  state,
  credentials,
  request = apiRequest,
  {
    reconcileTimeoutMs = CREATE_RECONCILE_TIMEOUT_MS,
    now = () => Date.now(),
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  } = {},
) {
  if (state.ssh_key_absence_confirmed_at) {
    if (state.ssh_key_create_outcome !== "definitively_rejected") {
      fail("SSH-key journal claims absence without a definitively rejected create response");
    }
    return null;
  }
  const attemptedAt = Date.parse(state.ssh_key_create_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile an SSH key without a create timestamp");
  const finishedAt = Date.parse(state.ssh_key_create_finished_at || "");
  const requestDeadline = Date.parse(state.ssh_key_create_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = state.ssh_key_create_outcome === "definitively_rejected"
    ? now()
    : Math.max(now(), uncertaintyEndedAt + reconcileTimeoutMs);
  while (now() < deadline) {
    try {
      const key = await reconcileSshKeyByName(state, credentials, request);
      if (key) return key;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      if (/refusing to adopt or delete/.test(error.message)) throw error;
    }
    const delay = Math.min(5000, Math.max(0, deadline - now()));
    if (delay > 0) await sleep(delay);
  }
  const key = await reconcileSshKeyByName(state, credentials, request);
  if (key) return key;
  if (state.ssh_key_create_outcome !== "definitively_rejected") {
    fail("Scaleway SSH-key create remains ambiguous with no visible key; cleanup is unresolved");
  }
  state.ssh_key_absence_confirmed_at = new Date(now()).toISOString();
  saveState(state);
  return null;
}

async function getOwnedSshKey(state, credentials, { allowMissing = false, forCleanup = false, request = apiRequest } = {}) {
  requireCredentialsForState(state, credentials);
  if (!state.ssh_key_id) return null;
  if (!UUID.test(state.ssh_key_id)) fail("owned Scaleway SSH-key ID is malformed");
  const response = await request(
    credentials.secret_key,
    "GET",
    `/iam/v1alpha1/ssh-keys/${state.ssh_key_id}`,
    { acceptedStatuses: [200, 404] },
  );
  if (response.status === 404) {
    if (allowMissing) return null;
    fail(`owned Scaleway SSH key ${state.ssh_key_id} was not found`);
  }
  const key = requireApiObject(response, `Scaleway SSH key ${state.ssh_key_id}`);
  const matches = forCleanup
    ? cleanupOwnedSshKeyMatches(key, state)
    : exactOwnedSshKey(key, state) ||
      (state.ssh_key_id_source === "create_response" && directCreateSshKeyMatches(key, state));
  if (key.id !== state.ssh_key_id || !matches) fail("Scaleway SSH key does not match the journaled ownership tuple");
  return key;
}

async function observeSshKeyForCleanup(state, credentials, request) {
  const byGet = await getOwnedSshKey(state, credentials, {
    allowMissing: true,
    forCleanup: true,
    request,
  });
  const keys = await listSshKeys(credentials, request);
  const byId = keys.find((key) => key && key.id === state.ssh_key_id) || null;
  const related = ownershipRelatedSshKeys(keys, state);
  if (!byGet && !byId && related.length === 0) return null;
  if (related.length > 1 || related.some((key) => key.id !== state.ssh_key_id)) {
    fail("refusing cleanup: Scaleway SSH-key ownership tuple is no longer uniquely exact");
  }
  for (const observed of [byGet, byId]) {
    if (observed && !cleanupOwnedSshKeyMatches(observed, state)) {
      fail("refusing cleanup: Scaleway SSH-key ownership tuple is no longer uniquely exact");
    }
  }
  return byGet || byId;
}

async function deleteSshKey(
  state,
  credentials,
  request = apiRequest,
  {
    timeoutMs = DELETE_CONFIRM_TIMEOUT_MS,
    pollIntervalMs = 3000,
    absenceSpacingMs = 3000,
    now = () => Date.now(),
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
    persist = saveState,
  } = {},
) {
  requireCredentialsForState(state, credentials);
  if (state.ssh_key_destroyed_at) return;
  if (!state.scaleway_destroyed_at) fail("refusing to delete the Scaleway SSH key before server deletion is confirmed");
  if (!state.ssh_key_id) {
    const reconciled = state.ssh_key_create_attempted_at
      ? await reconcileSshKeyAfterCreateAttempt(state, credentials, request)
      : await reconcileSshKeyByName(state, credentials, request);
    if (!reconciled) {
      state.ssh_key_destroyed_at = new Date().toISOString();
      state.ssh_key_cleanup_note = state.ssh_key_create_attempted_at
        ? "definitively rejected key create and complete listing confirmed the unique key name absent"
        : "no key create was attempted and complete listing confirmed the unique key name absent";
      persist(state);
      return;
    }
  }
  if (!UUID.test(state.ssh_key_id)) fail("owned Scaleway SSH-key ID is malformed");
  if (state.ssh_key_id_source === "list_reconciliation") {
    recordCleanupEvidence(state, "ssh_key_positive_observed_at", now, persist);
  }
  const deadline = now() + timeoutMs;
  let absenceConfirmations = 0;
  let lastAbsenceConfirmedAt = null;
  while (now() < deadline) {
    try {
      const deleteResponse = await request(
        credentials.secret_key,
        "DELETE",
        `/iam/v1alpha1/ssh-keys/${state.ssh_key_id}`,
        { acceptedStatuses: [204, 404] },
      );
      if (deleteResponse.status === 204) {
        recordCleanupEvidence(state, "ssh_key_delete_accepted_at", now, persist);
      }
    } catch (error) {
      if (!(error instanceof ScalewayApiError)) throw error;
      // A later authoritative response or positive read plus spaced absence scans is still required.
    }
    try {
      const observed = await observeSshKeyForCleanup(state, credentials, request);
      if (observed) {
        recordCleanupEvidence(state, "ssh_key_positive_observed_at", now, persist);
        absenceConfirmations = 0;
        lastAbsenceConfirmedAt = null;
      } else {
        const authoritative =
          hasCleanupEvidence(state, "ssh_key_delete_accepted_at") ||
          hasCleanupEvidence(state, "ssh_key_positive_observed_at");
        const observedAt = now();
        if (
          authoritative &&
          (lastAbsenceConfirmedAt === null || observedAt - lastAbsenceConfirmedAt >= absenceSpacingMs)
        ) {
          absenceConfirmations += 1;
          lastAbsenceConfirmedAt = observedAt;
          if (absenceConfirmations >= 2) {
            state.ssh_key_destroyed_at = new Date(observedAt).toISOString();
            state.ssh_key_cleanup_note =
              "authoritative delete/observation followed by two spaced GET-and-complete-list absence confirmations";
            persist(state);
            return;
          }
        }
      }
    } catch (error) {
      if (!(error instanceof ScalewayApiError)) throw error;
    }
    const delay = Math.min(pollIntervalMs, Math.max(0, deadline - now()));
    if (delay > 0) await sleep(delay);
  }
  fail(`could not confirm deletion of owned Scaleway SSH key ${state.ssh_key_id}`);
}

async function cleanupOwned(state, credentials) {
  requireCredentialsForState(state, credentials);
  await deleteScaleway(state, credentials);
  await deleteSshKey(state, credentials);
  if (state.scaleway_destroyed_at && state.ssh_key_destroyed_at) {
    for (const file of [
      state.private_key_path,
      state.public_key_path,
      state.ssh_host_private_key_path,
      state.ssh_host_public_key_path,
      state.known_hosts_path,
    ]) {
      if (file && fs.existsSync(file)) fs.rmSync(file);
    }
  }
}

function sshOptions(state) {
  if (!/^[a-z0-9-]{1,63}$/.test(state.ssh_host_key_alias || "")) {
    fail("owned run has no safe pinned SSH host-key alias");
  }
  return [
    "-F",
    "/dev/null",
    "-i",
    state.private_key_path,
    "-o",
    "BatchMode=yes",
    "-o",
    "IdentitiesOnly=yes",
    "-o",
    "ConnectTimeout=5",
    "-o",
    "StrictHostKeyChecking=yes",
    "-o",
    "CheckHostIP=no",
    "-o",
    "VerifyHostKeyDNS=no",
    "-o",
    "GlobalKnownHostsFile=/dev/null",
    "-o",
    "KnownHostsCommand=none",
    "-o",
    "HostKeyAlgorithms=ssh-ed25519",
    "-o",
    "UpdateHostKeys=no",
    "-o",
    `HostKeyAlias=${state.ssh_host_key_alias}`,
    "-o",
    `UserKnownHostsFile=${state.known_hosts_path}`,
  ];
}

function isPublicIpv4(address) {
  if (!net.isIPv4(address)) return false;
  const octets = address.split(".").map(Number);
  if (octets[0] === 0 || octets[0] === 10 || octets[0] === 127 || octets[0] >= 224) return false;
  if (octets[0] === 100 && octets[1] >= 64 && octets[1] <= 127) return false;
  if (octets[0] === 169 && octets[1] === 254) return false;
  if (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) return false;
  if (
    octets[0] === 192 &&
    ((octets[1] === 0 && (octets[2] === 0 || octets[2] === 2)) ||
      (octets[1] === 88 && octets[2] === 99) ||
      octets[1] === 168)
  ) {
    return false;
  }
  if (octets[0] === 198 && (octets[1] === 18 || octets[1] === 19 || (octets[1] === 51 && octets[2] === 100))) {
    return false;
  }
  if (octets[0] === 203 && octets[1] === 0 && octets[2] === 113) return false;
  return true;
}


async function waitForSsh(state) {
  const deadline = Date.now() + remainingWorkTimeout(state, 5 * 60 * 1000, "SSH readiness");
  while (Date.now() < deadline) {
    const result = await run("ssh", [...sshOptions(state), `root@${state.public_ipv4}`, "true"], {
      allowFailure: true,
      timeoutMs: 15 * 1000,
    });
    if (result.status === 0) return;
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`SSH did not become ready on owned Scaleway ${state.scaleway_id}`);
}


function parseEd25519PublicKey(publicKey, location) {
  if (typeof publicKey !== "string" || publicKey.includes("\n") || publicKey.includes("\r")) {
    fail(`${location} is not a single valid ssh-ed25519 line`);
  }
  const match = /^ssh-ed25519 ([A-Za-z0-9+/]+={0,3})(?:\s+\S.*)?$/.exec(publicKey);
  if (!match) fail(`${location} is not a single valid ssh-ed25519 line`);
  return { line: publicKey, base64: match[1] };
}

function normalizeOpenSshPrivateKey(privateKey, location) {
  if (
    typeof privateKey !== "string" ||
    !/^-----BEGIN OPENSSH PRIVATE KEY-----\n(?:[A-Za-z0-9+/=]+\n)+-----END OPENSSH PRIVATE KEY-----\n?$/.test(privateKey)
  ) {
    fail(`${location} is not a valid OpenSSH private-key envelope`);
  }
  return privateKey.trimEnd();
}

function writePinnedKnownHost(state, hostPublicKey) {
  if (!/^[a-z0-9-]{1,63}$/.test(state.ssh_host_key_alias || "")) {
    fail("generated SSH host-key alias must be a lowercase safe identifier");
  }
  const parsed = parseEd25519PublicKey(hostPublicKey, "generated SSH host public key");
  writeBytesAtomic(
    state.known_hosts_path,
    Buffer.from(`${state.ssh_host_key_alias} ssh-ed25519 ${parsed.base64}\n`, "utf8"),
    0o600,
  );
}


function publicIpv4(server) {
  if (!server || !Array.isArray(server.ips)) return null;
  const addresses = server.ips
    .filter((ip) => ip && ip.version === "IPv4" && typeof ip.address === "string" && isPublicIpv4(ip.address))
    .map((ip) => ip.address);
  return addresses.length === 1 && new Set(addresses).size === 1 ? addresses[0] : null;
}

async function waitForScaleway(state, credentials) {
  const deadline = Date.now() + remainingWorkTimeout(state, ACTIVATION_TIMEOUT_MS, "Scaleway activation");
  while (Date.now() < deadline) {
    const server = await getOwnedScaleway(state, credentials, { allowMissing: true });
    if (server) {
      recordCleanupEvidence(state, "scaleway_positive_observed_at", () => Date.now(), saveState);
    }
    if (server && ["error", "out_of_stock"].includes(server.status)) {
      fail(`Scaleway server entered terminal activation status ${server.status}`);
    }
    if (server && server.install && server.install.status === "error") {
      fail("Scaleway server OS installation entered error status");
    }
    const address = publicIpv4(server);
    if (
      server &&
      server.status === "ready" &&
      server.install &&
      server.install.status === "completed" &&
      server.boot_type === "normal" &&
      address
    ) {
      state.public_ipv4 = address;
      state.scaleway_created_at = server.created_at;
      saveState(state);
      return server;
    }
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`Scaleway server ${state.scaleway_id} did not finish installation with one public IPv4 address`);
}

function verifyScalewayMatchesPlan(server, plan, state) {
  const install = requireObject(server.install, "activated Scaleway install metadata");
  const checks = [
    ["project", server.project_id, state.project_id],
    ["name", server.name, state.scaleway_label],
    ["description", server.description, state.scaleway_description],
    ["zone", server.zone, plan.zone],
    ["offer ID", server.offer_id, plan.size.id],
    ["offer", server.offer_name, plan.size.name],
    ["status", server.status, "ready"],
    ["boot type", server.boot_type, "normal"],
    ["protection", server.protected, false],
    ["OS ID", install.os_id, plan.os.id],
    ["hostname", install.hostname, state.scaleway_label],
    ["installation status", install.status, "completed"],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`activated Scaleway ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if (!Array.isArray(install.ssh_key_ids) || JSON.stringify(install.ssh_key_ids) !== JSON.stringify([state.ssh_key_id])) {
    fail("activated Scaleway install did not use exactly the ephemeral SSH key");
  }
  const expectedTags = ["rustia-benchmark", state.scaleway_ownership_tag].sort();
  if (!Array.isArray(server.tags) || JSON.stringify([...server.tags].sort()) !== JSON.stringify(expectedTags)) {
    fail("activated Scaleway server tags do not exactly match the ownership tuple");
  }
  if (!publicIpv4(server)) fail("activated Scaleway server does not expose exactly one public IPv4 address");
  if (server.user_data !== undefined && server.user_data !== null) {
    if (
      typeof server.user_data !== "string" ||
      !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(server.user_data)
    ) {
      fail("activated Scaleway server returned malformed cloud-init base64");
    }
    const userData = Buffer.from(server.user_data, "base64");
    if (userData.toString("base64") !== server.user_data || sha256(userData) !== state.cloud_init_sha256) {
      fail("activated Scaleway server cloud-init bytes differ from the journaled request");
    }
  }
}

function safeScalewayMetadata(server, plan, state) {
  return {
    provider: "scaleway",
    scaleway_id: server.id,
    scaleway_label: server.name,
    resource_type: "elastic_metal",
    created_at: server.created_at,
    status: server.status,
    zone: server.zone,
    offer_id: server.offer_id,
    offer_name: server.offer_name,
    os_id: server.install.os_id,
    os_name: plan.os.name,
    os_version: plan.os.version,
    vcpus: plan.size.vcpus,
    public_ipv4: state.public_ipv4,
    boot_type: server.boot_type,
    install: {
      hostname: server.install.hostname,
      status: server.install.status,
      ssh_key_ids: server.install.ssh_key_ids,
      user: server.install.user,
    },
    protected: server.protected,
    tags: server.tags,
    catalog_plan: plan,
  };
}

function createSshKeyBody(state, publicKey) {
  parseEd25519PublicKey(publicKey, "generated client public key");
  if (!/^[a-z0-9-]{1,63}$/.test(state.ssh_key_name || "")) {
    fail("generated Scaleway SSH-key name must be lowercase and safe");
  }
  return { name: state.ssh_key_name, public_key: publicKey, project_id: state.project_id };
}

function createScalewayBody(state, publicKey, hostPrivateKey, hostPublicKey) {
  if (
    !/^[a-z0-9-]{1,63}$/.test(state.scaleway_label || "") ||
    !/^[a-z0-9-]{1,63}$/.test(state.scaleway_ownership_tag || "") ||
    !UUID.test(state.ssh_key_id || "")
  ) {
    fail("generated Scaleway ownership fields or SSH-key ID are malformed");
  }
  const clientKey = parseEd25519PublicKey(publicKey, "generated client public key");
  const hostKey = parseEd25519PublicKey(hostPublicKey, "generated SSH host public key");
  if (clientKey.base64 === hostKey.base64) fail("generated SSH client and host public keys must be distinct");
  const normalizedHostPrivateKey = normalizeOpenSshPrivateKey(hostPrivateKey, "generated SSH host private key");
  const keyScalar = JSON.stringify(publicKey);
  const hostPublicKeyScalar = JSON.stringify(hostPublicKey);
  const userData = [
    "#cloud-config",
    "disable_root: false",
    "ssh_pwauth: false",
    "package_update: false",
    "package_upgrade: false",
    "ssh_deletekeys: true",
    "ssh_genkeytypes: []",
    "ssh_keys:",
    "  ed25519_private: |",
    ...normalizedHostPrivateKey.split("\n").map((line) => `    ${line}`),
    `  ed25519_public: ${hostPublicKeyScalar}`,
    "users:",
    "  - name: root",
    "    lock_passwd: true",
    "    ssh_authorized_keys:",
    `      - ${keyScalar}`,
    "",
  ].join("\n");
  state.cloud_init_sha256 = sha256(Buffer.from(userData, "utf8"));
  return {
    offer_id: state.plan.size.id,
    project_id: state.project_id,
    name: state.scaleway_label,
    description: state.scaleway_description,
    tags: ["rustia-benchmark", state.scaleway_ownership_tag],
    install: {
      os_id: state.plan.os.id,
      hostname: state.scaleway_label,
      ssh_key_ids: [state.ssh_key_id],
    },
    option_ids: [],
    protected: false,
    user_data: Buffer.from(userData, "utf8").toString("base64"),
  };
}

function verifyProviderPlanUnchanged(original, refreshed) {
  const before = canonicalJson({ zone: original.zone, os: original.os, size: original.size });
  const after = canonicalJson({ zone: refreshed.zone, os: refreshed.os, size: refreshed.size });
  if (before !== after) fail("Scaleway provider offer, stock, OS, zone, or price changed before billing");
}

function tarAsciiField(header, start, length, location) {
  const field = header.subarray(start, start + length);
  const nul = field.indexOf(0);
  const bytes = nul === -1 ? field : field.subarray(0, nul);
  for (const byte of bytes) {
    if (byte < 0x20 || byte > 0x7e) fail(`${location} contains non-ASCII or control bytes`);
  }
  return bytes.toString("ascii");
}

function tarOctalField(header, start, length, location) {
  const field = header.subarray(start, start + length);
  if ((field[0] & 0x80) !== 0) fail(`${location} uses an unsupported base-256 number`);
  const text = field.toString("ascii").replace(/[\0 ]+$/g, "").replace(/^ +/g, "");
  if (text === "") return 0;
  if (!/^[0-7]+$/.test(text)) fail(`${location} is not a valid octal number`);
  const value = Number.parseInt(text, 8);
  if (!Number.isSafeInteger(value) || value < 0) fail(`${location} is outside the safe integer range`);
  return value;
}

function safeArchivePath(rawName, type) {
  if (rawName === "" || rawName.includes("\\") || !/^[A-Za-z0-9._/-]+$/.test(rawName)) {
    fail(`unsafe archive path ${JSON.stringify(rawName)}`);
  }
  if (path.posix.isAbsolute(rawName)) fail(`archive path is absolute: ${JSON.stringify(rawName)}`);
  const parts = rawName.split("/");
  if (parts.includes("..")) fail(`archive path contains a parent component: ${JSON.stringify(rawName)}`);
  while (parts[0] === ".") parts.shift();
  while (parts[parts.length - 1] === "") parts.pop();
  if (parts.some((part) => part === "" || part === ".")) {
    fail(`archive path is not canonical: ${JSON.stringify(rawName)}`);
  }
  const canonical = parts.length === 0 ? "." : parts.join("/");
  if (canonical === "." && type !== "directory") {
    fail("archive root entry must be a directory");
  }
  return canonical;
}

function inspectTarBytes(bytes, location = "export archive") {
  if (!Buffer.isBuffer(bytes) || bytes.length === 0 || bytes.length > MAX_EXPORT_UNCOMPRESSED_BYTES) {
    fail(`${location} has an invalid uncompressed size`);
  }
  if (bytes.length % 512 !== 0) fail(`${location} is not aligned to 512-byte tar records`);
  const entries = [];
  const seen = new Map();
  let offset = 0;
  let zeroBlocks = 0;
  let totalPayloadBytes = 0;
  while (offset < bytes.length) {
    const header = bytes.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) {
      zeroBlocks += 1;
      offset += 512;
      if (zeroBlocks >= 2) {
        if (!bytes.subarray(offset).every((byte) => byte === 0)) {
          fail(`${location} contains data after its end-of-archive marker`);
        }
        break;
      }
      continue;
    }
    if (zeroBlocks !== 0) fail(`${location} contains an entry after a partial end marker`);
    if (entries.length >= MAX_EXPORT_ENTRIES) fail(`${location} contains too many entries`);

    const recordedChecksum = tarOctalField(header, 148, 8, `${location} header checksum`);
    let computedChecksum = 0;
    for (let index = 0; index < header.length; index += 1) {
      computedChecksum += index >= 148 && index < 156 ? 0x20 : header[index];
    }
    if (recordedChecksum !== computedChecksum) fail(`${location} contains a header with a bad checksum`);

    const name = tarAsciiField(header, 0, 100, `${location} entry name`);
    const magic = header.subarray(257, 263);
    const version = header.subarray(263, 265);
    let prefix = "";
    if (magic.equals(Buffer.from("ustar\0", "binary")) && version.equals(Buffer.from("00", "ascii"))) {
      prefix = tarAsciiField(header, 345, 155, `${location} entry prefix`);
    } else {
      fail(`${location} entry ${JSON.stringify(name)} is not exact POSIX ustar format`);
    }
    const rawName = prefix ? `${prefix}/${name}` : name;
    const typeFlag = header[156];
    const type = typeFlag === 0 || typeFlag === 0x30 ? "file" : typeFlag === 0x35 ? "directory" : null;
    if (!type) {
      const printableType = typeFlag >= 0x20 && typeFlag <= 0x7e ? String.fromCharCode(typeFlag) : `0x${typeFlag.toString(16)}`;
      fail(`${location} entry ${JSON.stringify(rawName)} has forbidden tar type ${printableType}`);
    }
    const canonical = safeArchivePath(rawName, type);
    if (seen.has(canonical)) {
      fail(`${location} contains duplicate path ${JSON.stringify(canonical)}`);
    }
    const mode = tarOctalField(header, 100, 8, `${location} entry mode`);
    if ((mode & 0o7000) !== 0) fail(`${location} entry ${JSON.stringify(rawName)} has special mode bits`);
    const size = tarOctalField(header, 124, 12, `${location} entry size`);
    if (type === "directory" && size !== 0) {
      fail(`${location} directory ${JSON.stringify(rawName)} has a non-zero payload`);
    }
    totalPayloadBytes += size;
    if (totalPayloadBytes > MAX_EXPORT_UNCOMPRESSED_BYTES) fail(`${location} payload is too large`);
    const payloadBlocks = Math.ceil(size / 512);
    const next = offset + 512 + payloadBlocks * 512;
    if (next > bytes.length) fail(`${location} entry ${JSON.stringify(rawName)} extends past EOF`);
    const entry = { path: canonical, type, size };
    seen.set(canonical, entry);
    entries.push(entry);
    offset = next;
  }
  if (zeroBlocks < 2) fail(`${location} lacks the two-block end-of-archive marker`);
  if (entries.length === 0) fail(`${location} contains no entries`);
  for (const entry of entries) {
    if (entry.path === ".") continue;
    const parts = entry.path.split("/");
    for (let index = 1; index < parts.length; index += 1) {
      const parent = parts.slice(0, index).join("/");
      const parentEntry = seen.get(parent);
      if (parentEntry && parentEntry.type !== "directory") {
        fail(`${location} places ${JSON.stringify(entry.path)} below non-directory ${JSON.stringify(parent)}`);
      }
    }
  }
  return entries;
}

function inspectArchive(archive) {
  const stat = fs.lstatSync(archive);
  if (!stat.isFile() || stat.nlink !== 1) fail("export archive must be a single regular file");
  if (stat.size <= 0 || stat.size > MAX_EXPORT_ARCHIVE_BYTES) fail("export archive compressed size is invalid");
  let bytes;
  try {
    bytes = zlib.gunzipSync(fs.readFileSync(archive), {
      maxOutputLength: MAX_EXPORT_UNCOMPRESSED_BYTES,
    });
  } catch (error) {
    fail(`cannot safely decompress export archive: ${error.message}`);
  }
  return inspectTarBytes(bytes);
}

function validateExtractedTree(destination, archiveEntries = null) {
  const root = fs.lstatSync(destination);
  if (!root.isDirectory() || root.isSymbolicLink()) fail("extraction destination is not a real directory");
  const actual = new Map();
  const visit = (directory, prefix = "") => {
    for (const name of fs.readdirSync(directory).sort()) {
      const relative = prefix ? `${prefix}/${name}` : name;
      const absolute = path.join(directory, name);
      const stat = fs.lstatSync(absolute);
      if (stat.isSymbolicLink()) fail(`extracted export contains symlink ${JSON.stringify(relative)}`);
      if (stat.isDirectory()) {
        actual.set(relative, "directory");
        visit(absolute, relative);
      } else if (stat.isFile()) {
        if (stat.nlink !== 1) fail(`extracted export contains hard-linked file ${JSON.stringify(relative)}`);
        actual.set(relative, "file");
      } else {
        fail(`extracted export contains non-file/non-directory ${JSON.stringify(relative)}`);
      }
    }
  };
  visit(destination);
  if (archiveEntries) {
    const expected = new Map(
      archiveEntries.filter((entry) => entry.path !== ".").map((entry) => [entry.path, entry.type]),
    );
    if (canonicalJson(Object.fromEntries(actual)) !== canonicalJson(Object.fromEntries(expected))) {
      fail("extracted export tree does not exactly match the validated archive listing");
    }
  }
  return actual;
}

async function extractArchive(archive, destination) {
  const entries = inspectArchive(archive);
  if (fs.existsSync(destination)) fail("refusing to extract an export over an existing path");
  fs.mkdirSync(destination, { recursive: false, mode: 0o700 });
  try {
    await run("tar", ["-xzf", archive, "-C", destination], { capture: false, announce: true });
    validateExtractedTree(destination, entries);
  } catch (error) {
    fs.rmSync(destination, { recursive: true, force: true });
    throw error;
  }
}

async function verifyPromotionCheckout(config, state) {
  const current = await verifySource(config, state.source_config_path || DEFAULT_CONFIG);
  const planned = state.plan && state.plan.source;
  if (!planned || JSON.stringify(current) !== JSON.stringify(planned)) {
    fail("local orchestration checkout changed while the Scaleway run was in flight; retaining export only");
  }
}

function requireJsonEqual(actual, expected, location) {
  if (canonicalJson(actual) !== canonicalJson(expected)) fail(`${location} does not match the frozen local value`);
}

function loadJsonRegular(file, location) {
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
    fail(`${location} must be a single regular file`);
  }
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`${location} is not valid JSON: ${error.message}`);
  }
}

function requireIsoTimestamp(value, location) {
  requireString(value, location);
  const milliseconds = Date.parse(value);
  if (!Number.isFinite(milliseconds) || new Date(milliseconds).toISOString() !== value) {
    fail(`${location} must be an exact ISO-8601 UTC timestamp`);
  }
  return milliseconds;
}

function validateAcceptedStatus(status, requiredVerdict) {
  requireObject(status, "accepted status");
  requireExactKeys(
    status,
    ["schema", "status", "detail", "started_at", "finished_at", "verdict"],
    "accepted status",
  );
  if (status.schema !== "rustia.bench.cloud-run-status.v1") fail("accepted status has the wrong schema");
  if (status.status !== "accepted_and_published") fail("accepted status is not accepted_and_published");
  if (status.detail !== "material regression excluded and three actual final-binary history rows appended") {
    fail("accepted status has an unexpected detail");
  }
  if (status.verdict !== requiredVerdict) fail("accepted status has the wrong verdict");
  const started = requireIsoTimestamp(status.started_at, "accepted status.started_at");
  const finished = requireIsoTimestamp(status.finished_at, "accepted status.finished_at");
  if (finished < started) fail("accepted status finished before it started");
  return status;
}

function evidenceFileHashes(evidenceDirectory) {
  const hashes = Object.create(null);
  const visit = (directory, prefix = "") => {
    for (const name of fs.readdirSync(directory).sort()) {
      const absolute = path.join(directory, name);
      const relative = prefix ? `${prefix}/${name}` : name;
      const stat = fs.lstatSync(absolute);
      if (stat.isSymbolicLink()) fail(`evidence contains symlink ${JSON.stringify(relative)}`);
      if (stat.isDirectory()) {
        visit(absolute, relative);
      } else if (stat.isFile() && stat.nlink === 1) {
        if (relative !== "manifest.json") hashes[relative] = sha256File(absolute);
      } else {
        fail(`evidence contains an unsafe entry ${JSON.stringify(relative)}`);
      }
    }
  };
  visit(evidenceDirectory);
  return hashes;
}

function validateBareMetalHost(config, hostValue) {
  const host = requireObject(hostValue, "host.json");
  if (host.virtualization !== "none") fail("host.json virtualization is not none");
  const attestation = requireObject(host.bare_metal_attestation, "host.json.bare_metal_attestation");
  requireExactKeys(
    attestation,
    [
      "systemd_detect_virt",
      "systemd_detect_virt_status",
      "hypervisor_cpu_flag",
      "sys_hypervisor_type",
      "logical_cpus",
      "physical_sockets",
      "physical_cores",
      "cpu_model",
      "controller_cpu",
      "benchmark_cpu",
      "controller_and_benchmark_are_smt_siblings",
      "topology",
    ],
    "host.json.bare_metal_attestation",
  );
  const exact = [
    [attestation.systemd_detect_virt, "none", "systemd-detect-virt output"],
    [attestation.systemd_detect_virt_status, 1, "systemd-detect-virt status"],
    [attestation.hypervisor_cpu_flag, false, "hypervisor CPU flag"],
    [attestation.sys_hypervisor_type, "absent_or_empty", "/sys/hypervisor/type"],
    [attestation.logical_cpus, config.provider.size.cpu_threads, "logical CPU count"],
    [attestation.physical_sockets, config.provider.size.physical_cpus, "physical socket count"],
    [attestation.physical_cores, config.provider.size.cpu_cores, "physical core count"],
    [attestation.controller_cpu, 0, "controller CPU"],
    [attestation.benchmark_cpu, config.host_gate.benchmark_cpu, "benchmark CPU"],
    [attestation.controller_and_benchmark_are_smt_siblings, false, "controller/benchmark SMT relation"],
  ];
  for (const [actual, expected, field] of exact) {
    if (actual !== expected) fail(`host bare-metal attestation has the wrong ${field}`);
  }
  if (
    typeof attestation.cpu_model !== "string" ||
    !attestation.cpu_model.includes(config.provider.size.host_cpu_model) ||
    host.cpu_model !== attestation.cpu_model
  ) {
    fail("host bare-metal attestation CPU model does not match the frozen physical plan");
  }
  const topology = requireObject(attestation.topology, "host bare-metal topology");
  const expectedCpuNames = Array.from({ length: config.provider.size.cpu_threads }, (_, cpu) => `cpu${cpu}`);
  if (canonicalJson(Object.keys(topology).sort()) !== canonicalJson(expectedCpuNames.sort())) {
    fail("host bare-metal topology does not cover exactly every configured logical CPU");
  }
  const siblingCount = config.provider.size.cpu_threads / config.provider.size.cpu_cores;
  const sockets = new Set();
  const cores = new Set();
  for (let cpu = 0; cpu < config.provider.size.cpu_threads; cpu += 1) {
    const entry = requireObject(topology[`cpu${cpu}`], `host topology.cpu${cpu}`);
    requireExactKeys(entry, ["socket", "core", "siblings"], `host topology.cpu${cpu}`);
    const socket = requireString(entry.socket, `host topology.cpu${cpu}.socket`);
    const core = requireString(entry.core, `host topology.cpu${cpu}.core`);
    if (
      !Array.isArray(entry.siblings) ||
      entry.siblings.length !== siblingCount ||
      new Set(entry.siblings).size !== siblingCount ||
      entry.siblings.some((sibling) => !Number.isSafeInteger(sibling) || sibling < 0 || sibling >= config.provider.size.cpu_threads) ||
      !entry.siblings.includes(cpu) ||
      canonicalJson(entry.siblings) !== canonicalJson(entry.siblings.slice().sort((left, right) => left - right))
    ) {
      fail(`host topology.cpu${cpu} has an invalid SMT sibling set`);
    }
    sockets.add(socket);
    cores.add(`${socket}:${core}`);
  }
  if (sockets.size !== config.provider.size.physical_cpus || cores.size !== config.provider.size.cpu_cores) {
    fail("host bare-metal topology has the wrong physical socket/core cardinality");
  }
  for (let cpu = 0; cpu < config.provider.size.cpu_threads; cpu += 1) {
    const entry = topology[`cpu${cpu}`];
    for (const sibling of entry.siblings) {
      const peer = topology[`cpu${sibling}`];
      if (
        peer.socket !== entry.socket ||
        peer.core !== entry.core ||
        canonicalJson(peer.siblings) !== canonicalJson(entry.siblings)
      ) {
        fail(`host topology.cpu${cpu} has asymmetric or cross-core SMT siblings`);
      }
    }
  }
  if (topology.cpu0.siblings.includes(config.host_gate.benchmark_cpu)) {
    fail("host controller and benchmark CPUs are SMT siblings");
  }
  return host;
}

function validateCpuDelta(deltaValue, location) {
  const delta = requireObject(deltaValue, location);
  requireExactKeys(delta, ["total_ticks", "idle_ticks", "idle_percent", "steal_ticks"], location);
  requireInteger(delta.total_ticks, `${location}.total_ticks`, 1);
  requireInteger(delta.idle_ticks, `${location}.idle_ticks`, 0);
  requireInteger(delta.steal_ticks, `${location}.steal_ticks`, 0);
  requireNumber(delta.idle_percent, `${location}.idle_percent`, 0);
  if (delta.idle_ticks > delta.total_ticks || delta.idle_percent > 100) fail(`${location} has impossible idle data`);
  const expectedIdle = Number(((delta.idle_ticks / delta.total_ticks) * 100).toFixed(6));
  if (delta.idle_percent !== expectedIdle) fail(`${location}.idle_percent does not match its tick counts`);
  return delta;
}

function validateGateLog(config, gateFile, declaredRecords) {
  const bytes = fs.readFileSync(gateFile);
  const text = strictUtf8(bytes, "gate.jsonl");
  if (!text.endsWith("\n") || text.includes("\r")) fail("gate.jsonl is not canonical JSONL");
  const lines = text.slice(0, -1).split("\n");
  if (lines.length !== declaredRecords || lines.some((line) => line === "")) {
    fail("gate.jsonl record count does not match the manifest");
  }
  const records = lines.map((line, index) => {
    let record;
    try {
      record = JSON.parse(line);
    } catch (error) {
      fail(`gate.jsonl record ${index + 1} is invalid JSON: ${error.message}`);
    }
    if (line !== JSON.stringify(record)) fail(`gate.jsonl record ${index + 1} is not canonical JSON`);
    return requireObject(record, `gate.jsonl record ${index + 1}`);
  });
  const expectedLabels = [
    ...Array.from({ length: config.qualification.candidate_pilot_legs }, (_, index) => `qualification-${index + 1}`),
    ...config.measurement.run_order.map((leg, index) => `accepted-${index + 1}-${leg.file}`),
    ...Array.from({ length: config.publication.final_measurements }, (_, index) => `publication-${index + 1}`),
  ];
  let recordIndex = 0;
  for (const label of expectedLabels) {
    let preCount = 0;
    let passedPre = false;
    while (recordIndex < records.length && records[recordIndex].label === label && records[recordIndex].phase === "pre_leg_idle_window") {
      const record = records[recordIndex];
      requireExactKeys(
        record,
        ["phase", "label", "timestamp", "cpu", "load_1m", "cpu_delta", "contaminants", "passed"],
        `gate pre-record ${recordIndex + 1}`,
      );
      requireIsoTimestamp(record.timestamp, `gate pre-record ${recordIndex + 1}.timestamp`);
      if (record.cpu !== config.host_gate.benchmark_cpu) fail(`gate pre-record ${recordIndex + 1} has the wrong CPU`);
      requireNumber(record.load_1m, `gate pre-record ${recordIndex + 1}.load_1m`, 0);
      const delta = validateCpuDelta(record.cpu_delta, `gate pre-record ${recordIndex + 1}.cpu_delta`);
      if (!Array.isArray(record.contaminants)) fail(`gate pre-record ${recordIndex + 1}.contaminants is not an array`);
      const shouldPass =
        record.load_1m <= config.host_gate.maximum_load_1m &&
        delta.idle_percent >= config.host_gate.minimum_idle_percent &&
        delta.steal_ticks <= config.host_gate.maximum_steal_ticks &&
        record.contaminants.length === 0;
      if (record.passed !== shouldPass) fail(`gate pre-record ${recordIndex + 1} has a false passed flag`);
      preCount += 1;
      passedPre = record.passed;
      recordIndex += 1;
      if (passedPre) break;
    }
    if (preCount === 0 || !passedPre) fail(`gate label ${label} lacks a final passing pre-idle record`);
    if (recordIndex >= records.length || records[recordIndex].label !== label || records[recordIndex].phase !== "during_leg") {
      fail(`gate label ${label} lacks exactly one during-leg record after its passing pre-idle record`);
    }
    const during = records[recordIndex];
    requireExactKeys(
      during,
      ["phase", "label", "timestamp", "cpu", "cpu_delta", "contaminants", "passed"],
      `gate during-record ${recordIndex + 1}`,
    );
    requireIsoTimestamp(during.timestamp, `gate during-record ${recordIndex + 1}.timestamp`);
    if (during.cpu !== config.host_gate.benchmark_cpu) fail(`gate during-record ${recordIndex + 1} has the wrong CPU`);
    const delta = validateCpuDelta(during.cpu_delta, `gate during-record ${recordIndex + 1}.cpu_delta`);
    if (
      during.passed !== true ||
      delta.steal_ticks > config.host_gate.maximum_steal_ticks ||
      !Array.isArray(during.contaminants) ||
      during.contaminants.length !== 0
    ) {
      fail(`gate during-record ${recordIndex + 1} did not pass the frozen zero-steal/clean-process gate`);
    }
    recordIndex += 1;
  }
  if (recordIndex !== records.length) fail("gate.jsonl contains unexpected labels or extra records");
  return records;
}

function validateManifestEvidenceHashes(manifest, evidenceDirectory) {
  const declared = requireObject(manifest.evidence_sha256, "manifest.evidence_sha256");
  for (const [relative, digest] of Object.entries(declared)) {
    if (relative === "manifest.json" || path.posix.isAbsolute(relative) || relative.split("/").includes("..")) {
      fail(`manifest.evidence_sha256 contains unsafe path ${JSON.stringify(relative)}`);
    }
    if (!SHA256.test(requireString(digest, `manifest.evidence_sha256[${JSON.stringify(relative)}]`))) {
      fail(`manifest.evidence_sha256 has an invalid digest for ${JSON.stringify(relative)}`);
    }
  }
  const actual = evidenceFileHashes(evidenceDirectory);
  requireJsonEqual(declared, actual, "manifest.evidence_sha256");
  return actual;
}

function numbersClose(left, right) {
  if (typeof left !== "number" || !Number.isFinite(left) || typeof right !== "number" || !Number.isFinite(right)) {
    return false;
  }
  const scale = Math.max(1, Math.abs(left), Math.abs(right));
  return Math.abs(left - right) <= scale * 1e-12;
}

function matchesHistoryPrecision(serialized, raw) {
  return (
    typeof serialized === "number" &&
    Number.isFinite(serialized) &&
    typeof raw === "number" &&
    Number.isFinite(raw) &&
    Math.abs(serialized - raw) <= 0.500001e-6
  );
}

function quantileSorted(values, probability) {
  if (values.length === 1) return values[0];
  const position = (values.length - 1) * probability;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  if (lower === upper) return values[lower];
  const fraction = position - lower;
  return values[lower] * (1 - fraction) + values[upper] * fraction;
}

function validateRawSidecar(file, config, location, expectedCommit, expectedIdentity = null) {
  const sidecar = requireObject(loadJsonRegular(file, location), location);
  if (sidecar.schema !== "rustia.bench.raw-samples.v1" || sidecar.schema_version !== 1) {
    fail(`${location} has the wrong raw-sample schema`);
  }
  if (sidecar.source !== "script-bench-harness") fail(`${location} has the wrong source`);
  const metadata = requireObject(sidecar.metadata, `${location}.metadata`);
  const expectedMetadata = {
    description:
      "Raw per-repetition timings in execution order. This sidecar is separate from the append-only public history schema.",
    measurement_isolation: "fresh prepared workload per repetition",
    warmup: "one untimed invocation immediately before each recorded sample",
    sample_order: "execution order (unsorted)",
  };
  for (const [field, expected] of Object.entries(expectedMetadata)) {
    if (metadata[field] !== expected) fail(`${location}.metadata.${field} is not frozen`);
  }
  if (!Array.isArray(sidecar.measurements) || sidecar.measurements.length !== 1) {
    fail(`${location} must contain exactly one measurement`);
  }
  if (metadata.measurement_count !== 1 || metadata.sample_count !== config.measurement.repetitions) {
    fail(`${location} metadata counts do not match the frozen protocol`);
  }
  const measurement = requireObject(sidecar.measurements[0], `${location}.measurements[0]`);
  const expected = {
    commit: expectedCommit,
    target: "linux-x86_64",
    engine: "rustia",
    workload: config.measurement.workload,
    n: config.measurement.fixed_n,
    n_selection: "fixed",
    reps: config.measurement.repetitions,
    checksum: config.measurement.expected_checksum,
  };
  for (const [field, wanted] of Object.entries(expected)) {
    if (measurement[field] !== wanted) {
      fail(`${location}.measurement.${field} does not match the frozen protocol`);
    }
  }
  requireInteger(measurement.timestamp_unix, `${location}.measurement.timestamp_unix`, 0);
  for (const field of ["dialect", "unit"]) requireString(measurement[field], `${location}.measurement.${field}`);
  if (
    expectedIdentity &&
    (measurement.dialect !== expectedIdentity.dialect || measurement.unit !== expectedIdentity.unit)
  ) {
    fail(`${location} dialect/unit do not match the independently recomputed comparison identity`);
  }
  if (!Array.isArray(measurement.samples_ns_per_unit) || measurement.samples_ns_per_unit.length !== measurement.reps) {
    fail(`${location} has the wrong sample count`);
  }
  const samples = measurement.samples_ns_per_unit.slice();
  if (samples.some((sample) => typeof sample !== "number" || !Number.isFinite(sample) || sample <= 0)) {
    fail(`${location} contains invalid raw samples`);
  }
  const ordered = samples.slice().sort((left, right) => left - right);
  const minimum = ordered[0];
  const median = ordered[Math.floor(ordered.length / 2)];
  if (!numbersClose(measurement.ns_per_unit_min, minimum)) fail(`${location} minimum does not match its samples`);
  if (!numbersClose(measurement.ns_per_unit_median, median)) fail(`${location} median does not match its samples`);
  if (!numbersClose(measurement.units_per_sec, 1e9 / minimum)) {
    fail(`${location} throughput does not match its minimum sample`);
  }
  const q1 = quantileSorted(ordered, 0.25);
  const q3 = quantileSorted(ordered, 0.75);
  return {
    file: path.basename(file),
    sha256: sha256File(file),
    measurement,
    derived: {
      sample_count: ordered.length,
      min: minimum,
      q1,
      median: quantileSorted(ordered, 0.5),
      harness_median: measurement.ns_per_unit_median,
      q3,
      max: ordered[ordered.length - 1],
      iqr: q3 - q1,
    },
  };
}

function strictUtf8(bytes, location) {
  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) fail(`${location} is not valid UTF-8`);
  return text;
}

function validatePublishedHistory(config, localHistory, remoteHistory, evidenceDirectory, analysisIdentity) {
  if (sha256(localHistory) !== config.source.expected_sha256.history_before) {
    fail("local benchmark history changed while the Scaleway run was in flight");
  }
  if (
    remoteHistory.length <= localHistory.length ||
    !remoteHistory.subarray(0, localHistory.length).equals(localHistory)
  ) {
    fail("remote history is not an append-only extension of the exact local history");
  }
  const suffix = strictUtf8(remoteHistory.subarray(localHistory.length), "remote history suffix");
  if (!suffix.endsWith("\n") || suffix.includes("\r")) fail("remote history suffix is not canonical JSONL");
  const lines = suffix.slice(0, -1).split("\n");
  if (lines.length !== config.publication.final_measurements || lines.some((line) => line === "")) {
    fail(`remote history appended ${lines.length} rows, expected ${config.publication.final_measurements}`);
  }
  const rows = lines.map((line, index) => {
    let row;
    try {
      row = JSON.parse(line);
    } catch (error) {
      fail(`remote history row ${index + 1} is invalid JSON: ${error.message}`);
    }
    requireObject(row, `remote history row ${index + 1}`);
    requireExactKeys(
      row,
      [
        "timestamp_unix",
        "commit",
        "target",
        "engine",
        "dialect",
        "workload",
        "unit",
        "n",
        "reps",
        "ns_per_unit_min",
        "ns_per_unit_median",
        "units_per_sec",
        "checksum",
      ],
      `remote history row ${index + 1}`,
    );
    return row;
  });
  const sidecars = [];
  for (let index = 1; index <= config.publication.final_measurements; index += 1) {
    const summary = validateRawSidecar(
      path.join(evidenceDirectory, `final-dense-${index}.json`),
      config,
      `final measurement sidecar ${index}`,
      config.source.candidate_commit,
      analysisIdentity,
    );
    const row = rows[index - 1];
    const measurement = summary.measurement;
    if (
      row.timestamp_unix !== measurement.timestamp_unix ||
      row.commit !== measurement.commit ||
      row.target !== measurement.target ||
      row.engine !== measurement.engine ||
      row.dialect !== measurement.dialect ||
      row.workload !== measurement.workload ||
      row.unit !== measurement.unit ||
      row.n !== measurement.n ||
      row.reps !== measurement.reps ||
      !matchesHistoryPrecision(row.ns_per_unit_min, measurement.ns_per_unit_min) ||
      !matchesHistoryPrecision(row.ns_per_unit_median, measurement.ns_per_unit_median) ||
      !matchesHistoryPrecision(row.units_per_sec, measurement.units_per_sec) ||
      row.checksum !== measurement.checksum
    ) {
      fail(`remote history row ${index} does not exactly correspond to final-dense-${index}.json`);
    }
    sidecars.push(summary);
  }
  return {
    beforeLines: strictUtf8(localHistory, "local history").trimEnd().split("\n").length,
    afterHash: sha256(remoteHistory),
    rows,
    sidecars,
  };
}

function normalizedAnalysis(value) {
  const copy = JSON.parse(JSON.stringify(value));
  for (const group of ["baseline", "candidate"]) {
    if (copy[group] && Array.isArray(copy[group].legs)) {
      for (const leg of copy[group].legs) leg.file = path.basename(leg.file);
    }
  }
  return copy;
}

async function recomputeAnalysis(config, evidenceDirectory) {
  const argumentsList = [path.join(REPO, "bench/analyze-fixed-samples.js")];
  for (const leg of config.measurement.run_order.filter((entry) => entry.variant === "baseline")) {
    argumentsList.push("--baseline", path.join(evidenceDirectory, leg.file));
  }
  for (const leg of config.measurement.run_order.filter((entry) => entry.variant === "candidate")) {
    argumentsList.push("--candidate", path.join(evidenceDirectory, leg.file));
  }
  argumentsList.push(
    "--bootstrap-reps",
    String(config.measurement.bootstrap_repetitions),
    "--seed",
    String(config.measurement.bootstrap_seed),
    "--material-threshold",
    String(config.measurement.material_regression_threshold_percent),
    "--expected-legs",
    String(config.measurement.expected_legs_per_group),
    "--require-full-commits",
  );
  const result = await run("node", argumentsList);
  try {
    return JSON.parse(result.stdout.toString("utf8"));
  } catch (error) {
    fail(`local fixed-sample analyzer returned invalid JSON: ${error.message}`);
  }
}

function requireCompleteSummary(summaryValue, sidecar, config, location, variant = null) {
  const summary = requireObject(summaryValue, location);
  const summaryKeys = [
    "file",
    "sha256",
    "timestamp_unix",
    "commit",
    "target",
    "engine",
    "dialect",
    "workload",
    "unit",
    "n",
    "reps",
    "checksum",
    "ns_per_unit_min",
    "ns_per_unit_median",
    "units_per_sec",
    "sample_count",
    "min",
    "q1",
    "median",
    "harness_median",
    "q3",
    "max",
    "iqr",
  ];
  if (variant !== null) summaryKeys.unshift("variant");
  requireExactKeys(summary, summaryKeys, location);
  const measurement = sidecar.measurement;
  const expected = {
    file: sidecar.file,
    sha256: sidecar.sha256,
    timestamp_unix: measurement.timestamp_unix,
    commit: measurement.commit,
    target: "linux-x86_64",
    engine: measurement.engine,
    dialect: measurement.dialect,
    workload: config.measurement.workload,
    unit: measurement.unit,
    n: config.measurement.fixed_n,
    reps: config.measurement.repetitions,
    checksum: config.measurement.expected_checksum,
    ns_per_unit_min: measurement.ns_per_unit_min,
    ns_per_unit_median: measurement.ns_per_unit_median,
    units_per_sec: measurement.units_per_sec,
    ...sidecar.derived,
  };
  if (variant !== null) expected.variant = variant;
  for (const [field, wanted] of Object.entries(expected)) {
    const matches = typeof wanted === "number" ? numbersClose(summary[field], wanted) : summary[field] === wanted;
    if (!matches) fail(`${location}.${field} does not match its raw sidecar`);
  }
}

async function validateAcceptedExport(config, state, downloadDirectory, status) {
  validateAcceptedStatus(status, config.publication.required_verdict);
  const candidate = config.source.candidate_commit;
  const evidenceDirectory = path.join(downloadDirectory, "evidence", candidate);
  const historyFile = path.join(downloadDirectory, "publication", "bench/history/runs.jsonl");
  const manifestFile = path.join(evidenceDirectory, "manifest.json");
  const analysisFile = path.join(evidenceDirectory, "analysis.json");
  const protocolFile = path.join(evidenceDirectory, "protocol.json");
  const providerFile = path.join(evidenceDirectory, "provider.json");
  for (const [file, location] of [
    [manifestFile, "evidence manifest"],
    [analysisFile, "fixed-sample analysis"],
    [protocolFile, "evidence protocol"],
    [providerFile, "evidence provider metadata"],
    [historyFile, "published history"],
  ]) {
    const stat = fs.lstatSync(file);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) fail(`${location} is not a regular file`);
  }

  const manifest = requireObject(loadJsonRegular(manifestFile, "evidence manifest"), "evidence manifest");
  requireExactKeys(
    manifest,
    [
      "schema_version",
      "schema",
      "created_at",
      "provider",
      "source",
      "build",
      "host",
      "protocol",
      "qualification",
      "comparison",
      "publication",
      "evidence_sha256",
    ],
    "evidence manifest",
  );
  if (manifest.schema_version !== 1 || manifest.schema !== "rustia.bench.cloud-evidence.v1") {
    fail("evidence manifest has the wrong schema");
  }
  requireIsoTimestamp(manifest.created_at, "evidence manifest.created_at");
  validateManifestEvidenceHashes(manifest, evidenceDirectory);

  const protocol = loadJsonRegular(protocolFile, "evidence protocol");
  validateConfig(protocol);
  requireJsonEqual(protocol, config, "evidence protocol");
  const expectedConfigHash = state.plan && state.plan.source && state.plan.source.config_sha256;
  if (!SHA256.test(expectedConfigHash || "") || sha256File(protocolFile) !== expectedConfigHash) {
    fail("evidence protocol bytes do not match the planned frozen config");
  }

  const provider = requireObject(loadJsonRegular(providerFile, "evidence provider metadata"), "evidence provider metadata");
  const localProviderFile = path.join(state.__directory, "provider.json");
  const localProvider = loadJsonRegular(localProviderFile, "local provider snapshot");
  requireJsonEqual(provider, localProvider, "evidence provider metadata");
  requireJsonEqual(provider.catalog_plan, state.plan, "evidence provider catalog plan");
  const providerIdentity = [
    [provider.provider, "scaleway", "provider"],
    [provider.resource_type, "elastic_metal", "resource type"],
    [provider.scaleway_id, state.scaleway_id, "Scaleway ID"],
    [provider.scaleway_label, state.scaleway_label, "Scaleway label"],
    [provider.zone, state.plan.zone, "zone"],
    [provider.offer_id, state.plan.size.id, "offer ID"],
    [provider.offer_name, config.provider.size.name, "offer"],
    [provider.os_id, state.plan.os.id, "OS"],
    [provider.os_name, config.provider.os.name, "OS name"],
    [provider.os_version, config.provider.os.version, "OS version"],
    [provider.vcpus, config.provider.size.cpu_threads, "vCPUs"],
    [provider.public_ipv4, state.public_ipv4, "public IPv4"],
  ];
  for (const [actual, expected, field] of providerIdentity) {
    if (actual !== expected) fail(`evidence provider ${field} does not match the destroyed instance`);
  }
  if (!Array.isArray(provider.tags) || !provider.tags.includes(state.scaleway_ownership_tag)) {
    fail("evidence provider metadata is missing the unique ownership tag");
  }
  requireJsonEqual(manifest.provider, provider, "manifest provider identity");

  const source = requireObject(manifest.source, "manifest.source");
  requireExactKeys(
    source,
    [
      "baseline_commit",
      "candidate_commit",
      "harness_commit",
      "baseline_original_harness_sha256",
      "baseline_overlay_harness_sha256",
      "baseline_overlay_disclosure",
      "workload_sha256",
      "cargo_lock_sha256",
      "toolchain_sha256",
      "analyzer_sha256",
      "git_bundle_sha256",
      "protocol_config_sha256",
      "provider_metadata_sha256",
    ],
    "manifest.source",
  );
  const expectedSource = {
    baseline_commit: config.source.baseline_commit,
    candidate_commit: config.source.candidate_commit,
    harness_commit: config.source.harness_commit,
    baseline_original_harness_sha256: config.source.expected_sha256.baseline_original_harness,
    baseline_overlay_harness_sha256: config.source.expected_sha256.overlay_harness,
    baseline_overlay_disclosure:
      "bench/src/main.rs from the harness commit was copied byte-for-byte over the baseline source before compilation",
    workload_sha256: config.source.expected_sha256.workload,
    cargo_lock_sha256: config.source.expected_sha256.cargo_lock,
    toolchain_sha256: config.source.expected_sha256.toolchain,
    analyzer_sha256: config.source.expected_sha256.analyzer,
    git_bundle_sha256: state.bundle_sha256,
    protocol_config_sha256: sha256File(protocolFile),
    provider_metadata_sha256: sha256File(providerFile),
  };
  requireJsonEqual(source, expectedSource, "manifest source identity");
  if (sha256File(path.join(REPO, "bench/analyze-fixed-samples.js")) !== source.analyzer_sha256) {
    fail("local analyzer does not match the frozen manifest identity");
  }

  const hostFile = path.join(evidenceDirectory, "host.json");
  const hostJson = validateBareMetalHost(config, loadJsonRegular(hostFile, "host.json"));
  const host = requireObject(manifest.host, "manifest.host");
  requireExactKeys(
    host,
    ["file", "sha256", "cpu_model", "microcode", "clocksource", "scaling_governors", "benchmark_cpu"],
    "manifest.host",
  );
  if (host.file !== "host.json" || host.sha256 !== sha256File(hostFile)) {
    fail("manifest host identity does not match host.json");
  }
  const expectedHostManifest = {
    cpu_model: hostJson.cpu_model,
    microcode: hostJson.microcode,
    clocksource: hostJson.clocksource,
    scaling_governors: hostJson.scaling_governors,
    benchmark_cpu: config.host_gate.benchmark_cpu,
  };
  for (const [field, wanted] of Object.entries(expectedHostManifest)) {
    requireJsonEqual(host[field], wanted, `manifest.host.${field}`);
  }
  const protocolManifest = requireObject(manifest.protocol, "manifest.protocol");
  requireExactKeys(
    protocolManifest,
    [
      "workload",
      "fixed_n",
      "repetitions_per_leg",
      "order",
      "common_arguments",
      "bootstrap_repetitions",
      "bootstrap_seed",
      "material_regression_threshold_percent",
      "required_verdict",
      "host_gate",
      "contaminating_process_comm_regex",
      "exclusion_rule",
      "gate_log",
    ],
    "manifest.protocol",
  );
  const expectedProtocol = {
    workload: config.measurement.workload,
    fixed_n: config.measurement.fixed_n,
    repetitions_per_leg: config.measurement.repetitions,
    order: config.measurement.run_order,
    common_arguments: [
      "--engines",
      "rustia",
      "--workloads",
      config.measurement.workload,
      "--fixed-n",
      String(config.measurement.fixed_n),
      "--reps",
      String(config.measurement.repetitions),
      "--no-history",
      "--no-results",
      "--no-latest-json",
    ],
    bootstrap_repetitions: config.measurement.bootstrap_repetitions,
    bootstrap_seed: config.measurement.bootstrap_seed,
    material_regression_threshold_percent: config.measurement.material_regression_threshold_percent,
    required_verdict: config.publication.required_verdict,
    host_gate: config.host_gate,
  };
  for (const [field, wanted] of Object.entries(expectedProtocol)) {
    requireJsonEqual(protocolManifest[field], wanted, `manifest.protocol.${field}`);
  }
  if (
    protocolManifest.contaminating_process_comm_regex !==
      "^(apt|apt-get|as|cargo|cc|cc1|cc1plus|clang|clang\\+\\+|curl|dpkg|g\\+\\+|gcc|git|ld|ld\\.lld|make|ninja|node|rustc|rustup|sccache|script-bench|unattended-upgr)$" ||
    protocolManifest.exclusion_rule !==
      "any pre/during-leg host-gate violation discards the whole eight-leg block; no leg is selected by timing outcome"
  ) {
    fail("manifest protocol process/exclusion rules are not frozen");
  }
  const gateLog = requireObject(protocolManifest.gate_log, "manifest.protocol.gate_log");
  requireExactKeys(gateLog, ["file", "sha256", "records"], "manifest.protocol.gate_log");
  const gateFile = path.join(evidenceDirectory, "gate.jsonl");
  if (gateLog.file !== "gate.jsonl" || gateLog.sha256 !== sha256File(gateFile)) {
    fail("manifest gate log identity does not match gate.jsonl");
  }
  requireInteger(gateLog.records, "manifest.protocol.gate_log.records", 1);
  validateGateLog(config, gateFile, gateLog.records);

  const analysis = requireObject(loadJsonRegular(analysisFile, "fixed-sample analysis"), "fixed-sample analysis");
  const recomputed = await recomputeAnalysis(config, evidenceDirectory);
  requireJsonEqual(normalizedAnalysis(analysis), normalizedAnalysis(recomputed), "fixed-sample analysis");
  if (
    analysis.schema_version !== 1 ||
    analysis.schema !== "rustia.bench.fixed-sample-analysis.v1" ||
    analysis.comparison.verdict.status !== config.publication.required_verdict
  ) {
    fail("fixed-sample analysis did not independently exclude a material regression");
  }
  const analysisIdentity = requireObject(analysis.identity, "fixed-sample analysis.identity");
  const expectedAnalysisIdentity = {
    target: "linux-x86_64",
    engine: "rustia",
    workload: config.measurement.workload,
    fixed_n: config.measurement.fixed_n,
    reps_per_leg: config.measurement.repetitions,
    legs_per_group: config.measurement.expected_legs_per_group,
    checksum: config.measurement.expected_checksum,
  };
  for (const [field, wanted] of Object.entries(expectedAnalysisIdentity)) {
    if (analysisIdentity[field] !== wanted) fail(`fixed-sample analysis.identity.${field} is not frozen`);
  }
  for (const field of ["dialect", "unit"]) requireString(analysisIdentity[field], `fixed-sample analysis.identity.${field}`);
  if (
    analysis.baseline.commit !== config.source.baseline_commit ||
    analysis.candidate.commit !== config.source.candidate_commit ||
    analysis.method.bootstrap_iterations !== config.measurement.bootstrap_repetitions ||
    analysis.method.bootstrap_seed_uint32 !== config.measurement.bootstrap_seed ||
    analysis.method.expected_legs !== config.measurement.expected_legs_per_group ||
    analysis.method.require_full_commits !== true ||
    analysis.comparison.material_regression_threshold_percent !==
      config.measurement.material_regression_threshold_percent
  ) {
    fail("fixed-sample analysis source/method identity is not frozen");
  }
  const qualification = requireObject(manifest.qualification, "manifest.qualification");
  requireExactKeys(
    qualification,
    [
      "schema",
      "pilot_legs",
      "median_spread_percent",
      "maximum_median_spread_percent",
      "passed",
      "note",
    ],
    "manifest.qualification",
  );
  if (
    qualification.schema !== "rustia.bench.cloud-qualification.v1" ||
    qualification.maximum_median_spread_percent !== config.qualification.max_median_spread_percent ||
    qualification.passed !== true ||
    qualification.note !== "qualification-only runs; never accepted as A/B evidence or public history"
  ) {
    fail("manifest qualification does not record the frozen passing host-stability gate");
  }
  if (
    !Array.isArray(qualification.pilot_legs) ||
    qualification.pilot_legs.length !== config.qualification.candidate_pilot_legs
  ) {
    fail("manifest qualification has the wrong number of pilot legs");
  }
  const qualificationDirectory = path.join(evidenceDirectory, "qualification-sidecars");
  const pilotMedians = [];
  qualification.pilot_legs.forEach((summary, index) => {
    const sidecar = validateRawSidecar(
      path.join(qualificationDirectory, `candidate-pilot-${index + 1}.json`),
      config,
      `qualification sidecar ${index + 1}`,
      candidate,
      analysisIdentity,
    );
    requireCompleteSummary(summary, sidecar, config, `manifest qualification.pilot_legs[${index}]`);
    pilotMedians.push(sidecar.measurement.ns_per_unit_median);
  });
  const recomputedSpread =
    ((Math.max(...pilotMedians) - Math.min(...pilotMedians)) / Math.min(...pilotMedians)) * 100;
  if (
    !numbersClose(qualification.median_spread_percent, recomputedSpread) ||
    recomputedSpread > config.qualification.max_median_spread_percent
  ) {
    fail("manifest qualification median spread does not match its pilot sidecars or threshold");
  }
  requireJsonEqual(
    loadJsonRegular(path.join(evidenceDirectory, "qualification.json"), "qualification.json"),
    qualification,
    "qualification.json",
  );
  requireJsonEqual(
    loadJsonRegular(path.join(qualificationDirectory, "qualification.json"), "qualification sidecar summary"),
    qualification,
    "qualification-sidecars/qualification.json",
  );
  const comparison = requireObject(manifest.comparison, "manifest.comparison");
  requireExactKeys(
    comparison,
    [
      "legs",
      "analyzer_command",
      "analysis_sha256",
      "candidate_delta_percent",
      "pooled_bootstrap_ci_95_percent",
      "hierarchical_bootstrap_ci_95_percent",
      "conservative_bootstrap_ci_95_percent",
      "verdict",
    ],
    "manifest.comparison",
  );
  if (comparison.analysis_sha256 !== sha256File(analysisFile)) fail("manifest analysis SHA-256 is wrong");
  requireJsonEqual(comparison.verdict, analysis.comparison.verdict, "manifest comparison verdict");
  if (comparison.verdict.status !== config.publication.required_verdict) {
    fail("manifest does not contain the required material_regression_excluded verdict");
  }
  const comparisonMetrics = {
    candidate_delta_percent: analysis.comparison.candidate_delta_percent,
    pooled_bootstrap_ci_95_percent: analysis.comparison.pooled_bootstrap_ci_95_percent,
    hierarchical_bootstrap_ci_95_percent: analysis.comparison.hierarchical_bootstrap_ci_95_percent,
    conservative_bootstrap_ci_95_percent: analysis.comparison.bootstrap_ci_95_percent,
  };
  for (const [field, wanted] of Object.entries(comparisonMetrics)) {
    requireJsonEqual(comparison[field], wanted, `manifest.comparison.${field}`);
  }
  const relativeEvidence = `bench/history/evidence/${candidate}`;
  const expectedAnalyzerCommand = ["node", "bench/analyze-fixed-samples.js"];
  for (const leg of config.measurement.run_order.filter((entry) => entry.variant === "baseline")) {
    expectedAnalyzerCommand.push("--baseline", `${relativeEvidence}/${leg.file}`);
  }
  for (const leg of config.measurement.run_order.filter((entry) => entry.variant === "candidate")) {
    expectedAnalyzerCommand.push("--candidate", `${relativeEvidence}/${leg.file}`);
  }
  expectedAnalyzerCommand.push(
    "--bootstrap-reps",
    String(config.measurement.bootstrap_repetitions),
    "--seed",
    String(config.measurement.bootstrap_seed),
    "--material-threshold",
    String(config.measurement.material_regression_threshold_percent),
    "--expected-legs",
    String(config.measurement.expected_legs_per_group),
    "--require-full-commits",
  );
  requireJsonEqual(comparison.analyzer_command, expectedAnalyzerCommand, "manifest comparison analyzer command");
  if (!Array.isArray(comparison.legs) || comparison.legs.length !== config.measurement.run_order.length) {
    fail("manifest comparison leg count is wrong");
  }
  config.measurement.run_order.forEach((leg, index) => {
    const expectedCommit = leg.variant === "baseline" ? config.source.baseline_commit : candidate;
    const sidecar = validateRawSidecar(
      path.join(evidenceDirectory, leg.file),
      config,
      `comparison sidecar ${leg.file}`,
      expectedCommit,
      analysisIdentity,
    );
    requireCompleteSummary(
      comparison.legs[index],
      sidecar,
      config,
      `manifest comparison.legs[${index}]`,
      leg.variant,
    );
  });

  const historyDestination = path.join(REPO, "bench/history/runs.jsonl");
  const localHistory = fs.readFileSync(historyDestination);
  const remoteHistory = fs.readFileSync(historyFile);
  const history = validatePublishedHistory(config, localHistory, remoteHistory, evidenceDirectory, analysisIdentity);
  const publication = requireObject(manifest.publication, "manifest.publication");
  requireExactKeys(
    publication,
    [
      "performed",
      "history_before_sha256",
      "history_after_sha256",
      "history_lines_before",
      "history_lines_after",
      "sidecars",
      "appended_rows",
      "commands",
    ],
    "manifest.publication",
  );
  if (publication.performed !== true) fail("manifest publication was not performed");
  const expectedPublication = {
    history_before_sha256: config.source.expected_sha256.history_before,
    history_after_sha256: history.afterHash,
    history_lines_before: history.beforeLines,
    history_lines_after: history.beforeLines + config.publication.final_measurements,
  };
  for (const [field, wanted] of Object.entries(expectedPublication)) {
    if (publication[field] !== wanted) fail(`manifest publication.${field} does not match published history`);
  }
  requireJsonEqual(publication.appended_rows, history.rows, "manifest publication appended rows");
  if (!Array.isArray(publication.sidecars) || publication.sidecars.length !== history.sidecars.length) {
    fail("manifest publication sidecar count is wrong");
  }
  history.sidecars.forEach((sidecar, index) =>
    requireCompleteSummary(publication.sidecars[index], sidecar, config, `manifest publication.sidecars[${index}]`),
  );
  const expectedPublicationCommands = Array.from(
    { length: config.publication.final_measurements },
    (_, index) => [
      "taskset",
      "-c",
      String(config.host_gate.benchmark_cpu),
      "/opt/rustia/target-b/release/script-bench",
      "--engines",
      "rustia",
      "--workloads",
      config.measurement.workload,
      "--fixed-n",
      String(config.measurement.fixed_n),
      "--reps",
      String(config.measurement.repetitions),
      "--history",
      "bench/history/runs.jsonl",
      "--no-results",
      "--no-latest-json",
      "--commit",
      candidate,
      "--samples-json",
      `${relativeEvidence}/final-dense-${index + 1}.json`,
    ],
  );
  requireJsonEqual(publication.commands, expectedPublicationCommands, "manifest publication commands");
  return { evidenceDirectory, historyFile, localHistory, remoteHistory };
}

async function promoteAcceptedResult(config, state, downloadDirectory) {
  if (!state.scaleway_destroyed_at || !state.ssh_key_destroyed_at) {
    fail("refusing publication before Scaleway has confirmed both server and SSH-key deletion");
  }
  throwIfInterrupted();
  const statusFile = path.join(downloadDirectory, "status.json");
  if (!fs.existsSync(statusFile)) fail("remote export did not contain status.json");
  const status = loadJsonRegular(statusFile, "remote status");
  if (status.status !== "accepted_and_published") {
    state.remote_status = status.status;
    state.remote_status_detail = status.detail || null;
    saveState(state);
    process.stdout.write(
      `Scaleway run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
        `(${status.status}${status.detail ? `: ${status.detail}` : ""}).\n`,
    );
    return false;
  }
  await verifyPromotionCheckout(config, state);

  const candidate = config.source.candidate_commit;
  const validated = await validateAcceptedExport(config, state, downloadDirectory, status);
  state.remote_status = status.status;
  state.remote_status_detail = status.detail;
  saveState(state);
  const evidenceSource = validated.evidenceDirectory;
  const evidenceDestination = path.join(REPO, "bench/history/evidence", candidate);
  const historySource = validated.historyFile;
  const historyDestination = path.join(REPO, "bench/history/runs.jsonl");
  if (fs.existsSync(evidenceDestination)) {
    fail(`refusing to overwrite existing evidence directory ${evidenceDestination}`);
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
    return { file, existed: true, bytes: fs.readFileSync(file), mode: fs.statSync(file).mode & 0o777 };
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
    if (evidenceCommitted) fs.rmSync(evidenceDestination, { recursive: true, force: true });
    if (historyCommitted) {
      for (const snapshot of snapshots) {
        if (snapshot.existed) writeBytesAtomic(snapshot.file, snapshot.bytes, snapshot.mode);
        else fs.rmSync(snapshot.file, { force: true });
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

async function benchLocked(config, configFile = DEFAULT_CONFIG) {
  const { credentials, plan, provider, source } = await buildPlan(config, { print: true, configFile });
  fs.mkdirSync(RUNS_ROOT, { recursive: true, mode: 0o700 });
  const runId = `${safeTimestamp()}-${crypto.randomBytes(4).toString("hex")}`;
  const directory = path.join(RUNS_ROOT, runId);
  fs.mkdirSync(directory, { recursive: false, mode: 0o700 });
  const shortCandidate = config.source.candidate_commit.slice(0, 8);
  const scalewayLabel = `rustia-bench-${shortCandidate}-${crypto.randomBytes(4).toString("hex")}`;
  const ownershipTag = `rustia-owner-${crypto.randomBytes(12).toString("hex")}`;
  const sshKeyName = `rustia-key-${shortCandidate}-${crypto.randomBytes(4).toString("hex")}`;
  const state = {
    __directory: directory,
    schema: "rustia.scaleway-benchmark-state.v1",
    run_id: runId,
    created_at: new Date().toISOString(),
    protocol_mode: "acceptance",
    project_id: credentials.project_id,
    scaleway_label: scalewayLabel,
    scaleway_description: `rustia fixed acceptance benchmark ${runId}`,
    scaleway_ownership_tag: ownershipTag,
    scaleway_id: null,
    scaleway_destroyed_at: null,
    ssh_key_name: sshKeyName,
    ssh_key_id: null,
    ssh_key_destroyed_at: null,
    private_key_path: path.join(directory, "id_ed25519"),
    public_key_path: path.join(directory, "id_ed25519.pub"),
    ssh_host_private_key_path: path.join(directory, "ssh_host_ed25519_key"),
    ssh_host_public_key_path: path.join(directory, "ssh_host_ed25519_key.pub"),
    ssh_host_key_alias: scalewayLabel,
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
      ["-q", "-t", "ed25519", "-N", "", "-C", `${scalewayLabel}-key`, "-f", state.private_key_path],
      { capture: false, announce: true },
    );
    fs.chmodSync(state.private_key_path, 0o600);
    await run(
      "ssh-keygen",
      [
        "-q",
        "-t",
        "ed25519",
        "-N",
        "",
        "-C",
        `${scalewayLabel}-host-key`,
        "-f",
        state.ssh_host_private_key_path,
      ],
      { capture: false, announce: true },
    );
    fs.chmodSync(state.ssh_host_private_key_path, 0o600);
    const hostPublicKey = fs.readFileSync(state.ssh_host_public_key_path, "utf8").trim();
    const derivedHostPublicKey = outputText(
      await run("ssh-keygen", ["-y", "-f", state.ssh_host_private_key_path]),
    );
    if (
      parseEd25519PublicKey(hostPublicKey, "generated SSH host public key").base64 !==
      parseEd25519PublicKey(derivedHostPublicKey, "derived SSH host public key").base64
    ) {
      fail("generated SSH host private and public keys do not match");
    }
    writePinnedKnownHost(state, hostPublicKey);
    await run("git", ["bundle", "create", state.bundle_path, "HEAD"], { announce: true });
    await run("git", ["bundle", "verify", state.bundle_path], { capture: false, announce: true });
    state.bundle_sha256 = sha256File(state.bundle_path);
    saveState(state);

    const publicKey = fs.readFileSync(state.public_key_path, "utf8").trim();
    state.ssh_public_key = publicKey;
    if (ownershipRelatedSshKeys(await listSshKeys(credentials), state).length !== 0) {
      fail("refusing to create because the unique Scaleway SSH-key name already exists");
    }
    state.ssh_key_precreate_absence_confirmed_at = new Date().toISOString();
    const sshKeyBody = createSshKeyBody(state, publicKey);
    state.ssh_key_create_request_sha256 = sha256(Buffer.from(JSON.stringify(sshKeyBody)));
    const sshKeyCreateStarted = Date.now();
    state.ssh_key_create_attempted_at = new Date(sshKeyCreateStarted).toISOString();
    state.ssh_key_create_request_deadline_at = new Date(sshKeyCreateStarted + API_TIMEOUT_MS).toISOString();
    state.ssh_key_create_outcome = "attempting";
    saveState(state);
    let sshKeyResponse = null;
    let sshKeyError = null;
    try {
      sshKeyResponse = await apiRequest(credentials.secret_key, "POST", "/iam/v1alpha1/ssh-keys", {
        body: sshKeyBody,
        acceptedStatuses: [200],
      });
      state.ssh_key_create_outcome = "accepted_response";
    } catch (error) {
      sshKeyError = error;
      state.ssh_key_create_outcome = createOutcomeForError(error);
    }
    state.ssh_key_create_finished_at = new Date().toISOString();
    saveState(state);
    if (sshKeyResponse) {
      const createdKey = requireApiObject(sshKeyResponse, "Scaleway SSH-key create response");
      if (directCreateSshKeyMatches(createdKey, state, { requireJournaledId: false })) {
        state.ssh_key_id = createdKey.id;
        state.ssh_key_id_source = "create_response";
        state.ssh_key_create_outcome = "resource_identified";
        saveState(state);
      }
      if (!state.ssh_key_id) {
        const reconciledKey = await reconcileSshKeyAfterCreateAttempt(state, credentials);
        if (!reconciledKey) fail("Scaleway SSH-key create returned no uniquely reconcilable key");
      }
    } else {
      const reconciledKey = await reconcileSshKeyAfterCreateAttempt(state, credentials);
      if (!reconciledKey) {
        fail(`Scaleway SSH-key creation failed: ${sshKeyError ? sshKeyError.message : "unknown API error"}`);
      }
    }

    // Repeat every cost/scope-sensitive check immediately before the first billable request.
    const refreshedProvider = await resolveProviderPlan(credentials, config);
    const refreshedPlan = sanitizedProviderPlan(config, refreshedProvider, source);
    verifyProviderPlanUnchanged(plan, refreshedPlan);
    await inspectIamPermissions(credentials);
    await verifyDeleteAuthority(credentials, refreshedProvider);
    await getOwnedSshKey(state, credentials);
    recordCleanupEvidence(state, "ssh_key_positive_observed_at", () => Date.now(), saveState);
    if (ownershipRelatedScaleways(await listScaleways(credentials), state).length !== 0) {
      fail("refusing to create because the unique Scaleway server name or tag already exists");
    }
    state.scaleway_precreate_absence_confirmed_at = new Date().toISOString();
    saveState(state);

    const hostPrivateKey = fs.readFileSync(state.ssh_host_private_key_path, "utf8");
    const createBody = createScalewayBody(state, publicKey, hostPrivateKey, hostPublicKey);
    state.scaleway_create_request_sha256 = sha256(Buffer.from(JSON.stringify(createBody)));
    saveState(state);
    startBillingWindow(state, config);
    const createStarted = Date.now();
    state.scaleway_create_attempted_at = new Date(createStarted).toISOString();
    state.scaleway_create_request_deadline_at = new Date(createStarted + API_TIMEOUT_MS).toISOString();
    state.scaleway_create_outcome = "attempting";
    saveState(state);

    let createResponse = null;
    let createError = null;
    try {
      createResponse = await apiRequest(
        credentials.secret_key,
        "POST",
        `/baremetal/v1/zones/${plan.zone}/servers`,
        {
        body: createBody,
          acceptedStatuses: [200],
        },
      );
      state.scaleway_create_outcome = "accepted_response";
    } catch (error) {
      createError = error;
      state.scaleway_create_outcome = createOutcomeForError(error);
    }
    state.scaleway_create_finished_at = new Date().toISOString();
    saveState(state);
    if (createResponse) {
      const created = bareMetalFromCreateResponse(createResponse);
      if (directCreateScalewayMatches(created, state, { requireJournaledId: false })) {
        state.scaleway_id = created.id;
        state.scaleway_id_source = "create_response";
        state.scaleway_create_outcome = "resource_identified";
        saveState(state);
      }
      if (!state.scaleway_id) {
        const reconciled = await reconcileScalewayAfterCreateAttempt(state, credentials);
        if (!reconciled) fail("Scaleway create returned no uniquely reconcilable owned server");
      }
    } else {
      const reconciled = await reconcileScalewayAfterCreateAttempt(state, credentials);
      if (!reconciled) fail(`Scaleway creation failed: ${createError ? createError.message : "unknown API error"}`);
    }

    const scaleway = await waitForScaleway(state, credentials);
    verifyScalewayMatchesPlan(scaleway, plan, state);
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
      { announce: true, timeoutMs: remainingWorkTimeout(state, 60 * 1000, "remote directory setup") },
    );

    const providerMetadata = path.join(directory, "provider.json");
    writeJsonAtomic(providerMetadata, safeScalewayMetadata(scaleway, plan, state));
    await run(
      "scp",
      [
        ...sshOptions(state),
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
      [...sshOptions(state), `root@${state.public_ipv4}:/opt/rustia/export.tar.gz`, archive],
      {
        capture: false,
        allowFailure: true,
        announce: true,
        timeoutMs: remainingWorkTimeout(state, 5 * 60 * 1000, "result retrieval"),
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
      await cleanupOwned(state, credentials);
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
  if (!state.scaleway_destroyed_at || !state.ssh_key_destroyed_at) {
    fail("Scaleway server and SSH-key deletion were not both confirmed");
  }
  throwIfInterrupted();
  const downloadDirectory = path.join(directory, "download");
  await extractArchive(state.export_path, downloadDirectory);
  const promoted = await promoteAcceptedResult(config, state, downloadDirectory);
  if (!promoted && state.remote_exit_status !== 0) {
    fail(`remote benchmark ended with status ${state.remote_exit_status}; diagnostics were retained`);
  }
}

async function bench(config, configFile = DEFAULT_CONFIG) {
  const lockToken = acquireCoordinatorLock();
  try {
    const incomplete = incompleteStates();
    if (incomplete.length > 0) {
      fail(
        "incomplete Scaleway benchmark cleanup exists; run status and clean up: " +
          incomplete.map((state) => state.run_id).join(", "),
      );
    }
    throwIfInterrupted();
    return await benchLocked(config, configFile);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function authenticate(config) {
  const lockToken = acquireCoordinatorLock();
  try {
    const projectId = await readTtyLine("Scaleway Project ID: ");
    if (!UUID.test(projectId)) fail("Scaleway project ID must be a lowercase UUID");
    const mismatchedRuns = incompleteStates().filter((state) => state.project_id !== projectId);
    if (mismatchedRuns.length > 0) {
      fail(
        "refusing to replace Scaleway credentials with a different project while cleanup is incomplete: " +
          mismatchedRuns.map((state) => state.run_id).join(", "),
      );
    }
    const secretKey = await readTtyLine("Scaleway secret key (input hidden): ", { hidden: true });
    const credentials = validateCredentials({
      schema: "rustia.scaleway-credentials.v1",
      project_id: projectId,
      secret_key: secretKey,
    });
    const provider = await resolveProviderPlan(credentials, config);
    await inspectIamPermissions(credentials);
    await verifyDeleteAuthority(credentials, provider);
    writeJsonAtomic(CREDENTIALS_FILE, credentials, 0o600);
    process.stdout.write(`Scaleway credentials validated and stored with mode 0600 at ${CREDENTIALS_FILE}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function status() {
  const states = listStates();
  if (states.length === 0) {
    process.stdout.write("No locally owned Scaleway benchmark runs.\n");
    return;
  }
  const credentials = readStoredCredentials({ allowMissing: true });
  const rows = [];
  for (const state of states) {
    let remote = null;
    let remoteSshKey = null;
    let projectError = null;
    if (credentials) {
      try {
        requireCredentialsForState(state, credentials);
      } catch (error) {
        projectError = error.message;
      }
    }
    if (!state.scaleway_destroyed_at) {
      if (!credentials) {
        remote = { error: "stored credentials unavailable; run auth from an authenticated host" };
      } else if (projectError) {
        remote = { error: projectError };
      } else {
        try {
          if (state.scaleway_id) {
            const scaleway = await getOwnedScaleway(state, credentials, { allowMissing: true });
            remote = scaleway
              ? { id: scaleway.id, name: scaleway.name, status: scaleway.status, public_ipv4: publicIpv4(scaleway) }
              : { status: "not returned by get; run destroy to reconcile safely" };
          } else {
            const related = ownershipRelatedScaleways(await listScaleways(credentials), state);
            const matches = related.filter((entry) => exactOwnedScaleway(entry, state));
            remote =
              related.length === 1 && matches.length === 1
                ? {
                    id: matches[0].id,
                    name: matches[0].name,
                    status: matches[0].status,
                    public_ipv4: publicIpv4(matches[0]),
                    note: "found by the journaled unique name/tag; run destroy to adopt and clean it safely",
                  }
                : related.length === 0
                  ? { status: "unique label absent from a complete list; run destroy to confirm cleanup" }
                  : {
                      error: `ownership label/tag returned ${related.length} related and ${matches.length} exact Scaleway resources; manual review required`,
                    };
          }
        } catch (error) {
          remote = { error: error.message };
        }
      }
    }
    if (!state.ssh_key_destroyed_at) {
      if (!credentials) {
        remoteSshKey = { error: "stored credentials unavailable; run auth from an authenticated host" };
      } else if (projectError) {
        remoteSshKey = { error: projectError };
      } else {
        try {
          if (state.ssh_key_id) {
            const key = await getOwnedSshKey(state, credentials, { allowMissing: true });
            remoteSshKey = key
              ? { id: key.id, name: key.name, disabled: key.disabled }
              : { status: "not returned by get; run destroy to reconcile safely" };
          } else {
            const related = ownershipRelatedSshKeys(await listSshKeys(credentials), state);
            const matches = related.filter((entry) => exactOwnedSshKey(entry, state));
            remoteSshKey =
              related.length === 1 && matches.length === 1
                ? { id: matches[0].id, name: matches[0].name, note: "found by unique key name; run destroy" }
                : related.length === 0
                  ? { status: "unique key name absent; run destroy to confirm cleanup" }
                  : { error: `key name returned ${related.length} related and ${matches.length} exact resources` };
          }
        } catch (error) {
          remoteSshKey = { error: error.message };
        }
      }
    }
    const started = Date.parse(state.billing_window_started_at || "");
    const ended = Date.parse(state.scaleway_destroyed_at || "");
    const seconds = Number.isFinite(started)
      ? Math.max(0, ((Number.isFinite(ended) ? ended : Date.now()) - started) / 1000)
      : 0;
    const hourly = state.plan && state.plan.size ? state.plan.size.price_hourly_eur : null;
    rows.push({
      run_id: state.run_id,
      created_at: state.created_at,
      scaleway_id: state.scaleway_id,
      scaleway_label: state.scaleway_label,
      scaleway_create_attempted_at: state.scaleway_create_attempted_at || null,
      scaleway_create_outcome: state.scaleway_create_outcome || null,
      scaleway_absence_confirmed_at: state.scaleway_absence_confirmed_at || null,
      scaleway_positive_observed_at: state.scaleway_positive_observed_at || null,
      scaleway_delete_accepted_at: state.scaleway_delete_accepted_at || null,
      ssh_key_id: state.ssh_key_id || null,
      ssh_key_name: state.ssh_key_name || null,
      ssh_key_create_outcome: state.ssh_key_create_outcome || null,
      ssh_key_positive_observed_at: state.ssh_key_positive_observed_at || null,
      ssh_key_delete_accepted_at: state.ssh_key_delete_accepted_at || null,
      ssh_key_destroyed_at: state.ssh_key_destroyed_at || null,
      destroyed_at: state.scaleway_destroyed_at,
      cleanup_complete: Boolean(state.scaleway_destroyed_at && state.ssh_key_destroyed_at),
      billing_deadline_at: state.billing_deadline_at || null,
      billable_seconds_upper_bound: Number(seconds.toFixed(3)),
      rounded_compute_charge_upper_bound_eur:
        typeof hourly === "number" ? roundedHourlyCharge(hourly, seconds) : null,
      remote_status: state.remote_status || null,
      promoted_at: state.promoted_at || null,
      remote,
      remote_ssh_key: remoteSshKey,
    });
  }
  process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
}

async function destroy(runId) {
  const lockToken = acquireCoordinatorLock();
  try {
    const credentials = readStoredCredentials();
    const state = stateForRunId(runId);
    cleanupInProgress = true;
    try {
      await cleanupOwned(state, credentials);
    } finally {
      cleanupInProgress = false;
    }
    process.stdout.write(`Confirmed cleanup for owned Scaleway run ${state.run_id}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}


async function selfTest(config) {
  assert.equal(sha256(Buffer.from(canonicalJson(config), "utf8")), FROZEN_CONFIG_SHA256);
  assert.equal(config.schema, "rustia.scaleway-fixed-benchmark.v1");
  assert.deepEqual(config.provider.zones, SCALEWAY_ZONES);
  assert.deepEqual(config.provider.allowed_stock, ["available", "low"]);
  assert.equal(config.provider.size.name, "EM-A610R-NVMe");
  assert.equal(config.provider.size.host_cpu_model, "AMD Ryzen 5 PRO 3600");
  assert.equal(config.provider.size.cpu_threads, 12);
  assert.equal(config.provider.max_runtime_minutes, 110);
  assert.deepEqual(
    config.measurement.run_order.map((entry) => entry.variant),
    ["baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate"],
  );
  assert.equal(config.host_gate.maximum_steal_ticks, 0);
  assert.equal(config.publication.required_verdict, "material_regression_excluded");
  assert.equal(config.publication.final_measurements, 3);
  assert.throws(() =>
    validateConfig({
      ...config,
      provider: { ...config.provider, max_runtime_minutes: 109 },
    }),
  );
  for (const status of [400, 403, 422]) {
    assert.equal(createOutcomeForError(new ScalewayApiError("fixture", status)), "definitively_rejected");
  }
  for (const status of [408, 409, 425, 429, 499, 500, null]) {
    assert.equal(createOutcomeForError(new ScalewayApiError("fixture", status)), "ambiguous");
  }
  assert.equal(createOutcomeForError({}), "ambiguous");

  const credentials = validateCredentials({
    schema: "rustia.scaleway-credentials.v1",
    project_id: "00000000-0000-4000-8000-000000000001",
    secret_key: "fixture-secret-key",
  });
  const money = (nanos) => ({ currency_code: "EUR", units: 0, nanos });
  const offerFixture = (zone, stock, id) => ({
    id,
    name: config.provider.size.name,
    zone,
    stock,
    subscription_period: "hourly",
    enable: true,
    cpus: [
      {
        name: "AMD Ryzen PRO 3600 6-Core Processor",
        core_count: 6,
        thread_count: 12,
        frequency: 3600,
      },
    ],
    memories: [
      {
        capacity: config.provider.size.memory_bytes,
        type: "DDR4",
        frequency: 3200,
        is_ecc: true,
      },
    ],
    disks: Array.from({ length: 2 }, () => ({
      capacity: config.provider.size.minimum_disk_bytes,
      type: "NVMe",
    })),
    price_per_hour: money(110_000_000),
    fee: money(0),
    gpus: [],
    incompatible_os_ids: [],
  });
  const lowOffer = offerFixture(
    "nl-ams-1",
    "low",
    "00000000-0000-4000-8000-000000000101",
  );
  const availableOffer = offerFixture(
    "nl-ams-2",
    "available",
    "00000000-0000-4000-8000-000000000102",
  );
  const offers = new Map([
    [lowOffer.zone, lowOffer],
    [availableOffer.zone, availableOffer],
  ]);
  const operatingSystemFixture = (zone, offer) => ({
    id: zone === "nl-ams-1"
      ? "00000000-0000-4000-8000-000000000201"
      : "00000000-0000-4000-8000-000000000202",
    name: config.provider.os.name,
    version: config.provider.os.version,
    zone,
    enabled: true,
    allowed: true,
    license_required: false,
    cloud_init_supported: true,
    cloud_init_version: "24.4",
    offer_id: offer.id,
  });
  const catalogCalls = [];
  const catalogRequest = async (secretKey, method, apiPath, options) => {
    assert.equal(secretKey, credentials.secret_key);
    assert.equal(method, "GET");
    assert.deepEqual(options.acceptedStatuses, [200]);
    const url = new URL(apiPath, "https://api.scaleway.com");
    const match = /^\/baremetal\/v1\/zones\/([^/]+)\/(offers|os)$/.exec(url.pathname);
    assert.ok(match, `unexpected catalog fixture path ${apiPath}`);
    assert.equal(url.searchParams.get("page"), "1");
    assert.equal(url.searchParams.get("page_size"), "100");
    const [, zone, collection] = match;
    catalogCalls.push([zone, collection]);
    const offer = offers.get(zone);
    if (collection === "offers") {
      assert.equal(url.searchParams.get("subscription_period"), "hourly");
      assert.equal(url.searchParams.get("name"), config.provider.size.name);
      const rows = offer ? [offer] : [];
      return { status: 200, data: { total_count: rows.length, offers: rows } };
    }
    assert.ok(offer, `OS catalog queried without an offer in ${zone}`);
    assert.equal(url.searchParams.get("offer_id"), offer.id);
    const rows = [operatingSystemFixture(zone, offer)];
    return { status: 200, data: { total_count: rows.length, os: rows } };
  };
  const provider = await resolveProviderPlan(credentials, config, catalogRequest);
  assert.equal(provider.zone, "nl-ams-2", "available stock must outrank earlier low stock");
  assert.equal(provider.offer.id, availableOffer.id);
  assert.equal(catalogCalls.filter(([, collection]) => collection === "offers").length, SCALEWAY_ZONES.length);
  assert.equal(catalogCalls.filter(([, collection]) => collection === "os").length, 2);

  const plan = sanitizedProviderPlan(config, provider, { fixture: true });
  assert.equal(plan.provider, "scaleway");
  assert.equal(plan.zone, "nl-ams-2");
  assert.equal(plan.size.resource_type, "elastic_metal");
  assert.equal(plan.size.vcpus, 12);
  assert.equal(plan.size.price_hourly_eur, 0.11);
  assert.equal(plan.planned_compute_cost_ceiling_eur, 0.22);
  verifyProviderPlanUnchanged(plan, JSON.parse(JSON.stringify(plan)));
  assert.throws(() =>
    verifyProviderPlanUnchanged(plan, {
      ...plan,
      size: { ...plan.size, stock: "low" },
    }),
  );

  const keyRows = Array.from({ length: 101 }, (_, index) => ({
    id: `00000000-0000-4000-8000-${String(index + 1).padStart(12, "0")}`,
    project_id: credentials.project_id,
  }));
  const keyPages = [];
  const listedKeys = await listSshKeys(credentials, async (secretKey, method, apiPath, options) => {
    assert.equal(secretKey, credentials.secret_key);
    assert.equal(method, "GET");
    assert.deepEqual(options.acceptedStatuses, [200]);
    const url = new URL(apiPath, "https://api.scaleway.com");
    assert.equal(url.pathname, "/iam/v1alpha1/ssh-keys");
    assert.equal(url.searchParams.get("page_size"), "100");
    assert.equal(url.searchParams.get("project_id"), credentials.project_id);
    const page = Number(url.searchParams.get("page"));
    keyPages.push(page);
    const rows = page === 1 ? keyRows.slice(0, 100) : keyRows.slice(100);
    return { status: 200, data: { total_count: keyRows.length, ssh_keys: rows } };
  });
  assert.equal(listedKeys.length, 101);
  assert.deepEqual(keyPages, [1, 2]);

  const operationalCalls = [];
  const emptyOperationalRequest = async (secretKey, method, apiPath, options) => {
    assert.equal(secretKey, credentials.secret_key);
    operationalCalls.push([method, apiPath]);
    if (method === "DELETE") {
      assert.deepEqual(options.acceptedStatuses, [404]);
      return { status: 404, data: { message: "not found" } };
    }
    assert.equal(method, "GET");
    assert.deepEqual(options.acceptedStatuses, [200]);
    const url = new URL(apiPath, "https://api.scaleway.com");
    if (url.pathname === "/iam/v1alpha1/ssh-keys") {
      return { status: 200, data: { total_count: 0, ssh_keys: [] } };
    }
    assert.match(url.pathname, /^\/baremetal\/v1\/zones\/[^/]+\/servers$/);
    assert.equal(url.searchParams.get("project_id"), credentials.project_id);
    return { status: 200, data: { total_count: 0, servers: [] } };
  };
  await inspectIamPermissions(credentials, emptyOperationalRequest, { announce: false });
  await verifyDeleteAuthority(credentials, provider, emptyOperationalRequest, { announce: false });
  assert.equal(operationalCalls.filter(([method]) => method === "DELETE").length, 2);

  const publicKey = `ssh-ed25519 ${Buffer.alloc(32, 7).toString("base64")} rustia-client-test`;
  const hostPublicKey = `ssh-ed25519 ${Buffer.alloc(32, 8).toString("base64")} rustia-host-test`;
  const hostPrivateKey = [
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    Buffer.alloc(96, 9).toString("base64"),
    "-----END OPENSSH PRIVATE KEY-----",
    "",
  ].join("\n");
  const state = {
    project_id: credentials.project_id,
    plan,
    scaleway_id: "00000000-0000-4000-8000-000000000301",
    scaleway_label: "rustia-bench-self-test",
    scaleway_description: "rustia benchmark self-test",
    scaleway_ownership_tag: "rustia-owner-self-test",
    scaleway_create_attempted_at: "2026-07-19T04:00:00.000Z",
    scaleway_create_finished_at: "2026-07-19T04:00:02.000Z",
    ssh_key_id: "00000000-0000-4000-8000-000000000302",
    ssh_key_name: "rustia-bench-self-test-key",
    ssh_key_create_attempted_at: "2026-07-19T03:59:55.000Z",
    ssh_key_create_finished_at: "2026-07-19T03:59:57.000Z",
    ssh_public_key: publicKey,
    public_ipv4: "8.8.8.8",
  };
  assert.deepEqual(createSshKeyBody(state, publicKey), {
    name: state.ssh_key_name,
    public_key: publicKey,
    project_id: credentials.project_id,
  });
  const body = createScalewayBody(state, publicKey, hostPrivateKey, hostPublicKey);
  assert.deepEqual(Object.keys(body).sort(), [
    "description",
    "install",
    "name",
    "offer_id",
    "option_ids",
    "project_id",
    "protected",
    "tags",
    "user_data",
  ]);
  assert.deepEqual(body.install.ssh_key_ids, [state.ssh_key_id]);
  assert.deepEqual(body.option_ids, []);
  assert.equal(body.protected, false);
  assert.equal(typeof body.user_data, "string");
  const decodedUserData = Buffer.from(body.user_data, "base64").toString("utf8");
  assert.match(decodedUserData, /^#cloud-config\n/);
  assert.match(decodedUserData, /ssh_pwauth: false/);
  assert.match(decodedUserData, new RegExp(publicKey.split(" ")[1]));
  assert.match(decodedUserData, new RegExp(hostPublicKey.split(" ")[1]));
  assert.equal(decodedUserData.includes(credentials.secret_key), false);
  assert.equal(sha256(Buffer.from(decodedUserData, "utf8")), state.cloud_init_sha256);

  const server = {
    id: state.scaleway_id,
    project_id: state.project_id,
    name: state.scaleway_label,
    description: state.scaleway_description,
    zone: plan.zone,
    offer_id: plan.size.id,
    offer_name: plan.size.name,
    created_at: "2026-07-19T04:00:01.000Z",
    status: "ready",
    boot_type: "normal",
    protected: false,
    tags: ["rustia-benchmark", state.scaleway_ownership_tag],
    ips: [{ version: "IPv4", address: state.public_ipv4 }],
    install: {
      os_id: plan.os.id,
      hostname: state.scaleway_label,
      status: "completed",
      ssh_key_ids: [state.ssh_key_id],
      user: "root",
    },
    user_data: body.user_data,
  };
  assert.equal(exactOwnedScaleway(server, state), true);
  verifyOwnedScalewayIdentity(server, state);
  verifyScalewayMatchesPlan(server, plan, state);
  assert.equal(exactOwnedScaleway({ ...server, tags: ["rustia-benchmark"] }, state), false);
  assert.throws(() => verifyScalewayMatchesPlan({ ...server, offer_id: "wrong" }, plan, state));
  const safeMetadata = safeScalewayMetadata(server, plan, state);
  assert.equal(Object.prototype.hasOwnProperty.call(safeMetadata, "user_data"), false);
  assert.equal(JSON.stringify(safeMetadata).includes(hostPrivateKey), false);

  const sshKey = {
    id: state.ssh_key_id,
    project_id: state.project_id,
    name: state.ssh_key_name,
    public_key: state.ssh_public_key,
    disabled: false,
    created_at: "2026-07-19T03:59:56.000Z",
  };
  assert.equal(exactOwnedSshKey(sshKey, state), true);
  assert.equal(exactOwnedSshKey({ ...sshKey, public_key: sshKey.public_key.split(" ").slice(0, 2).join(" ") }, state), true);
  assert.equal(exactOwnedSshKey({ ...sshKey, disabled: true }, state), false);

  const directState = {
    ...state,
    scaleway_id_source: "create_response",
    ssh_key_id_source: "create_response",
  };
  const nullCreatedServer = { ...server, created_at: null };
  const nullCreatedKey = { ...sshKey, created_at: null };
  assert.equal(exactOwnedScaleway(nullCreatedServer, directState), false);
  assert.equal(cleanupOwnedScalewayMatches(nullCreatedServer, directState), true);
  assert.equal(directCreateScalewayMatches(nullCreatedServer, directState), true);
  assert.equal(directCreateScalewayMatches({ ...server, created_at: "2000-01-01T00:00:00.000Z" }, directState), true);
  assert.equal(
    directCreateScalewayMatches(nullCreatedServer, { ...directState, scaleway_id: null }, {
      requireJournaledId: false,
    }),
    true,
  );
  assert.equal(exactOwnedSshKey(nullCreatedKey, directState), false);
  assert.equal(cleanupOwnedSshKeyMatches(nullCreatedKey, directState), true);
  assert.equal(directCreateSshKeyMatches(nullCreatedKey, directState), true);
  assert.equal(directCreateSshKeyMatches({ ...sshKey, created_at: "2000-01-01T00:00:00.000Z" }, directState), true);
  assert.equal(
    directCreateSshKeyMatches(nullCreatedKey, { ...directState, ssh_key_id: null }, {
      requireJournaledId: false,
    }),
    true,
  );
  assert.equal(
    directCreateSshKeyMatches({ ...nullCreatedKey, public_key: null }, directState),
    true,
  );
  assert.equal(
    directCreateSshKeyMatches({ ...nullCreatedKey, public_key: "ssh-ed25519 invalid" }, directState),
    false,
  );
  const directServerRead = await getOwnedScaleway(directState, credentials, {
    request: async (secretKey, method, apiPath, options) => {
      assert.equal(secretKey, credentials.secret_key);
      assert.equal(method, "GET");
      assert.equal(apiPath, `/baremetal/v1/zones/${plan.zone}/servers/${state.scaleway_id}`);
      assert.deepEqual(options.acceptedStatuses, [200, 404]);
      return { status: 200, data: nullCreatedServer };
    },
  });
  assert.equal(directServerRead.id, state.scaleway_id);
  const directKeyRead = await getOwnedSshKey(directState, credentials, {
    request: async (secretKey, method, apiPath, options) => {
      assert.equal(secretKey, credentials.secret_key);
      assert.equal(method, "GET");
      assert.equal(apiPath, `/iam/v1alpha1/ssh-keys/${state.ssh_key_id}`);
      assert.deepEqual(options.acceptedStatuses, [200, 404]);
      return { status: 200, data: nullCreatedKey };
    },
  });
  assert.equal(directKeyRead.id, state.ssh_key_id);

  function fakeCleanupControl(timeoutMs = 100) {
    let current = Date.parse("2026-07-19T05:00:00.000Z");
    return {
      control: {
        timeoutMs,
        pollIntervalMs: 10,
        absenceSpacingMs: 10,
        now: () => current,
        sleep: async (milliseconds) => {
          current += milliseconds;
        },
        persist: () => {},
      },
    };
  }

  const eventualServerState = { ...directState, run_id: "eventual-server" };
  let serverReads = 0;
  let serverDeletes = 0;
  const eventualServerRequest = async (secretKey, method, apiPath) => {
    assert.equal(secretKey, credentials.secret_key);
    const url = new URL(apiPath, "https://api.scaleway.com");
    const exactPath = `/baremetal/v1/zones/${plan.zone}/servers/${state.scaleway_id}`;
    if (method === "DELETE") {
      assert.equal(url.pathname, exactPath);
      serverDeletes += 1;
      return serverReads === 2 ? { status: 200, data: {} } : { status: 404, data: {} };
    }
    assert.equal(method, "GET");
    if (url.pathname === exactPath) {
      serverReads += 1;
      return serverReads === 2
        ? { status: 200, data: nullCreatedServer }
        : { status: 404, data: {} };
    }
    const listMatch = /^\/baremetal\/v1\/zones\/([^/]+)\/servers$/.exec(url.pathname);
    assert.ok(listMatch, `unexpected eventual-server path ${apiPath}`);
    const rows = listMatch[1] === plan.zone && serverReads === 2 ? [nullCreatedServer] : [];
    return { status: 200, data: { total_count: rows.length, servers: rows } };
  };
  await deleteScaleway(
    eventualServerState,
    credentials,
    eventualServerRequest,
    fakeCleanupControl().control,
  );
  assert.ok(eventualServerState.scaleway_positive_observed_at);
  assert.ok(eventualServerState.scaleway_delete_accepted_at);
  assert.ok(eventualServerState.scaleway_destroyed_at);
  assert.ok(serverDeletes >= 4, "known server ID must be deleted on every visibility pass");

  const neverVisibleServerState = { ...directState, run_id: "never-visible-server" };
  let neverVisibleServerDeletes = 0;
  const neverVisibleServerRequest = async (secretKey, method, apiPath) => {
    assert.equal(secretKey, credentials.secret_key);
    const url = new URL(apiPath, "https://api.scaleway.com");
    if (method === "DELETE") {
      neverVisibleServerDeletes += 1;
      return { status: 404, data: {} };
    }
    if (url.pathname.endsWith(`/servers/${state.scaleway_id}`)) return { status: 404, data: {} };
    return { status: 200, data: { total_count: 0, servers: [] } };
  };
  await assert.rejects(
    () =>
      deleteScaleway(
        neverVisibleServerState,
        credentials,
        neverVisibleServerRequest,
        fakeCleanupControl(35).control,
      ),
    /could not confirm deletion/,
  );
  assert.equal(neverVisibleServerState.scaleway_destroyed_at, undefined);
  assert.equal(neverVisibleServerState.scaleway_delete_accepted_at, undefined);
  assert.ok(neverVisibleServerDeletes >= 3, "never-visible server ID must still receive repeated DELETEs");

  const eventualKeyState = {
    ...directState,
    scaleway_destroyed_at: "2026-07-19T04:59:00.000Z",
  };
  let keyReads = 0;
  let keyDeletes = 0;
  const eventualKeyRequest = async (secretKey, method, apiPath) => {
    assert.equal(secretKey, credentials.secret_key);
    const url = new URL(apiPath, "https://api.scaleway.com");
    const exactPath = `/iam/v1alpha1/ssh-keys/${state.ssh_key_id}`;
    if (method === "DELETE") {
      assert.equal(url.pathname, exactPath);
      keyDeletes += 1;
      return keyReads === 2 ? { status: 204, data: null } : { status: 404, data: {} };
    }
    assert.equal(method, "GET");
    if (url.pathname === exactPath) {
      keyReads += 1;
      return keyReads === 2 ? { status: 200, data: nullCreatedKey } : { status: 404, data: {} };
    }
    assert.equal(url.pathname, "/iam/v1alpha1/ssh-keys");
    const rows = keyReads === 2 ? [nullCreatedKey] : [];
    return { status: 200, data: { total_count: rows.length, ssh_keys: rows } };
  };
  await deleteSshKey(eventualKeyState, credentials, eventualKeyRequest, fakeCleanupControl().control);
  assert.ok(eventualKeyState.ssh_key_positive_observed_at);
  assert.ok(eventualKeyState.ssh_key_delete_accepted_at);
  assert.ok(eventualKeyState.ssh_key_destroyed_at);
  assert.ok(keyDeletes >= 4, "known SSH-key ID must be deleted on every visibility pass");

  const neverVisibleKeyState = {
    ...directState,
    scaleway_destroyed_at: "2026-07-19T04:59:00.000Z",
  };
  let neverVisibleKeyDeletes = 0;
  const neverVisibleKeyRequest = async (secretKey, method, apiPath) => {
    assert.equal(secretKey, credentials.secret_key);
    const url = new URL(apiPath, "https://api.scaleway.com");
    if (method === "DELETE") {
      neverVisibleKeyDeletes += 1;
      return { status: 404, data: {} };
    }
    if (url.pathname.endsWith(`/${state.ssh_key_id}`)) return { status: 404, data: {} };
    return { status: 200, data: { total_count: 0, ssh_keys: [] } };
  };
  await assert.rejects(
    () =>
      deleteSshKey(
        neverVisibleKeyState,
        credentials,
        neverVisibleKeyRequest,
        fakeCleanupControl(35).control,
      ),
    /could not confirm deletion/,
  );
  assert.equal(neverVisibleKeyState.ssh_key_destroyed_at, undefined);
  assert.equal(neverVisibleKeyState.ssh_key_delete_accepted_at, undefined);
  assert.ok(neverVisibleKeyDeletes >= 3, "never-visible SSH-key ID must still receive repeated DELETEs");

  const wrongProjectState = { ...state, project_id: "00000000-0000-4000-8000-000000000999" };
  let mismatchedProjectRequests = 0;
  await assert.rejects(
    () =>
      reconcileScalewayByLabel(wrongProjectState, credentials, async () => {
        mismatchedProjectRequests += 1;
        assert.fail("a project-mismatched run must not issue an API request");
      }),
    /does not match owned run project/,
  );
  assert.equal(mismatchedProjectRequests, 0);
  await assert.rejects(() => cleanupOwned(wrongProjectState, credentials), /does not match owned run project/);
  await assert.rejects(
    () => deleteSshKey(state, credentials, async () => assert.fail("delete request must not run")),
    /before server deletion is confirmed/,
  );

  const runner = await run("node", [REMOTE_RUNNER, "self-test"]);
  assert.match(outputText(runner), /self-test passed/);
  process.stdout.write("scaleway benchmark coordinator offline self-test passed\n");
}

function requireCommandArguments(minimum, maximum = minimum) {
  const count = process.argv.length - 2;
  if (count < minimum || count > maximum) fail("unexpected command arguments; tokens are never accepted in argv");
}

function usage() {
  process.stdout.write(`Usage:
  node tools/run-scaleway-benchmark.js auth
  node tools/run-scaleway-benchmark.js plan
  node tools/run-scaleway-benchmark.js bench
  node tools/run-scaleway-benchmark.js status
  node tools/run-scaleway-benchmark.js destroy [RUN-ID]
  node tools/run-scaleway-benchmark.js unlock
  node tools/run-scaleway-benchmark.js self-test
`);
}

async function main() {
  const command = process.argv[2];
  switch (command) {
    case "auth":
      requireCommandArguments(1);
      await authenticate(loadConfig());
      break;
    case "plan":
      requireCommandArguments(1);
      await buildPlan(loadConfig());
      break;
    case "bench":
      requireCommandArguments(1);
      await bench(loadConfig());
      break;
    case "status":
      requireCommandArguments(1);
      await status();
      break;
    case "destroy":
      requireCommandArguments(1, 2);
      await destroy(process.argv[3] || "");
      break;
    case "unlock":
      requireCommandArguments(1);
      unlockStaleCoordinator();
      break;
    case "self-test":
      requireCommandArguments(1);
      await selfTest(loadConfig());
      break;
    case "--help":
    case "-h":
    case "help":
      requireCommandArguments(1);
      usage();
      break;
    default:
      usage();
      process.exitCode = 2;
  }
}

main().catch((error) => {
  process.stderr.write(`run-scaleway-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
