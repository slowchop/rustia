#!/usr/bin/env node
"use strict";

const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const ANALYSIS_SCHEMA = "rustia.bench.memory-analysis.v1";
const { MEMORY_PROTOCOL, requireSingleProtocol, validateProtocolMetrics } = require("./memory-protocol.js");
const COHORT_SCHEMA = "rustia.bench.memory-cohort.v1";
const CURRENT_SCHEMA = "rustia.bench.memory-current-results.v1";
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const SAFE_ID = /^[A-Za-z0-9._-]{1,128}$/;
// The machine a cohort names is a provenance label, copied from here into the
// history line and the memory table that are pushed to the public data
// repository. A label, never an address: no dots and no colons, so it can be
// neither a fully qualified name nor an IPv4 or IPv6 address. Every emitter and
// validator carries this same literal; the self-test in
// tools/check-benchmark-results.js asserts it.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { command: "self-test" };
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value == null) fail(`${flag ?? "argument"} requires a value`);
    if (values.has(flag)) fail(`${flag} was supplied more than once`);
    values.set(flag, value);
  }
  const take = (name) => {
    const value = values.get(name);
    values.delete(name);
    if (value == null) fail(`missing ${name}`);
    return value;
  };
  const config = {
    command: "publish",
    analysis: path.resolve(take("--analysis")),
    cohortId: take("--cohort-id"),
    evidence: take("--evidence"),
    history: path.resolve(take("--history")),
    current: path.resolve(take("--current")),
    commit: take("--commit"),
  };
  if (values.size !== 0) fail(`unknown argument(s): ${[...values.keys()].join(", ")}`);
  if (!SAFE_ID.test(config.cohortId)) fail("--cohort-id must be a safe identifier");
  if (!FULL_COMMIT.test(config.commit)) fail("--commit must be a lowercase full commit");
  if (
    config.evidence.startsWith("/") ||
    config.evidence.split(/[\\/]+/).some((component) => component === "..")
  ) {
    fail("--evidence must be a safe repository-relative path");
  }
  return config;
}

function readJson(file, label) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    fail(`cannot parse ${label}: ${error.message}`);
  }
}

function sha256File(file) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(file));
  return hash.digest("hex");
}

function validateAnalysis(analysis, config) {
  const corrected = analysis?.schema_version === 2 && analysis?.schema === "rustia.bench.memory-analysis.v2";
  if (!corrected && (analysis?.schema_version !== 1 || analysis?.schema !== ANALYSIS_SCHEMA)) {
    fail("analysis uses the wrong schema");
  }
  if (!Array.isArray(analysis.rows) || analysis.rows.length === 0) fail("analysis has no rows");
  if (!Array.isArray(analysis.detector_coverage)) fail("analysis lacks detector coverage");
  const protocol = requireSingleProtocol(analysis.rows, "memory analysis");
  if ((protocol === MEMORY_PROTOCOL) !== corrected) fail("analysis schema and row protocol differ");
  if (corrected && analysis.method?.measurement_protocol !== protocol) fail("analysis method and row protocol differ");
  analysis.rows.forEach(validateProtocolMetrics);
  for (const row of analysis.rows) {
    if (row.commit !== config.commit) fail("analysis contains a different commit");
    if (typeof row.target !== "string" || row.target === "") fail("analysis row target is missing");
    if (typeof row.host !== "string" || row.host === "") fail("analysis row host is missing");
    if (!HOST_LABEL.test(row.host)) {
      fail(
        `analysis row ${row.engine}/${row.workload} names its machine ${JSON.stringify(row.host)}, ` +
        "which is not a label: a label may contain only [A-Za-z0-9_-], never an address",
      );
    }
    if (row.classification === "detector_failed") {
      fail(`${row.engine}/${row.workload} failed the intentional-retention detector`);
    }
  }
  // One cohort is one run on one machine. Two hosts here means the analysis was
  // assembled from separate runs, and the memory table would inherit the mix.
  const hosts = [...new Set(analysis.rows.map((row) => row.host))].sort();
  if (hosts.length !== 1) {
    fail(`analysis rows name ${hosts.length} hosts (${hosts.join(", ")}); a cohort is one machine`);
  }
  // Checked as a label before it is compared with the rows, so an address is
  // refused as an address rather than as a disagreement.
  if (typeof analysis.host !== "string" || analysis.host === "") fail("analysis host is missing");
  if (!HOST_LABEL.test(analysis.host)) {
    fail(
      `analysis names host ${JSON.stringify(analysis.host)}, ` +
      "which is not a label: a label may contain only [A-Za-z0-9_-], never an address",
    );
  }
  if (analysis.host !== hosts[0]) {
    fail(`analysis names host ${JSON.stringify(analysis.host)} but its rows name ${JSON.stringify(hosts[0])}`);
  }
  const identities = new Set(
    analysis.rows.map((row) =>
      [row.commit, row.target, row.engine, row.execution_mode, row.workload, row.n, row.epochs].join("\0"),
    ),
  );
  if (identities.size !== analysis.rows.length) fail("analysis contains duplicate summary identities");
}

