#!/usr/bin/env node
"use strict";

const assert = require("assert");
const crypto = require("crypto");
const fs = require("fs");
const os = require("os");
const path = require("path");
const readline = require("readline");
const { spawn } = require("child_process");

const RAW_SCHEMA = "rustia.bench.memory-raw.v1";
const CHILD_SCHEMA = "rustia.bench.memory-child-event.v1";
const DEFAULT_CGROUP_ROOT = "/sys/fs/cgroup";
const DEFAULT_TIMEOUT_MS = 10 * 60 * 1000;
const DEFAULT_SAMPLE_INTERVAL_MS = 10;
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_ID = /^[A-Za-z0-9._-]{1,128}$/;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  const [command, ...rest] = argv;
  if (!command || command === "--help" || command === "-h") return { command: "help" };
  if (command === "self-test") {
    if (rest.length !== 0) fail("self-test takes no arguments");
    return { command };
  }
  if (command !== "sample") fail(`unknown command ${JSON.stringify(command)}`);

  const values = new Map();
  const flags = new Set();
  for (let index = 0; index < rest.length; index += 1) {
    const flag = rest[index];
    if (flag === "--allow-root-cgroup") {
      if (flags.has(flag)) fail(`${flag} was supplied more than once`);
      flags.add(flag);
      continue;
    }
    if (!flag.startsWith("--")) fail(`unexpected argument ${JSON.stringify(flag)}`);
    if (values.has(flag)) fail(`${flag} was supplied more than once`);
    const value = rest[index + 1];
    if (value == null || value.startsWith("--")) fail(`${flag} requires a value`);
    values.set(flag, value);
    index += 1;
  }

  const take = (name, required = true) => {
    const value = values.get(name);
    values.delete(name);
    if (required && value == null) fail(`missing ${name}`);
    return value;
  };
  const integer = (name, minimum, fallback = null) => {
    const raw = take(name, fallback == null);
    if (raw == null) return fallback;
    const value = Number(raw);
    if (!Number.isSafeInteger(value) || value < minimum) {
      fail(`${name} must be an integer >= ${minimum}`);
    }
    return value;
  };

  const config = {
    command,
    binary: path.resolve(take("--binary")),
    engine: take("--engine"),
    workload: take("--workload"),
    n: integer("--n", 1),
    epochs: integer("--epochs", 1),
    output: path.resolve(take("--output")),
    commit: take("--commit"),
    sampleId: take("--sample-id"),
    benchmarkCpu: integer("--benchmark-cpu", 0),
    supervisorCpu: integer("--supervisor-cpu", 0),
    timeoutMs: integer("--timeout-ms", 1, DEFAULT_TIMEOUT_MS),
    sampleIntervalMs: integer("--sample-interval-ms", 1, DEFAULT_SAMPLE_INTERVAL_MS),
    cgroupRoot: path.resolve(take("--cgroup-root", false) ?? DEFAULT_CGROUP_ROOT),
    allowRootCgroup: flags.has("--allow-root-cgroup"),
  };
  if (values.size !== 0) fail(`unknown argument(s): ${[...values.keys()].join(", ")}`);
  if (!SAFE_ID.test(config.engine)) fail("--engine is not a safe identifier");
  if (!SAFE_ID.test(config.workload)) fail("--workload is not a safe identifier");
  if (!SAFE_ID.test(config.sampleId)) fail("--sample-id is not a safe identifier");
  if (!FULL_COMMIT.test(config.commit)) fail("--commit must be a lowercase full 40-hex commit");
  if (config.benchmarkCpu === config.supervisorCpu) {
    fail("--benchmark-cpu and --supervisor-cpu must differ");
  }
  return config;
}

function printUsage() {
  process.stdout.write(
    "usage:\n" +
      "  node bench/run-memory-supervisor.js self-test\n" +
      "  node bench/run-memory-supervisor.js sample --binary <script-bench> --engine <id> " +
      "--workload <id> --n <units> --epochs <count> --output <raw.json> " +
      "--commit <40-hex> --sample-id <id> --benchmark-cpu <n> --supervisor-cpu <n> " +
      "[--sample-interval-ms 10] [--timeout-ms 600000] [--cgroup-root /sys/fs/cgroup] " +
      "[--allow-root-cgroup]\n",
  );
}

