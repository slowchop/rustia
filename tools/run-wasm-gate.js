#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const wasmManifest = path.join(root, "bench", "wasm", "adapter-runner", "Cargo.toml");
const wasmPath = path.join(
  root,
  "bench",
  "wasm",
  "adapter-runner",
  "target",
  "wasm32-unknown-unknown",
  "release",
  "script_bench_wasm_adapter.wasm",
);
const measureScript = path.join(root, "bench", "wasm", "measure-rustia-wasm.mjs");
const measureScriptSpiderMonkey = path.join(root, "bench", "wasm", "measure-rustia-wasm-sm.js");
const outputPath = path.join(root, "bench", "wasm", "current-results.json");

function fail(message) {
  console.error(`run-wasm-gate: ${message}`);
  process.exit(1);
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function relative(filePath) {
  return slash(path.relative(root, filePath));
}

function parseArgs(argv) {
  const options = {
    build: true,
    runtimes: ["node", "deno", "bun"],
    output: outputPath,
    targetMs: 80,
    reps: 5,
    adapters: ["rustia", "koto"],
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--no-build") {
      options.build = false;
    } else if (arg === "--runtime") {
      options.runtimes = requiredValue(argv, ++index, arg)
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
    } else if (arg === "--output") {
      options.output = path.resolve(root, requiredValue(argv, ++index, arg));
    } else if (arg === "--target-ms") {
      options.targetMs = positiveNumber(requiredValue(argv, ++index, arg), arg);
    } else if (arg === "--reps") {
      options.reps = positiveInteger(requiredValue(argv, ++index, arg), arg);
    } else if (arg === "--adapters") {
      options.adapters = requiredValue(argv, ++index, arg)
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
    } else {
      fail(`unknown argument ${JSON.stringify(arg)}`);
    }
  }

  return options;
}

function requiredValue(argv, index, flag) {
  if (index >= argv.length) {
    fail(`${flag} requires a value`);
  }
  return argv[index];
}

function positiveNumber(value, label) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) {
    fail(`${label} must be a positive number`);
  }
  return number;
}

function positiveInteger(value, label) {
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) {
    fail(`${label} must be a positive integer`);
  }
  return number;
}

