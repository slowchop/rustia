"use strict";

// Synthetic counters and timestamps only: never launch a benchmark or read a
// host memory counter in this test suite.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { createHash } = require("node:crypto");
const { analyze, sampleMetrics, memoryAreaKibSeconds } = require("./analyze-memory-samples.js");
const { MEMORY_PROTOCOL, requireSingleProtocol, validateProtocolMetrics } = require("./memory-protocol.js");
const { memoryResultsSection, memoryComparisonCharts, memoryHistoryModel } = require("./render-current-results.js");
const root = path.resolve(__dirname, "..");
const kib = 1024;

function raw(lane, role, round) {
  const identity = { commit: "a".repeat(40), binary_sha256: "b".repeat(64),
    engine: "rustia", workload: "dense_tables", n: 1, epochs: 8, lane, role, round };
  const shapes = ["spawned", "prepared", "warmed", ...Array(8).fill("epoch"), "quiescent", "dropped"];
  const events = shapes.map((phase, index) => {
    const epoch = phase === "epoch" ? index - 2 : null;
    const elapsed = index <= 10 ? index * 10 : index === 11 ? 200 : 210;
    let anon = index === 0 ? 100 : index < 3 ? 105 : 105 + (lane === "workload" ? (epoch ?? 3) : 0);
    if (phase === "quiescent") anon = 105 + round * 2 + (lane === "workload" ? 20 : 0);
    const checksum = phase === "warmed" || phase === "epoch"
      ? `i64:${lane === "null-control" || phase === "warmed" ? 1 : epoch + 1}` : null;
    return {
      event: { ...identity, schema_version: 2, schema: "rustia.bench.memory-child-event.v2",
        null_control: lane === "null-control", phase, epoch, checksum,
        target: "linux-x86_64", dialect: "luau", execution_mode: "interpreter" },
      elapsed_ms: elapsed,
      process_memory: { smaps_rollup_kib: { anonymous: anon, pss: anon + 50, swap: 0 } },
      cgroup_memory: { current_bytes: (anon + 400 + index * 100) * kib,
        stat_bytes: { anon: anon * kib }, swap_current_bytes: 0 },
    };
  });
  return { schema_version: 2, schema: "rustia.bench.memory-raw.v2",
    sample_id: `${role}-${round}-${lane}`, identity,
    protocol: { quiescence_ms: 100, forced_collection: false, allocator_purge: false },
    platform: { resettable_memory_peak: false }, events,
    timeline: events.map(entry => ({ elapsed_ms: entry.elapsed_ms,
      anon_bytes: entry.cgroup_memory.stat_bytes.anon, current_bytes: entry.cgroup_memory.current_bytes })),
    final_cgroup: { peak_bytes: 4000 * kib, swap_current_bytes: 0,
      events: {}, pressure: { some: { total: 0 }, full: { total: 0 } } },
  };
}

function fixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-memory-v2-test-"));
  const inputs = [];
  for (let round = 0; round <= 9; round += 1) {
    for (const lane of ["workload", "null-control"]) {
      const data = raw(lane, round === 0 ? "warmup" : "measurement", round);
      const file = path.join(directory, `${data.sample_id}.json`);
      fs.writeFileSync(file, JSON.stringify(data));
      inputs.push(file);
    }
  }
  return { directory, config: { inputs: inputs.reverse(), minimumSamples: 9,
    bootstrapRepetitions: 100, bootstrapSeed: 7, absoluteThreshold: 4,
    proportionalThreshold: 0.05, host: "fixture-host" } };
}

function cli(script, args) {
  return spawnSync(process.execPath, [path.join(root, script), ...args], { cwd: root, encoding: "utf8" });
}

function stats(value) {
  return { sample_count: 9, min: value, q1: value, median: value, q3: value, max: value };
}

test("anonymous area excludes file/kernel charge and integrates exact endpoints", () => {
  const timeline = [
    { elapsed_ms: 0, anon_bytes: 100 * kib, current_bytes: 1000 * kib },
    { elapsed_ms: 20, anon_bytes: 102 * kib, current_bytes: 4000 * kib },
    { elapsed_ms: 50, anon_bytes: 106 * kib, current_bytes: 10000 * kib },
    { elapsed_ms: 100, anon_bytes: 100 * kib, current_bytes: 40000 * kib },
  ];
  assert(Math.abs(memoryAreaKibSeconds(timeline, 100 * kib, 0, 100, "anon_bytes") - 0.29) < 1e-12);
  assert(memoryAreaKibSeconds(timeline, 1000 * kib, 0, 100) > 1000);
});