function readText(file, label = file) {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (error) {
    fail(`cannot read ${label}: ${error.message}`);
  }
}

function readInteger(file, label = file) {
  const raw = readText(file, label).trim();
  if (!/^[0-9]+$/.test(raw)) fail(`${label} is not a nonnegative integer`);
  const value = Number(raw);
  if (!Number.isSafeInteger(value)) fail(`${label} exceeds JavaScript's safe integer range`);
  return value;
}

function parseKeyValueIntegers(text, label) {
  const result = {};
  for (const [index, rawLine] of text.split(/\r?\n/).entries()) {
    const line = rawLine.trim();
    if (line === "") continue;
    const parts = line.split(/\s+/);
    if (parts.length !== 2 || !/^[A-Za-z0-9_()]+$/.test(parts[0]) || !/^[0-9]+$/.test(parts[1])) {
      fail(`${label} line ${index + 1} is not a canonical key/integer pair`);
    }
    if (Object.hasOwn(result, parts[0])) fail(`${label} repeats key ${parts[0]}`);
    result[parts[0]] = Number(parts[1]);
    if (!Number.isSafeInteger(result[parts[0]])) fail(`${label}.${parts[0]} is too large`);
  }
  return result;
}

function parseKibFields(text, label) {
  const result = {};
  for (const [index, rawLine] of text.split(/\r?\n/).entries()) {
    const match = rawLine.match(/^([A-Za-z_()]+):\s+([0-9]+)\s+kB$/);
    if (!match) continue;
    if (Object.hasOwn(result, match[1])) fail(`${label} repeats ${match[1]} at line ${index + 1}`);
    result[match[1]] = Number(match[2]);
  }
  return result;
}

function requireKeys(value, keys, label) {
  for (const key of keys) {
    if (!Number.isSafeInteger(value[key]) || value[key] < 0) {
      fail(`${label} is missing nonnegative ${key}`);
    }
  }
}

function parsePressure(text, label) {
  const result = {};
  for (const [index, rawLine] of text.split(/\r?\n/).entries()) {
    const line = rawLine.trim();
    if (line === "") continue;
    const parts = line.split(/\s+/);
    const kind = parts.shift();
    if (kind !== "some" && kind !== "full") fail(`${label} line ${index + 1} has an invalid kind`);
    const row = {};
    for (const field of parts) {
      const [key, raw] = field.split("=");
      const number = Number(raw);
      if (!key || !Number.isFinite(number) || number < 0) {
        fail(`${label} line ${index + 1} has an invalid field`);
      }
      row[key] = number;
    }
    if (!Number.isSafeInteger(row.total)) fail(`${label}.${kind}.total is missing`);
    result[kind] = row;
  }
  if (!result.some || !result.full) fail(`${label} must contain some and full rows`);
  return result;
}

function processSnapshot(pid) {
  const smapsPath = `/proc/${pid}/smaps_rollup`;
  const statusPath = `/proc/${pid}/status`;
  const smaps = parseKibFields(readText(smapsPath), smapsPath);
  const status = parseKibFields(readText(statusPath), statusPath);
  requireKeys(
    smaps,
    ["Rss", "Pss", "Private_Clean", "Private_Dirty", "Anonymous", "Swap"],
    smapsPath,
  );
  requireKeys(status, ["VmRSS", "VmHWM", "RssAnon", "RssFile", "RssShmem", "VmSwap"], statusPath);
  return {
    smaps_rollup_kib: {
      rss: smaps.Rss,
      pss: smaps.Pss,
      pss_anon: smaps.Pss_Anon ?? null,
      pss_file: smaps.Pss_File ?? null,
      pss_shmem: smaps.Pss_Shmem ?? null,
      private_clean: smaps.Private_Clean,
      private_dirty: smaps.Private_Dirty,
      anonymous: smaps.Anonymous,
      swap: smaps.Swap,
    },
    status_kib: {
      vm_rss: status.VmRSS,
      vm_hwm: status.VmHWM,
      rss_anon: status.RssAnon,
      rss_file: status.RssFile,
      rss_shmem: status.RssShmem,
      vm_swap: status.VmSwap,
    },
  };
}

