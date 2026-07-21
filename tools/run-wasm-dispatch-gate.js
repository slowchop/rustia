#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const measureScript = path.join(root, "bench", "wasm", "measure-dispatch-wasm.js");
const outputPath = path.join(root, "bench", "wasm", "dispatch-current-results.json");
const requiredFamilies = ["V8"];
const fullCommit = /^[0-9a-f]{40}$/;

function fail(message) {
  console.error(`run-wasm-dispatch-gate: ${message}`);
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
    runtimes: ["node", "deno", "bun", "spidermonkey"],
    output: outputPath,
    targetMs: 80,
    reps: 5,
    maxN: 1_000_000_000,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--runtime") {
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
    } else if (arg === "--max-n") {
      options.maxN = positiveInteger(requiredValue(argv, ++index, arg), arg);
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

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`;
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

function run(command, args) {
  try {
    return childProcess.execFileSync(command, args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`${command} ${args.join(" ")} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function gitCommit(runCommand = run) {
  const commit = runCommand("git", ["rev-parse", "HEAD"]).trim();
  if (!fullCommit.test(commit)) {
    fail(`git rev-parse HEAD must return a full 40-character lowercase commit, got ${JSON.stringify(commit)}`);
  }
  return commit;
}

function selfTest() {
  const expected = "a".repeat(40);
  let invocation = null;
  const actual = gitCommit((command, args) => {
    invocation = { command, args };
    return `${expected}\n`;
  });
  if (
    actual !== expected ||
    invocation?.command !== "git" ||
    invocation.args.length !== 2 ||
    invocation.args[0] !== "rev-parse" ||
    invocation.args[1] !== "HEAD" ||
    fullCommit.test("a".repeat(39))
  ) {
    fail("full-commit provenance self-test failed");
  }
  console.log("run-wasm-dispatch-gate full-commit provenance self-test passed");
}

function runtimeCommand(runtime, commonArgs) {
  if (runtime === "node") {
    return { command: process.execPath, args: commonArgs };
  }
  if (runtime === "deno") {
    return { command: "deno", args: ["run", ...commonArgs] };
  }
  if (runtime === "bun") {
    return { command: "bun", args: commonArgs };
  }
  if (runtime === "spidermonkey") {
    const candidates = ["js", "sm", "js128", "js125", "js115", "js102"];
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

function measureRuntime(runtime, options, commit) {
  const commandArgs = [
    relative(measureScript),
    "--commit",
    commit,
    "--target-ms",
    String(options.targetMs),
    "--reps",
    String(options.reps),
    "--max-n",
    String(options.maxN),
  ];
  if (runtime === "spidermonkey") commandArgs.push("--host", `${process.platform}-${process.arch}`);
  const runtimeSpec = runtimeCommand(runtime, commandArgs);
  if (!runtimeSpec) {
    return {
      skipped: {
        runtime,
        reason: "command not found",
      },
    };
  }
  const text = run(runtimeSpec.command, runtimeSpec.args);
  try {
    const result = JSON.parse(text);
    validateRuntimeResult(result, commit);
    return { result };
  } catch (error) {
    fail(`${runtime} dispatch measurement did not return valid JSON: ${error.message}\n${text}`);
  }
}

function validateRuntimeResult(result, commit) {
  assertEqual(result.schema_version, 1, "dispatch result schema_version");
  assertEqual(result.schema, "rustia.bench.wasm-dispatch-results.v1", "dispatch result schema");
  assertEqual(result.source, "script-bench-harness", "dispatch result source");
  assertEqual(result.commit, commit, "dispatch result full HEAD commit");
  assertText(result.target, "dispatch result target");
  assertObject(result.runtime, "dispatch result runtime");
  assertText(result.runtime.name, "dispatch result runtime.name");
  assertText(result.runtime.family, "dispatch result runtime.family");
  assertObject(result.microbench, "dispatch result microbench");
  assertEqual(result.microbench.id, "call_ref_vs_br_table", "dispatch result microbench.id");
  if (!Array.isArray(result.rows)) {
    fail(`${result.target} rows must be an array`);
  }
  if (!Array.isArray(result.skipped)) {
    fail(`${result.target} skipped must be an array`);
  }
  assertObject(result.comparison, "dispatch result comparison");
  if (result.comparison.status === "measured") {
    if (result.rows.length !== 2) {
      fail(`${result.target} measured comparison must contain br_table and call_ref rows`);
    }
    assertObject(result.verification, `${result.target} verification`);
    assertPositiveFinite(result.verification.n, `${result.target} verification.n`);
    assertText(result.verification.checksum, `${result.target} verification.checksum`);
    const styles = new Set(result.rows.map((row) => row.style));
    for (const style of ["br_table", "call_ref"]) {
      if (!styles.has(style)) {
        fail(`${result.target} is missing ${style} row`);
      }
    }
    for (const [index, row] of result.rows.entries()) {
      const label = `${result.target} rows[${index}]`;
      assertText(row.style, `${label}.style`);
      assertText(row.feature, `${label}.feature`);
      assertPositiveFinite(row.n, `${label}.n`);
      assertPositiveFinite(row.reps, `${label}.reps`);
      assertPositiveFinite(row.wasm_bytes, `${label}.wasm_bytes`);
      assertPositiveFinite(row.ns_per_dispatch_min, `${label}.ns_per_dispatch_min`);
      assertPositiveFinite(row.ns_per_dispatch_median, `${label}.ns_per_dispatch_median`);
      assertPositiveFinite(row.dispatches_per_sec, `${label}.dispatches_per_sec`);
      assertText(row.checksum, `${label}.checksum`);
      if (row.ns_per_dispatch_median < row.ns_per_dispatch_min) {
        fail(`${label}.ns_per_dispatch_median must be >= ns_per_dispatch_min`);
      }
    }
    assertPositiveFinite(result.comparison.candidate_vs_baseline_min, "comparison.candidate_vs_baseline_min");
    assertPositiveFinite(result.comparison.candidate_vs_baseline_median, "comparison.candidate_vs_baseline_median");
  } else if (result.comparison.status !== "unsupported") {
    fail(`${result.target} comparison.status must be measured or unsupported`);
  }
}

function assertEqual(actual, expected, description) {
  if (actual !== expected) {
    fail(`${description}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
  }
}

function assertText(value, description) {
  if (typeof value !== "string" || value.trim() === "") {
    fail(`${description} must be a non-empty string`);
  }
}

function assertPositiveFinite(value, description) {
  if (!Number.isFinite(value) || value <= 0) {
    fail(`${description} must be positive and finite, got ${JSON.stringify(value)}`);
  }
}

function assertObject(value, description) {
  if (value == null || typeof value !== "object" || Array.isArray(value)) {
    fail(`${description} must be an object`);
  }
}

function summarizeRequirements(runs) {
  const measuredFamilies = new Set(
    runs
      .filter((runResult) => runResult.comparison.status === "measured")
      .map((runResult) => runResult.runtime.family),
  );
  return {
    "call-ref-vs-br-table": requiredFamilies.every((family) => measuredFamilies.has(family)),
  };
}

if (process.argv.length === 3 && process.argv[2] === "self-test") {
  selfTest();
  process.exit(0);
}

const options = parseArgs(process.argv.slice(2));
if (!fs.existsSync(measureScript)) {
  fail(`missing measure script ${relative(measureScript)}`);
}

const commit = gitCommit();
const runs = [];
const skipped = [];
for (const runtime of options.runtimes) {
  const measured = measureRuntime(runtime, options, commit);
  if (measured.skipped) {
    skipped.push(measured.skipped);
  } else {
    runs.push(measured.result);
  }
}

if (runs.length === 0) {
  fail("no wasm dispatch runtimes were measured");
}

const value = {
  schema_version: 1,
  schema: "rustia.bench.wasm-dispatch-current-results.v1",
  source: "script-bench-harness",
  updated: new Date().toISOString().slice(0, 10),
  commit,
  requirements: summarizeRequirements(runs),
  required_families: requiredFamilies,
  runs,
  skipped,
};

const text = `${JSON.stringify(value, null, 2)}\n`;
fs.mkdirSync(path.dirname(options.output), { recursive: true });
fs.writeFileSync(options.output, text);
console.log(`wrote ${relative(options.output)} with ${runs.length} wasm dispatch runtime(s)`);
