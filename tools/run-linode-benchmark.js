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
const DEFAULT_CONFIG = path.join(REPO, "bench/cloud/linode-dense-ed65077.json");
const REMOTE_RUNNER = path.join(REPO, "tools/run-linux-fixed-benchmark.js");
const STATE_ROOT = path.join(REPO, ".rustia-benchmark-state/linode");
const RUNS_ROOT = path.join(STATE_ROOT, "runs");
const TOKEN_FILE = path.join(STATE_ROOT, "token");
const COORDINATOR_LOCK = path.join(STATE_ROOT, "coordinator.lock");
const API_HOST = "api.linode.com";
const API_TIMEOUT_MS = 30 * 1000;
const CREATE_RECONCILE_TIMEOUT_MS = 2 * 60 * 1000;
const DELETE_CONFIRM_TIMEOUT_MS = 4 * 60 * 1000;
const CLEANUP_RESERVE_MS = 8 * 60 * 1000;
const FORCE_KILL_DELAY_MS = 10 * 1000;
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const SAFE_FILE = /^[A-Za-z0-9][A-Za-z0-9._-]*\.json$/;
const SAFE_WORKLOAD_FILE_COMPONENT = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/;
const RUN_ID = /^\d{8}T\d{6}Z-[0-9a-f]{8}$/;
const MAX_EXPORT_ARCHIVE_BYTES = 256 * 1024 * 1024;
const MAX_EXPORT_UNCOMPRESSED_BYTES = 512 * 1024 * 1024;
const MAX_EXPORT_ENTRIES = 10_000;

let activeChild = null;
let activeRequest = null;
let activeForceKillTimer = null;
let interruptedSignal = null;
let cleanupInProgress = false;