test("same-round controls are subtracted before summary; warmups are excluded", () => {
  const { directory, config } = fixture();
  try {
    const result = analyze(config);
    assert.equal(result.schema_version, 2);
    assert.equal(result.raw_sample_count, 20);
    assert.equal(result.warmup_sample_count, 2);
    assert.equal(result.control_sample_count, 9);
    const row = result.rows[0];
    assert.equal(row.sample_ids.length, 9);
    assert(row.sample_ids.every(id => !id.includes("warmup")));
    assert.equal(row.metrics.quiescent_retained_delta_anon_kib.median, 30);
    assert.equal(row.metrics.null_retained_delta_anon_kib.median, 10);
    assert.deepEqual(row.metrics.control_adjusted_retained_delta_anon_kib, stats(20));
    assert.notEqual(row.metrics.memory_area_kib_seconds_per_unit.median,
      row.metrics.anonymous_memory_area_kib_seconds_per_unit.median);
    validateProtocolMetrics(row);
    const negative = { ...row, metrics: { ...row.metrics, control_adjusted_retained_delta_anon_kib: stats(-2) } };
    validateProtocolMetrics(negative);
    const html = memoryResultsSection({ status: "ready", rows: [negative], detector_coverage: [] }, "fixture-host");
    assert.match(html, /subtract a matched no-work control/);
    assert.match(html, /-2(?:\.0)? KiB/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("v2 evidence refuses missing controls, warmups, duplicate rounds and mixed binaries", () => {
  for (const [name, mutate, pattern] of [
    ["missing control", config => { config.inputs = config.inputs.filter(file => !file.endsWith("measurement-9-null-control.json")); }, /nine|counts differ/],
    ["missing warmup", config => { config.inputs = config.inputs.filter(file => !file.endsWith("warmup-0-workload.json")); }, /separate warmup/],
    ["duplicate file", config => { config.inputs.push(config.inputs[0]); }, /duplicate memory sample identity/],
    ["duplicate round", config => {
      const file = config.inputs.find(file => file.endsWith("measurement-9-null-control.json"));
      const data = JSON.parse(fs.readFileSync(file)); data.identity.round = 8; fs.writeFileSync(file, JSON.stringify(data));
    }, /duplicate or missing/],
    ["binary mismatch", config => {
      const file = config.inputs[0]; const data = JSON.parse(fs.readFileSync(file)); data.identity.binary_sha256 = "c".repeat(64); fs.writeFileSync(file, JSON.stringify(data));
    }, /different binaries/],
    ["mixed protocols", config => {
      const file = config.inputs[0]; const data = JSON.parse(fs.readFileSync(file)); data.schema_version = 1; data.schema = "rustia.bench.memory-raw.v1"; fs.writeFileSync(file, JSON.stringify(data));
    }, /cannot mix legacy/],
  ]) {
    const { directory, config } = fixture();
    try { mutate(config); assert.throws(() => analyze(config), pattern, name); }
    finally { fs.rmSync(directory, { recursive: true, force: true }); }
  }
});

test("short quiescence, missing anonymous counters, reversed time and wrong lanes fail", () => {
  for (const [mutate, pattern] of [
    [data => { data.events[11].elapsed_ms = 199; }, /quiescence interval/],
    [data => { delete data.timeline[5].anon_bytes; }, /anonymous bytes/],
    [data => { data.timeline.reverse(); }, /unordered timeline/],
    [data => { data.timeline.splice(2, 1); }, /exact phase timeline endpoint/],
    [data => { data.events[5].event.null_control = true; }, /lane\/schema/],
    [data => { data.final_cgroup.events.oom = 1; }, /contaminated/],
  ]) {
    const data = raw("workload", "measurement", 1); mutate(data);
    assert.throws(() => sampleMetrics(data, "fixture"), pattern);
  }
});

test("legacy analysis is unchanged by v2 support", () => {
  const { directory, config } = fixture();
  try {
    const legacy = raw("workload", "measurement", 1);
    legacy.schema_version = 1; legacy.schema = "rustia.bench.memory-raw.v1";
    const input = path.join(directory, "legacy.json"); fs.writeFileSync(input, JSON.stringify(legacy));
    const actual = analyze({ ...config, minimumSamples: 1, inputs: [input] });
    delete actual.generated_at;
    // Golden output verified against the v1 analyzer at dfd8c18b88e7c53d6bfbb19539419f409af4b657.
    // Keep the test independent of Git history so shallow checkouts can run it.
    assert.equal(createHash("sha256").update(JSON.stringify(actual)).digest("hex"),
      "c41ded6267f0091b69d2a3e60bfb09da05ebe26aa52ad869b03030a9bb156846");
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("publication preserves v1 history and mixed-version comparison is refused", () => {
  const { directory, config } = fixture();
  try {
    const analysis = analyze(config);
    const analysisFile = path.join(directory, "analysis.json");
    const history = path.join(directory, "runs.jsonl");
    const current = path.join(directory, "current.json");
    const legacy = { ...analysis.rows[0], metrics: { ...analysis.rows[0].metrics }, engine: "koto", measurement_protocol: undefined };
    delete legacy.metrics.control_adjusted_retained_delta_anon_kib;
    delete legacy.metrics.null_retained_delta_anon_kib;
    delete legacy.metrics.anonymous_memory_area_kib_seconds_per_unit;
    const prior = { schema_version: 1, schema: "rustia.bench.memory-cohort.v1", cohort_id: "legacy",
      commit: "d".repeat(40), host: "fixture-host", rows: [legacy] };
    const priorBytes = JSON.stringify(prior) + "\n";
    fs.writeFileSync(history, priorBytes); fs.writeFileSync(analysisFile, JSON.stringify(analysis));
    const result = cli("bench/publish-memory-analysis.js", ["--analysis", analysisFile,
      "--cohort-id", "corrected", "--evidence", "bench/memory/raw/test", "--history", history,
      "--current", current, "--commit", "a".repeat(40)]);
    assert.equal(result.status, 0, result.stderr);
    assert(fs.readFileSync(history, "utf8").startsWith(priorBytes));
    assert.throws(() => requireSingleProtocol([legacy, analysis.rows[0]], "fixture"), /mixes memory/);
    assert.throws(() => memoryComparisonCharts([legacy, analysis.rows[0]]), /mixes memory/);
    const storedRoot = path.join(directory, "data");
    fs.mkdirSync(path.join(storedRoot, "bench/memory/history"), { recursive: true });
    fs.writeFileSync(path.join(storedRoot, "bench/memory/current-results.json"), JSON.stringify({
      schema_version: 1, schema: "rustia.bench.memory-current-results.v1", status: "ready",
      metadata: {}, rows: [legacy], detector_coverage: [],
    }));
    fs.writeFileSync(path.join(storedRoot, "bench/memory/history/runs.jsonl"), priorBytes);
    const merged = cli("tools/merge-rustia-ci-memory-data.js", ["--generated-current", current,
      "--generated-history", history, "--data-root", storedRoot]);
    assert.notEqual(merged.status, 0);
    assert.match(merged.stderr, /mixes memory measurement protocols/);
    assert.equal(fs.readFileSync(path.join(storedRoot, "bench/memory/history/runs.jsonl"), "utf8"), priorBytes);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("paired subtraction precedes the median instead of subtracting group medians", () => {
  const { directory, config } = fixture();
  try {
    for (const file of config.inputs) {
      const data = JSON.parse(fs.readFileSync(file));
      if (data.identity.role === "warmup") continue;
      const index = (data.identity.round - 1) % 3;
      const delta = data.identity.lane === "workload" ? [2, 20, 12][index] : [0, 15, 1][index];
      const endpoint = data.events.find(entry => entry.event.phase === "quiescent");
      endpoint.process_memory.smaps_rollup_kib.anonymous = 105 + delta;
      endpoint.cgroup_memory.stat_bytes.anon = (105 + delta) * kib;
      const point = data.timeline.find(point => point.elapsed_ms === endpoint.elapsed_ms);
      point.anon_bytes = endpoint.cgroup_memory.stat_bytes.anon;
      fs.writeFileSync(file, JSON.stringify(data));
    }
    const row = analyze(config).rows[0];
    assert.equal(row.metrics.control_adjusted_retained_delta_anon_kib.median, 5);
    assert.equal(row.metrics.quiescent_retained_delta_anon_kib.median - row.metrics.null_retained_delta_anon_kib.median, 11);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});

test("independent evidence validation replays v2 and rejects invalid warmup/source records", async () => {
  const { directory, config } = fixture();
  try {
    const { validateCrossEngineMemoryEvidence } = require("../tools/run-linode-benchmark.js");
    const hash = file => require("node:crypto").createHash("sha256").update(fs.readFileSync(file)).digest("hex");
    const evidence = path.join(directory, "evidence");
    fs.mkdirSync(path.join(evidence, "raw"), { recursive: true });
    for (const input of config.inputs) fs.copyFileSync(input, path.join(evidence, "raw", path.basename(input)));
    const protocol = { schema: "rustia.bench.memory-protocol.v2", warmup_rounds: 1, quiescence_ms: 100,
      cohort_id: "fixture", commit: "a".repeat(40), samples: 9, bootstrap_repetitions: 100,
      bootstrap_seed: 7, growth_thresholds: { absolute_kib_per_epoch: 4, proportional_percent_per_epoch: 0.05 },
      host: { name: "fixture-host", benchmark_cpu: 3, supervisor_cpu: 2, sample_interval_ms: 10, timeout_ms: 600000 },
      groups: [{ binary: "all", engines: ["rustia"], workloads: [{ id: "dense_tables", n: 1, epochs: 8 }] }] };
    const write = (name, value) => fs.writeFileSync(path.join(evidence, name), JSON.stringify(value));
    write("protocol.json", protocol);
    write("analysis.json", analyze(config));
    write("host.json", {}); write("run-gate.json", {});
    write("build.json", { all_binary_sha256: "b".repeat(64), lua54_binary_sha256: "b".repeat(64) });
    const rawFiles = fs.readdirSync(path.join(evidence, "raw")).sort();
    const order = rawFiles.map(file => {
      const raw = JSON.parse(fs.readFileSync(path.join(evidence, "raw", file)));
      return { ...raw.identity, sample_id: raw.sample_id, binary_role: "all", sha256: hash(path.join(evidence, "raw", file)) };
    }).sort((a, b) => a.round - b.round);
    write("run-order.json", order);
    const manifest = { schema_version: 2, schema: "rustia.bench.memory-cohort-evidence.v2",
      commit: protocol.commit, cohort_id: "fixture", case_count: 1, samples_per_case: 9,
      warmup_sample_count: 2, control_sample_count: 9, raw_sample_count: 20,
      protocol_sha256: hash(path.join(evidence, "protocol.json")),
      supervisor_sha256: hash(path.join(root, "bench/run-memory-supervisor.js")),
      analyzer_sha256: hash(path.join(root, "bench/analyze-memory-samples.js")),
      measurement_protocol_sha256: hash(path.join(root, "bench/memory-protocol.js")),
      binaries: { all_sha256: "b".repeat(64), lua54_sha256: "b".repeat(64) },
      evidence_sha256: { raw: Object.fromEntries(rawFiles.map(file => [file, hash(path.join(evidence, "raw", file))])) } };
    write("manifest.json", manifest);
    const frozen = { schema: "rustia.dedicated-memory-benchmark.v1", provider: { host: { name: "fixture-host" } },
      source: { benchmark_commit: protocol.commit, expected_sha256: { memory_supervisor: manifest.supervisor_sha256,
        memory_analyzer: manifest.analyzer_sha256, memory_protocol: manifest.measurement_protocol_sha256 } },
      measurement: { ...protocol, protocol: MEMORY_PROTOCOL, sample_interval_ms: 10, sample_timeout_ms: 600000 },
      host_gate: { benchmark_cpu: 3, supervisor_cpu: 2 }, publication: { rows: 1 } };
    await validateCrossEngineMemoryEvidence(frozen, evidence);
    write("manifest.json", { ...manifest, warmup_sample_count: 0 });
    await assert.rejects(validateCrossEngineMemoryEvidence(frozen, evidence), /warmup\/control\/protocol identity/);
    write("manifest.json", manifest);
    write("run-order.json", [...order].reverse());
    await assert.rejects(validateCrossEngineMemoryEvidence(frozen, evidence), /warmup processes/);
    write("run-order.json", order);
    write("build.json", { all_binary_sha256: "c".repeat(64), lua54_binary_sha256: "b".repeat(64) });
    await assert.rejects(validateCrossEngineMemoryEvidence(frozen, evidence), /binary hashes differ/);
  } finally { fs.rmSync(directory, { recursive: true, force: true }); }
});