function parseHistory(file) {
  if (!fs.existsSync(file)) return [];
  const records = [];
  for (const [index, raw] of fs.readFileSync(file, "utf8").split(/\r?\n/).entries()) {
    const line = raw.trim();
    if (line === "") continue;
    let value;
    try {
      value = JSON.parse(line);
    } catch (error) {
      fail(`${file} line ${index + 1} is invalid JSON: ${error.message}`);
    }
    if (value.schema_version !== 1 || value.schema !== COHORT_SCHEMA) {
      fail(`${file} line ${index + 1} has the wrong cohort schema`);
    }
    records.push(value);
  }
  return records;
}

function writeAtomic(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const temporary = `${file}.tmp-${process.pid}-${crypto.randomBytes(6).toString("hex")}`;
  fs.writeFileSync(temporary, text, { flag: "wx" });
  fs.renameSync(temporary, file);
}

function publish(config) {
  const analysis = readJson(config.analysis, "memory analysis");
  validateAnalysis(analysis, config);
  const history = parseHistory(config.history);
  if (history.some((record) => record.cohort_id === config.cohortId)) {
    fail(`memory history already contains cohort ${config.cohortId}`);
  }
  const targets = [...new Set(analysis.rows.map((row) => row.target))].sort();
  const host = analysis.rows[0].host;
  const engines = [...new Set(analysis.rows.map((row) => row.engine))].sort();
  const workloads = [...new Set(analysis.rows.map((row) => row.workload))].sort();
  const cohort = {
    schema_version: 1,
    schema: COHORT_SCHEMA,
    cohort_id: config.cohortId,
    created_at: new Date().toISOString(),
    commit: config.commit,
    host,
    evidence: config.evidence,
    analysis_sha256: sha256File(config.analysis),
    raw_sample_count: analysis.raw_sample_count,
    ...(analysis.schema_version === 2 ? { warmup_sample_count: analysis.warmup_sample_count,
      control_sample_count: analysis.control_sample_count } : {}),
    method: analysis.method,
    detector_coverage: analysis.detector_coverage,
    rows: analysis.rows,
  };
  const historyText = `${history.map((record) => JSON.stringify(record)).join("\n")}${
    history.length === 0 ? "" : "\n"
  }${JSON.stringify(cohort)}\n`;
  const current = {
    schema_version: 1,
    schema: CURRENT_SCHEMA,
    status: "ready",
    metadata: {
      description: "Latest coherent dedicated-Linux cross-engine memory cohort.",
      cohort_id: config.cohortId,
      commit: config.commit,
      host,
      evidence: config.evidence,
      history: path.relative(path.dirname(config.current), config.history),
      targets,
      engines,
      workloads,
      row_count: analysis.rows.length,
      raw_sample_count: analysis.raw_sample_count,
      ...(analysis.schema_version === 2 ? { warmup_sample_count: analysis.warmup_sample_count,
        control_sample_count: analysis.control_sample_count } : {}),
      method: analysis.method,
    },
    detector_coverage: analysis.detector_coverage,
    rows: analysis.rows,
  };
  writeAtomic(config.history, historyText);
  try {
    writeAtomic(config.current, `${JSON.stringify(current, null, 2)}\n`);
  } catch (error) {
    writeAtomic(config.history, `${history.map((record) => JSON.stringify(record)).join("\n")}${
      history.length === 0 ? "" : "\n"
    }`);
    throw error;
  }
  return current;
}