function cgroupSnapshot(directory) {
  const memoryStat = parseKeyValueIntegers(readText(path.join(directory, "memory.stat")), "memory.stat");
  const events = parseKeyValueIntegers(
    readText(path.join(directory, "memory.events.local")),
    "memory.events.local",
  );
  const pressure = parsePressure(readText(path.join(directory, "memory.pressure")), "memory.pressure");
  return {
    current_bytes: readInteger(path.join(directory, "memory.current"), "memory.current"),
    peak_bytes: readInteger(path.join(directory, "memory.peak"), "memory.peak"),
    swap_current_bytes: readInteger(path.join(directory, "memory.swap.current"), "memory.swap.current"),
    stat_bytes: memoryStat,
    events,
    pressure,
  };
}

function writeControl(file, value, label = file) {
  try {
    fs.writeFileSync(file, value);
  } catch (error) {
    fail(`cannot write ${label}: ${error.message}`);
  }
}

function enableController(directory, controller) {
  const controllers = readText(path.join(directory, "cgroup.controllers"), "cgroup.controllers")
    .trim()
    .split(/\s+/);
  if (!controllers.includes(controller)) fail(`cgroup v2 ${controller} controller is unavailable`);
  const control = path.join(directory, "cgroup.subtree_control");
  const enabled = readText(control, "cgroup.subtree_control").trim().split(/\s+/);
  if (!enabled.includes(controller)) writeControl(control, `+${controller}`, "cgroup.subtree_control");
}

// What this actually needs is a cgroup-v2 subtree it can write to, with the
// memory controller available. Being root is one way to have that; a systemd
// delegated user subtree is another, and the benchmark host deliberately runs
// its CI as an unprivileged user. Asserting uid 0 named a sufficient condition
// as if it were the requirement, and refused a host that could measure
// perfectly well.
function assertUsableCgroupRoot(root) {
  if (!fs.existsSync(path.join(root, "cgroup.controllers"))) {
    fail(`${root} is not a cgroup-v2 mount`);
  }
  const controllers = readText(path.join(root, "cgroup.controllers"), "cgroup.controllers")
    .trim()
    .split(/\s+/);
  if (!controllers.includes("memory")) {
    fail(
      `${root} does not offer the memory controller (has: ${controllers.join(" ") || "none"}). ` +
        "Per-VM accounting needs it; on a delegated user subtree that means Delegate=memory.",
    );
  }
  try {
    fs.accessSync(root, fs.constants.W_OK | fs.constants.X_OK);
    fs.accessSync(path.join(root, "cgroup.subtree_control"), fs.constants.W_OK);
  } catch (error) {
    fail(
      `${root} is not writable by this user (${error.code}). Run as root, or delegate a cgroup ` +
        "subtree to the benchmark user and pass it with --cgroup-root.",
    );
  }
}

function createCgroup(config) {
  if (process.platform !== "linux") fail("memory publication supervisor requires Linux");
  assertUsableCgroupRoot(config.cgroupRoot);
  if (config.cgroupRoot === DEFAULT_CGROUP_ROOT && !config.allowRootCgroup) {
    fail("using the root cgroup mount requires --allow-root-cgroup");
  }
  enableController(config.cgroupRoot, "memory");
  const parent = path.join(config.cgroupRoot, "rustia-memory-bench");
  fs.mkdirSync(parent, { mode: 0o755, recursive: true });
  enableController(parent, "memory");
  const leaf = path.join(
    parent,
    `${config.sampleId}-${process.pid}-${crypto.randomBytes(6).toString("hex")}`,
  );
  fs.mkdirSync(leaf, { mode: 0o755 });
  writeControl(path.join(leaf, "memory.max"), "max");
  writeControl(path.join(leaf, "memory.swap.max"), "0");
  return {
    directory: leaf,
    peakDescriptor: openResettablePeak(leaf),
  };
}

