#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const fs = require("node:fs");
const https = require("node:https");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
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

function extractArchive(archive, destination) {
  fs.mkdirSync(destination, { recursive: true });
  return run("tar", ["-xzf", archive, "-C", destination], { capture: false, announce: true });
}

async function verifyPromotionCheckout(config, state) {
  const current = await verifySource(config, state.source_config_path || DEFAULT_CONFIG);
  const planned = state.plan && state.plan.source;
  if (!planned || JSON.stringify(current) !== JSON.stringify(planned)) {
    fail("local orchestration checkout changed while the Linode run was in flight; retaining export only");
  }
}

async function promoteAcceptedResult(config, state, downloadDirectory) {
  if (!state.linode_destroyed_at) {
    fail("refusing publication before the Linode API has confirmed instance deletion");
  }
  throwIfInterrupted();
  const statusFile = path.join(downloadDirectory, "status.json");
  if (!fs.existsSync(statusFile)) fail("remote export did not contain status.json");
  const status = JSON.parse(fs.readFileSync(statusFile, "utf8"));
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

  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-linode-self-test-"));
  try {
    const file = path.join(temporary, "secret");
    writeBytesAtomic(file, Buffer.from("fixture\n"), 0o600);
    assert.equal(fs.statSync(file).mode & 0o777, 0o600);
    assert.equal(fs.readFileSync(file, "utf8"), "fixture\n");
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
  node tools/run-linode-benchmark.js plan
  node tools/run-linode-benchmark.js bench
  node tools/run-linode-benchmark.js status
  node tools/run-linode-benchmark.js destroy [RUN-ID]
  node tools/run-linode-benchmark.js unlock
  node tools/run-linode-benchmark.js self-test
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
  process.stderr.write(`run-linode-benchmark: ${error.message}\n`);
  process.exitCode = 1;
});