function selfTest() {
  const temporary = fs.mkdtempSync(path.join(require("os").tmpdir(), "rustia-memory-publish-"));
  try {
    const analysisPath = path.join(temporary, "analysis.json");
    const historyPath = path.join(temporary, "runs.jsonl");
    const currentPath = path.join(temporary, "current.json");
    const commit = "a".repeat(40);
    fs.writeFileSync(analysisPath, `${JSON.stringify({
      schema_version: 1,
      schema: ANALYSIS_SCHEMA,
      raw_sample_count: 9,
      host: "fixture-host",
      method: { fixture: true },
      detector_coverage: [{ engine: "rustia", detector_validated: true }],
      rows: [{
        commit,
        target: "linux-x86_64",
        host: "fixture-host",
        engine: "rustia",
        execution_mode: "interpreter",
        workload: "memory_churn",
        n: 1,
        epochs: 40,
        classification: "stable_in_tested_window",
      }],
    })}\n`);
    const current = publish({
      analysis: analysisPath,
      cohortId: "fixture",
      evidence: "bench/memory/raw/fixture",
      history: historyPath,
      current: currentPath,
      commit,
    });
    if (current.status !== "ready" || parseHistory(historyPath).length !== 1) {
      fail("publish self-test produced the wrong artifacts");
    }
    if (current.metadata.host !== "fixture-host") fail("publication did not record its host");
    const analysis = JSON.parse(fs.readFileSync(analysisPath, "utf8"));
    // A row that cannot name its machine, a cohort assembled from two, and an
    // analysis whose own host label disagrees with its rows are all refused --
    // the memory table takes each engine's rows from the newest cohort that
    // measured it, so an unlabelled or mislabelled row here is a row that can
    // outlive the machine it came from.
    const strays = [
      ["unlabelled row", { rows: [{ ...analysis.rows[0], host: undefined }] }, /analysis row host is missing/],
      [
        "two machines",
        { rows: [analysis.rows[0], { ...analysis.rows[0], workload: "memory_burst", host: "other-host" }] },
        /name 2 hosts/,
      ],
      ["analysis host disagrees with its rows", { host: "other-host" }, /names host "other-host" but its rows name "fixture-host"/],
      ["unlabelled analysis", { host: undefined }, /^analysis host is missing$/],
    ];
    // A label, never an address: the rows and the analysis's own host are what
    // the history line and the memory table name, and both are pushed to the
    // public data repository. Each is refused as an address, not as a
    // disagreement with the other.
    const literal = (text) => new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
    for (const address of ["bench.example.com", "10.0.0.5", "fe80::1", "user@box"]) {
      strays.push(
        [
          `rows naming ${address}`,
          { host: address, rows: [{ ...analysis.rows[0], host: address }] },
          literal(`analysis row rustia/memory_churn names its machine ${JSON.stringify(address)}, which is not a label`),
        ],
        [
          `analysis naming ${address}`,
          { host: address },
          literal(`analysis names host ${JSON.stringify(address)}, which is not a label`),
        ],
      );
    }
    for (const [name, override, expected] of strays) {
      const strayPath = path.join(temporary, `stray-${name.replace(/[^a-z]+/g, "-")}.json`);
      fs.writeFileSync(strayPath, `${JSON.stringify({ ...analysis, ...override })}\n`);
      let message = null;
      try {
        publish({
          analysis: strayPath,
          cohortId: "stray",
          evidence: "bench/memory/raw/stray",
          history: path.join(temporary, "stray-runs.jsonl"),
          current: path.join(temporary, "stray-current.json"),
          commit,
        });
      } catch (error) {
        message = error.message;
      }
      if (message == null || !expected.test(message)) {
        fail(`publication did not refuse ${name} as expected (got ${JSON.stringify(message)})`);
      }
    }
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
  process.stdout.write("memory publication self-test passed\n");
}

function main() {
  const config = parseArgs(process.argv.slice(2));
  if (config.command === "self-test") return selfTest();
  const current = publish(config);
  process.stdout.write(
    `published ${current.metadata.row_count} memory rows from cohort ${current.metadata.cohort_id}\n`,
  );
}

try {
  main();
} catch (error) {
  process.stderr.write(`publish-memory-analysis: ${error.message}\n`);
  process.exitCode = 1;
}