class LinodeApiError extends Error {
  constructor(message, status = null, response = null) {
    super(message);
    this.name = "LinodeApiError";
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
  if (root.schema !== "rustia.linode-fixed-benchmark.v2") {
    fail("unsupported Linode benchmark config schema");
  }

  const provider = requireObject(root.provider, "config.provider");
  requireExactKeys(
    provider,
    ["region", "image", "size", "max_runtime_minutes"],
    "config.provider",
  );
  requireString(provider.region, "config.provider.region");
  requireString(provider.image, "config.provider.image");
  requireInteger(provider.max_runtime_minutes, "config.provider.max_runtime_minutes", 1);
  const size = requireObject(provider.size, "config.provider.size");
  requireExactKeys(
    size,
    ["id", "description", "class", "vcpus", "memory_mib", "disk_gib", "max_hourly_price_usd"],
    "config.provider.size",
  );
  requireString(size.id, "config.provider.size.id");
  requireString(size.description, "config.provider.size.description");
  requireString(size.class, "config.provider.size.class");
  requireInteger(size.vcpus, "config.provider.size.vcpus", 2);
  requireInteger(size.memory_mib, "config.provider.size.memory_mib", 1);
  requireInteger(size.disk_gib, "config.provider.size.disk_gib", 1);
  requireNumber(size.max_hourly_price_usd, "config.provider.size.max_hourly_price_usd", 0.001);
  if (provider.max_runtime_minutes >= 60) {
    fail("config.provider.max_runtime_minutes must remain below Linode's first whole billing hour");
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
      "maximum_timed_steal_percent",
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
  requireNumber(gate.maximum_timed_steal_percent, "config.host_gate.maximum_timed_steal_percent", 0);
  if (gate.maximum_steal_ticks !== 0 || gate.maximum_timed_steal_percent !== 0.1) {
    fail("Linode v2 steal budget must remain zero while idle and 0.1% during a timed leg");
  }
  requireInteger(gate.wait_timeout_seconds, "config.host_gate.wait_timeout_seconds", 1);

  const publication = requireObject(root.publication, "config.publication");
  requireExactKeys(publication, ["mode", "required_verdict", "final_measurements"], "config.publication");
  if (publication.mode !== "acceptance") fail("Linode coordinator is acceptance-only");
  if (publication.required_verdict !== "material_regression_excluded") {
    fail("config.publication.required_verdict must be material_regression_excluded");
  }
  requireInteger(publication.final_measurements, "config.publication.final_measurements", 3);
  return root;
}

function pathIsInside(directory, candidate) {
  const relative = path.relative(directory, candidate);
  return relative !== "" && relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function resolveConfigFile(argument, { repo = REPO, defaultConfig = DEFAULT_CONFIG } = {}) {
  let file;
  if (argument === undefined) {
    file = path.resolve(defaultConfig);
  } else {
    const relative = requireString(argument, "Linode benchmark config path");
    if (path.isAbsolute(relative) || relative.split(/[\\/]+/).includes("..")) {
      fail("Linode benchmark config path must be repository-relative and contain no .. segments");
    }
    file = path.resolve(repo, relative);
  }

  const resolvedRepo = fs.realpathSync(repo);
  if (!pathIsInside(path.resolve(repo), file)) {
    fail("Linode benchmark config path must resolve inside the repository");
  }

  let metadata;
  try {
    metadata = fs.lstatSync(file);
  } catch (error) {
    fail(`cannot inspect Linode benchmark config ${path.relative(repo, file)}: ${error.message}`);
  }
  if (metadata.isSymbolicLink()) fail("Linode benchmark config must not be a symbolic link");
  if (!metadata.isFile()) fail("Linode benchmark config must be a regular file");

  let realFile;
  try {
    realFile = fs.realpathSync(file);
  } catch (error) {
    fail(`cannot resolve Linode benchmark config ${path.relative(repo, file)}: ${error.message}`);
  }
  if (!pathIsInside(resolvedRepo, realFile)) {
    fail("Linode benchmark config path must resolve inside the repository");
  }
  return file;
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

function workloadFileComponent(workload) {
  const value = requireString(workload, "benchmark workload");
  if (SAFE_WORKLOAD_FILE_COMPONENT.test(value)) return value;
  const slug = value
    .toLowerCase()
    .replace(/[^a-z0-9._-]+/g, "-")
    .replace(/^[-._]+|[-._]+$/g, "")
    .slice(0, 48)
    .replace(/[-._]+$/g, "");
  return `${slug || "workload"}-${sha256(Buffer.from(value, "utf8")).slice(0, 12)}`;
}

function finalSidecarName(config, index) {
  requireInteger(index, "final sidecar index", 1);
  const file = `final-${workloadFileComponent(config.measurement.workload)}-${index}.json`;
  if (!SAFE_FILE.test(file)) fail("generated final sidecar name is not a safe JSON basename");
  return file;
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
  if (Array.isArray(response.errors)) {
    const detail = response.errors
      .map((entry) => {
        if (!entry || typeof entry !== "object") return String(entry);
        const reason = typeof entry.reason === "string" ? entry.reason : "API error";
        const field = typeof entry.field === "string" ? ` (${entry.field})` : "";
        return `${reason}${field}`;
      })
      .join("; ");
    if (detail) return detail.slice(0, 2000);
  }
  return "unrecognized API error response";
}

async function apiRequest(token, method, apiPath, options = {}) {
  throwIfInterrupted();
  if (!/^\/v4\//.test(apiPath)) fail("Linode API path must stay under /v4/");
  const {
    body = null,
    acceptedStatuses = null,
    timeoutMs = API_TIMEOUT_MS,
    unauthenticated = false,
  } = options;
  if (unauthenticated) {
    const publicCatalogPath = [
      /^\/v4\/linode\/types\/[^/?]+$/,
      /^\/v4\/images\/[^/?]+$/,
      /^\/v4\/regions\/[^/?]+$/,
      /^\/v4\/regions\/[^/?]+\/availability$/,
    ].some((pattern) => pattern.test(apiPath));
    if (method !== "GET" || !publicCatalogPath || body !== null) {
      fail("unauthenticated Linode requests are limited to exact public catalog GETs");
    }
  } else if (typeof token !== "string" || token.length === 0) {
    fail("Linode token is unavailable");
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
          "User-Agent": "rustia-linode-benchmark/1",
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
            response.destroy(new Error("Linode API response exceeded 32 MiB"));
            return;
          }
          chunks.push(Buffer.from(chunk));
        });
        response.on("error", (error) => finish(reject, new LinodeApiError(`Linode API response failed: ${error.message}`)));
        response.on("end", () => {
          const bytes = Buffer.concat(chunks);
          let data = null;
          if (bytes.length > 0) {
            try {
              data = JSON.parse(bytes.toString("utf8"));
            } catch (error) {
              finish(reject, new LinodeApiError(`Linode API returned invalid JSON: ${error.message}`, response.statusCode));
              return;
            }
          }
          const status = response.statusCode || 0;
          const accepted = acceptedStatuses
            ? acceptedStatuses.includes(status)
            : status >= 200 && status < 300;
          if (!accepted) {
            finish(reject, new LinodeApiError(`Linode API ${method} ${apiPath} returned ${status}: ${apiErrorDetail(data)}`, status, data));
            return;
          }
          finish(resolve, { status, data, headers: response.headers });
        });
      },
    );
    activeRequest = request;
    request.setTimeout(timeoutMs, () => request.destroy(new Error("Linode API request timed out")));
    request.on("error", (error) => {
      finish(reject, new LinodeApiError(`Linode API ${method} ${apiPath} failed: ${error.message}`));
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
    fail("Linode token is not configured; run the auth command from an interactive terminal");
  }
  if (!stat.isFile() || stat.isSymbolicLink()) fail(`refusing non-regular token file ${TOKEN_FILE}`);
  if ((stat.mode & 0o077) !== 0) fail(`Linode token file permissions must be 0600: ${TOKEN_FILE}`);
  const token = fs.readFileSync(TOKEN_FILE, "utf8").trim();
  if (!token || /\s/.test(token)) fail(`Linode token file is empty or malformed: ${TOKEN_FILE}`);
  return token;
}

async function readHiddenToken() {
  if (!process.stdin.isTTY || !process.stderr.isTTY || typeof process.stdin.setRawMode !== "function") {
    fail("auth requires an interactive TTY; the token is never accepted in argv or environment variables");
  }
  process.stderr.write("Linode personal access token (input hidden): ");
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

async function listLinodes(token, request = apiRequest) {
  const all = [];
  let page = 1;
  let pages = 1;
  do {
    const response = await request(token, "GET", `/v4/linode/instances?page=${page}&page_size=500`);
    const envelope = requireApiObject(response, "Linode instance list response");
    if (!Array.isArray(envelope.data)) fail("Linode instance list response.data must be an array");
    all.push(...envelope.data);
    pages = requireInteger(envelope.pages, "Linode instance list response.pages", 1);
    page += 1;
  } while (page <= pages);
  return all;
}

async function preflightAccount(token, request = apiRequest) {
  const settings = requireApiObject(
    await request(token, "GET", "/v4/account/settings"),
    "Linode account settings",
  );
  if (settings.backups_enabled !== false) {
    fail(
      "Linode account-wide backups_enabled must be false before billing; otherwise the " +
        "ephemeral instance would incur an additional backup charge",
    );
  }
  if (settings.interfaces_for_new_linodes === "linode_only") {
    fail(
      "Linode account settings require linode-only interfaces, but this coordinator pins " +
        "legacy_config so firewall_id=-1 can explicitly prevent a default firewall attachment",
    );
  }
  return settings;
}

async function preflightAccountRegion(token, config, request = apiRequest) {
  const regionId = encodeURIComponent(config.provider.region);
  const availability = requireApiObject(
    await request(token, "GET", `/v4/account/availability/${regionId}`),
    "Linode account region availability",
  );
  if (!Array.isArray(availability.available) || !Array.isArray(availability.unavailable)) {
    fail("Linode account region availability must contain available and unavailable arrays");
  }
  const malformed = [...availability.available, ...availability.unavailable].some(
    (service) => typeof service !== "string" || service.length === 0,
  );
  if (malformed) fail("Linode account region availability contains a malformed service name");
  const available = availability.available.filter((service) => service === "Linodes").length;
  const unavailable = availability.unavailable.filter((service) => service === "Linodes").length;
  if (available !== 1 || unavailable !== 0) {
    fail(
      `Linode instances are not available to this account in ${config.provider.region}; ` +
        "select an account-enabled region before billing",
    );
  }
  return availability;
}

function effectiveTypePrice(type, region) {
  const regional = Array.isArray(type.region_prices)
    ? type.region_prices.filter((entry) => entry && entry.id === region)
    : [];
  if (regional.length > 1) fail(`Linode type ${type.id} has duplicate regional prices for ${region}`);
  const price = regional.length === 1 ? regional[0] : type.price;
  requireObject(price, `Linode type ${type.id} effective price`);
  return {
    hourly: requireNumber(price.hourly, `Linode type ${type.id} hourly price`, 0),
    monthly:
      typeof price.monthly === "number" && Number.isFinite(price.monthly)
        ? price.monthly
        : null,
    regional: regional.length === 1,
  };
}

function validateProviderCatalog(config, responses) {
  const wanted = config.provider.size;
  const type = requireApiObject(responses.type, "Linode type");
  const image = requireApiObject(responses.image, "Linode image");
  const region = requireApiObject(responses.region, "Linode region");
  const availability = responses.availability && responses.availability.data;
  if (!Array.isArray(availability)) fail("Linode region availability must be an array");

  const typeChecks = [
    ["id", type.id, wanted.id],
    ["description", type.label, wanted.description],
    ["class", type.class, wanted.class],
    ["vCPU count", type.vcpus, wanted.vcpus],
    ["memory MiB", type.memory, wanted.memory_mib],
    ["disk MiB", type.disk, wanted.disk_gib * 1024],
  ];
  for (const [field, actual, expected] of typeChecks) {
    if (String(actual) !== String(expected)) {
      fail(`Linode public type ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  const price = effectiveTypePrice(type, config.provider.region);
  if (price.hourly > wanted.max_hourly_price_usd) {
    fail(
      `Linode type ${wanted.id} costs $${price.hourly}/h in ${config.provider.region}, ` +
        `above the configured $${wanted.max_hourly_price_usd}/h maximum`,
    );
  }

  if (
    image.id !== config.provider.image ||
    image.is_public !== true ||
    image.status !== "available"
  ) {
    fail(
      `Linode image ${config.provider.image} must resolve exactly as a public, available image`,
    );
  }
  if (region.id !== config.provider.region || region.status !== "ok") {
    fail(`Linode region ${config.provider.region} is not an exact, healthy public region match`);
  }
  const availabilityMatches = availability.filter(
    (entry) =>
      entry &&
      entry.region === config.provider.region &&
      entry.plan === wanted.id,
  );
  if (availabilityMatches.length !== 1 || availabilityMatches[0].available !== true) {
    fail(
      `Linode type ${wanted.id} must have one available catalog row in ${config.provider.region}; ` +
        `found ${availabilityMatches.length}`,
    );
  }
  return { type, image, region, availability: availabilityMatches[0], price };
}

async function resolveProviderPlan(token, config, request = apiRequest) {
  const typeId = encodeURIComponent(config.provider.size.id);
  const imageId = encodeURIComponent(config.provider.image);
  const regionId = encodeURIComponent(config.provider.region);
  const [type, image, region, availability] = await Promise.all([
    request(null, "GET", `/v4/linode/types/${typeId}`, { unauthenticated: true }),
    request(null, "GET", `/v4/images/${imageId}`, { unauthenticated: true }),
    request(null, "GET", `/v4/regions/${regionId}`, { unauthenticated: true }),
    request(null, "GET", `/v4/regions/${regionId}/availability`, { unauthenticated: true }),
  ]);
  return validateProviderCatalog(config, { type, image, region, availability });
}

async function verifyDeleteAuthority(token, request = apiRequest, { announce = true } = {}) {
  const sentinel = 2_147_483_647;
  if ((await listLinodes(token, request)).some((entry) => entry && entry.id === sentinel)) {
    fail(`refusing absent-ID probe because Linode ${sentinel} exists on the account`);
  }
  let response;
  try {
    response = await request(token, "DELETE", `/v4/linode/instances/${sentinel}`, {
      acceptedStatuses: [404],
    });
  } catch (error) {
    fail(
      "Linode delete authority could not be proven before billing; the token needs " +
        `linodes:read_write (${error.message})`,
    );
  }
  if (response.status !== 404) fail("absent-ID Linode delete probe did not return 404");
  if (announce) process.stdout.write("Pre-billing absent-ID probe confirmed Linode delete authority.\n");
}

async function gitBlob(commit, file) {
  return (await run("git", ["show", `${commit}:${file}`])).stdout;
}

async function verifySource(config, configFile = DEFAULT_CONFIG) {
  const relativeConfig = path.relative(REPO, configFile);
  if (relativeConfig.startsWith("..") || path.isAbsolute(relativeConfig)) {
    fail("Linode benchmark config must be inside the repository");
  }
  const trackedStatus = outputText(
    await run("git", ["status", "--porcelain=v1", "--untracked-files=no"]),
  );
  if (trackedStatus.length > 0) fail("tracked worktree changes must be committed before a cloud benchmark");
  for (const field of ["baseline_commit", "candidate_commit", "harness_commit"]) {
    await run("git", ["cat-file", "-e", `${config.source[field]}^{commit}`]);
  }
  for (const file of [
    "tools/run-linode-benchmark.js",
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
  const price = provider.price;
  return {
    schema: config.schema,
    provider: "linode",
    protocol_mode: "acceptance",
    region: {
      id: provider.region.id,
      label: provider.region.label,
      country: provider.region.country,
      status: provider.region.status,
    },
    image: {
      id: provider.image.id,
      label: provider.image.label,
      vendor: provider.image.vendor,
      created: provider.image.created,
      updated: provider.image.updated,
      status: provider.image.status,
      is_public: provider.image.is_public,
    },
    size: {
      id: provider.type.id,
      description: provider.type.label,
      class: provider.type.class,
      vcpus: provider.type.vcpus,
      memory_mib: provider.type.memory,
      disk_gib: provider.type.disk / 1024,
      price_hourly_usd: price.hourly,
      price_monthly_usd: price.monthly,
      regional_price: price.regional,
    },
    billable_lifetime_limit_minutes: config.provider.max_runtime_minutes,
    billing_increment_minutes: 60,
    planned_compute_cost_ceiling_usd: roundedHourlyCharge(
      price.hourly,
      config.provider.max_runtime_minutes * 60,
    ),
    cleanup_reserve_minutes: CLEANUP_RESERVE_MS / 60_000,
    billing_caveat:
      "Linode usage is rounded up to a whole hour; deletion, not shutdown, stops further accrual, and an API outage can delay confirmed deletion",
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
  await preflightAccount(token);
  await preflightAccountRegion(token, config);
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
    schema: "rustia.linode-benchmark-lock.v1",
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
      fail(`Linode coordinator lock exists without readable ownership at ${COORDINATOR_LOCK}`);
    }
    if (!lockOwnerIsLive(owner)) {
      fail(`stale Linode coordinator lock for PID ${owner.pid}; run the unlock command`);
    }
    fail(`Linode benchmark coordinator PID ${owner.pid} is already active`);
  }
  return token;
}

function releaseCoordinatorLock(token) {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot verify Linode coordinator lock ownership: ${error.message}`);
  }
  if (owner.pid !== process.pid || owner.token !== token) {
    fail("refusing to remove a Linode coordinator lock owned by another process");
  }
  fs.rmSync(COORDINATOR_LOCK, { recursive: true });
}

function unlockStaleCoordinator() {
  let owner;
  try {
    owner = JSON.parse(fs.readFileSync(path.join(COORDINATOR_LOCK, "owner.json"), "utf8"));
  } catch (error) {
    fail(`cannot read Linode coordinator lock ownership: ${error.message}`);
  }
  if (lockOwnerIsLive(owner)) fail(`refusing to unlock live Linode coordinator PID ${owner.pid}`);
  const quarantine = `${COORDINATOR_LOCK}.stale-${crypto.randomBytes(8).toString("hex")}`;
  fs.renameSync(COORDINATOR_LOCK, quarantine);
  const moved = JSON.parse(fs.readFileSync(path.join(quarantine, "owner.json"), "utf8"));
  if (moved.pid !== owner.pid || moved.token !== owner.token) {
    fail(`stale lock ownership changed during quarantine; retained at ${quarantine}`);
  }
  fs.rmSync(quarantine, { recursive: true });
  process.stdout.write(`Removed stale Linode coordinator lock for PID ${owner.pid}.\n`);
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
  return listStates().filter((state) => !state.linode_destroyed_at);
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
        ? "no active owned Linode benchmark run exists"
        : `more than one active run exists; pass one of: ${states.map((state) => state.run_id).join(", ")}`,
    );
  }
  return states[0];
}

function completedStateForRunId(runId) {
  if (!RUN_ID.test(runId)) fail("a valid explicit Linode run id is required for offline promotion");
  const matches = listStates().filter((state) => state.run_id === runId);
  if (matches.length !== 1) fail(`no uniquely owned Linode run has id ${runId}`);
  return matches[0];
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

function matchingLinodesByLabel(linodes, label) {
  if (!Array.isArray(linodes)) fail("Linode list must be an array");
  return linodes.filter((entry) => entry && entry.label === label);
}

async function reconcileLinodeByLabel(state, token, request = apiRequest) {
  const matches = matchingLinodesByLabel(await listLinodes(token, request), state.linode_label);
  if (matches.length > 1) {
    fail(`refusing to adopt or delete ${matches.length} Linodes labelled ${state.linode_label}`);
  }
  if (matches.length === 1) {
    const id = requireInteger(matches[0].id, "reconciled Linode ID", 1);
    state.linode_id = id;
    saveState(state);
    return matches[0];
  }
  return null;
}

async function reconcileLinodeAfterCreateAttempt(state, token, request = apiRequest) {
  if (state.linode_absence_confirmed_at) return null;
  const attemptedAt = Date.parse(state.linode_create_attempted_at || "");
  if (!Number.isFinite(attemptedAt)) fail("cannot reconcile a Linode without a create-attempt timestamp");
  const finishedAt = Date.parse(state.linode_create_finished_at || "");
  const requestDeadline = Date.parse(state.linode_create_request_deadline_at || "");
  const uncertaintyEndedAt = Number.isFinite(finishedAt)
    ? finishedAt
    : Math.max(attemptedAt, Number.isFinite(requestDeadline) ? requestDeadline : attemptedAt);
  const deadline = Math.max(Date.now(), uncertaintyEndedAt + CREATE_RECONCILE_TIMEOUT_MS);
  while (Date.now() < deadline) {
    try {
      const linode = await reconcileLinodeByLabel(state, token, request);
      if (linode) return linode;
    } catch (error) {
      if (interruptedSignal && !cleanupInProgress) throw error;
      if (/refusing to adopt or delete/.test(error.message)) throw error;
      // Only a successful list after the uncertainty window proves absence.
    }
    const delay = Math.min(5000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  const linode = await reconcileLinodeByLabel(state, token, request);
  if (linode) return linode;
  state.linode_absence_confirmed_at = new Date().toISOString();
  saveState(state);
  return null;
}

async function getOwnedLinode(state, token, { allowMissing = false, request = apiRequest } = {}) {
  if (!state.linode_id) return null;
  const response = await request(token, "GET", `/v4/linode/instances/${state.linode_id}`, {
    acceptedStatuses: [200, 404],
  });
  if (response.status === 404) {
    if (allowMissing) return null;
    fail(`owned Linode ${state.linode_id} was not found`);
  }
  const linode = requireApiObject(response, `Linode ${state.linode_id}`);
  if (linode.id !== state.linode_id || linode.label !== state.linode_label) {
    fail(`Linode ${state.linode_id} does not match owned label ${state.linode_label}`);
  }
  return linode;
}

function verifyOwnedLinodeIdentity(linode, state) {
  const checks = [
    ["ID", linode.id, state.linode_id],
    ["label", linode.label, state.linode_label],
    ["region", linode.region, state.plan.region.id],
    ["type", linode.type, state.plan.size.id],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`refusing to delete owned Linode: ${field} is ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
}

async function deleteLinode(state, token, request = apiRequest) {
  if (state.linode_destroyed_at) return;
  if (!state.linode_id) {
    const reconciled = state.linode_create_attempted_at
      ? await reconcileLinodeAfterCreateAttempt(state, token, request)
      : await reconcileLinodeByLabel(state, token, request);
    if (!reconciled) {
      state.linode_destroyed_at = new Date().toISOString();
      state.cleanup_note = state.linode_create_attempted_at
        ? "post-attempt grace window and a final successful list confirmed the unique Linode label absent"
        : "no create was attempted and a successful list confirmed the unique Linode label absent";
      saveState(state);
      return;
    }
  }

  let existing = await getOwnedLinode(state, token, { allowMissing: true, request });
  if (!existing) {
    const linodes = await listLinodes(token, request);
    const listed = linodes.find((entry) => entry && entry.id === state.linode_id);
    const matchingLabels = matchingLinodesByLabel(linodes, state.linode_label);
    if (!listed && matchingLabels.length === 0) {
      state.linode_destroyed_at = new Date().toISOString();
      saveState(state);
      return;
    }
    if (!listed) {
      fail(
        `refusing cleanup: owned Linode ID ${state.linode_id} is absent but label ` +
          `${state.linode_label} exists under ${matchingLabels.length} other ID(s)`,
      );
    }
    existing = listed;
  }
  verifyOwnedLinodeIdentity(existing, state);

  try {
    await request(token, "DELETE", `/v4/linode/instances/${existing.id}`, {
      acceptedStatuses: [200, 204, 404],
    });
  } catch {
    // A read below, never the DELETE response, decides whether deletion completed.
  }

  const deadline = Date.now() + DELETE_CONFIRM_TIMEOUT_MS;
  while (Date.now() < deadline) {
    try {
      const linodes = await listLinodes(token, request);
      const byId = linodes.find((entry) => entry && entry.id === existing.id);
      const byLabel = matchingLinodesByLabel(linodes, state.linode_label);
      if (!byId && byLabel.length === 0) {
        state.linode_destroyed_at = new Date().toISOString();
        saveState(state);
        return;
      }
      if (byLabel.length > 1) {
        fail(`refusing cleanup: duplicate Linode label ${state.linode_label} appeared`);
      }
      if (!byId || byId.label !== state.linode_label) {
        fail(`refusing cleanup: owned Linode ID/label identity changed`);
      }
      verifyOwnedLinodeIdentity(byId, state);
      await request(token, "DELETE", `/v4/linode/instances/${existing.id}`, {
        acceptedStatuses: [200, 204, 404],
      });
    } catch (error) {
      if (/refusing cleanup|refusing to delete/.test(error.message)) throw error;
      // API failure is not deletion proof; retry within the bounded confirmation window.
    }
    const delay = Math.min(4000, Math.max(0, deadline - Date.now()));
    if (delay > 0) await new Promise((resolve) => setTimeout(resolve, delay));
  }
  fail(
    `could not confirm deletion of owned Linode ${existing.id}; run ` +
      `\`node tools/run-linode-benchmark.js destroy ${state.run_id}\` and do not assume billing stopped`,
  );
}

async function cleanupOwned(state, token) {
  await deleteLinode(state, token);
  if (state.linode_destroyed_at) {
    for (const file of [state.private_key_path, state.public_key_path]) {
      if (file && fs.existsSync(file)) fs.rmSync(file);
    }
  }
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

function isPublicIpv4(address) {
  if (!net.isIPv4(address)) return false;
  const octets = address.split(".").map(Number);
  if (octets[0] === 10 || octets[0] === 127) return false;
  if (octets[0] === 169 && octets[1] === 254) return false;
  if (octets[0] === 172 && octets[1] >= 16 && octets[1] <= 31) return false;
  if (octets[0] === 192 && octets[1] === 168) return false;
  return true;
}

function publicIpv4(linode) {
  if (!linode || !Array.isArray(linode.ipv4)) return null;
  const address = linode.ipv4.find((entry) => typeof entry === "string" && isPublicIpv4(entry));
  return address || null;
}

async function waitForLinode(state, token) {
  const deadline = Date.now() + remainingWorkTimeout(state, 5 * 60 * 1000, "Linode activation");
  while (Date.now() < deadline) {
    const linode = await getOwnedLinode(state, token, { allowMissing: true });
    const address = publicIpv4(linode);
    if (linode && linode.status === "running" && address) {
      state.public_ipv4 = address;
      state.linode_created_at = linode.created || new Date().toISOString();
      saveState(state);
      return linode;
    }
    await new Promise((resolve) => setTimeout(resolve, 5000));
  }
  fail(`Linode ${state.linode_id} did not become running with a public IPv4 address`);
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
  fail(`SSH did not become ready on owned Linode ${state.linode_id}`);
}

function verifyLinodeMatchesPlan(linode, plan) {
  const specs = requireObject(linode.specs, "activated Linode specs");
  const checks = [
    ["type", linode.type, plan.size.id],
    ["region", linode.region, plan.region.id],
    ["image", linode.image, plan.image.id],
    ["vCPU count", specs.vcpus, plan.size.vcpus],
    ["memory MiB", specs.memory, plan.size.memory_mib],
    ["disk MiB", specs.disk, plan.size.disk_gib * 1024],
  ];
  for (const [field, actual, expected] of checks) {
    if (String(actual) !== String(expected)) {
      fail(`activated Linode ${field} was ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
    }
  }
  if (!linode.backups || linode.backups.enabled !== false) {
    fail("activated Linode unexpectedly has paid backups enabled");
  }
  if (linode.interface_generation !== "legacy_config") {
    fail(
      `activated Linode interface generation was ${JSON.stringify(linode.interface_generation)}, ` +
        'expected "legacy_config"',
    );
  }
  if (linode.maintenance_policy !== "linode/power_off_on") {
    fail(
      `activated Linode maintenance policy was ${JSON.stringify(linode.maintenance_policy)}, ` +
        'expected "linode/power_off_on"',
    );
  }
}

function safeLinodeMetadata(linode, plan, state) {
  return {
    provider: "linode",
    linode_id: linode.id,
    linode_label: linode.label,
    created_at: linode.created,
    status: linode.status,
    region: linode.region,
    size_slug: linode.type,
    image: linode.image,
    vcpus: linode.specs.vcpus,
    memory_mib: linode.specs.memory,
    disk_gib: linode.specs.disk / 1024,
    public_ipv4: state.public_ipv4,
    backups_enabled: linode.backups.enabled,
    interface_generation: linode.interface_generation,
    maintenance_policy: linode.maintenance_policy,
    catalog_plan: plan,
  };
}

function createLinodeBody(config, label, publicKey) {
  if (typeof publicKey !== "string" || !/^ssh-ed25519 [A-Za-z0-9+/]+={0,3}(?:\s+\S.*)?$/.test(publicKey)) {
    fail("generated public key is not a single valid ssh-ed25519 line");
  }
  return {
    region: config.provider.region,
    type: config.provider.size.id,
    label,
    image: config.provider.image,
    authorized_keys: [publicKey],
    backups_enabled: false,
    booted: true,
    interface_generation: "legacy_config",
    firewall_id: -1,
    private_ip: false,
    maintenance_policy: "linode/power_off_on",
  };
}

function verifyProviderPlanUnchanged(original, refreshed) {
  const fields = [
    ["region", original.region.id, refreshed.region.id],
    ["image", original.image.id, refreshed.image.id],
    ["type", original.size.id, refreshed.size.id],
    ["class", original.size.class, refreshed.size.class],
    ["vCPUs", original.size.vcpus, refreshed.size.vcpus],
    ["memory", original.size.memory_mib, refreshed.size.memory_mib],
    ["disk", original.size.disk_gib, refreshed.size.disk_gib],
    ["hourly price", original.size.price_hourly_usd, refreshed.size.price_hourly_usd],
  ];
  for (const [field, before, after] of fields) {
    if (String(before) !== String(after)) {
      fail(`Linode provider ${field} changed between plan and pre-billing check`);
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
  if (canonical === "." && type !== "directory") fail("archive root entry must be a directory");
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
    if (!type) fail(`${location} entry ${JSON.stringify(rawName)} has a forbidden tar type`);
    const canonical = safeArchivePath(rawName, type);
    if (seen.has(canonical)) fail(`${location} contains duplicate path ${JSON.stringify(canonical)}`);
    const mode = tarOctalField(header, 100, 8, `${location} entry mode`);
    if ((mode & 0o7000) !== 0) fail(`${location} entry ${JSON.stringify(rawName)} has special mode bits`);
    const size = tarOctalField(header, 124, 12, `${location} entry size`);
    if (type === "directory" && size !== 0) {
      fail(`${location} directory ${JSON.stringify(rawName)} has a non-zero payload`);
    }
    totalPayloadBytes += size;
    if (totalPayloadBytes > MAX_EXPORT_UNCOMPRESSED_BYTES) fail(`${location} payload is too large`);
    const next = offset + 512 + Math.ceil(size / 512) * 512;
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
        fail(`${location} places ${JSON.stringify(entry.path)} below a non-directory`);
      }
    }
  }
  return entries;
}

function inspectArchive(archive) {
  const stat = fs.lstatSync(archive);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
    fail("export archive must be a single regular file");
  }
  if (stat.size <= 0 || stat.size > MAX_EXPORT_ARCHIVE_BYTES) fail("export archive compressed size is invalid");
  let bytes;
  try {
    bytes = zlib.gunzipSync(fs.readFileSync(archive), { maxOutputLength: MAX_EXPORT_UNCOMPRESSED_BYTES });
  } catch (error) {
    fail(`cannot safely decompress export archive: ${error.message}`);
  }
  return inspectTarBytes(bytes);
}

function validateExtractedTree(destination, archiveEntries) {
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
      } else if (stat.isFile() && stat.nlink === 1) {
        actual.set(relative, "file");
      } else {
        fail(`extracted export contains unsafe entry ${JSON.stringify(relative)}`);
      }
    }
  };
  visit(destination);
  const expected = new Map(
    archiveEntries.filter((entry) => entry.path !== ".").map((entry) => [entry.path, entry.type]),
  );
  if (canonicalJson(Object.fromEntries(actual)) !== canonicalJson(Object.fromEntries(expected))) {
    fail("extracted export tree does not exactly match the validated archive listing");
  }
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
    fail("local orchestration checkout changed while the Linode run was in flight; retaining export only");
  }
}

async function verifyRecoveryCheckout(config, state) {
  const planned = requireObject(state.plan && state.plan.source, "state.plan.source");
  if (!FULL_COMMIT.test(planned.orchestration_commit)) {
    fail("recorded Linode orchestration commit is invalid");
  }
  const configFile = path.join(REPO, planned.config_path);
  if (
    path.relative(REPO, configFile).startsWith("..") ||
    path.isAbsolute(path.relative(REPO, configFile)) ||
    path.resolve(state.source_config_path || "") !== configFile
  ) {
    fail("recorded Linode config path is outside or inconsistent with the repository");
  }
  const originalFiles = [
    [planned.config_path, planned.config_sha256, "config"],
    ["tools/run-linode-benchmark.js", planned.coordinator_sha256, "coordinator"],
    ["tools/run-linux-fixed-benchmark.js", planned.remote_runner_sha256, "remote runner"],
  ];
  for (const [file, wanted, label] of originalFiles) {
    if (!SHA256.test(wanted) || sha256(await gitBlob(planned.orchestration_commit, file)) !== wanted) {
      fail(`recorded ${label} does not match the original Linode orchestration commit`);
    }
  }
  const ancestry = await run(
    "git",
    ["merge-base", "--is-ancestor", planned.orchestration_commit, "HEAD"],
    { allowFailure: true },
  );
  if (ancestry.status !== 0) {
    fail("offline promotion requires a checkout descended from the recorded orchestration commit");
  }
  if (sha256File(state.config_path) !== planned.config_sha256) {
    fail("saved Linode protocol snapshot no longer matches the recorded plan");
  }
  if (sha256File(state.runner_path) !== planned.remote_runner_sha256) {
    fail("saved Linode runner snapshot no longer matches the recorded plan");
  }
  const current = await verifySource(config, configFile);
  for (const field of ["config_path", "config_sha256", "remote_runner_sha256"]) {
    if (current[field] !== planned[field]) {
      fail(`current ${field} is incompatible with the recorded Linode run`);
    }
  }
  if (JSON.stringify(current.verified_source_hashes) !== JSON.stringify(planned.verified_source_hashes)) {
    fail("current verified sources are incompatible with the recorded Linode run");
  }
}

function evidenceFiles(directory, relative = "") {
  const files = [];
  for (const entry of fs.readdirSync(path.join(directory, relative), { withFileTypes: true })) {
    const child = path.posix.join(relative.split(path.sep).join(path.posix.sep), entry.name);
    const full = path.join(directory, ...child.split("/"));
    const stat = fs.lstatSync(full);
    if (stat.isSymbolicLink()) fail(`accepted Linode evidence contains symlink ${child}`);
    if (stat.isDirectory()) files.push(...evidenceFiles(directory, path.join(relative, entry.name)));
    else if (stat.isFile() && stat.nlink === 1) files.push(child);
    else fail(`accepted Linode evidence contains non-regular entry ${child}`);
  }
  return files.sort();
}

function requireJsonEqual(actual, expected, location) {
  if (canonicalJson(actual) !== canonicalJson(expected)) fail(`${location} does not match the frozen value`);
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
}

function strictUtf8(bytes, location) {
  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) fail(`${location} is not valid UTF-8`);
  return text;
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

function validateCpuDelta(deltaValue, location) {
  const delta = requireObject(deltaValue, location);
  requireExactKeys(delta, ["total_ticks", "idle_ticks", "idle_percent", "steal_ticks"], location);
  requireInteger(delta.total_ticks, `${location}.total_ticks`, 1);
  requireInteger(delta.idle_ticks, `${location}.idle_ticks`, 0);
  requireInteger(delta.steal_ticks, `${location}.steal_ticks`, 0);
  requireNumber(delta.idle_percent, `${location}.idle_percent`, 0);
  if (delta.idle_ticks > delta.total_ticks || delta.idle_percent > 100) fail(`${location} has impossible data`);
  const expectedIdle = Number(((delta.idle_ticks / delta.total_ticks) * 100).toFixed(6));
  if (delta.idle_percent !== expectedIdle) fail(`${location}.idle_percent does not match its tick counts`);
  return delta;
}

function timedStealGatePasses(config, delta) {
  return (
    delta.steal_ticks * 100 <=
    config.host_gate.maximum_timed_steal_percent * delta.total_ticks
  );
}

function validateGateLog(config, gateFile, declaredRecords) {
  const text = strictUtf8(fs.readFileSync(gateFile), "gate.jsonl");
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
    let passedPre = false;
    while (
      recordIndex < records.length &&
      records[recordIndex].label === label &&
      records[recordIndex].phase === "pre_leg_idle_window"
    ) {
      const record = records[recordIndex];
      requireExactKeys(
        record,
        ["phase", "label", "timestamp", "cpu", "load_1m", "cpu_delta", "contaminants", "passed"],
        `gate pre-record ${recordIndex + 1}`,
      );
      requireIsoTimestamp(record.timestamp, `gate pre-record ${recordIndex + 1}.timestamp`);
      if (record.cpu !== config.host_gate.benchmark_cpu) fail("gate pre-record has the wrong CPU");
      requireNumber(record.load_1m, `gate pre-record ${recordIndex + 1}.load_1m`, 0);
      const delta = validateCpuDelta(record.cpu_delta, `gate pre-record ${recordIndex + 1}.cpu_delta`);
      if (!Array.isArray(record.contaminants)) fail("gate pre-record contaminants is not an array");
      const shouldPass =
        record.load_1m <= config.host_gate.maximum_load_1m &&
        delta.idle_percent >= config.host_gate.minimum_idle_percent &&
        delta.steal_ticks <= config.host_gate.maximum_steal_ticks &&
        record.contaminants.length === 0;
      if (record.passed !== shouldPass) fail("gate pre-record has a false passed flag");
      passedPre = record.passed;
      recordIndex += 1;
      if (passedPre) break;
    }
    if (!passedPre) fail(`gate label ${label} lacks a final passing pre-idle record`);
    if (
      recordIndex >= records.length ||
      records[recordIndex].label !== label ||
      records[recordIndex].phase !== "during_leg"
    ) {
      fail(`gate label ${label} lacks exactly one during-leg record`);
    }
    const during = records[recordIndex];
    requireExactKeys(
      during,
      ["phase", "label", "timestamp", "cpu", "cpu_delta", "contaminants", "passed"],
      `gate during-record ${recordIndex + 1}`,
    );
    requireIsoTimestamp(during.timestamp, `gate during-record ${recordIndex + 1}.timestamp`);
    if (during.cpu !== config.host_gate.benchmark_cpu) fail("gate during-record has the wrong CPU");
    const delta = validateCpuDelta(during.cpu_delta, `gate during-record ${recordIndex + 1}.cpu_delta`);
    if (!Array.isArray(during.contaminants)) fail("gate during-record contaminants is not an array");
    const shouldPass = timedStealGatePasses(config, delta) && during.contaminants.length === 0;
    if (during.passed !== shouldPass || !shouldPass) {
      fail(`gate label ${label} did not pass the 0.1%-steal/clean-process timed gate`);
    }
    recordIndex += 1;
  }
  if (recordIndex !== records.length) fail("gate.jsonl contains unexpected labels or extra records");
}

function validateRawSidecar(file, config, location, expectedCommit, expectedIdentity = null) {
  const sidecar = requireObject(loadJsonRegular(file, location), location);
  if (sidecar.schema !== "rustia.bench.raw-samples.v1" || sidecar.schema_version !== 1) {
    fail(`${location} has the wrong raw-sample schema`);
  }
  if (sidecar.source !== "script-bench-harness") fail(`${location} has the wrong source`);
  const metadata = requireObject(sidecar.metadata, `${location}.metadata`);
  if (
    metadata.measurement_count !== 1 ||
    metadata.sample_count !== config.measurement.repetitions ||
    !Array.isArray(sidecar.measurements) ||
    sidecar.measurements.length !== 1
  ) {
    fail(`${location} has the wrong measurement/sample counts`);
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
    if (measurement[field] !== wanted) fail(`${location}.measurement.${field} is not frozen`);
  }
  requireInteger(measurement.timestamp_unix, `${location}.measurement.timestamp_unix`, 1);
  for (const field of ["dialect", "unit"]) requireString(measurement[field], `${location}.measurement.${field}`);
  if (
    expectedIdentity &&
    (measurement.dialect !== expectedIdentity.dialect || measurement.unit !== expectedIdentity.unit)
  ) {
    fail(`${location} dialect/unit do not match the recomputed comparison identity`);
  }
  if (!Array.isArray(measurement.samples_ns_per_unit) || measurement.samples_ns_per_unit.length !== measurement.reps) {
    fail(`${location} has the wrong sample count`);
  }
  const ordered = measurement.samples_ns_per_unit.slice().sort((left, right) => left - right);
  if (ordered.some((sample) => typeof sample !== "number" || !Number.isFinite(sample) || sample <= 0)) {
    fail(`${location} contains invalid raw samples`);
  }
  const minimum = ordered[0];
  const harnessMedian = ordered[Math.floor(ordered.length / 2)];
  if (!numbersClose(measurement.ns_per_unit_min, minimum)) fail(`${location} minimum does not match its samples`);
  if (!numbersClose(measurement.ns_per_unit_median, harnessMedian)) fail(`${location} median does not match its samples`);
  if (!numbersClose(measurement.units_per_sec, 1e9 / minimum)) fail(`${location} throughput does not match its samples`);
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

function requireCompleteSummary(summaryValue, sidecar, config, location, variant = null) {
  const summary = requireObject(summaryValue, location);
  const measurement = sidecar.measurement;
  const expected = {
    file: sidecar.file,
    sha256: sidecar.sha256,
    timestamp_unix: measurement.timestamp_unix,
    commit: measurement.commit,
    target: measurement.target,
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
  if (Object.keys(summary).length !== Object.keys(expected).length) fail(`${location} has unexpected fields`);
  for (const [field, wanted] of Object.entries(expected)) {
    const matches = typeof wanted === "number" ? numbersClose(summary[field], wanted) : summary[field] === wanted;
    if (!matches) fail(`${location}.${field} does not match its raw sidecar`);
  }
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

async function validateAcceptedEvidence(config, state, evidenceDirectory, localHistory, remoteHistory) {
  const manifestFile = path.join(evidenceDirectory, "manifest.json");
  const analysisFile = path.join(evidenceDirectory, "analysis.json");
  const protocolFile = path.join(evidenceDirectory, "protocol.json");
  const providerFile = path.join(evidenceDirectory, "provider.json");
  for (const [file, location] of [
    [manifestFile, "evidence manifest"],
    [analysisFile, "fixed-sample analysis"],
    [protocolFile, "evidence protocol"],
    [providerFile, "evidence provider metadata"],
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
    fail("accepted Linode evidence has the wrong manifest schema");
  }
  requireIsoTimestamp(manifest.created_at, "evidence manifest.created_at");
  const actualFiles = evidenceFiles(evidenceDirectory).filter((file) => file !== "manifest.json");
  const hashes = requireObject(manifest.evidence_sha256, "evidence manifest.evidence_sha256");
  const listed = Object.keys(hashes).sort();
  if (canonicalJson(listed) !== canonicalJson(actualFiles)) {
    fail("accepted Linode evidence hash inventory does not exactly match its files");
  }
  for (const file of listed) {
    if (
      file.split("/").includes("..") ||
      !SHA256.test(hashes[file]) ||
      sha256File(path.join(evidenceDirectory, ...file.split("/"))) !== hashes[file]
    ) {
      fail(`accepted Linode evidence hash is wrong or unsafe for ${file}`);
    }
  }

  const protocol = loadJsonRegular(protocolFile, "evidence protocol");
  validateConfig(protocol);
  requireJsonEqual(protocol, config, "evidence protocol");
  if (sha256File(protocolFile) !== state.plan.source.config_sha256) {
    fail("evidence protocol bytes do not match the planned frozen config");
  }
  const provider = requireObject(loadJsonRegular(providerFile, "evidence provider metadata"), "evidence provider metadata");
  const localProvider = loadJsonRegular(path.join(state.__directory, "provider.json"), "local provider snapshot");
  requireJsonEqual(provider, localProvider, "evidence provider metadata");
  requireJsonEqual(provider.catalog_plan, state.plan, "evidence provider catalog plan");
  const providerIdentity = [
    [provider.provider, "linode", "provider"],
    [provider.linode_id, state.linode_id, "Linode ID"],
    [provider.linode_label, state.linode_label, "Linode label"],
    [provider.region, config.provider.region, "region"],
    [provider.size_slug, config.provider.size.id, "plan"],
    [provider.image, config.provider.image, "image"],
    [provider.vcpus, config.provider.size.vcpus, "vCPUs"],
    [provider.memory_mib, config.provider.size.memory_mib, "memory"],
    [provider.disk_gib, config.provider.size.disk_gib, "disk"],
    [provider.public_ipv4, state.public_ipv4, "public IPv4"],
    [provider.backups_enabled, false, "backup state"],
    [provider.interface_generation, "legacy_config", "interface generation"],
  ];
  for (const [actual, expected, field] of providerIdentity) {
    if (actual !== expected) fail(`evidence provider ${field} does not match the destroyed Linode`);
  }
  requireJsonEqual(manifest.provider, provider, "manifest provider identity");

  const source = requireObject(manifest.source, "evidence manifest.source");
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
  const hostJson = requireObject(loadJsonRegular(hostFile, "host.json"), "host.json");
  if (hostJson.virtualization !== "kvm") fail("Linode host evidence is not a KVM guest");
  const host = requireObject(manifest.host, "manifest.host");
  if (host.file !== "host.json" || host.sha256 !== sha256File(hostFile)) {
    fail("manifest host identity does not match host.json");
  }
  for (const [field, wanted] of Object.entries({
    cpu_model: hostJson.cpu_model,
    microcode: hostJson.microcode,
    clocksource: hostJson.clocksource,
    scaling_governors: hostJson.scaling_governors,
    benchmark_cpu: config.host_gate.benchmark_cpu,
  })) {
    requireJsonEqual(host[field], wanted, `manifest.host.${field}`);
  }

  const protocolManifest = requireObject(manifest.protocol, "manifest.protocol");
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
    protocolManifest.exclusion_rule !==
    "any pre/during-leg host-gate violation discards the whole eight-leg block; no leg is selected by timing outcome"
  ) {
    fail("manifest protocol exclusion rule is not frozen");
  }
  const gateLog = requireObject(protocolManifest.gate_log, "manifest.protocol.gate_log");
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
  for (const field of ["dialect", "unit"]) requireString(analysisIdentity[field], `analysis.identity.${field}`);
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
    fail("fixed-sample analysis source or method identity is not frozen");
  }

  const qualification = requireObject(manifest.qualification, "evidence manifest.qualification");
  if (
    qualification.schema !== "rustia.bench.cloud-qualification.v1" ||
    qualification.passed !== true ||
    qualification.maximum_median_spread_percent !== config.qualification.max_median_spread_percent ||
    qualification.note !== "qualification-only runs; never accepted as A/B evidence or public history" ||
    !Array.isArray(qualification.pilot_legs) ||
    qualification.pilot_legs.length !== config.qualification.candidate_pilot_legs
  ) {
    fail("accepted Linode evidence did not pass the frozen qualification gate");
  }
  const qualificationDirectory = path.join(evidenceDirectory, "qualification-sidecars");
  const pilotMedians = [];
  qualification.pilot_legs.forEach((summary, index) => {
    const sidecar = validateRawSidecar(
      path.join(qualificationDirectory, `candidate-pilot-${index + 1}.json`),
      config,
      `qualification sidecar ${index + 1}`,
      config.source.candidate_commit,
      analysisIdentity,
    );
    requireCompleteSummary(summary, sidecar, config, `manifest qualification.pilot_legs[${index}]`);
    pilotMedians.push(sidecar.measurement.ns_per_unit_median);
  });
  const spread = ((Math.max(...pilotMedians) - Math.min(...pilotMedians)) / Math.min(...pilotMedians)) * 100;
  if (!numbersClose(qualification.median_spread_percent, spread) || spread > config.qualification.max_median_spread_percent) {
    fail("qualification spread does not match its raw pilot sidecars or threshold");
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

  const comparison = requireObject(manifest.comparison, "evidence manifest.comparison");
  if (
    comparison.analysis_sha256 !== sha256File(analysisFile) ||
    comparison.verdict.status !== config.publication.required_verdict
  ) {
    fail("manifest comparison analysis identity or verdict is wrong");
  }
  requireJsonEqual(comparison.verdict, analysis.comparison.verdict, "manifest comparison verdict");
  for (const [field, wanted] of Object.entries({
    candidate_delta_percent: analysis.comparison.candidate_delta_percent,
    pooled_bootstrap_ci_95_percent: analysis.comparison.pooled_bootstrap_ci_95_percent,
    hierarchical_bootstrap_ci_95_percent: analysis.comparison.hierarchical_bootstrap_ci_95_percent,
    conservative_bootstrap_ci_95_percent: analysis.comparison.bootstrap_ci_95_percent,
  })) {
    requireJsonEqual(comparison[field], wanted, `manifest.comparison.${field}`);
  }
  if (!Array.isArray(comparison.legs) || comparison.legs.length !== config.measurement.run_order.length) {
    fail("manifest comparison has the wrong number of legs");
  }
  config.measurement.run_order.forEach((leg, index) => {
    const expectedCommit = leg.variant === "baseline" ? config.source.baseline_commit : config.source.candidate_commit;
    const sidecar = validateRawSidecar(
      path.join(evidenceDirectory, leg.file),
      config,
      `comparison sidecar ${leg.file}`,
      expectedCommit,
      analysisIdentity,
    );
    requireCompleteSummary(comparison.legs[index], sidecar, config, `manifest comparison.legs[${index}]`, leg.variant);
  });

  const suffix = strictUtf8(remoteHistory.subarray(localHistory.length), "remote history suffix");
  if (!suffix.endsWith("\n") || suffix.includes("\r")) fail("remote history suffix is not canonical JSONL");
  const historyRows = suffix.slice(0, -1).split("\n").map((line, index) => {
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
  if (historyRows.length !== config.publication.final_measurements) fail("remote history has the wrong suffix length");
  const finalSidecars = historyRows.map((row, index) => {
    const finalFile = finalSidecarName(config, index + 1);
    const sidecar = validateRawSidecar(
      path.join(evidenceDirectory, finalFile),
      config,
      `final measurement sidecar ${index + 1}`,
      config.source.candidate_commit,
      analysisIdentity,
    );
    const measurement = sidecar.measurement;
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
      fail(`remote history row ${index + 1} does not match ${finalFile}`);
    }
    return sidecar;
  });

  const publication = requireObject(manifest.publication, "evidence manifest.publication");
  const beforeLines = strictUtf8(localHistory, "local history").trimEnd().split("\n").length;
  const expectedPublication = {
    performed: true,
    history_before_sha256: config.source.expected_sha256.history_before,
    history_after_sha256: sha256(remoteHistory),
    history_lines_before: beforeLines,
    history_lines_after: beforeLines + config.publication.final_measurements,
  };
  for (const [field, wanted] of Object.entries(expectedPublication)) {
    if (publication[field] !== wanted) fail(`accepted Linode evidence publication.${field} is wrong`);
  }
  requireJsonEqual(publication.appended_rows, historyRows, "manifest publication rows");
  if (!Array.isArray(publication.sidecars) || publication.sidecars.length !== finalSidecars.length) {
    fail("manifest publication has the wrong number of sidecars");
  }
  finalSidecars.forEach((sidecar, index) =>
    requireCompleteSummary(publication.sidecars[index], sidecar, config, `manifest publication.sidecars[${index}]`),
  );
}

async function promoteAcceptedResult(
  config,
  state,
  downloadDirectory,
  verifyCheckout = verifyPromotionCheckout,
) {
  if (!state.linode_destroyed_at) {
    fail("refusing publication before the Linode API has confirmed instance deletion");
  }
  throwIfInterrupted();
  const statusFile = path.join(downloadDirectory, "status.json");
  const status = loadJsonRegular(statusFile, "remote status");
  state.remote_status = status.status;
  state.remote_status_detail = status.detail || null;
  saveState(state);
  if (status.status !== "accepted_and_published") {
    process.stdout.write(
      `Linode run was retained as diagnostics at ${path.relative(REPO, downloadDirectory)} ` +
        `(${status.status}${status.detail ? `: ${status.detail}` : ""}).\n`,
    );
    return false;
  }
  validateAcceptedStatus(status, config.publication.required_verdict);
  await verifyCheckout(config, state);

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
    const stat = fs.lstatSync(required);
    if (!stat.isFile() || stat.isSymbolicLink() || stat.nlink !== 1) {
      fail(`remote accepted export is missing a single regular file at ${required}`);
    }
  }
  if (fs.existsSync(evidenceDestination)) {
    fail(`refusing to overwrite existing evidence directory ${evidenceDestination}`);
  }
  const localHistory = fs.readFileSync(historyDestination);
  if (sha256(localHistory) !== config.source.expected_sha256.history_before) {
    fail("local benchmark history changed while the Linode run was in flight");
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
    fail(`remote history appended ${newLines - oldLines} rows, expected ${config.publication.final_measurements}`);
  }
  await validateAcceptedEvidence(config, state, evidenceSource, localHistory, remoteHistory);

  const stageNonce = crypto.randomBytes(6).toString("hex");
  const evidenceStage = `${evidenceDestination}.incoming-${state.run_id}-${stageNonce}`;
  const historyStage = `${historyDestination}.incoming-${state.run_id}-${stageNonce}`;
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
    if (fs.existsSync(evidenceStage) || fs.existsSync(historyStage)) fail("randomized promotion stage already exists");
    fs.cpSync(evidenceSource, evidenceStage, { recursive: true, errorOnExist: true, force: false });
    fs.copyFileSync(historySource, historyStage, fs.constants.COPYFILE_EXCL);
    const sourceFiles = evidenceFiles(evidenceSource);
    const stagedFiles = evidenceFiles(evidenceStage);
    requireJsonEqual(stagedFiles, sourceFiles, "staged evidence inventory");
    for (const file of sourceFiles) {
      if (
        sha256File(path.join(evidenceStage, ...file.split("/"))) !==
        sha256File(path.join(evidenceSource, ...file.split("/")))
      ) {
        fail(`staged evidence bytes differ for ${file}`);
      }
    }
    if (sha256File(historyStage) !== sha256(remoteHistory)) fail("staged history bytes differ from the export");
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
  const linodeLabel = `rustia-bench-${shortCandidate}-${crypto.randomBytes(4).toString("hex")}`;
  const state = {
    __directory: directory,
    schema: "rustia.linode-benchmark-state.v1",
    run_id: runId,
    created_at: new Date().toISOString(),
    protocol_mode: "acceptance",
    linode_label: linodeLabel,
    linode_id: null,
    linode_destroyed_at: null,
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
      ["-q", "-t", "ed25519", "-N", "", "-C", `${linodeLabel}-key`, "-f", state.private_key_path],
      { capture: false, announce: true },
    );
    fs.chmodSync(state.private_key_path, 0o600);
    await run("git", ["bundle", "create", state.bundle_path, "HEAD"], { announce: true });
    await run("git", ["bundle", "verify", state.bundle_path], { capture: false, announce: true });
    state.bundle_sha256 = sha256File(state.bundle_path);
    saveState(state);

    // Repeat every cost/scope-sensitive check immediately before the first billable request.
    await preflightAccount(token);
    await preflightAccountRegion(token, config);
    const refreshedProvider = await resolveProviderPlan(token, config);
    const refreshedPlan = sanitizedProviderPlan(config, refreshedProvider, source);
    verifyProviderPlanUnchanged(plan, refreshedPlan);
    await verifyDeleteAuthority(token);

    const publicKey = fs.readFileSync(state.public_key_path, "utf8").trim();
    const createBody = createLinodeBody(config, linodeLabel, publicKey);
    startBillingWindow(state, config);
    const createStarted = Date.now();
    state.linode_create_attempted_at = new Date(createStarted).toISOString();
    state.linode_create_request_deadline_at = new Date(createStarted + API_TIMEOUT_MS).toISOString();
    saveState(state);

    let createResponse = null;
    let createError = null;
    try {
      createResponse = await apiRequest(token, "POST", "/v4/linode/instances", { body: createBody });
    } catch (error) {
      createError = error;
    }
    state.linode_create_finished_at = new Date().toISOString();
    saveState(state);
    if (createResponse) {
      const created = requireApiObject(createResponse, "Linode create response");
      if (Number.isSafeInteger(created.id) && created.id > 0) {
        state.linode_id = created.id;
        saveState(state);
      }
      if (!state.linode_id || created.label !== state.linode_label) {
        const reconciled = await reconcileLinodeAfterCreateAttempt(state, token);
        if (!reconciled) fail("Linode create returned no uniquely reconcilable owned instance");
      }
    } else {
      const reconciled = await reconcileLinodeAfterCreateAttempt(state, token);
      if (!reconciled) fail(`Linode creation failed: ${createError ? createError.message : "unknown API error"}`);
    }

    const linode = await waitForLinode(state, token);
    verifyLinodeMatchesPlan(linode, plan);
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
    writeJsonAtomic(providerMetadata, safeLinodeMetadata(linode, plan, state));
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
  if (!state.linode_destroyed_at) fail("Linode deletion was not confirmed");
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
        "incomplete Linode benchmark cleanup exists; run status and clean up: " +
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
    await preflightAccount(token);
    await preflightAccountRegion(token, config);
    await resolveProviderPlan(token, config);
    await verifyDeleteAuthority(token);
    writeBytesAtomic(TOKEN_FILE, Buffer.from(`${token}\n`), 0o600);
    process.stdout.write(`Linode token validated and stored with mode 0600 at ${TOKEN_FILE}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function status() {
  const states = listStates();
  if (states.length === 0) {
    process.stdout.write("No locally owned Linode benchmark runs.\n");
    return;
  }
  const token = readStoredToken({ allowMissing: true });
  const rows = [];
  for (const state of states) {
    let remote = null;
    if (!state.linode_destroyed_at) {
      if (!token) {
        remote = { error: "stored token unavailable; run auth or supply cleanup from an authenticated host" };
      } else {
        try {
          if (state.linode_id) {
            const linode = await getOwnedLinode(state, token, { allowMissing: true });
            remote = linode
              ? { id: linode.id, label: linode.label, status: linode.status, public_ipv4: publicIpv4(linode) }
              : { status: "not returned by get; run destroy to reconcile safely" };
          } else {
            const matches = matchingLinodesByLabel(await listLinodes(token), state.linode_label);
            remote =
              matches.length === 1
                ? {
                    id: matches[0].id,
                    label: matches[0].label,
                    status: matches[0].status,
                    public_ipv4: publicIpv4(matches[0]),
                    note: "found by the journaled unique label; run destroy to adopt and clean it safely",
                  }
                : matches.length === 0
                  ? { status: "unique label absent from a complete list; run destroy to confirm cleanup" }
                  : {
                      error: `duplicate owned label returned ${matches.length} Linodes; manual review required`,
                    };
          }
        } catch (error) {
          remote = { error: error.message };
        }
      }
    }
    const started = Date.parse(state.billing_window_started_at || "");
    const ended = Date.parse(state.linode_destroyed_at || "");
    const seconds = Number.isFinite(started)
      ? Math.max(0, ((Number.isFinite(ended) ? ended : Date.now()) - started) / 1000)
      : 0;
    const hourly = state.plan && state.plan.size ? state.plan.size.price_hourly_usd : null;
    rows.push({
      run_id: state.run_id,
      created_at: state.created_at,
      linode_id: state.linode_id,
      linode_label: state.linode_label,
      linode_create_attempted_at: state.linode_create_attempted_at || null,
      linode_absence_confirmed_at: state.linode_absence_confirmed_at || null,
      destroyed_at: state.linode_destroyed_at,
      cleanup_complete: Boolean(state.linode_destroyed_at),
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
    process.stdout.write(`Confirmed cleanup for owned Linode run ${state.run_id}.\n`);
  } finally {
    releaseCoordinatorLock(lockToken);
  }
}

async function promote(runId) {
  const lockToken = acquireCoordinatorLock();
  let recoveryDirectory = null;
  try {
    const state = completedStateForRunId(runId);
    const expectedDirectory = path.join(RUNS_ROOT, runId);
    if (
      state.schema !== "rustia.linode-benchmark-state.v1" ||
      path.resolve(state.__directory) !== expectedDirectory
    ) {
      fail("Linode run state identity or directory is invalid");
    }
    if (!state.linode_destroyed_at || !Number.isFinite(Date.parse(state.linode_destroyed_at))) {
      fail("offline promotion requires API-confirmed Linode deletion");
    }
    if (state.remote_exit_status !== 0 || state.remote_timed_out !== false) {
      fail("offline promotion requires a successful, non-timeout remote benchmark");
    }
    if (state.remote_status !== "accepted_and_published") {
      fail("offline promotion requires a recorded accepted_and_published result");
    }
    if (!state.promotion_rolled_back_at || typeof state.promotion_rollback_reason !== "string") {
      fail("offline promotion is limited to an accepted result whose first promotion rolled back");
    }
    if (state.promoted_at) fail("Linode result is already promoted");

    const archive = path.join(expectedDirectory, "export.tar.gz");
    if (
      path.resolve(state.export_path || "") !== archive ||
      !SHA256.test(state.export_sha256 || "") ||
      !fs.lstatSync(archive).isFile() ||
      sha256File(archive) !== state.export_sha256
    ) {
      fail("Linode export archive is missing, redirected, or differs from the journaled download");
    }
    const config = loadConfig(state.config_path);
    await verifyRecoveryCheckout(config, state);
    await run("env", ["-u", "RUSTC_WRAPPER", "just", "check-benchmark-results"], {
      capture: false,
      announce: true,
    });

    recoveryDirectory = path.join(
      expectedDirectory,
      `recovery-download-${safeTimestamp()}-${crypto.randomBytes(4).toString("hex")}`,
    );
    state.recovery_promotion_started_at = new Date().toISOString();
    state.recovery_promotion_commit = outputText(await run("git", ["rev-parse", "HEAD"]));
    saveState(state);
    await extractArchive(archive, recoveryDirectory);
    if (sha256File(archive) !== state.export_sha256) {
      fail("Linode export archive changed during offline promotion");
    }
    const promoted = await promoteAcceptedResult(
      config,
      state,
      recoveryDirectory,
      verifyRecoveryCheckout,
    );
    if (!promoted) fail("journaled Linode export is no longer accepted_and_published");
    state.recovery_promoted_at = new Date().toISOString();
    saveState(state);
    process.stdout.write(`Offline promotion completed for Linode run ${runId}.\n`);
  } catch (error) {
    if (runId && RUN_ID.test(runId)) {
      const matches = listStates().filter((state) => state.run_id === runId);
      if (matches.length === 1 && matches[0].recovery_promotion_started_at && !matches[0].promoted_at) {
        matches[0].recovery_promotion_failed_at = new Date().toISOString();
        matches[0].recovery_promotion_failure = error.message;
        saveState(matches[0]);
      }
    }
    throw error;
  } finally {
    if (recoveryDirectory) fs.rmSync(recoveryDirectory, { recursive: true, force: true });
    releaseCoordinatorLock(lockToken);
  }
}

async function selfTest(config) {
  assert.equal(config.publication.mode, "acceptance");
  assert.equal(config.provider.region, "ap-southeast");
  assert.equal(config.provider.image, "linode/ubuntu24.04");
  assert.equal(config.provider.size.id, "g6-dedicated-2");
  assert.equal(config.provider.size.class, "dedicated");
  assert.deepEqual(
    config.measurement.run_order.map((entry) => entry.variant),
    ["baseline", "candidate", "candidate", "baseline", "candidate", "baseline", "baseline", "candidate"],
  );

  const responses = {
    type: {
      data: {
        id: "g6-dedicated-2",
        label: "Dedicated 4GB",
        class: "dedicated",
        vcpus: 2,
        memory: 4096,
        disk: 81920,
        price: { hourly: 0.054, monthly: 36 },
        region_prices: [],
      },
    },
    image: {
      data: {
        id: "linode/ubuntu24.04",
        label: "Ubuntu 24.04 LTS",
        is_public: true,
        status: "available",
      },
    },
    region: { data: { id: "ap-southeast", label: "Sydney, AU", country: "au", status: "ok" } },
    availability: { data: [{ region: "ap-southeast", plan: "g6-dedicated-2", available: true }] },
  };
  const selected = validateProviderCatalog(config, responses);
  assert.equal(selected.type.id, "g6-dedicated-2");
  assert.equal(selected.price.hourly, 0.054);
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      type: { data: { ...responses.type.data, vcpus: 4 } },
    }),
  );
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      availability: { data: [{ region: "ap-southeast", plan: "g6-dedicated-2", available: false }] },
    }),
  );
  assert.throws(() =>
    validateProviderCatalog(config, {
      ...responses,
      type: { data: { ...responses.type.data, price: { hourly: 0.055, monthly: 37 } } },
    }),
  );

  const readOnlyCalls = [];
  const catalogRequest = async (token, method, apiPath, options) => {
    readOnlyCalls.push([token, method, apiPath, options]);
    assert.equal(token, null);
    assert.equal(options.unauthenticated, true);
    if (apiPath.includes("/linode/types/")) return responses.type;
    if (apiPath.includes("/images/")) return responses.image;
    if (apiPath.endsWith("/availability")) return responses.availability;
    if (apiPath.includes("/regions/")) return responses.region;
    throw new Error(`unexpected fixture request ${method} ${apiPath}`);
  };
  const resolved = await resolveProviderPlan("fixture-token", config, catalogRequest);
  assert.equal(resolved.region.id, "ap-southeast");
  assert.equal(readOnlyCalls.length, 4);
  assert(readOnlyCalls.every(([, method]) => method === "GET"));

  await preflightAccount("fixture-token", async (_token, method, apiPath) => {
    assert.equal(method, "GET");
    assert.equal(apiPath, "/v4/account/settings");
    return { data: { backups_enabled: false, interfaces_for_new_linodes: "legacy_config_only" } };
  });
  await assert.rejects(() =>
    preflightAccount("fixture-token", async () => ({
      data: { backups_enabled: true, interfaces_for_new_linodes: "legacy_config_only" },
    })),
  );
  await assert.rejects(() =>
    preflightAccount("fixture-token", async () => ({
      data: { backups_enabled: false, interfaces_for_new_linodes: "linode_only" },
    })),
  );
  await preflightAccountRegion("fixture-token", config, async (_token, method, apiPath) => {
    assert.equal(method, "GET");
    assert.equal(apiPath, "/v4/account/availability/ap-southeast");
    return {
      data: {
        available: ["Linodes", "NodeBalancers", "Block Storage", "Kubernetes"],
        unavailable: [],
      },
    };
  });
  await assert.rejects(() =>
    preflightAccountRegion("fixture-token", config, async () => ({
      data: { available: ["NodeBalancers"], unavailable: ["Linodes"] },
    })),
  );
  await assert.rejects(() =>
    preflightAccountRegion("fixture-token", config, async () => ({
      data: { available: null, unavailable: [] },
    })),
  );

  const authorityCalls = [];
  await verifyDeleteAuthority(
    "fixture-token",
    async (_token, method, apiPath, options = {}) => {
      authorityCalls.push([method, apiPath, options]);
      if (method === "GET") return { data: { data: [], page: 1, pages: 1, results: 0 } };
      assert.equal(method, "DELETE");
      assert.deepEqual(options.acceptedStatuses, [404]);
      return { status: 404, data: { errors: [{ reason: "Not found" }] } };
    },
    { announce: false },
  );
  assert.deepEqual(authorityCalls.map(([method]) => method), ["GET", "DELETE"]);

  assert.equal(roundedHourlyCharge(0.054, 0), 0);
  assert.equal(roundedHourlyCharge(0.054, 1), 0.054);
  assert.equal(roundedHourlyCharge(0.054, 3599.999), 0.054);
  assert.equal(roundedHourlyCharge(0.054, 3600), 0.054);
  assert.equal(roundedHourlyCharge(0.054, 3600.001), 0.108);
  assert.equal(roundedHourlyCharge(0.054, config.provider.max_runtime_minutes * 60), 0.054);

  const publicKey = `ssh-ed25519 ${Buffer.alloc(32, 7).toString("base64")} rustia-test`;
  const body = createLinodeBody(config, "rustia-bench-test", publicKey);
  assert.deepEqual(body.authorized_keys, [publicKey]);
  assert.equal(body.backups_enabled, false);
  assert.equal(body.booted, true);
  assert.equal(body.interface_generation, "legacy_config");
  assert.equal(body.firewall_id, -1);
  assert.equal(body.private_ip, false);
  assert.equal(body.maintenance_policy, "linode/power_off_on");
  assert.equal(Object.prototype.hasOwnProperty.call(body, "root_pass"), false);
  assert.equal(Object.prototype.hasOwnProperty.call(body, "token"), false);
  assert.throws(() => createLinodeBody(config, "rustia-bench-test", "not-a-key"));

  assert.equal(isPublicIpv4("203.0.113.9"), true);
  for (const address of ["999.1.2.3", "10.0.0.1", "127.0.0.1", "169.254.1.1", "172.16.0.1", "192.168.1.1"]) {
    assert.equal(isPublicIpv4(address), false);
  }
  assert.equal(publicIpv4({ ipv4: ["10.0.0.1", "203.0.113.9"] }), "203.0.113.9");
  assert.equal(matchingLinodesByLabel([{ label: "a" }, { label: "b" }, { label: "a" }], "a").length, 2);

  const plannedShape = {
    region: { id: "ap-southeast" },
    image: { id: "linode/ubuntu24.04" },
    size: { id: "g6-dedicated-2", vcpus: 2, memory_mib: 4096, disk_gib: 80 },
  };
  const actualShape = {
    type: "g6-dedicated-2",
    region: "ap-southeast",
    image: "linode/ubuntu24.04",
    specs: { vcpus: 2, memory: 4096, disk: 81920 },
    backups: { enabled: false },
    interface_generation: "legacy_config",
    maintenance_policy: "linode/power_off_on",
  };
  verifyLinodeMatchesPlan(actualShape, plannedShape);
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, specs: { ...actualShape.specs, disk: 81921 } }, plannedShape));
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, backups: { enabled: true } }, plannedShape));
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, interface_generation: "linode" }, plannedShape));
  assert.throws(() => verifyLinodeMatchesPlan({ ...actualShape, maintenance_policy: null }, plannedShape));
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
      host_gate: { ...config.host_gate, maximum_timed_steal_percent: 0.2 },
    }),
  );

  assert.equal(finalSidecarName(config, 1), `final-${config.measurement.workload}-1.json`);
  assert.equal(
    finalSidecarName({ measurement: { workload: "oop_metamethods" } }, 3),
    "final-oop_metamethods-3.json",
  );
  const escapedWorkloadFile = finalSidecarName(
    { measurement: { workload: "../../Odd workload/\u2728" } },
    2,
  );
  assert.match(escapedWorkloadFile, /^final-odd-workload-[0-9a-f]{12}-2\.json$/);
  assert.equal(path.basename(escapedWorkloadFile), escapedWorkloadFile);
  assert.throws(() => finalSidecarName(config, 0));

  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-linode-self-test-"));
  try {
    const file = path.join(temporary, "secret");
    writeBytesAtomic(file, Buffer.from("fixture\n"), 0o600);
    assert.equal(fs.statSync(file).mode & 0o777, 0o600);
    assert.equal(fs.readFileSync(file, "utf8"), "fixture\n");

    const fixtureRepo = path.join(temporary, "repo");
    const outside = path.join(temporary, "outside");
    fs.mkdirSync(path.join(fixtureRepo, "bench", "cloud"), { recursive: true });
    fs.mkdirSync(outside);
    const defaultFixture = path.join(fixtureRepo, "bench", "cloud", "default.json");
    const alternateFixture = path.join(fixtureRepo, "bench", "cloud", "alternate.json");
    const outsideFixture = path.join(outside, "outside.json");
    fs.writeFileSync(defaultFixture, "{}\n");
    fs.writeFileSync(alternateFixture, "{}\n");
    fs.writeFileSync(outsideFixture, "{}\n");
    const resolverOptions = { repo: fixtureRepo, defaultConfig: defaultFixture };
    assert.equal(resolveConfigFile(undefined, resolverOptions), defaultFixture);
    assert.equal(resolveConfigFile("bench/cloud/alternate.json", resolverOptions), alternateFixture);
    assert.throws(() => resolveConfigFile("", resolverOptions));
    assert.throws(() => resolveConfigFile("../outside/outside.json", resolverOptions));
    assert.throws(() => resolveConfigFile("bench/../bench/cloud/alternate.json", resolverOptions));
    assert.throws(() => resolveConfigFile(outsideFixture, resolverOptions));
    assert.throws(() => resolveConfigFile("bench", resolverOptions));
    fs.symlinkSync(alternateFixture, path.join(fixtureRepo, "linked-config.json"));
    assert.throws(() => resolveConfigFile("linked-config.json", resolverOptions));
    fs.symlinkSync(outside, path.join(fixtureRepo, "linked-directory"));
    assert.throws(() => resolveConfigFile("linked-directory/outside.json", resolverOptions));
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }

  const runner = await run("node", [REMOTE_RUNNER, "self-test"]);
  assert.match(outputText(runner), /self-test passed/);
  process.stdout.write("linode benchmark coordinator offline self-test passed\n");
}