function run(command, args, options = {}) {
  try {
    return childProcess.execFileSync(command, args, {
      cwd: root,
      encoding: options.encoding ?? "utf8",
      stdio: options.stdio ?? ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`${command} ${args.join(" ")} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function commandExists(command) {
  return commandPath(command) != null;
}

function commandPath(command) {
  const result = childProcess.spawnSync("sh", ["-lc", `command -v ${shellQuote(command)}`], {
    cwd: root,
    encoding: "utf8",
  });
  if (result.status !== 0) {
    return null;
  }
  const text = result.stdout.trim();
  return text === "" ? null : text;
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
}

function gitCommit() {
  return run("git", ["rev-parse", "--short", "HEAD"]).trim();
}

function buildWasm() {
  run(
    "cargo",
    [
      "build",
      "--manifest-path",
      relative(wasmManifest),
      "--target",
      "wasm32-unknown-unknown",
      "--release",
    ],
    { stdio: "inherit" },
  );
}

function runtimeCommand(runtime, commonArgs) {
  if (runtime === "node") {
    return { command: process.execPath, args: commonArgs };
  }
  if (runtime === "deno") {
    return {
      command: "deno",
      args: ["run", "--allow-read", ...commonArgs],
    };
  }
  if (runtime === "bun") {
    return { command: "bun", args: commonArgs };
  }
  if (runtime === "spidermonkey") {
    const candidates = ["js", "sm", "js140", "js128", "js125", "js115", "js102"];
    for (const candidate of candidates) {
      const found = commandPath(candidate);
      if (found) {
        return { command: found, args: commonArgs };
      }
    }
    return null;
  }
  fail(`unsupported runtime ${JSON.stringify(runtime)}`);
}

function commandOutput(command, args) {
  try {
    return childProcess.execFileSync(command, args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch (_) {
    return "unknown";
  }
}

function measureRuntime(runtime, options, commit) {
  const script = runtime === "spidermonkey" ? measureScriptSpiderMonkey : measureScript;
  const commandArgs = [
    relative(script),
    "--wasm",
    relative(wasmPath),
    "--commit",
    commit,
    "--target-ms",
    String(options.targetMs),
    "--reps",
    String(options.reps),
    "--adapters",
    options.adapters.join(","),
  ];
  const runtimeSpec = runtimeCommand(runtime, commandArgs);
  if (!runtimeSpec) {
    return {
      skipped: {
        runtime,
        reason: "command not found",
      },
    };
  }
  if (runtime === "spidermonkey") {
    runtimeSpec.args.push("--runtime-version", commandOutput(runtimeSpec.command, ["--version"]));
  }
  const { command, args } = runtimeSpec;
  const text = run(command, args);
  try {
    return JSON.parse(text);
  } catch (error) {
    fail(`${runtime} measurement did not return JSON: ${error.message}\n${text}`);
  }
}

function validateRuntimeBatch(batch) {
  if (batch.schema !== "rustia.bench.wasm-runtime-results-batch.v1") {
    fail(`unexpected measurement schema ${JSON.stringify(batch.schema)}`);
  }
  if (!Array.isArray(batch.runs) || batch.runs.length === 0) {
    fail(`${batch.runtime?.name ?? "runtime"} measurement must contain adapter runs`);
  }
  for (const runResult of batch.runs) {
    validateAdapterRun(runResult);
  }
}

function validateAdapterRun(runResult) {
  if (runResult.schema !== "rustia.bench.wasm-runtime-results.v1") {
    fail(`unexpected adapter measurement schema ${JSON.stringify(runResult.schema)}`);
  }
  if (runResult.adapter == null || typeof runResult.adapter !== "object") {
    fail(`${runResult.runtime?.name ?? "runtime"} measurement must include adapter metadata`);
  }
  if (typeof runResult.adapter.id !== "string" || runResult.adapter.id.trim() === "") {
    fail(`${runResult.runtime?.name ?? "runtime"} adapter id must be a non-empty string`);
  }
  if (!Array.isArray(runResult.rows) || runResult.rows.length === 0) {
    fail(`${runResult.runtime?.name ?? "runtime"} ${runResult.adapter.id} measurement must contain rows`);
  }
  for (const [index, row] of runResult.rows.entries()) {
    for (const field of ["engine", "dialect", "workload", "unit", "checksum"]) {
      if (typeof row[field] !== "string" || row[field].trim() === "") {
        fail(`${runResult.runtime.name}/${runResult.adapter.id} rows[${index}].${field} must be a non-empty string`);
      }
    }
    if (row.engine !== runResult.adapter.id) {
      fail(`${runResult.runtime.name}/${runResult.adapter.id} rows[${index}].engine must match adapter`);
    }
    for (const field of ["n", "reps", "ns_per_unit_min", "ns_per_unit_median", "units_per_sec"]) {
      if (!Number.isFinite(row[field]) || row[field] <= 0) {
        fail(`${runResult.runtime.name}/${runResult.adapter.id} rows[${index}].${field} must be positive and finite`);
      }
    }
    if (row.ns_per_unit_median < row.ns_per_unit_min) {
      fail(`${runResult.runtime.name}/${runResult.adapter.id} rows[${index}] median must be >= min`);
    }
  }
}

function summarizeRequirements(runs) {
  const families = new Set(
    runs
      .filter((runResult) => runResult.adapter.id === "rustia")
      .map((runResult) => runResult.runtime.family),
  );
  return {
    "v8-production-rustia-wasm": families.has("V8"),
    "javascriptcore-production-rustia-wasm": families.has("JavaScriptCore"),
    "spidermonkey-production-rustia-wasm": families.has("SpiderMonkey"),
  };
}

const options = parseArgs(process.argv.slice(2));
if (options.build) {
  buildWasm();
}
if (!fs.existsSync(wasmPath)) {
  fail(`missing wasm artifact ${relative(wasmPath)}`);
}

const commit = gitCommit();
const runs = [];
const skipped = [];
for (const runtime of options.runtimes) {
  if (runtime !== "node" && !commandExists(runtime)) {
    const runtimeSpec = runtime === "spidermonkey" ? runtimeCommand(runtime, []) : null;
    if (!runtimeSpec) {
      skipped.push({ runtime, reason: "command not found" });
      continue;
    }
  }
  const measured = measureRuntime(runtime, options, commit);
  if (measured.skipped) {
    skipped.push(measured.skipped);
    continue;
  }
  const batch = measured;
  validateRuntimeBatch(batch);
  runs.push(...batch.runs);
}

if (runs.length === 0) {
  fail("no wasm runtimes were measured");
}

const value = {
  schema_version: 1,
  schema: "rustia.bench.wasm-gate-current-results.v1",
  source: "script-bench-harness",
  updated: new Date().toISOString().slice(0, 10),
  commit,
  wasm: {
    manifest: relative(wasmManifest),
    artifact: relative(wasmPath),
    bytes: fs.statSync(wasmPath).size,
  },
  requirements: summarizeRequirements(runs),
  runs,
  skipped,
};
const text = `${JSON.stringify(value, null, 2)}\n`;
fs.mkdirSync(path.dirname(options.output), { recursive: true });
fs.writeFileSync(options.output, text);
console.log(`wrote ${relative(options.output)} with ${runs.length} wasm adapter measurement(s)`);
