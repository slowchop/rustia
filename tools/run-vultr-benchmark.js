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
const DEFAULT_CONFIG = path.join(REPO, "bench/cloud/vultr-dense-ed65077.json");
const REMOTE_RUNNER = path.join(REPO, "tools/run-linux-fixed-benchmark.js");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/vultr");
const RUNS_ROOT = path.join(STATE_ROOT, "runs");
const TOKEN_FILE = path.join(STATE_ROOT, "token");
const COORDINATOR_LOCK = path.join(STATE_ROOT, "coordinator.lock");
const API_HOST = "api.vultr.com";
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
const FROZEN_CONFIG_SHA256 = "8b3d3f27b35c2da9e687cbe567595881bd6f21d242f73686fa7858a998450b15";
const MAX_EXPORT_ARCHIVE_BYTES = 256 * 1024 * 1024;
const MAX_EXPORT_UNCOMPRESSED_BYTES = 512 * 1024 * 1024;
const MAX_EXPORT_ENTRIES = 10_000;

let activeChild = null;
let activeRequest = null;
let activeForceKillTimer = null;
let interruptedSignal = null;
let cleanupInProgress = false;

class VultrApiError extends Error {
  constructor(message, status = null, response = null) {
    super(message);
    this.name = "VultrApiError";
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
  if (root.schema !== "rustia.vultr-fixed-benchmark.v1") {
    fail("unsupported Vultr benchmark config schema");
  }

  const provider = requireObject(root.provider, "config.provider");
  requireExactKeys(
    provider,
    ["region", "os", "size", "max_runtime_minutes"],
    "config.provider",
  );
  requireString(provider.region, "config.provider.region");
  requireInteger(provider.max_runtime_minutes, "config.provider.max_runtime_minutes", 1);
  if (provider.region !== "sgp" || provider.max_runtime_minutes !== 110) {
    fail("Vultr benchmark provider is pinned to sgp with a 110-minute lifecycle");
  }
  const operatingSystem = requireObject(provider.os, "config.provider.os");
  requireExactKeys(operatingSystem, ["id", "name", "arch", "family"], "config.provider.os");
  requireInteger(operatingSystem.id, "config.provider.os.id", 1);
  requireString(operatingSystem.name, "config.provider.os.name");
  requireString(operatingSystem.arch, "config.provider.os.arch");
  requireString(operatingSystem.family, "config.provider.os.family");
  if (
    operatingSystem.id !== 2284 ||
    operatingSystem.name !== "Ubuntu 24.04 LTS x64" ||
    operatingSystem.arch !== "x64" ||
    operatingSystem.family !== "ubuntu"
  ) {
    fail("Vultr benchmark OS must remain exact Ubuntu 24.04 LTS x64 ID 2284");
  }
  const size = requireObject(provider.size, "config.provider.size");
  requireExactKeys(
    size,
    [
      "id",
      "physical_cpus",
      "cpu_count",
      "vcpus",
      "cpu_cores",
      "cpu_threads",
      "cpu_manufacturer",
      "cpu_model",
      "cpu_mhz",
      "memory_mib",
      "disk_gib",
      "disk_count",
      "disk_type",
      "bandwidth_gib",
      "monthly_price_usd",
      "invoice_type",
      "gpu_brand",
      "deploy_ondemand",
      "deploy_preemptible",
      "max_hourly_price_usd",
    ],
    "config.provider.size",
  );
  requireString(size.id, "config.provider.size.id");
  requireInteger(size.physical_cpus, "config.provider.size.physical_cpus", 1);
  requireInteger(size.cpu_count, "config.provider.size.cpu_count", 1);
  requireInteger(size.vcpus, "config.provider.size.vcpus", 2);
  requireInteger(size.cpu_cores, "config.provider.size.cpu_cores", 1);
  requireInteger(size.cpu_threads, "config.provider.size.cpu_threads", 1);
  requireString(size.cpu_manufacturer, "config.provider.size.cpu_manufacturer");
  requireString(size.cpu_model, "config.provider.size.cpu_model");
  requireInteger(size.cpu_mhz, "config.provider.size.cpu_mhz", 1);
  requireInteger(size.memory_mib, "config.provider.size.memory_mib", 1);
  requireInteger(size.disk_gib, "config.provider.size.disk_gib", 1);
  requireInteger(size.disk_count, "config.provider.size.disk_count", 1);
  requireString(size.disk_type, "config.provider.size.disk_type");
  requireInteger(size.bandwidth_gib, "config.provider.size.bandwidth_gib", 1);
  requireNumber(size.monthly_price_usd, "config.provider.size.monthly_price_usd", 0.01);
  requireString(size.invoice_type, "config.provider.size.invoice_type");
  requireString(size.gpu_brand, "config.provider.size.gpu_brand");
  if (size.deploy_ondemand !== true) fail("config.provider.size.deploy_ondemand must be true");
  if (size.deploy_preemptible !== false) fail("config.provider.size.deploy_preemptible must be false");
  requireNumber(size.max_hourly_price_usd, "config.provider.size.max_hourly_price_usd", 0.001);
  if (
    size.id !== "vbm-4c-32gb" ||
    size.vcpus !== size.cpu_threads ||
    size.cpu_threads % size.cpu_cores !== 0 ||
    size.max_hourly_price_usd !== 0.164
  ) {
    fail("Vultr benchmark size must remain the exact vbm-4c-32gb bare-metal profile at <= $0.164/h");
  }
  if (provider.max_runtime_minutes >= 120) {
    fail("config.provider.max_runtime_minutes must remain below Vultr's third whole billing hour");
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
  if (publication.mode !== "acceptance") fail("Vultr coordinator is acceptance-only");
  if (publication.required_verdict !== "material_regression_excluded") {
    fail("config.publication.required_verdict must be material_regression_excluded");
  }
  requireInteger(publication.final_measurements, "config.publication.final_measurements", 3);
  const frozenHash = sha256(Buffer.from(canonicalJson(root), "utf8"));
  if (frozenHash !== FROZEN_CONFIG_SHA256) {
    fail(
      `Vultr acceptance config changed from the frozen protocol: ${frozenHash}, ` +
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
  return "unrecognized API error response";
}

function createOutcomeForError(error) {
  return error && Number.isInteger(error.status) && error.status >= 400 && error.status <= 499
    ? "definitively_rejected"
    : "ambiguous";
}

function bareMetalFromCreateResponse(response) {
  const envelope = requireApiObject(response, "Vultr create response");
  // Vultr's published schema and example disagree on this envelope key, while
  // the current official client uses bare_metal. Accept exactly one spelling.
  const keys = ["bare_metal", "baremetal"].filter((key) => envelope[key] !== undefined);
  if (keys.length !== 1) {
    fail("Vultr create response must contain exactly one bare_metal envelope");
  }
  return requireObject(envelope[keys[0]], `Vultr create response.${keys[0]}`);
}

async function apiRequest(token, method, apiPath, options = {}) {
  throwIfInterrupted();
  if (!/^\/v2\//.test(apiPath)) fail("Vultr API path must stay under /v2/");
  const {
    body = null,
    acceptedStatuses = null,
    timeoutMs = API_TIMEOUT_MS,
    unauthenticated = false,
  } = options;
  if (unauthenticated) {
    const publicCatalogPath = [
      /^\/v2\/plans-metal\?per_page=500$/,
      /^\/v2\/regions\?per_page=500$/,
      /^\/v2\/os\?per_page=500$/,
      /^\/v2\/regions\/[^/?]+\/availability$/,
    ].some((pattern) => pattern.test(apiPath));
    if (method !== "GET" || !publicCatalogPath || body !== null) {
      fail("unauthenticated Vultr requests are limited to exact public catalog GETs");
    }
  } else if (typeof token !== "string" || token.length === 0) {
    fail("Vultr token is unavailable");
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
          ...(unauthenticated ? {} : { Authorization: `Bearer ${token}` }),
          "User-Agent": "rustia-vultr-benchmark/1",
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
            response.destroy(new Error("Vultr API response exceeded 32 MiB"));
            return;
          }
          chunks.push(Buffer.from(chunk));
        });
        response.on("error", (error) => finish(reject, new VultrApiError(`Vultr API response failed: ${error.message}`)));
        response.on("end", () => {
          const bytes = Buffer.concat(chunks);
          let data = null;
          if (bytes.length > 0) {
            try {
              data = JSON.parse(bytes.toString("utf8"));
            } catch (error) {
              finish(reject, new VultrApiError(`Vultr API returned invalid JSON: ${error.message}`, response.statusCode));
              return;
            }
          }
          const status = response.statusCode || 0;
          const accepted = acceptedStatuses
            ? acceptedStatuses.includes(status)
            : status >= 200 && status < 300;
          if (!accepted) {
            finish(reject, new VultrApiError(`Vultr API ${method} ${apiPath} returned ${status}: ${apiErrorDetail(data)}`, status, data));
            return;
          }
          finish(resolve, { status, data, headers: response.headers });
        });
      },
    );
    activeRequest = request;
    request.setTimeout(timeoutMs, () => request.destroy(new Error("Vultr API request timed out")));
    request.on("error", (error) => {
      finish(reject, new VultrApiError(`Vultr API ${method} ${apiPath} failed: ${error.message}`));
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

function readStoredToken({ allowMissing = false } = {}) {
  let stat;
  try {
    stat = fs.lstatSync(TOKEN_FILE);
  } catch (error) {
    if (allowMissing && error && error.code === "ENOENT") return null;
    fail("Vultr token is not configured; run the auth command from an interactive terminal");
  }
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`refusing non-regular token file ${TOKEN_FILE}`);
  if ((stat.mode & 0o077) !== 0) fail(`Vultr token file permissions must be 0600: ${TOKEN_FILE}`);
  const token = fs.readFileSync(TOKEN_FILE, "utf8").trim();
  if (!token || /\s/.test(token)) fail(`Vultr token file is empty or malformed: ${TOKEN_FILE}`);
  return token;
}

async function readHiddenToken() {
  if (!process.stdin.isTTY || !process.stderr.isTTY || typeof process.stdin.setRawMode !== "function") {
    fail("auth requires an interactive TTY; the token is never accepted in argv or environment variables");
  }
  process.stderr.write("Vultr API key (input hidden): ");
  const input = process.stdin;
  const wasRaw = Boolean(input.isRaw);
  input.setEncoding("utf8");
  input.setRawMode(true);
  input.resume();
  return await new Promise((resolve, reject) => {
    let secret = "";
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
    const onEnd = () => abort("token entry ended before a token was submitted");
    const onError = (error) => abort(`token entry failed: ${error.message}`);
    for (const signal of signals) {
      signalHandlers.set(signal, () => abort(`token entry interrupted by ${signal}`));
    }
    const onData = (chunk) => {
      for (const character of chunk) {
        if (character === "\r" || character === "\n") {
          cleanup();
          if (!secret || /\s/.test(secret)) reject(new Error("token cannot be empty or contain whitespace"));
          else resolve(secret);
          return;
        }
        if (character === "\u0003") {
          abort("token entry interrupted");
          return;
        }
        if (character === "\u007f" || character === "\b") {
          secret = secret.slice(0, -1);
        } else if (character >= " ") {
          secret += character;
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

async function listVultrs(token, request = apiRequest) {
  const all = [];
  const seen = new Set();
  let apiPath = "/v2/bare-metals?per_page=500";
  while (apiPath) {
    if (seen.has(apiPath)) fail("Vultr bare-metal pagination repeated a page");
    seen.add(apiPath);
    const response = await request(token, "GET", apiPath, { acceptedStatuses: [200] });
    const envelope = requireApiObject(response, "Vultr instance list response");
    if (!Array.isArray(envelope.bare_metals)) {
      fail("Vultr instance list response.bare_metals must be an array");
    }
    all.push(...envelope.bare_metals);
    const meta = requireObject(envelope.meta, "Vultr instance list response.meta");
    const total = requireInteger(meta.total, "Vultr instance list response.meta.total", 0);
    const links = requireObject(meta.links, "Vultr instance list response.meta.links");
    const next = links.next;
    if (next !== "" && typeof next !== "string") {
      fail("Vultr instance list response.meta.links.next must be a string");
    }
    if (!next) {
      if (all.length !== total) {
        fail(`Vultr bare-metal list returned ${all.length} rows but declared ${total}`);
      }
      apiPath = "";
    } else {
      if (!/^[A-Za-z0-9_+=/-]{1,2048}$/.test(next)) {
        fail("Vultr bare-metal next-page cursor was malformed");
      }
      apiPath = `/v2/bare-metals?per_page=500&cursor=${encodeURIComponent(next)}`;
    }
  }
  const ids = new Set();
  for (const [index, entry] of all.entries()) {
    requireObject(entry, `Vultr instance list row ${index}`);
    if (!UUID.test(entry.id)) fail(`Vultr instance list row ${index} has a malformed ID`);
    if (ids.has(entry.id)) fail(`Vultr instance list repeated ID ${entry.id}`);
    ids.add(entry.id);
  }
  return all;
}

function validateProviderCatalog(config, responses) {
  const wanted = config.provider.size;
  const plans = requireApiObject(responses.plans, "Vultr bare-metal plan catalog");
  const regions = requireApiObject(responses.regions, "Vultr region catalog");
  const operatingSystems = requireApiObject(responses.operatingSystems, "Vultr OS catalog");
  const availability = requireApiObject(responses.availability, "Vultr region availability");
  if (!Array.isArray(plans.plans_metal)) fail("Vultr plan catalog.plans_metal must be an array");
  if (!Array.isArray(regions.regions)) fail("Vultr region catalog.regions must be an array");
  if (!Array.isArray(operatingSystems.os)) fail("Vultr OS catalog.os must be an array");
  if (!Array.isArray(availability.available_plans) || !Array.isArray(availability.available_vpc_only_plans)) {
    fail("Vultr region availability must expose both available plan arrays");
  }
  for (const [description, envelope, rows] of [
    ["bare-metal plan", plans, plans.plans_metal],
    ["region", regions, regions.regions],
    ["OS", operatingSystems, operatingSystems.os],
  ]) {
    const meta = requireObject(envelope.meta, `Vultr ${description} catalog.meta`);
    const links = requireObject(meta.links, `Vultr ${description} catalog.meta.links`);
    if (requireInteger(meta.total, `Vultr ${description} catalog.meta.total`, 0) !== rows.length) {
      fail(`Vultr ${description} catalog was not a complete single page`);
    }
    if (links.next !== "" || links.prev !== "") {
      fail(`Vultr ${description} catalog was not returned as one complete first page`);
    }
  }
  const planMatches = plans.plans_metal.filter((entry) => entry && entry.id === wanted.id);
  const regionMatches = regions.regions.filter((entry) => entry && entry.id === config.provider.region);
  const osMatches = operatingSystems.os.filter((entry) => entry && entry.id === config.provider.os.id);
  if (planMatches.length !== 1) fail(`Vultr public catalog returned ${planMatches.length} exact ${wanted.id} plans`);
  if (regionMatches.length !== 1) {
    fail(`Vultr public catalog returned ${regionMatches.length} exact ${config.provider.region} regions`);
  }
  if (osMatches.length !== 1) {
    fail(`Vultr public catalog returned ${osMatches.length} exact OS ${config.provider.os.id} rows`);
  }
  const plan = planMatches[0];
  const region = regionMatches[0];
  const operatingSystem = osMatches[0];
  const planChecks = [
    ["id", plan.id, wanted.id],
    ["physical CPU count", plan.physical_cpus, wanted.physical_cpus],
    ["CPU count", plan.cpu_count, wanted.cpu_count],
    ["CPU core count", plan.cpu_cores, wanted.cpu_cores],
    ["CPU thread count", plan.cpu_threads, wanted.cpu_threads],
    ["CPU manufacturer", plan.cpu_manufacturer, wanted.cpu_manufacturer],
    ["CPU model", plan.cpu_model, wanted.cpu_model],
    ["CPU MHz", plan.cpu_mhz, wanted.cpu_mhz],
    ["memory MiB", plan.ram, wanted.memory_mib],
    ["disk GiB", plan.disk, wanted.disk_gib],
    ["disk count", plan.disk_count, wanted.disk_count],
    ["disk type", plan.type, wanted.disk_type],
    ["bandwidth GiB", plan.bandwidth, wanted.bandwidth_gib],
    ["monthly price", plan.monthly_cost, wanted.monthly_price_usd],
    ["invoice type", plan.invoice_type, wanted.invoice_type],
    ["GPU brand", plan.gpu_brand, wanted.gpu_brand],
    ["on-demand deployment", plan.deploy_ondemand, wanted.deploy_ondemand],
    ["preemptible deployment", plan.deploy_preemptible, wanted.deploy_preemptible],
  ];
  for (const [field, actual, expected] of planChecks) {
    if (String(actual) !== String(expected)) {
      fail(`Vultr public bare-metal plan ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  const hourlyPrice = requireNumber(plan.hourly_cost, `Vultr plan ${plan.id} hourly price`, 0);
  if (hourlyPrice > wanted.max_hourly_price_usd) {
    fail(
      `Vultr plan ${wanted.id} costs $${hourlyPrice}/h, ` +
        `above the configured $${wanted.max_hourly_price_usd}/h maximum`,
    );
  }
  if (!Array.isArray(plan.locations) || plan.locations.filter((id) => id === config.provider.region).length !== 1) {
    fail(`Vultr plan ${wanted.id} is not exactly available in ${config.provider.region}`);
  }
  if (
    availability.available_plans.filter((id) => id === wanted.id).length !== 1 ||
    availability.available_vpc_only_plans.includes(wanted.id)
  ) {
    fail(`Vultr plan ${wanted.id} is not exactly available as a public-IP plan in ${config.provider.region}`);
  }
  const osChecks = [
    ["id", operatingSystem.id, config.provider.os.id],
    ["name", operatingSystem.name, config.provider.os.name],
    ["architecture", operatingSystem.arch, config.provider.os.arch],
    ["family", operatingSystem.family, config.provider.os.family],
  ];
  for (const [field, actual, expected] of osChecks) {
    if (String(actual) !== String(expected)) {
      fail(`Vultr public OS ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if (region.city !== "Singapore" || region.country !== "SG" || region.continent !== "Asia") {
    fail("Vultr sgp region metadata no longer exactly identifies Singapore, SG, Asia");
  }
  return { plan, operatingSystem, region, hourlyPrice };
}

async function resolveProviderPlan(_token, config, request = apiRequest) {
  const [plans, regions, operatingSystems, availability] = await Promise.all([
    request(null, "GET", "/v2/plans-metal?per_page=500", { unauthenticated: true, acceptedStatuses: [200] }),
    request(null, "GET", "/v2/regions?per_page=500", { unauthenticated: true, acceptedStatuses: [200] }),
    request(null, "GET", "/v2/os?per_page=500", { unauthenticated: true, acceptedStatuses: [200] }),
    request(null, "GET", `/v2/regions/${encodeURIComponent(config.provider.region)}/availability`, {
      unauthenticated: true,
      acceptedStatuses: [200],
    }),
  ]);
  return validateProviderCatalog(config, { plans, regions, operatingSystems, availability });
}

async function verifyDeleteAuthority(token, request = apiRequest, { announce = true } = {}) {
  const sentinel = "ffffffff-ffff-4fff-bfff-ffffffffffff";
  if ((await listVultrs(token, request)).some((entry) => entry && entry.id === sentinel)) {
    fail(`refusing absent-ID probe because Vultr ${sentinel} exists on the account`);
  }
  let response;
  try {
    response = await request(token, "DELETE", `/v2/bare-metals/${sentinel}`, {
      acceptedStatuses: [404],
    });
  } catch (error) {
    fail(
      "Vultr delete endpoint probe failed before billing; the service user needs " +
        `Manage Servers (${error.message})`,
    );
  }
  if (response.status !== 404) fail("absent-ID Vultr delete probe did not return 404");
  if (announce) process.stdout.write("Pre-billing absent-ID delete probe returned the expected 404.\n");
}

async function verifyCreateAuthority(token, request = apiRequest, { announce = true } = {}) {
  let response;
  try {
    response = await request(token, "POST", "/v2/bare-metals", {
      body: {},
      acceptedStatuses: [400],
    });
  } catch (error) {
    fail(
      "Vultr create endpoint probe failed without billing; the service user needs " +
        `Deploy New Servers (${error.message})`,
    );
  }
  if (response.status !== 400) fail("invalid empty Vultr create probe did not return 400");
  if (announce) process.stdout.write("Pre-billing invalid-body create probe returned the expected 400.\n");
}

function iamPatternMatches(pattern, value) {
  if (typeof pattern !== "string" || typeof value !== "string") return false;
  const escaped = pattern.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*");
  return new RegExp(`^${escaped}$`).test(value);
}

function stringList(value, description) {
  const values = typeof value === "string" ? [value] : value;
  if (!Array.isArray(values) || values.length === 0 || values.some((entry) => typeof entry !== "string")) {
    fail(`${description} must be a string or non-empty string array`);
  }
  return values;
}

function verifyRequiredIamActions(response) {
  const envelope = requireApiObject(response, "Vultr current-user policies response");
  const policies = requireObject(envelope.policies, "Vultr current-user policies");
  if (!Array.isArray(policies.all)) fail("Vultr current-user policies.all must be an array");
  const statements = [];
  for (const [policyIndex, policy] of policies.all.entries()) {
    requireObject(policy, `Vultr policy ${policyIndex}`);
    let document = policy.policy_document;
    if (typeof document === "string") {
      try {
        document = JSON.parse(document);
      } catch (error) {
        fail(`Vultr policy ${policyIndex} document is invalid JSON: ${error.message}`);
      }
    }
    document = requireObject(document, `Vultr policy ${policyIndex} document`);
    if (!Array.isArray(document.Statement)) fail(`Vultr policy ${policyIndex} Statement must be an array`);
    statements.push(...document.Statement);
  }
  // Vultr's managed user roles are exposed through compatibility IAM actions,
  // while custom policies use the granular compute.baremetal actions. Both are
  // documented effective-permission representations for the same API access.
  const requirements = [
    ["compute.baremetal.Create", "user.acl.Provisioning"],
    ["compute.baremetal.Delete", "user.acl.Subscriptions"],
    ["compute.baremetal.List", "user.acl.SubscriptionsView"],
    ["compute.baremetal.Read", "user.acl.SubscriptionsView"],
  ];
  for (const [action, managedRoleAction] of requirements) {
    let allowed = false;
    let denied = false;
    for (const [index, rawStatement] of statements.entries()) {
      const statement = requireObject(rawStatement, `Vultr IAM statement ${index}`);
      if (!["Allow", "Deny"].includes(statement.Effect)) {
        fail(`Vultr IAM statement ${index} Effect must be Allow or Deny`);
      }
      const actions = stringList(statement.Action, `Vultr IAM statement ${index} Action`);
      const resources = stringList(statement.Resource, `Vultr IAM statement ${index} Resource`);
      if (
        !actions.some(
          (pattern) =>
            iamPatternMatches(pattern, action) || iamPatternMatches(pattern, managedRoleAction),
        )
      ) {
        continue;
      }
      if (statement.Effect === "Deny") denied = true;
      else if (resources.includes("*")) allowed = true;
    }
    if (!allowed || denied) {
      fail(
        `Vultr IAM effective policies do not grant unrestricted ${action} ` +
          `(or managed-role equivalent ${managedRoleAction})`,
      );
    }
  }
  return requirements.map(([action]) => action);
}

function verifyRequiredManagedPolicyNames(response) {
  const envelope = requireApiObject(response, "Vultr current-user policies response");
  const policies = requireObject(envelope.policies, "Vultr current-user policies");
  if (!Array.isArray(policies.all)) fail("Vultr current-user policies.all must be an array");
  const names = policies.all.map((policy, index) => {
    requireObject(policy, `Vultr policy ${index}`);
    const name = requireString(policy.name, `Vultr policy ${index}.name`);
    if (policy.is_system_policy !== undefined && policy.is_system_policy !== true) {
      return "";
    }
    return name;
  });
  const required = ["Deploy New Servers", "Manage Servers", "View Servers"];
  const missing = required.filter((name) => !names.includes(name));
  if (missing.length > 0) {
    const observed = names.filter(Boolean).length > 0 ? names.filter(Boolean).sort().join(", ") : "none";
    fail(`missing Vultr managed policy/policies: ${missing.join(", ")}; effective policies: ${observed}`);
  }
  return required;
}

function explicitRequiredIamDeny(response) {
  const envelope = requireApiObject(response, "Vultr current-user policies response");
  const policies = requireObject(envelope.policies, "Vultr current-user policies");
  if (!Array.isArray(policies.all)) fail("Vultr current-user policies.all must be an array");
  const protectedActions = [
    "compute.baremetal.Create",
    "compute.baremetal.Delete",
    "compute.baremetal.List",
    "compute.baremetal.Read",
    "user.acl.Provisioning",
    "user.acl.Subscriptions",
    "user.acl.SubscriptionsView",
  ];
  for (const policy of policies.all) {
    if (!policy || typeof policy !== "object" || Array.isArray(policy)) continue;
    let document = policy.policy_document;
    if (typeof document === "string") {
      try {
        document = JSON.parse(document);
      } catch {
        continue;
      }
    }
    if (!document || typeof document !== "object" || !Array.isArray(document.Statement)) continue;
    for (const statement of document.Statement) {
      if (!statement || typeof statement !== "object" || statement.Effect !== "Deny") continue;
      const actions = typeof statement.Action === "string" ? [statement.Action] : statement.Action;
      if (!Array.isArray(actions)) continue;
      for (const pattern of actions) {
        const denied = protectedActions.find((action) => iamPatternMatches(pattern, action));
        if (denied) return denied;
      }
    }
  }
  return null;
}

function verifyRequiredManagedRoles(response) {
  const envelope = requireApiObject(response, "Vultr current-user roles response");
  const roles = requireObject(envelope.roles, "Vultr current-user roles");
  if (!Array.isArray(roles.all)) fail("Vultr current-user roles.all must be an array");
  // Vultr's deployed IAM response currently omits `meta`, while its published
  // OpenAPI description includes it. Validate pagination when present; the API
  // caps organization roles below the requested 500-row page either way.
  if (envelope.meta !== undefined && envelope.meta !== null) {
    const meta = requireObject(envelope.meta, "Vultr current-user roles meta");
    const total = requireInteger(meta.total, "Vultr current-user roles meta.total", 0);
    const links = requireObject(meta.links, "Vultr current-user roles meta.links");
    if (roles.all.length !== total || links.next !== "") {
      fail("Vultr current-user roles response was not a complete single page");
    }
  }
  const names = roles.all.map((role, index) => {
    requireObject(role, `Vultr current-user role ${index}`);
    return requireString(role.name, `Vultr current-user role ${index}.name`);
  });
  const required = ["Deploy New Servers", "Manage Servers", "View Servers"];
  const missing = required.filter((name) => !names.includes(name));
  if (missing.length > 0) {
    const observed = names.length > 0 ? names.sort().join(", ") : "none";
    fail(`missing Vultr managed role(s): ${missing.join(", ")}; assigned roles: ${observed}`);
  }
  return required;
}

async function inspectIamPermissions(token, request = apiRequest, { announce = true } = {}) {
  let response = null;
  let policyRows = null;
  try {
    response = await request(token, "GET", "/v2/users/me/policies?per_page=500", {
      acceptedStatuses: [200],
    });
  } catch {
    // Current service-user IAM introspection may be unavailable even though the
    // resource endpoints enforce the attached role. Continue only to probes.
  }
  if (response) {
    const envelope = requireApiObject(response, "Vultr current-user policies response");
    const policies = requireObject(envelope.policies, "Vultr current-user policies");
    if (!Array.isArray(policies.all)) fail("Vultr current-user policies.all must be an array");
    policyRows = policies.all;
  }
  if (policyRows && policyRows.length > 0) {
    try {
      const required = verifyRequiredIamActions(response);
      if (announce) process.stdout.write(`Observed Vultr IAM actions: ${required.join(", ")}.\n`);
      return required;
    } catch (actionError) {
      const denied = explicitRequiredIamDeny(response);
      if (denied) fail(`Vultr IAM contains an explicit Deny matching ${denied}`);
      try {
        const policies = verifyRequiredManagedPolicyNames(response);
        if (announce) {
          process.stdout.write(
            `Observed Vultr managed policies: ${policies.join(", ")} ` +
              "(their policy documents use an opaque managed representation).\n",
          );
        }
        return policies;
      } catch (policyNameError) {
        fail(
          `non-empty Vultr IAM policies did not prove required bare-metal access: ` +
            `${actionError.message}; ${policyNameError.message}`,
        );
      }
    }
  }

  let roleResponse = null;
  let roleRows = null;
  try {
    roleResponse = await request(token, "GET", "/v2/users/me/roles?per_page=500", {
      acceptedStatuses: [200],
    });
  } catch {
    // Treat unavailable role introspection like the empty service-user result.
  }
  if (roleResponse) {
    const envelope = requireApiObject(roleResponse, "Vultr current-user roles response");
    const roles = requireObject(envelope.roles, "Vultr current-user roles");
    if (!Array.isArray(roles.all)) fail("Vultr current-user roles.all must be an array");
    roleRows = roles.all;
  }
  if (roleRows && roleRows.length > 0) {
    try {
      const roles = verifyRequiredManagedRoles(roleResponse);
      if (announce) process.stdout.write(`Observed Vultr managed roles: ${roles.join(", ")}.\n`);
      return roles;
    } catch (error) {
      fail(`non-empty Vultr IAM roles did not prove required bare-metal access: ${error.message}`);
    }
  }
  if (announce) {
    process.stdout.write(
      "Vultr IAM introspection returned empty or unavailable service-user permissions; " +
        "continuing to non-billing bare-metal endpoint probes.\n",
    );
  }
  return [];
}

async function gitBlob(commit, file) {
  return (await run("git", ["show", `${commit}:${file}`])).stdout;
}

async function verifySource(config, configFile = DEFAULT_CONFIG) {
  const relativeConfig = path.relative(REPO, configFile);
  if (relativeConfig.startsWith("..") || path.isAbsolute(relativeConfig)) {
    fail("Vultr benchmark config must be inside the repository");
  }
  const trackedStatus = outputText(
    await run("git", ["status", "--porcelain=v1", "--untracked-files=no"]),
  );
  if (trackedStatus.length > 0) fail("tracked worktree changes must be committed before a cloud benchmark");
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    await run("git", ["cat-file", "-e", `${config.source[field]}^{commit}`]);
  }
  for (const file of [
    "tools/run-vultr-benchmark.js",
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
  return {
    schema: config.schema,
    provider: "vultr",
    protocol_mode: "acceptance",
    region: {
      id: provider.region.id,
      city: provider.region.city,
      country: provider.region.country,
      continent: provider.region.continent,
    },
    os: {
      id: provider.operatingSystem.id,
      name: provider.operatingSystem.name,
      arch: provider.operatingSystem.arch,
      family: provider.operatingSystem.family,
    },
    size: {
      id: provider.plan.id,
      resource_type: "bare_metal",
      physical_cpus: provider.plan.physical_cpus,
      catalog_cpu_count: provider.plan.cpu_count,
      vcpus: provider.plan.cpu_threads,
      cpu_cores: provider.plan.cpu_cores,
      cpu_threads: provider.plan.cpu_threads,
      cpu_manufacturer: provider.plan.cpu_manufacturer,
      cpu_model: provider.plan.cpu_model,
      cpu_mhz: provider.plan.cpu_mhz,
      memory_mib: provider.plan.ram,
      disk_gib: provider.plan.disk,
      disk_count: provider.plan.disk_count,
      disk_type: provider.plan.type,
      bandwidth_gib: provider.plan.bandwidth,
      price_hourly_usd: provider.hourlyPrice,
      price_monthly_usd: provider.plan.monthly_cost,
      invoice_type: provider.plan.invoice_type,
      deploy_ondemand: provider.plan.deploy_ondemand,
      deploy_preemptible: provider.plan.deploy_preemptible,
    },
    billable_lifetime_limit_minutes: config.provider.max_runtime_minutes,
    billing_increment_minutes: 60,
    planned_compute_cost_ceiling_usd: roundedHourlyCharge(
      provider.hourlyPrice,
      config.provider.max_runtime_minutes * 60,
    ),
    cleanup_reserve_minutes: CLEANUP_RESERVE_MS / 60_000,
    billing_caveat:
      "Vultr usage is rounded up to a whole hour; deletion, not shutdown, stops further accrual, and an API outage can delay confirmed deletion",
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
  const token = readStoredToken();
  await inspectIamPermissions(token);
  await listVultrs(token);
  const source = await verifySource(config, configFile);
  const provider = await resolveProviderPlan(token, config);
  const plan = sanitizedProviderPlan(config, provider, source);
  if (print) process.stdout.write(`${JSON.stringify(plan, null, 2)}\n`);
  return { token, plan, provider, source };
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
    schema: "rustia.vultr-benchmark-lock.v1",
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
      fail(`Vultr coordinator lock exists without readable ownership at ${COORDINATOR_LOCK}`);
    }
    if (!lockOwnerIsLive(owner)) {
      fail(`stale Vultr coordinator lock for PID ${owner.pid}; run the unlock command`);
    }
    fail(`Vultr benchmark coordinator PID ${owner.pid} is already active`);
  }
  return token;
}

function releaseCoordinatorLock(token) {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot verify Vultr coordinator lock ownership: ${error.message}`);
  }
  if (owner.pid !== process.pid || owner.token !== token) {
    fail("refusing to remove a Vultr coordinator lock owned by another process");
  }
  fs.rmSync(COORDINATOR_LOCK, { recursive: true });
}

function unlockStaleCoordinator() {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot read Vultr coordinator lock ownership: ${error.message}`);
  }
  if (lockOwnerIsLive(owner)) fail(`refusing to unlock live Vultr coordinator PID ${owner.pid}`);
  const quarantine = `${COORDINATOR_LOCK}.stale-${crypto.randomBytes(8).toString("hex")}`;
  fs.renameSync(COORDINATOR_LOCK, quarantine);
  const moved = JSON.parse(fs.readFileSync(path.join(quarantine, "owner.json"), "utf8"));
  if (moved.pid !== owner.pid || moved.token !== owner.token) {
    fail(`stale lock ownership changed during quarantine; retained at ${quarantine}`);
  }
  fs.rmSync(quarantine, { recursive: true });
  process.stdout.write(`Removed stale Vultr coordinator lock for PID ${owner.pid}.\n`);
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
  return listStates().filter((state) => !state.vultr_destroyed_at);
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
        ? "no active owned Vultr benchmark run exists"
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

function matchingVultrsByLabel(vultrs, label) {
  if (!Array.isArray(vultrs)) fail("Vultr list must be an array");
  return vultrs.filter((entry) => entry && entry.label === label);
}

function vultrCreationTimeMatches(vultr, state) {
  if (!state.vultr_create_attempted_at) return false;
  const attempted = Date.parse(state.vultr_create_attempted_at);
  const created = Date.parse(vultr && vultr.date_created);
  const finished = Date.parse(state.vultr_create_finished_at || "");
  const requestDeadline = Date.parse(state.vultr_create_request_deadline_at || "");
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

function vultrHasOwnershipTag(vultr, state) {
  return Array.isArray(vultr && vultr.tags) &&
    vultr.tags.filter((tag) => tag === state.vultr_ownership_tag).length === 1;
}

function ownedVultrShapeMatches(vultr, state) {
  return Boolean(
    vultr &&
      vultr.id &&
      vultr.label === state.vultr_label &&
      vultr.plan === state.plan.size.id &&
      vultr.region === state.plan.region.id &&
      vultr.os_id === state.plan.os.id &&
      vultrCreationTimeMatches(vultr, state),
  );
}

function exactOwnedVultr(vultr, state) {
  return ownedVultrShapeMatches(vultr, state) && vultrHasOwnershipTag(vultr, state);
}

function cleanupOwnedVultrMatches(vultr, state) {
  if (!vultr || vultr.id !== state.vultr_id || vultr.label !== state.vultr_label) return false;
  if (exactOwnedVultr(vultr, state)) return true;
  return state.vultr_id_source === "create_response" && vultrCreationTimeMatches(vultr, state);
}

function ownershipRelatedVultrs(vultrs, state) {
  if (!Array.isArray(vultrs)) fail("Vultr list must be an array");
  return vultrs.filter(
    (entry) =>
      entry &&
      (entry.label === state.vultr_label ||
        (Array.isArray(entry.tags) && entry.tags.includes(state.vultr_ownership_tag))),
  );
}

async function reconcileVultrByLabel(state, token, request = apiRequest) {
  const related = ownershipRelatedVultrs(await listVultrs(token, request), state);
  const matches = related.filter((entry) => exactOwnedVultr(entry, state));
  if (related.length !== matches.length || matches.length > 1) {
    fail(
      `refusing to adopt or delete Vultr ownership set: ${related.length} label/tag related, ` +
        `${matches.length} exact`,
    );
  }
  if (matches.length === 1) {
    const id = requireString(matches[0].id, "reconciled Vultr ID");
    if (!UUID.test(id)) fail("reconciled Vultr ID is not a lowercase UUID");
    state.vultr_id = id;
    if (state.vultr_create_attempted_at) state.vultr_create_outcome = "resource_identified";
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileVultrAfterCreateAttempt(
  state,
  token,
  request = apiRequest,
  {
    reconcileTimeoutMs = CREATE_RECONCILE_TIMEOUT_MS,
    now = () => Date.now(),
    sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  } = {},
) {
  if (state.vultr_absence_confirmed_at) {
    if (state.vultr_create_outcome !== "definitively_rejected") {
      fail("Vultr journal claims absence without a definitively rejected create response");
    }
    return null;
  }
  const attemptedAt = Date.parse(state.vultr_create_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile a Vultr without a create-attempt timestamp");
  const finishedAt = Date.parse(state.vultr_create_finished_at || "");
  const requestDeadline = Date.parse(state.vultr_create_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = state.vultr_create_outcome === "definitively_rejected"
    ? now()
    : Math.max(now(), uncertaintyEndedAt + reconcileTimeoutMs);
  while (now() < deadline) {
    try {
      const vultr = await reconcileVultrByLabel(state, token, request);
      if (vultr) return vultr;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      if (/refusing to adopt or delete/.test(error.message)) throw error;
      // Only a successful list after the uncertainty window proves absence.
    }
    const delay = Math.min(5000, Math.max(0, deadline - now()));
    if (delay > 0) await sleep(delay);
  }
  const vultr = await reconcileVultrByLabel(state, token, request);
  if (vultr) return vultr;
  if (state.vultr_create_outcome !== "definitively_rejected") {
    fail(
      "Vultr create outcome remains ambiguous with no visible owned resource; cleanup is unresolved, " +
        "this run must continue blocking future creates until provider-confirmed safe",
    );
  }
  state.vultr_absence_confirmed_at = new Date(now()).toISOString();
  saveState(state);
  return null;
}

async function getOwnedVultr(
  state,
  token,
  { allowMissing = false, forCleanup = false, request = apiRequest } = {},
) {
  if (!state.vultr_id) return null;
  if (!UUID.test(state.vultr_id)) fail("owned Vultr ID is not a lowercase UUID");
  const response = await request(token, "GET", `/v2/bare-metals/${state.vultr_id}`, {
    acceptedStatuses: [200, 404],
  });
  if (response.status === 404) {
    if (allowMissing) return null;
    fail(`owned Vultr ${state.vultr_id} was not found`);
  }
  const envelope = requireApiObject(response, `Vultr ${state.vultr_id} response`);
  const vultr = requireObject(envelope.bare_metal, `Vultr ${state.vultr_id}`);
  const matches = forCleanup ? cleanupOwnedVultrMatches(vultr, state) : exactOwnedVultr(vultr, state);
  if (vultr.id !== state.vultr_id || !matches) {
    fail(`Vultr ${state.vultr_id} does not match the journaled exact ownership tuple`);
  }
  return vultr;
}

function verifyOwnedVultrIdentity(vultr, state, { forCleanup = false } = {}) {
  const directCreateAuthority = forCleanup && state.vultr_id_source === "create_response";
  const checks = [
    ["ID", vultr.id, state.vultr_id],
    ["label", vultr.label, state.vultr_label],
    [
      "ownership tag or direct create-response authority",
      vultrHasOwnershipTag(vultr, state) || directCreateAuthority,
      true,
    ],
    ["creation window", vultrCreationTimeMatches(vultr, state), true],
  ];
  if (!directCreateAuthority) {
    checks.push(
      ["region", vultr.region, state.plan.region.id],
      ["plan", vultr.plan, state.plan.size.id],
      ["OS ID", vultr.os_id, state.plan.os.id],
    );
  }
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`refusing to delete owned Vultr: ${field} is ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
}

async function deleteVultr(state, token, request = apiRequest) {
  if (state.vultr_destroyed_at) return;
  if (!state.vultr_id) {
    const reconciled = state.vultr_create_attempted_at
      ? await reconcileVultrAfterCreateAttempt(state, token, request)
      : await reconcileVultrByLabel(state, token, request);
    if (!reconciled) {
      state.vultr_destroyed_at = new Date().toISOString();
      state.cleanup_note = state.vultr_create_attempted_at
        ? "definitively rejected create response and a complete list confirmed the unique Vultr label absent"
        : "no create was attempted and a successful list confirmed the unique Vultr label absent";
      saveState(state);
      return;
    }
  }

  let existing = await getOwnedVultr(state, token, { allowMissing: true, forCleanup: true, request });
  if (!existing) {
    const vultrs = await listVultrs(token, request);
    const listed = vultrs.find((entry) => entry && entry.id === state.vultr_id);
    const related = ownershipRelatedVultrs(vultrs, state);
    if (!listed && related.length === 0) {
      const confirmation = await listVultrs(token, request);
      if (
        confirmation.some((entry) => entry && entry.id === state.vultr_id) ||
        ownershipRelatedVultrs(confirmation, state).length !== 0
      ) {
        fail("refusing cleanup: Vultr reappeared during consecutive absence confirmation");
      }
      state.vultr_destroyed_at = new Date().toISOString();
      state.cleanup_note = "GET returned 404 and two complete lists confirmed the owned ID/label/tag absent";
      saveState(state);
      return;
    }
    if (!listed) {
      fail(
        `refusing cleanup: owned Vultr ID ${state.vultr_id} is absent but its label/tag ` +
          `exists under ${related.length} other ID(s)`,
      );
    }
    if (related.length !== 1 || related[0].id !== listed.id || !cleanupOwnedVultrMatches(listed, state)) {
      fail("refusing cleanup: Vultr ownership tuple is no longer uniquely exact");
    }
    existing = listed;
  }
  verifyOwnedVultrIdentity(existing, state, { forCleanup: true });

  try {
    await request(token, "DELETE", `/v2/bare-metals/${existing.id}`, {
      acceptedStatuses: [204, 404],
    });
  } catch {
    // A read below, never the DELETE response, decides whether deletion completed.
  }

  const deadline = Date.now() + DELETE_CONFIRM_TIMEOUT_MS;
  let consecutiveAbsenceScans = 0;
  while (Date.now() < deadline) {
    try {
      const byGet = await getOwnedVultr(state, token, {
        allowMissing: true,
        forCleanup: true,
        request,
      });
      const vultrs = await listVultrs(token, request);
      const byId = vultrs.find((entry) => entry && entry.id === existing.id);
      const related = ownershipRelatedVultrs(vultrs, state);
      if (!byGet && !byId && related.length === 0) {
        consecutiveAbsenceScans += 1;
        if (consecutiveAbsenceScans >= 2) {
          state.vultr_destroyed_at = new Date().toISOString();
          state.cleanup_note = "GET returned 404 and two complete lists confirmed the owned ID/label/tag absent";
          saveState(state);
          return;
        }
      } else {
        consecutiveAbsenceScans = 0;
      }
      if (!byGet && !byId && related.length === 0) {
        // Require a second independent scan before declaring billing stopped.
      } else {
        if (related.length !== 1) {
          fail(`refusing cleanup: Vultr label/tag ownership set contains ${related.length} entries`);
        }
        if (!byGet || !byId || related[0].id !== byId.id || !cleanupOwnedVultrMatches(byId, state)) {
          fail(`refusing cleanup: owned Vultr ID/label/tag/shape identity changed`);
        }
        verifyOwnedVultrIdentity(byId, state, { forCleanup: true });
        await request(token, "DELETE", `/v2/bare-metals/${existing.id}`, {
          acceptedStatuses: [204, 404],
        });
      }
    } catch (error) {
      if (/refusing cleanup|refusing to delete/.test(error.message)) throw error;
      // API failure is not deletion proof; retry within the bounded confirmation window.
    }
    const delay = Math.min(4000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  fail(
    `could not confirm deletion of owned Vultr ${existing.id}; run ` +
      `\`node tools/run-vultr-benchmark.js destroy ${state.run_id}\` and do not assume billing stopped`,
  );
}

async function cleanupOwned(state, token) {
  await deleteVultr(state, token);
  if (state.vultr_destroyed_at) {
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

function publicIpv4(vultr) {
  return vultr && typeof vultr.main_ip === "string" && isPublicIpv4(vultr.main_ip)
    ? vultr.main_ip
    : null;
}

async function waitForVultr(state, token) {
  const deadline = Date.now() + remainingWorkTimeout(state, ACTIVATION_TIMEOUT_MS, "Vultr activation");
  while (Date.now() < deadline) {
    const vultr = await getOwnedVultr(state, token, { allowMissing: true });
    const address = publicIpv4(vultr);
    if (vultr && vultr.status === "active" && address) {
      state.public_ipv4 = address;
      state.vultr_created_at = vultr.date_created || new Date().toISOString();
      saveState(state);
      return vultr;
    }
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`Vultr ${state.vultr_id} did not become running with a public IPv4 address`);
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
  fail(`SSH did not become ready on owned Vultr ${state.vultr_id}`);
}

function verifyVultrMatchesPlan(vultr, plan, state) {
  const checks = [
    ["plan", vultr.plan, plan.size.id],
    ["region", vultr.region, plan.region.id],
    ["OS ID", vultr.os_id, plan.os.id],
    ["OS", vultr.os, plan.os.name],
    ["catalog CPU count", vultr.cpu_count, plan.size.catalog_cpu_count],
    ["user scheme", vultr.user_scheme, "root"],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`activated Vultr ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if ((vultr.image_id || "") !== "" || (vultr.snapshot_id || "") !== "" || (vultr.app_id || 0) !== 0) {
    fail("activated Vultr bare metal unexpectedly used an image, snapshot, or application source");
  }
  if (!Array.isArray(vultr.features) || vultr.features.includes("ipv6")) {
    fail("activated Vultr bare metal has malformed features or unexpected IPv6 enabled");
  }
  if (!vultrHasOwnershipTag(vultr, state)) {
    fail("activated Vultr bare metal is missing the unique ownership tag");
  }
}

function safeVultrMetadata(vultr, plan, state) {
  return {
    provider: "vultr",
    vultr_id: vultr.id,
    vultr_label: vultr.label,
    resource_type: "bare_metal",
    created_at: vultr.date_created,
    status: vultr.status,
    region: vultr.region,
    size_slug: vultr.plan,
    os_id: vultr.os_id,
    os: vultr.os,
    catalog_cpu_count: vultr.cpu_count,
    vcpus: plan.size.vcpus,
    ram: vultr.ram,
    disk: vultr.disk,
    public_ipv4: state.public_ipv4,
    user_scheme: vultr.user_scheme,
    tags: vultr.tags,
    catalog_plan: plan,
  };
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

function createVultrBody(config, label, ownershipTag, publicKey, hostPrivateKey, hostPublicKey) {
  if (!/^[a-z0-9-]{1,63}$/.test(label) || !/^[a-z0-9-]{1,63}$/.test(ownershipTag)) {
    fail("generated Vultr label and ownership tag must be lowercase safe identifiers");
  }
  const clientKey = parseEd25519PublicKey(publicKey, "generated client public key");
  const hostKey = parseEd25519PublicKey(hostPublicKey, "generated SSH host public key");
  if (clientKey.base64 === hostKey.base64) {
    fail("generated SSH client and host public keys must be distinct");
  }
  const normalizedHostPrivateKey = normalizeOpenSshPrivateKey(
    hostPrivateKey,
    "generated SSH host private key",
  );
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
  return {
    region: config.provider.region,
    plan: config.provider.size.id,
    os_id: config.provider.os.id,
    label,
    hostname: label,
    sshkey_id: [],
    user_data: Buffer.from(userData, "utf8").toString("base64"),
    user_scheme: "root",
    enable_ipv6: false,
    activation_email: false,
    mdisk_mode: "none",
    persistent_pxe: false,
    tags: [ownershipTag],
  };
}

function verifyProviderPlanUnchanged(original, refreshed) {
  const fields = [
    ["region", original.region.id, refreshed.region.id],
    ["OS", original.os.id, refreshed.os.id],
    ["plan", original.size.id, refreshed.size.id],
    ["resource type", original.size.resource_type, refreshed.size.resource_type],
    ["physical CPUs", original.size.physical_cpus, refreshed.size.physical_cpus],
    ["catalog CPU count", original.size.catalog_cpu_count, refreshed.size.catalog_cpu_count],
    ["vCPUs", original.size.vcpus, refreshed.size.vcpus],
    ["CPU cores", original.size.cpu_cores, refreshed.size.cpu_cores],
    ["CPU threads", original.size.cpu_threads, refreshed.size.cpu_threads],
    ["CPU model", original.size.cpu_model, refreshed.size.cpu_model],
    ["memory", original.size.memory_mib, refreshed.size.memory_mib],
    ["disk", original.size.disk_gib, refreshed.size.disk_gib],
    ["hourly price", original.size.price_hourly_usd, refreshed.size.price_hourly_usd],
  ];
  for (const [field, before, after] of fields) {
    if (String(before) !== String(after)) {
      fail(`Vultr provider ${field} changed between plan and pre-billing check`);
    }
  }
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
    fail("local orchestration checkout changed while the Vultr run was in flight; retaining export only");
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
    [attestation.logical_cpus, config.provider.size.vcpus, "logical CPU count"],
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
    !attestation.cpu_model.includes(config.provider.size.cpu_model) ||
    host.cpu_model !== attestation.cpu_model
  ) {
    fail("host bare-metal attestation CPU model does not match the frozen physical plan");
  }
  const topology = requireObject(attestation.topology, "host bare-metal topology");
  const expectedCpuNames = Array.from({ length: config.provider.size.vcpus }, (_, cpu) => `cpu${cpu}`);
  if (canonicalJson(Object.keys(topology).sort()) !== canonicalJson(expectedCpuNames.sort())) {
    fail("host bare-metal topology does not cover exactly every configured logical CPU");
  }
  const siblingCount = config.provider.size.cpu_threads / config.provider.size.cpu_cores;
  const sockets = new Set();
  const cores = new Set();
  for (let cpu = 0; cpu < config.provider.size.vcpus; cpu += 1) {
    const entry = requireObject(topology[`cpu${cpu}`], `host topology.cpu${cpu}`);
    requireExactKeys(entry, ["socket", "core", "siblings"], `host topology.cpu${cpu}`);
    const socket = requireString(entry.socket, `host topology.cpu${cpu}.socket`);
    const core = requireString(entry.core, `host topology.cpu${cpu}.core`);
    if (
      !Array.isArray(entry.siblings) ||
      entry.siblings.length !== siblingCount ||
      new Set(entry.siblings).size !== siblingCount ||
      entry.siblings.some((sibling) => !Number.isSafeInteger(sibling) || sibling < 0 || sibling >= config.provider.size.vcpus) ||
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
  for (let cpu = 0; cpu < config.provider.size.vcpus; cpu += 1) {
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
    fail("local benchmark history changed while the Vultr run was in flight");
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
    [provider.provider, "vultr", "provider"],
    [provider.resource_type, "bare_metal", "resource type"],
    [provider.vultr_id, state.vultr_id, "Vultr ID"],
    [provider.vultr_label, state.vultr_label, "Vultr label"],
    [provider.region, config.provider.region, "region"],
    [provider.size_slug, config.provider.size.id, "plan"],
    [provider.os_id, config.provider.os.id, "OS"],
    [provider.vcpus, config.provider.size.vcpus, "vCPUs"],
    [provider.public_ipv4, state.public_ipv4, "public IPv4"],
  ];
  for (const [actual, expected, field] of providerIdentity) {
    if (actual !== expected) fail(`evidence provider ${field} does not match the destroyed instance`);
  }
  if (!Array.isArray(provider.tags) || !provider.tags.includes(state.vultr_ownership_tag)) {
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
  if (!state.vultr_destroyed_at) {
    fail("refusing publication before the Vultr API has confirmed instance deletion");
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
      `Vultr run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
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
  const { token, plan, provider, source } = await buildPlan(config, { print: true, configFile });
  fs.mkdirSync(RUNS_ROOT, { recursive: true, mode: 0o700 });
  const runId = `${safeTimestamp()}-${crypto.randomBytes(4).toString("hex")}`;
  const directory = path.join(RUNS_ROOT, runId);
  fs.mkdirSync(directory, { recursive: false, mode: 0o700 });
  const shortCandidate = config.source.candidate_commit.slice(0, 8);
  const vultrLabel = `rustia-bench-${shortCandidate}-${crypto.randomBytes(4).toString("hex")}`;
  const ownershipTag = `rustia-owner-${crypto.randomBytes(12).toString("hex")}`;
  const state = {
    __directory: directory,
    schema: "rustia.vultr-benchmark-state.v1",
    run_id: runId,
    created_at: new Date().toISOString(),
    protocol_mode: "acceptance",
    vultr_label: vultrLabel,
    vultr_ownership_tag: ownershipTag,
    vultr_id: null,
    vultr_destroyed_at: null,
    private_key_path: path.join(directory, "id_ed25519"),
    public_key_path: path.join(directory, "id_ed25519.pub"),
    ssh_host_private_key_path: path.join(directory, "ssh_host_ed25519_key"),
    ssh_host_public_key_path: path.join(directory, "ssh_host_ed25519_key.pub"),
    ssh_host_key_alias: vultrLabel,
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
      ["-q", "-t", "ed25519", "-N", "", "-C", `${vultrLabel}-key`, "-f", state.private_key_path],
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
        `${vultrLabel}-host-key`,
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

    // Repeat every cost/scope-sensitive check immediately before the first billable request.
    const refreshedProvider = await resolveProviderPlan(token, config);
    const refreshedPlan = sanitizedProviderPlan(config, refreshedProvider, source);
    verifyProviderPlanUnchanged(plan, refreshedPlan);
    await inspectIamPermissions(token);
    await verifyDeleteAuthority(token);
    await verifyCreateAuthority(token);
    if (ownershipRelatedVultrs(await listVultrs(token), state).length !== 0) {
      fail("refusing to create because the unique Vultr ownership label or tag already exists");
    }
    state.vultr_precreate_absence_confirmed_at = new Date().toISOString();
    saveState(state);

    const publicKey = fs.readFileSync(state.public_key_path, "utf8").trim();
    const hostPrivateKey = fs.readFileSync(state.ssh_host_private_key_path, "utf8");
    const createBody = createVultrBody(
      config,
      vultrLabel,
      ownershipTag,
      publicKey,
      hostPrivateKey,
      hostPublicKey,
    );
    state.vultr_create_request_sha256 = sha256(Buffer.from(JSON.stringify(createBody)));
    saveState(state);
    startBillingWindow(state, config);
    const createStarted = Date.now();
    state.vultr_create_attempted_at = new Date(createStarted).toISOString();
    state.vultr_create_request_deadline_at = new Date(createStarted + API_TIMEOUT_MS).toISOString();
    state.vultr_create_outcome = "attempting";
    saveState(state);

    let createResponse = null;
    let createError = null;
    try {
      createResponse = await apiRequest(token, "POST", "/v2/bare-metals", {
        body: createBody,
        acceptedStatuses: [202],
      });
      state.vultr_create_outcome = "accepted_response";
    } catch (error) {
      createError = error;
      state.vultr_create_outcome = createOutcomeForError(error);
    }
    state.vultr_create_finished_at = new Date().toISOString();
    saveState(state);
    if (createResponse) {
      const created = bareMetalFromCreateResponse(createResponse);
      if (UUID.test(created.id) && created.label === state.vultr_label) {
        state.vultr_id = created.id;
        state.vultr_id_source = "create_response";
        state.vultr_create_outcome = "resource_identified";
        saveState(state);
      }
      if (!state.vultr_id) {
        const reconciled = await reconcileVultrAfterCreateAttempt(state, token);
        if (!reconciled) fail("Vultr create returned no uniquely reconcilable owned instance");
      }
    } else {
      const reconciled = await reconcileVultrAfterCreateAttempt(state, token);
      if (!reconciled) fail(`Vultr creation failed: ${createError ? createError.message : "unknown API error"}`);
    }

    const vultr = await waitForVultr(state, token);
    verifyVultrMatchesPlan(vultr, plan, state);
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
    writeJsonAtomic(providerMetadata, safeVultrMetadata(vultr, plan, state));
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
      await cleanupOwned(state, token);
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
  if (!state.vultr_destroyed_at) fail("Vultr deletion was not confirmed");
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
        "incomplete Vultr benchmark cleanup exists; run status and clean up: " +
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
    const token = await readHiddenToken();
    await resolveProviderPlan(token, config);
    await inspectIamPermissions(token);
    await verifyDeleteAuthority(token);
    await verifyCreateAuthority(token);
    writeBytesAtomic(TOKEN_FILE, Buffer.from(`${token}\n`), 0o600);
    process.stdout.write(`Vultr token validated and stored with mode 0600 at ${TOKEN_FILE}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function status() {
  const states = listStates();
  if (states.length === 0) {
    process.stdout.write("No locally owned Vultr benchmark runs.\n");
    return;
  }
  const token = readStoredToken({ allowMissing: true });
  const rows = [];
  for (const state of states) {
    let remote = null;
    if (!state.vultr_destroyed_at) {
      if (!token) {
        remote = { error: "stored token unavailable; run auth or supply cleanup from an authenticated host" };
      } else {
        try {
          if (state.vultr_id) {
            const vultr = await getOwnedVultr(state, token, { allowMissing: true });
            remote = vultr
              ? { id: vultr.id, label: vultr.label, status: vultr.status, public_ipv4: publicIpv4(vultr) }
              : { status: "not returned by get; run destroy to reconcile safely" };
          } else {
            const related = ownershipRelatedVultrs(await listVultrs(token), state);
            const matches = related.filter((entry) => exactOwnedVultr(entry, state));
            remote =
              related.length === 1 && matches.length === 1
                ? {
                    id: matches[0].id,
                    label: matches[0].label,
                    status: matches[0].status,
                    public_ipv4: publicIpv4(matches[0]),
                    note: "found by the journaled unique label; run destroy to adopt and clean it safely",
                  }
                : related.length === 0
                  ? { status: "unique label absent from a complete list; run destroy to confirm cleanup" }
                  : {
                      error: `ownership label/tag returned ${related.length} related and ${matches.length} exact Vultr resources; manual review required`,
                    };
          }
        } catch (error) {
          remote = { error: error.message };
        }
      }
    }
    const started = Date.parse(state.billing_window_started_at || "");
    const ended = Date.parse(state.vultr_destroyed_at || "");
    const seconds = Number.isFinite(started)
      ? Math.max(0, ((Number.isFinite(ended) ? ended : Date.now()) - started) / 1000)
      : 0;
    const hourly = state.plan && state.plan.size ? state.plan.size.price_hourly_usd : null;
    rows.push({
      run_id: state.run_id,
      created_at: state.created_at,
      vultr_id: state.vultr_id,
      vultr_label: state.vultr_label,
      vultr_create_attempted_at: state.vultr_create_attempted_at || null,
      vultr_create_outcome: state.vultr_create_outcome || null,
      vultr_absence_confirmed_at: state.vultr_absence_confirmed_at || null,
      destroyed_at: state.vultr_destroyed_at,
      cleanup_complete: Boolean(state.vultr_destroyed_at),
      billing_deadline_at: state.billing_deadline_at || null,
      billable_seconds_upper_bound: Number(seconds.toFixed(3)),
      rounded_compute_charge_upper_bound_usd:
        typeof hourly === "number" ? roundedHourlyCharge(hourly, seconds) : null,
      remote_status: state.remote_status || null,
      promoted_at: state.promoted_at || null,
      remote,
    });
  }
  process.stdout.write(`${JSON.stringify(rows, null, 2)}\n`);
}

async function destroy(runId) {
  const lockToken = acquireCoordinatorLock();
  try {
    const token = readStoredToken();
    const state = stateForRunId(runId);
    cleanupInProgress = true;
    try {
      await cleanupOwned(state, token);
    } finally {
      cleanupInProgress = false;
    }
    process.stdout.write(`Confirmed cleanup for owned Vultr run ${state.run_id}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function selfTest(config) {
  assert.equal(sha256(Buffer.from(canonicalJson(config), "utf8")), FROZEN_CONFIG_SHA256);
  assert.equal(config.schema, "rustia.vultr-fixed-benchmark.v1");
  assert.equal(config.publication.mode, "acceptance");
  assert.equal(config.provider.region, "sgp");
  assert.equal(config.provider.os.id, 2284);
  assert.equal(config.provider.size.id, "vbm-4c-32gb");
  assert.equal(config.provider.size.physical_cpus, 1);
  assert.equal(config.provider.size.cpu_count, 4);
  assert.equal(config.provider.size.cpu_cores, 4);
  assert.equal(config.provider.size.cpu_threads, 8);
  assert.equal(config.provider.size.vcpus, 8);
  assert.deepEqual(
    config.measurement.run_order.map((entry) => entry.variant),
    ["baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate"],
  );

  const planRow = {
    id: "vbm-4c-32gb",
    physical_cpus: 1,
    cpu_count: 4,
    cpu_cores: 4,
    cpu_threads: 8,
    cpu_manufacturer: "Intel",
    cpu_model: "E3-1270",
    cpu_mhz: 3800,
    ram: 32768,
    disk: 240,
    disk_count: 2,
    bandwidth: 5120,
    monthly_cost: 120,
    hourly_cost: 0.164,
    invoice_type: "monthly",
    type: "SSD",
    deploy_ondemand: true,
    deploy_preemptible: false,
    locations: ["ewr", "sgp"],
    gpu_brand: "none",
  };
  const responses = {
    plans: {
      data: {
        plans_metal: [planRow],
        meta: { total: 1, links: { next: "", prev: "" } },
      },
    },
    regions: {
      data: {
        regions: [{ id: "sgp", city: "Singapore", country: "SG", continent: "Asia" }],
        meta: { total: 1, links: { next: "", prev: "" } },
      },
    },
    operatingSystems: {
      data: {
        os: [{ id: 2284, name: "Ubuntu 24.04 LTS x64", arch: "x64", family: "ubuntu" }],
        meta: { total: 1, links: { next: "", prev: "" } },
      },
    },
    availability: {
      data: { available_plans: ["vbm-4c-32gb"], available_vpc_only_plans: [] },
    },
  };
  const selected = validateProviderCatalog(config, responses);
  assert.equal(selected.plan.id, "vbm-4c-32gb");
  assert.equal(selected.hourlyPrice, 0.164);
  const safePlan = sanitizedProviderPlan(config, selected, { fixture: true });
  assert.equal(safePlan.size.catalog_cpu_count, 4);
  assert.equal(safePlan.size.vcpus, 8);
  assert.equal(safePlan.planned_compute_cost_ceiling_usd, 0.328);
  assert.equal(safePlan.billing_increment_minutes, 60);
  assert.equal(safePlan.cleanup_reserve_minutes, 8);
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      plans: { data: { ...responses.plans.data, plans_metal: [{ ...planRow, cpu_model: "wrong" }] } },
    }),
  );
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      plans: { data: { ...responses.plans.data, plans_metal: [{ ...planRow, hourly_cost: 0.165 }] } },
    }),
  );
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      availability: { data: { available_plans: [], available_vpc_only_plans: ["vbm-4c-32gb"] } },
    }),
  );

  const catalogCalls = [];
  const catalogRequest = async (token, method, apiPath, options) => {
    catalogCalls.push([token, method, apiPath, options]);
    assert.equal(token, null);
    assert.equal(method, "GET");
    assert.equal(options.unauthenticated, true);
    if (apiPath === "/v2/plans-metal?per_page=500") return responses.plans;
    if (apiPath === "/v2/regions?per_page=500") return responses.regions;
    if (apiPath === "/v2/os?per_page=500") return responses.operatingSystems;
    if (apiPath === "/v2/regions/sgp/availability") return responses.availability;
    throw new Error(`unexpected fixture request ${method} ${apiPath}`);
  };
  const resolved = await resolveProviderPlan("fixture-token", config, catalogRequest);
  assert.equal(resolved.region.id, "sgp");
  assert.equal(catalogCalls.length, 4);

  const listed = await listVultrs("fixture-token", async (_token, method, apiPath) => {
    assert.equal(method, "GET");
    if (apiPath === "/v2/bare-metals?per_page=500") {
      return {
        data: {
          bare_metals: [{ id: "00000000-0000-4000-8000-000000000001" }],
          meta: {
            total: 2,
            links: { next: "next==", prev: "" },
          },
        },
      };
    }
    assert.equal(apiPath, "/v2/bare-metals?per_page=500&cursor=next%3D%3D");
    return {
      data: {
        bare_metals: [{ id: "00000000-0000-4000-8000-000000000002" }],
        meta: { total: 2, links: { next: "", prev: "previous" } },
      },
    };
  });
  assert.equal(listed.length, 2);
  await assert.rejects(() =>
    listVultrs("fixture-token", async () => ({
      data: {
        bare_metals: [],
        meta: { total: 0, links: { next: "https://example.com/v2/bare-metals", prev: "" } },
      },
    })),
  );

  const policyResponse = {
    data: {
      policies: {
        all: [
          {
            policy_document: {
              Statement: [
                { Effect: "Allow", Action: "compute.baremetal.*", Resource: "*" },
              ],
            },
          },
        ],
      },
    },
  };
  assert.equal(verifyRequiredIamActions(policyResponse).length, 4);
  const managedPolicyResponse = {
    data: {
      policies: {
        all: ["Deploy New Servers", "Manage Servers", "View Servers"].map((name) => ({
          name,
          is_system_policy: true,
          policy_document: { Statement: [] },
        })),
      },
    },
  };
  assert.equal(verifyRequiredManagedPolicyNames(managedPolicyResponse).length, 3);
  assert.equal(
    verifyRequiredIamActions({
      data: {
        policies: {
          all: [
            {
              policy_document: {
                Statement: [
                  { Effect: "Allow", Action: "user.acl.Provisioning", Resource: "*" },
                  { Effect: "Allow", Action: "user.acl.Subscriptions", Resource: "*" },
                  { Effect: "Allow", Action: "user.acl.SubscriptionsView", Resource: "*" },
                ],
              },
            },
          ],
        },
      },
    }).length,
    4,
  );
  const managedRoleResponse = {
    data: {
      roles: {
        all: [
          { name: "Deploy New Servers" },
          { name: "Manage Servers" },
          { name: "View Servers" },
        ],
      },
      meta: { total: 3, links: { next: "", prev: "" } },
    },
  };
  assert.equal(verifyRequiredManagedRoles(managedRoleResponse).length, 3);
  assert.equal(
    verifyRequiredManagedRoles({ data: { roles: managedRoleResponse.data.roles } }).length,
    3,
  );
  assert.throws(() =>
    verifyRequiredManagedRoles({
      data: {
        roles: { all: [{ name: "View Servers" }] },
        meta: { total: 1, links: { next: "", prev: "" } },
      },
    }),
  );
  assert.throws(() =>
    verifyRequiredIamActions({
      data: {
        policies: {
          all: [
            ...policyResponse.data.policies.all,
            {
              policy_document: JSON.stringify({
                Statement: [{ Effect: "Deny", Action: "compute.baremetal.Delete", Resource: "*" }],
              }),
            },
          ],
        },
      },
    }),
  );
  await inspectIamPermissions(
    "fixture-token",
    async (_token, method, apiPath) => {
      assert.equal(method, "GET");
      assert.equal(apiPath, "/v2/users/me/policies?per_page=500");
      return policyResponse;
    },
    { announce: false },
  );
  await assert.rejects(() =>
    inspectIamPermissions(
      "fixture-token",
      async (_token, method, apiPath) => {
        assert.equal(method, "GET");
        assert.equal(apiPath, "/v2/users/me/policies?per_page=500");
        return {
          data: {
            policies: {
              all: [
                ...managedPolicyResponse.data.policies.all,
                {
                  name: "deny-delete",
                  policy_document: {
                    Statement: [
                      { Effect: "Deny", Action: "compute.baremetal.Delete", Resource: "*" },
                    ],
                  },
                },
              ],
            },
          },
        };
      },
      { announce: false },
    ),
  );
  await assert.rejects(() =>
    inspectIamPermissions(
      "fixture-token",
      async (_token, method, apiPath) => {
        assert.equal(method, "GET");
        assert.equal(apiPath, "/v2/users/me/policies?per_page=500");
        return {
          data: {
            policies: {
              all: [{ name: "unrelated", policy_document: { Statement: [] } }],
            },
          },
        };
      },
      { announce: false },
    ),
  );
  await assert.rejects(() =>
    inspectIamPermissions(
      "fixture-token",
      async (_token, method, apiPath) => {
        assert.equal(method, "GET");
        assert.equal(apiPath, "/v2/users/me/policies?per_page=500");
        return { data: { policies: {} } };
      },
      { announce: false },
    ),
  );
  await assert.rejects(() =>
    inspectIamPermissions(
      "fixture-token",
      async (_token, method, apiPath) => {
        assert.equal(method, "GET");
        if (apiPath === "/v2/users/me/policies?per_page=500") {
          throw new VultrApiError("forbidden", 403);
        }
        assert.equal(apiPath, "/v2/users/me/roles?per_page=500");
        return { data: { roles: {} } };
      },
      { announce: false },
    ),
  );
  await inspectIamPermissions(
    "fixture-token",
    async (_token, method, apiPath) => {
      assert.equal(method, "GET");
      assert.equal(apiPath, "/v2/users/me/policies?per_page=500");
      return managedPolicyResponse;
    },
    { announce: false },
  );
  await inspectIamPermissions(
    "fixture-token",
    async (_token, method, apiPath) => {
      assert.equal(method, "GET");
      if (apiPath === "/v2/users/me/policies?per_page=500") {
        return { data: { policies: { all: [] } } };
      }
      assert.equal(apiPath, "/v2/users/me/roles?per_page=500");
      return managedRoleResponse;
    },
    { announce: false },
  );
  const emptyIntrospectionCalls = [];
  assert.deepEqual(await inspectIamPermissions(
    "fixture-token",
    async (_token, method, apiPath) => {
      assert.equal(method, "GET");
      emptyIntrospectionCalls.push(apiPath);
      if (apiPath === "/v2/users/me/policies?per_page=500") {
        return { data: { policies: { all: [] } } };
      }
      assert.equal(apiPath, "/v2/users/me/roles?per_page=500");
      return { data: { roles: { all: [] } } };
    },
    { announce: false },
  ), []);
  assert.deepEqual(emptyIntrospectionCalls, [
    "/v2/users/me/policies?per_page=500",
    "/v2/users/me/roles?per_page=500",
  ]);

  const authorityCalls = [];
  await verifyDeleteAuthority(
    "fixture-token",
    async (_token, method, apiPath, options = {}) => {
      authorityCalls.push([method, apiPath, options]);
      if (method === "GET") {
        return { data: { bare_metals: [], meta: { total: 0, links: { next: "", prev: "" } } } };
      }
      assert.equal(method, "DELETE");
      assert.deepEqual(options.acceptedStatuses, [404]);
      return { status: 404, data: { error: "Not found" } };
    },
    { announce: false },
  );
  assert.deepEqual(authorityCalls.map(([method]) => method), ["GET", "DELETE"]);
  await verifyCreateAuthority(
    "fixture-token",
    async (_token, method, apiPath, options) => {
      assert.equal(method, "POST");
      assert.equal(apiPath, "/v2/bare-metals");
      assert.deepEqual(options.body, {});
      assert.deepEqual(options.acceptedStatuses, [400]);
      return { status: 400, data: { error: "invalid request" } };
    },
    { announce: false },
  );
  assert.equal(createOutcomeForError(new VultrApiError("fixture", 422)), "definitively_rejected");
  assert.equal(createOutcomeForError(new VultrApiError("fixture", 500)), "ambiguous");
  assert.equal(createOutcomeForError(new VultrApiError("fixture")), "ambiguous");
  const createdFixture = { id: "00000000-0000-4000-8000-000000000003", label: "rustia-bench-fixture" };
  assert.equal(
    bareMetalFromCreateResponse({ data: { bare_metal: createdFixture } }),
    createdFixture,
  );
  assert.equal(
    bareMetalFromCreateResponse({ data: { baremetal: createdFixture } }),
    createdFixture,
  );
  assert.throws(() => bareMetalFromCreateResponse({ data: {} }));
  assert.throws(() =>
    bareMetalFromCreateResponse({ data: { bare_metal: createdFixture, baremetal: createdFixture } }),
  );

  assert.equal(roundedHourlyCharge(0.164, 0), 0);
  assert.equal(roundedHourlyCharge(0.164, 1), 0.164);
  assert.equal(roundedHourlyCharge(0.164, 3600), 0.164);
  assert.equal(roundedHourlyCharge(0.164, 3600.001), 0.328);
  assert.equal(roundedHourlyCharge(0.164, config.provider.max_runtime_minutes * 60), 0.328);
  assert.equal(ACTIVATION_TIMEOUT_MS, 60 * 60 * 1000);

  const publicKey = `ssh-ed25519 ${Buffer.alloc(32, 7).toString("base64")} rustia-test`;
  const hostPublicKey = `ssh-ed25519 ${Buffer.alloc(32, 8).toString("base64")} rustia-host-test`;
  const hostPrivateKey = [
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    Buffer.alloc(96, 9).toString("base64"),
    "-----END OPENSSH PRIVATE KEY-----",
    "",
  ].join("\n");
  const ownershipTag = "rustia-owner-00112233445566778899aabb";
  const body = createVultrBody(
    config,
    "rustia-bench-test",
    ownershipTag,
    publicKey,
    hostPrivateKey,
    hostPublicKey,
  );
  assert.equal(body.region, "sgp");
  assert.equal(body.plan, "vbm-4c-32gb");
  assert.equal(body.os_id, 2284);
  assert.equal(body.mdisk_mode, "none");
  assert.deepEqual(body.sshkey_id, []);
  assert.deepEqual(body.tags, [ownershipTag]);
  assert.equal(body.activation_email, false);
  assert.equal(body.enable_ipv6, false);
  assert.equal(body.persistent_pxe, false);
  const decodedUserData = Buffer.from(body.user_data, "base64").toString("utf8");
  assert.match(decodedUserData, /#cloud-config/);
  assert.match(decodedUserData, /ssh_deletekeys: true/);
  assert.match(decodedUserData, /ssh_genkeytypes: \[\]/);
  assert.match(decodedUserData, /ssh_keys:/);
  assert.match(decodedUserData, /ed25519_private: \|/);
  assert.match(decodedUserData, new RegExp(publicKey.split(" ")[1]));
  assert.match(decodedUserData, new RegExp(hostPublicKey.split(" ")[1]));
  assert.equal(Object.prototype.hasOwnProperty.call(body, "default_password"), false);
  assert.equal(Object.prototype.hasOwnProperty.call(body, "token"), false);
  assert.throws(() =>
    createVultrBody(config, "rustia-bench-test", ownershipTag, "not-a-key", hostPrivateKey, hostPublicKey),
  );
  assert.throws(() =>
    createVultrBody(config, "rustia-bench-test", ownershipTag, publicKey, "not-a-key", hostPublicKey),
  );
  const pinnedOptions = sshOptions({
    private_key_path: "/fixture/client-key",
    known_hosts_path: "/fixture/known-hosts",
    ssh_host_key_alias: "rustia-bench-test",
  });
  assert.equal(pinnedOptions.includes("StrictHostKeyChecking=yes"), true);
  assert.equal(pinnedOptions.includes("StrictHostKeyChecking=accept-new"), false);
  assert.equal(pinnedOptions.includes("HostKeyAlias=rustia-bench-test"), true);
  assert.equal(pinnedOptions.includes("GlobalKnownHostsFile=/dev/null"), true);
  assert.equal(pinnedOptions.includes("KnownHostsCommand=none"), true);
  assert.equal(pinnedOptions.includes("HostKeyAlgorithms=ssh-ed25519"), true);
  assert.deepEqual(pinnedOptions.slice(0, 2), ["-F", "/dev/null"]);

  assert.equal(isPublicIpv4("8.8.8.8"), true);
  for (const address of [
    "999.1.2.3",
    "0.1.2.3",
    "10.0.0.1",
    "100.64.0.1",
    "127.0.0.1",
    "169.254.1.1",
    "172.16.0.1",
    "192.0.2.1",
    "192.168.1.1",
    "198.18.0.1",
    "198.51.100.1",
    "203.0.113.9",
    "224.0.0.1",
  ]) {
    assert.equal(isPublicIpv4(address), false, address);
  }
  assert.equal(publicIpv4({ main_ip: "8.8.8.8" }), "8.8.8.8");

  const createdAt = "2026-07-19T04:00:01+00:00";
  const ownershipState = {
    vultr_label: "rustia-bench-test",
    vultr_ownership_tag: ownershipTag,
    vultr_create_attempted_at: "2026-07-19T04:00:00.000Z",
    vultr_id: "00000000-0000-4000-8000-000000000123",
    plan: {
      region: { id: "sgp" },
      os: { id: 2284, name: "Ubuntu 24.04 LTS x64" },
      size: { id: "vbm-4c-32gb", catalog_cpu_count: 4, vcpus: 8 },
    },
  };
  const actualShape = {
    id: ownershipState.vultr_id,
    label: ownershipState.vultr_label,
    tags: [ownershipTag],
    date_created: createdAt,
    plan: "vbm-4c-32gb",
    region: "sgp",
    os_id: 2284,
    os: "Ubuntu 24.04 LTS x64",
    cpu_count: 4,
    user_scheme: "root",
    image_id: "",
    snapshot_id: "",
    app_id: 0,
    features: [],
  };
  assert.equal(exactOwnedVultr(actualShape, ownershipState), true);
  assert.equal(exactOwnedVultr({ ...actualShape, tags: ["wrong"] }, ownershipState), false);
  assert.equal(
    cleanupOwnedVultrMatches(
      { ...actualShape, tags: [] },
      { ...ownershipState, vultr_id_source: "create_response" },
    ),
    true,
  );
  assert.equal(ownershipRelatedVultrs([actualShape], ownershipState).length, 1);
  verifyOwnedVultrIdentity(actualShape, ownershipState);
  verifyVultrMatchesPlan(actualShape, ownershipState.plan, ownershipState);
  assert.throws(() => verifyVultrMatchesPlan({ ...actualShape, cpu_count: 8 }, ownershipState.plan, ownershipState));
  assert.throws(() => verifyOwnedVultrIdentity({ ...actualShape, plan: "wrong" }, ownershipState));

  assert.throws(() =>
    validateConfig({
      ...config,
      source: { ...config.source, candidate_commit: config.source.baseline_commit },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      publication: { ...config.publication, mode: "diagnostic" },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      measurement: { ...config.measurement, fixed_n: config.measurement.fixed_n + 1 },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      measurement: {
        ...config.measurement,
        run_order: config.measurement.run_order.map((leg, index) =>
          index === 0 ? { ...leg, file: "changed.json" } : leg,
        ),
      },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      host_gate: { ...config.host_gate, maximum_steal_ticks: 1 },
    }),
  );
  assert.throws(() =>
    validateConfig({
      ...config,
      publication: { ...config.publication, final_measurements: 4 },
    }),
  );

  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-vultr-self-test-"));
  try {
    const file = path.join(temporary, "secret");
    writeBytesAtomic(file, Buffer.from("fixture\n"), 0o600);
    assert.equal(fs.statSync(file).mode & 0o777, 0o600);
    assert.equal(fs.readFileSync(file, "utf8"), "fixture\n");

    const knownHosts = path.join(temporary, "known_hosts");
    writePinnedKnownHost(
      { ssh_host_key_alias: "rustia-bench-test", known_hosts_path: knownHosts },
      hostPublicKey,
    );
    assert.equal(fs.statSync(knownHosts).mode & 0o777, 0o600);
    assert.equal(
      fs.readFileSync(knownHosts, "utf8"),
      `rustia-bench-test ssh-ed25519 ${hostPublicKey.split(" ")[1]}\n`,
    );

    const tarFixture = (entries, format = "ustar") => {
      const records = [];
      const writeOctal = (header, offset, length, value) => {
        const text = value.toString(8).padStart(length - 1, "0");
        header.write(text, offset, length - 1, "ascii");
        header[offset + length - 1] = 0;
      };
      for (const entry of entries) {
        const payload = Buffer.from(entry.payload || "", "utf8");
        const header = Buffer.alloc(512);
        header.write(entry.name, 0, 100, "ascii");
        writeOctal(header, 100, 8, entry.type === "5" ? 0o755 : 0o644);
        writeOctal(header, 108, 8, 0);
        writeOctal(header, 116, 8, 0);
        writeOctal(header, 124, 12, payload.length);
        writeOctal(header, 136, 12, 0);
        header.fill(0x20, 148, 156);
        header[156] = entry.type.charCodeAt(0);
        if (entry.linkname) header.write(entry.linkname, 157, 100, "ascii");
        if (format === "ustar") {
          header.write("ustar\0", 257, 6, "ascii");
          header.write("00", 263, 2, "ascii");
        } else if (format === "oldgnu") {
          header.write("ustar ", 257, 6, "ascii");
          header[263] = 0x20;
          header[264] = 0;
        }
        let checksum = 0;
        for (const byte of header) checksum += byte;
        const checksumText = checksum.toString(8).padStart(6, "0");
        header.write(checksumText, 148, 6, "ascii");
        header[154] = 0;
        header[155] = 0x20;
        records.push(header, payload, Buffer.alloc((512 - (payload.length % 512)) % 512));
      }
      records.push(Buffer.alloc(1024));
      return Buffer.concat(records);
    };
    const safeTar = tarFixture([
      { name: "./", type: "5" },
      { name: "./status.json", type: "0", payload: "{}\n" },
    ]);
    assert.deepEqual(
      inspectTarBytes(safeTar).map((entry) => [entry.path, entry.type]),
      [[".", "directory"], ["status.json", "file"]],
    );
    assert.throws(
      () => inspectTarBytes(tarFixture([{ name: "./", type: "5" }], "unknown")),
      /not exact POSIX ustar/,
    );
    for (const [entry, pattern] of [
      [{ name: "../escape", type: "0" }, /parent component/],
      [{ name: "/absolute", type: "0" }, /absolute/],
      [{ name: "./link", type: "2", linkname: "status.json" }, /forbidden tar type/],
      [{ name: "./hardlink", type: "1", linkname: "status.json" }, /forbidden tar type/],
      [{ name: "./device", type: "3" }, /forbidden tar type/],
    ]) {
      assert.throws(() => inspectTarBytes(tarFixture([{ name: "./", type: "5" }, entry])), pattern);
    }
    const archiveFixture = path.join(temporary, "safe-export.tar.gz");
    fs.writeFileSync(archiveFixture, zlib.gzipSync(safeTar));
    assert.equal(inspectArchive(archiveFixture).length, 2);

    const safeTree = path.join(temporary, "safe-tree");
    fs.mkdirSync(safeTree);
    fs.writeFileSync(path.join(safeTree, "status.json"), "{}\n");
    validateExtractedTree(safeTree, [
      { path: ".", type: "directory" },
      { path: "status.json", type: "file" },
    ]);
    fs.symlinkSync("status.json", path.join(safeTree, "status-link"));
    assert.throws(() => validateExtractedTree(safeTree), /symlink/);
    fs.unlinkSync(path.join(safeTree, "status-link"));
    fs.linkSync(path.join(safeTree, "status.json"), path.join(safeTree, "status-hardlink"));
    assert.throws(() => validateExtractedTree(safeTree), /hard-linked/);

    const hashFixture = path.join(temporary, "hash-fixture");
    fs.mkdirSync(hashFixture);
    fs.writeFileSync(path.join(hashFixture, "analysis.json"), "{}\n");
    const hashManifest = { evidence_sha256: evidenceFileHashes(hashFixture) };
    validateManifestEvidenceHashes(hashManifest, hashFixture);
    fs.writeFileSync(path.join(hashFixture, "analysis.json"), "{\"tampered\":true}\n");
    assert.throws(() => validateManifestEvidenceHashes(hashManifest, hashFixture), /does not match/);

    const prototypeHashFixture = path.join(temporary, "prototype-hash-fixture");
    fs.mkdirSync(prototypeHashFixture);
    fs.writeFileSync(path.join(prototypeHashFixture, "__proto__"), "owned\n");
    const prototypeHashes = evidenceFileHashes(prototypeHashFixture);
    assert.equal(Object.prototype.hasOwnProperty.call(prototypeHashes, "__proto__"), true);
    validateManifestEvidenceHashes({ evidence_sha256: prototypeHashes }, prototypeHashFixture);
    assert.throws(
      () => validateManifestEvidenceHashes({ evidence_sha256: {} }, prototypeHashFixture),
      /does not match/,
    );

    const fixtureTopology = {};
    for (let cpu = 0; cpu < config.provider.size.vcpus; cpu += 1) {
      const core = cpu % config.provider.size.cpu_cores;
      fixtureTopology[`cpu${cpu}`] = {
        socket: "0",
        core: String(core),
        siblings: [core, core + config.provider.size.cpu_cores],
      };
    }
    const hostAttestationFixture = {
      virtualization: "none",
      cpu_model: "Intel(R) Xeon(R) CPU E3-1270 v6 @ 3.80GHz",
      bare_metal_attestation: {
        systemd_detect_virt: "none",
        systemd_detect_virt_status: 1,
        hypervisor_cpu_flag: false,
        sys_hypervisor_type: "absent_or_empty",
        logical_cpus: config.provider.size.vcpus,
        physical_sockets: config.provider.size.physical_cpus,
        physical_cores: config.provider.size.cpu_cores,
        cpu_model: "Intel(R) Xeon(R) CPU E3-1270 v6 @ 3.80GHz",
        controller_cpu: 0,
        benchmark_cpu: config.host_gate.benchmark_cpu,
        controller_and_benchmark_are_smt_siblings: false,
        topology: fixtureTopology,
      },
    };
    validateBareMetalHost(config, hostAttestationFixture);
    assert.throws(
      () =>
        validateBareMetalHost(config, {
          ...hostAttestationFixture,
          bare_metal_attestation: {
            ...hostAttestationFixture.bare_metal_attestation,
            hypervisor_cpu_flag: true,
          },
        }),
      /hypervisor CPU flag/,
    );

    const gateLabels = [
      ...Array.from({ length: config.qualification.candidate_pilot_legs }, (_, index) => `qualification-${index + 1}`),
      ...config.measurement.run_order.map((leg, index) => `accepted-${index + 1}-${leg.file}`),
      ...Array.from({ length: config.publication.final_measurements }, (_, index) => `publication-${index + 1}`),
    ];
    const passingPre = (label) => ({
      phase: "pre_leg_idle_window",
      label,
      timestamp: "2026-07-19T05:00:00.000Z",
      cpu: config.host_gate.benchmark_cpu,
      load_1m: 0,
      cpu_delta: { total_ticks: 1000, idle_ticks: 1000, idle_percent: 100, steal_ticks: 0 },
      contaminants: [],
      passed: true,
    });
    const passingDuring = (label) => ({
      phase: "during_leg",
      label,
      timestamp: "2026-07-19T05:00:01.000Z",
      cpu: config.host_gate.benchmark_cpu,
      cpu_delta: { total_ticks: 100, idle_ticks: 50, idle_percent: 50, steal_ticks: 0 },
      contaminants: [],
      passed: true,
    });
    const gateRecordsFixture = gateLabels.flatMap((label) => [passingPre(label), passingDuring(label)]);
    gateRecordsFixture.unshift({
      ...passingPre(gateLabels[0]),
      load_1m: config.host_gate.maximum_load_1m + 1,
      passed: false,
    });
    const gateFixture = path.join(temporary, "gate.jsonl");
    fs.writeFileSync(gateFixture, `${gateRecordsFixture.map((record) => JSON.stringify(record)).join("\n")}\n`);
    validateGateLog(config, gateFixture, gateRecordsFixture.length);
    const stolenGateRecords = gateRecordsFixture.map((record) => ({ ...record, cpu_delta: { ...record.cpu_delta } }));
    const duringIndex = stolenGateRecords.findIndex((record) => record.phase === "during_leg");
    stolenGateRecords[duringIndex].cpu_delta.steal_ticks = 1;
    fs.writeFileSync(gateFixture, `${stolenGateRecords.map((record) => JSON.stringify(record)).join("\n")}\n`);
    assert.throws(() => validateGateLog(config, gateFixture, stolenGateRecords.length), /zero-steal/);

    const historyEvidenceFixture = path.join(temporary, "history-evidence");
    fs.mkdirSync(historyEvidenceFixture);
    const historyRowsFixture = [];
    for (let index = 1; index <= config.publication.final_measurements; index += 1) {
      const samples = Array.from(
        { length: config.measurement.repetitions },
        (_, sampleIndex) => 1000 + index * 10 + sampleIndex,
      );
      const minimum = samples[0];
      const median = samples[Math.floor(samples.length / 2)];
      const measurement = {
        timestamp_unix: 1_800_000_000 + index,
        commit: config.source.candidate_commit,
        target: "linux-x86_64",
        engine: "rustia",
        dialect: "luau",
        workload: config.measurement.workload,
        unit: "fixture dense unit",
        n: config.measurement.fixed_n,
        n_selection: "fixed",
        reps: config.measurement.repetitions,
        ns_per_unit_min: minimum,
        ns_per_unit_median: median,
        units_per_sec: 1e9 / minimum,
        checksum: config.measurement.expected_checksum,
        samples_ns_per_unit: samples,
      };
      const sidecar = {
        schema_version: 1,
        schema: "rustia.bench.raw-samples.v1",
        source: "script-bench-harness",
        metadata: {
          description:
            "Raw per-repetition timings in execution order. This sidecar is separate from the append-only public history schema.",
          measurement_isolation: "fresh prepared workload per repetition",
          warmup: "one untimed invocation immediately before each recorded sample",
          sample_order: "execution order (unsorted)",
          measurement_count: 1,
          sample_count: config.measurement.repetitions,
        },
        measurements: [measurement],
      };
      fs.writeFileSync(
        path.join(historyEvidenceFixture, `final-dense-${index}.json`),
        `${JSON.stringify(sidecar, null, 2)}\n`,
      );
      historyRowsFixture.push({
        timestamp_unix: measurement.timestamp_unix,
        commit: measurement.commit,
        target: measurement.target,
        engine: measurement.engine,
        dialect: measurement.dialect,
        workload: measurement.workload,
        unit: measurement.unit,
        n: measurement.n,
        reps: measurement.reps,
        ns_per_unit_min: measurement.ns_per_unit_min,
        ns_per_unit_median: measurement.ns_per_unit_median,
        units_per_sec: measurement.units_per_sec,
        checksum: measurement.checksum,
      });
    }
    const localHistoryFixture = await gitBlob(
      config.source.candidate_commit,
      "bench/history/runs.jsonl",
    );
    assert.equal(
      sha256(localHistoryFixture),
      config.source.expected_sha256.history_before,
      "candidate history fixture does not match frozen history_before",
    );
    const remoteHistoryFixture = Buffer.concat([
      localHistoryFixture,
      Buffer.from(`${historyRowsFixture.map((row) => JSON.stringify(row)).join("\n")}\n`),
    ]);
    validatePublishedHistory(config, localHistoryFixture, remoteHistoryFixture, historyEvidenceFixture, {
      dialect: "luau",
      unit: "fixture dense unit",
    });
    const tamperedRows = historyRowsFixture.map((row) => ({ ...row }));
    tamperedRows[1].commit = config.source.baseline_commit;
    const tamperedHistory = Buffer.concat([
      localHistoryFixture,
      Buffer.from(`${tamperedRows.map((row) => JSON.stringify(row)).join("\n")}\n`),
    ]);
    assert.throws(
      () =>
        validatePublishedHistory(config, localHistoryFixture, tamperedHistory, historyEvidenceFixture, {
          dialect: "luau",
          unit: "fixture dense unit",
        }),
      /does not exactly correspond/,
    );

    const acceptedStatusFixture = {
      schema: "rustia.bench.cloud-run-status.v1",
      status: "accepted_and_published",
      detail: "material regression excluded and three actual final-binary history rows appended",
      started_at: "2026-07-19T05:00:00.000Z",
      finished_at: "2026-07-19T05:30:00.000Z",
      verdict: "material_regression_excluded",
    };
    validateAcceptedStatus(acceptedStatusFixture, config.publication.required_verdict);
    assert.throws(
      () => validateAcceptedStatus({ ...acceptedStatusFixture, verdict: "inconclusive" }, config.publication.required_verdict),
      /wrong verdict/,
    );
    assert.throws(
      () => validateAcceptedStatus({ ...acceptedStatusFixture, unexpected: true }, config.publication.required_verdict),
      /keys must be/,
    );

    const reconciliationNow = Date.parse("2026-07-19T05:00:00.000Z");
    const emptyListRequest = async (_token, method, apiPath, options) => {
      assert.equal(method, "GET");
      assert.equal(apiPath, "/v2/bare-metals?per_page=500");
      assert.deepEqual(options.acceptedStatuses, [200]);
      return {
        status: 200,
        data: { bare_metals: [], meta: { total: 0, links: { next: "", prev: "" } } },
      };
    };
    const reconciliationState = (name, outcome) => {
      const directory = path.join(temporary, name);
      fs.mkdirSync(directory);
      const state = {
        __directory: directory,
        vultr_label: `rustia-bench-${name}`,
        vultr_ownership_tag: `rustia-owner-${name}`,
        vultr_create_attempted_at: "2026-07-19T05:00:00.000Z",
        vultr_create_finished_at: "2026-07-19T05:00:00.000Z",
        vultr_create_request_deadline_at: "2026-07-19T05:00:00.000Z",
        vultr_create_outcome: outcome,
      };
      saveState(state);
      return state;
    };
    const reconciliationOptions = {
      reconcileTimeoutMs: 0,
      now: () => reconciliationNow,
      sleep: async () => assert.fail("zero-time reconciliation must not sleep"),
    };
    const ambiguousState = reconciliationState("ambiguous", "ambiguous");
    await assert.rejects(
      () => reconcileVultrAfterCreateAttempt(
        ambiguousState,
        "fixture-token",
        emptyListRequest,
        reconciliationOptions,
      ),
      /create outcome remains ambiguous/,
    );
    assert.equal(ambiguousState.vultr_absence_confirmed_at, undefined);
    assert.equal(ambiguousState.vultr_destroyed_at, undefined);

    const rejectedState = reconciliationState("rejected", "definitively_rejected");
    assert.equal(
      await reconcileVultrAfterCreateAttempt(
        rejectedState,
        "fixture-token",
        emptyListRequest,
        reconciliationOptions,
      ),
      null,
    );
    assert.equal(rejectedState.vultr_absence_confirmed_at, "2026-07-19T05:00:00.000Z");

    const absentDirectory = path.join(temporary, "confirmed-delete");
    fs.mkdirSync(absentDirectory);
    const absentId = "00000000-0000-4000-8000-000000000456";
    const absentState = {
      __directory: absentDirectory,
      run_id: "confirmed-delete",
      vultr_id: absentId,
      vultr_label: "rustia-bench-confirmed-delete",
      vultr_ownership_tag: "rustia-owner-confirmed-delete",
      vultr_create_attempted_at: "2026-07-19T05:00:00.000Z",
      vultr_create_finished_at: "2026-07-19T05:00:01.000Z",
      vultr_create_outcome: "resource_identified",
    };
    saveState(absentState);
    const deletionCalls = [];
    await deleteVultr(absentState, "fixture-token", async (_token, method, apiPath, options) => {
      deletionCalls.push([method, apiPath]);
      if (apiPath === `/v2/bare-metals/${absentId}`) {
        assert.equal(method, "GET");
        assert.deepEqual(options.acceptedStatuses, [200, 404]);
        return { status: 404, data: { error: "not found" } };
      }
      assert.equal(method, "GET");
      assert.equal(apiPath, "/v2/bare-metals?per_page=500");
      return {
        status: 200,
        data: { bare_metals: [], meta: { total: 0, links: { next: "", prev: "" } } },
      };
    });
    assert.deepEqual(deletionCalls.map(([method]) => method), ["GET", "GET", "GET"]);
    assert.equal(Boolean(absentState.vultr_destroyed_at), true);
    assert.match(absentState.cleanup_note, /GET returned 404 and two complete lists/);
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }

  const runner = await run("node", [REMOTE_RUNNER, "self-test"]);
  assert.match(outputText(runner), /self-test passed/);
  process.stdout.write("vultr benchmark coordinator offline self-test passed\n");
}

function requireCommandArguments(minimum, maximum = minimum) {
  const count = process.argv.length - 2;
  if (count < minimum || count > maximum) fail("unexpected command arguments; tokens are never accepted in argv");
}

function usage() {
  process.stdout.write(`Usage:
  node tools/run-vultr-benchmark.js auth
  node tools/run-vultr-benchmark.js plan
  node tools/run-vultr-benchmark.js bench
  node tools/run-vultr-benchmark.js status
  node tools/run-vultr-benchmark.js destroy [RUN-ID]
  node tools/run-vultr-benchmark.js unlock
  node tools/run-vultr-benchmark.js self-test
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
  process.stderr.write(`run-vultr-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