function requireCommandArguments(minimum, maximum = minimum) {
  const count = process.argv.length - 2;
  if (count < minimum || count > maximum) fail("unexpected command arguments; tokens are never accepted in argv");
}

function usage() {
  process.stdout.write(`Usage:
  node tools/run-linode-benchmark.js auth
  node tools/run-linode-benchmark.js plan [CONFIG.json]
  node tools/run-linode-benchmark.js bench [CONFIG.json]
  node tools/run-linode-benchmark.js status
  node tools/run-linode-benchmark.js destroy [RUN-ID]
  node tools/run-linode-benchmark.js promote RUN-ID
  node tools/run-linode-benchmark.js unlock
  node tools/run-linode-benchmark.js self-test

CONFIG.json must be a regular, non-symlink repository-relative file. The default is
${path.relative(REPO, DEFAULT_CONFIG)}.
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
      requireCommandArguments(1, 2);
      {
        const configFile = resolveConfigFile(process.argv[3]);
        await buildPlan(loadConfig(configFile), { configFile });
      }
      break;
    case "bench":
      requireCommandArguments(1, 2);
      {
        const configFile = resolveConfigFile(process.argv[3]);
        await bench(loadConfig(configFile), configFile);
      }
      break;
    case "status":
      requireCommandArguments(1);
      await status();
      break;
    case "destroy":
      requireCommandArguments(1, 2);
      await destroy(process.argv[3] || "");
      break;
    case "promote":
      requireCommandArguments(2);
      await promote(process.argv[3]);
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
  process.stderr.write(`run-linode-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