function resetPeakDescriptor(descriptor) {
  fs.writeSync(descriptor, Buffer.from("reset"), 0, 5, 0);
}

function readPeakDescriptor(descriptor) {
  const buffer = Buffer.alloc(64);
  const length = fs.readSync(descriptor, buffer, 0, buffer.length, 0);
  const raw = buffer.subarray(0, length).toString("utf8").trim();
  if (!/^[0-9]+$/.test(raw)) fail("resettable memory.peak returned a non-integer");
  const value = Number(raw);
  if (!Number.isSafeInteger(value)) fail("resettable memory.peak exceeds JavaScript's safe integer range");
  return value;
}

function openResettablePeak(directory) {
  const peak = path.join(directory, "memory.peak");
  let descriptor;
  try {
    descriptor = fs.openSync(peak, "r+");
    resetPeakDescriptor(descriptor);
    readPeakDescriptor(descriptor);
    return descriptor;
  } catch (_) {
    if (descriptor != null) fs.closeSync(descriptor);
    return false;
  }
}

function removeCgroup(directory) {
  try {
    fs.rmdirSync(directory);
  } catch (error) {
    fail(`cannot remove empty measurement cgroup ${directory}: ${error.message}`);
  }
}

function sha256File(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function writeJsonAtomic(file, value) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomBytes(6).toString("hex")}`;
  fs.writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { flag: "wx" });
  fs.renameSync(temporary, file);
}

function validateChildEvent(value, expected, priorSequence) {
  if (value == null || typeof value !== "object" || Array.isArray(value)) fail("child event must be an object");
  if (value.schema_version !== 1 || value.schema !== CHILD_SCHEMA) fail("child event schema is invalid");
  if (value.sequence !== priorSequence + 1) fail("child event sequence is not contiguous");
  if (value.pid !== expected.pid) fail("child event pid does not match spawned process");
  for (const field of ["engine", "workload", "commit"]) {
    if (value[field] !== expected[field]) fail(`child event ${field} does not match the supervisor request`);
  }
  if (value.n !== expected.n || value.epochs !== expected.epochs) fail("child event work shape is wrong");
  if (!["spawned", "prepared", "warmed", "epoch", "quiescent", "dropped"].includes(value.phase)) {
    fail(`child event has unknown phase ${JSON.stringify(value.phase)}`);
  }
  if (value.phase === "epoch") {
    if (!Number.isSafeInteger(value.epoch) || value.epoch < 1 || value.epoch > expected.epochs) {
      fail("child epoch marker is invalid");
    }
    if (typeof value.checksum !== "string" || value.checksum === "") fail("child epoch checksum is missing");
  } else if (value.epoch != null) {
    fail("non-epoch child event unexpectedly has an epoch");
  }
  return value;
}

function phaseShape(epochs) {
  return [
    "spawned",
    "prepared",
    "warmed",
    ...Array.from({ length: epochs }, () => "epoch"),
    "quiescent",
    "dropped",
  ];
}

async function collectSample(config) {
  if (!fs.existsSync(config.binary) || !fs.statSync(config.binary).isFile()) {
    fail(`benchmark binary does not exist: ${config.binary}`);
  }
  if (fs.existsSync(config.output)) fail(`refusing to overwrite ${config.output}`);

  const cgroup = createCgroup(config);
  const childArgs = [
    "-c",
    String(config.benchmarkCpu),
    config.binary,
    "--memory-child",
    "--engines",
    config.engine,
    "--workloads",
    config.workload,
    "--fixed-n",
    String(config.n),
    "--memory-epochs",
    String(config.epochs),
    "--commit",
    config.commit,
    "--no-history",
    "--no-results",
    "--no-latest-json",
  ];
  const startedAt = new Date().toISOString();
  const startedNs = process.hrtime.bigint();
  const child = spawn("taskset", childArgs, {
    cwd: path.resolve(__dirname, ".."),
    stdio: ["pipe", "pipe", "pipe"],
    env: { ...process.env, RUSTC_WRAPPER: "" },
  });
  if (!Number.isSafeInteger(child.pid) || child.pid <= 0) fail("child process has no pid");
  writeControl(path.join(cgroup.directory, "cgroup.procs"), String(child.pid), "cgroup.procs");

  const events = [];
  const timeline = [];
  let stderr = "";
  let protocolError = null;
  let sequence = 0;
  let lineChain = Promise.resolve();
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk) => {
    stderr += chunk;
  });

  const sampleTimeline = () => {
    try {
      timeline.push({
        elapsed_ms: Number(process.hrtime.bigint() - startedNs) / 1e6,
        current_bytes: readInteger(path.join(cgroup.directory, "memory.current"), "memory.current"),
      });
    } catch (error) {
      if (protocolError == null) protocolError = error;
    }
  };
  sampleTimeline();
  const timer = setInterval(sampleTimeline, config.sampleIntervalMs);

  const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
  lines.on("line", (line) => {
    lineChain = lineChain.then(() => {
      if (protocolError != null) return;
      try {
        const parsed = JSON.parse(line);
        const value = validateChildEvent(
          parsed,
          {
            pid: child.pid,
            engine: config.engine,
            workload: config.workload,
            commit: config.commit,
            n: config.n,
            epochs: config.epochs,
          },
          sequence,
        );
        sequence = value.sequence;
        const processMemory = processSnapshot(child.pid);
        const cgroupMemory = cgroupSnapshot(cgroup.directory);
        if (processMemory.smaps_rollup_kib.swap !== 0 || processMemory.status_kib.vm_swap !== 0) {
          fail(`child phase ${value.phase} used swap`);
        }
        if (cgroupMemory.swap_current_bytes !== 0) fail(`child phase ${value.phase} used cgroup swap`);
        const phasePeakBytes =
          cgroup.peakDescriptor === false ? null : readPeakDescriptor(cgroup.peakDescriptor);
        events.push({
          event: value,
          elapsed_ms: Number(process.hrtime.bigint() - startedNs) / 1e6,
          process_memory: processMemory,
          cgroup_memory: cgroupMemory,
          phase_peak_bytes: phasePeakBytes,
        });
        if (cgroup.peakDescriptor !== false) resetPeakDescriptor(cgroup.peakDescriptor);
        child.stdin.write("continue\n");
      } catch (error) {
        protocolError = error;
        child.kill("SIGKILL");
      }
    });
  });

  const timeout = setTimeout(() => {
    if (protocolError == null) protocolError = new Error(`memory child exceeded ${config.timeoutMs} ms`);
    child.kill("SIGKILL");
  }, config.timeoutMs);

  const exit = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code, signal) => resolve({ code, signal }));
  });
  await lineChain;
  clearInterval(timer);
  clearTimeout(timeout);
  lines.close();
  sampleTimeline();

  let finalCgroup;
  try {
    finalCgroup = cgroupSnapshot(cgroup.directory);
  } finally {
    if (cgroup.peakDescriptor !== false) fs.closeSync(cgroup.peakDescriptor);
    removeCgroup(cgroup.directory);
  }

  if (protocolError != null) throw protocolError;
  if (exit.code !== 0 || exit.signal != null) {
    fail(`memory child failed with code=${exit.code} signal=${exit.signal}: ${stderr.trim()}`);
  }
  const actualShape = events.map((entry) => entry.event.phase);
  assert.deepEqual(actualShape, phaseShape(config.epochs), "memory child phase sequence");
  for (let index = 0; index < config.epochs; index += 1) {
    const event = events[3 + index].event;
    if (event.epoch !== index + 1) fail("memory child epoch order is not contiguous");
  }
  const finalEvents = finalCgroup.events;
  for (const key of ["oom", "oom_kill", "oom_group_kill"]) {
    if ((finalEvents[key] ?? 0) !== 0) fail(`memory cgroup recorded ${key}=${finalEvents[key]}`);
  }
  for (const key of ["high", "max"]) {
    if ((finalEvents[key] ?? 0) !== 0) fail(`memory cgroup recorded ${key}=${finalEvents[key]}`);
  }
  if (
    (finalCgroup.pressure.some.total ?? 0) !== 0 ||
    (finalCgroup.pressure.full.total ?? 0) !== 0
  ) {
    fail("memory cgroup recorded reclaim pressure");
  }
  if (finalCgroup.swap_current_bytes !== 0) fail("memory cgroup ended with nonzero swap");

  const result = {
    schema_version: 1,
    schema: RAW_SCHEMA,
    sample_id: config.sampleId,
    captured_at: new Date().toISOString(),
    started_at: startedAt,
    identity: {
      commit: config.commit,
      binary: config.binary,
      binary_sha256: sha256File(config.binary),
      engine: config.engine,
      workload: config.workload,
      n: config.n,
      epochs: config.epochs,
    },
    platform: {
      os: process.platform,
      arch: process.arch,
      release: os.release(),
      hostname: os.hostname(),
      benchmark_cpu: config.benchmarkCpu,
      supervisor_cpu: config.supervisorCpu,
      page_size_bytes: Number(process.env.PAGESIZE || 4096),
      cgroup_v2: true,
      resettable_memory_peak: cgroup.peakDescriptor !== false,
      sample_interval_ms: config.sampleIntervalMs,
      vm_rss_hwm_is_diagnostic_only: true,
    },
    protocol: {
      child_schema: CHILD_SCHEMA,
      isolation: "fresh process and cgroup-v2 leaf",
      primary_phase_memory: "process_memory.smaps_rollup_kib.anonymous",
      corroborating_phase_memory: ["pss", "private_dirty", "rss"],
      primary_short_spike_memory: "final_cgroup.peak_bytes",
      memory_peak_scope: "whole sample",
      phase_peak_memory: "events[].phase_peak_bytes when resettable_memory_peak is true",
      forced_collection: false,
      allocator_purge: false,
    },
    events,
    timeline,
    final_cgroup: finalCgroup,
    child_stderr: stderr,
  };
  writeJsonAtomic(config.output, result);
  return result;
}

function selfTest() {
  assert.deepEqual(parseKeyValueIntegers("anon 1024\npgfault 9\n", "fixture"), {
    anon: 1024,
    pgfault: 9,
  });
  const kib = parseKibFields(
    "Rss:                 100 kB\nPss:                  90 kB\nPss_Anon:             70 kB\n",
    "fixture",
  );
  assert.equal(kib.Rss, 100);
  assert.equal(kib.Pss_Anon, 70);
  assert.deepEqual(parsePressure(
    "some avg10=0.00 avg60=0.00 avg300=0.00 total=12\n" +
      "full avg10=0.00 avg60=0.00 avg300=0.00 total=3\n",
    "fixture",
  ), {
    some: { avg10: 0, avg60: 0, avg300: 0, total: 12 },
    full: { avg10: 0, avg60: 0, avg300: 0, total: 3 },
  });
  assert.deepEqual(phaseShape(3), [
    "spawned",
    "prepared",
    "warmed",
    "epoch",
    "epoch",
    "epoch",
    "quiescent",
    "dropped",
  ]);
  assert.throws(() => parseArgs(["sample", "--engine", "rustia"]));
  process.stdout.write("memory supervisor self-test passed\n");
}

async function main() {
  const config = parseArgs(process.argv.slice(2));
  if (config.command === "help") {
    printUsage();
    return;
  }
  if (config.command === "self-test") {
    selfTest();
    return;
  }
  const result = await collectSample(config);
  process.stdout.write(
    `${result.sample_id}: ${result.identity.engine}/${result.identity.workload} ` +
      `peak=${result.final_cgroup.peak_bytes} bytes\n`,
  );
}

main().catch((error) => {
  process.stderr.write(`run-memory-supervisor: ${error.message}\n`);
  process.exitCode = 1;
});
