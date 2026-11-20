"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

// Immutable provenance of the independently reviewed source reports. This
// selected public dataset is NOT a copy of the reports or their private logs.
const SOURCES = [
  ["lto-off", 34283789035, "46410aadddbf6d7ff08b24f0d7c49b4522d44595",
    "52a54f85bcf438675c1ccbc2ba68d3da91687a5086b60cd627ad567870bc2dc1",
    1024, 34299717714, "70f006989877149ccfdb38e9e9677dadda17092c"],
  ["dev-quick", 34303222573, "24205bdcbbb306f434e3daf41078dfc00b49f5cb",
    "b477c32320d399ffcf9e9f0f5cddbdb5a7fa0b262e800e2c0ec9b5adc563eaa3",
    1026, 34311717985, "8483e0bd7af73d109a1294bf9d28195076bdbe4a"],
];
const LABELS = ["Disable development LTO", "Opt-in reduced debugging"];
const SHA = /^[0-9a-f]{40}$/;
const DIGEST = /^[0-9a-f]{64}$/;
const REPO = "https://github.com/slowchop/grabby-next";

function keys(object, allowed) {
  assert.deepEqual(Object.keys(object).sort(), [...allowed].sort(), "only selected public fields are allowed");
}

function positive(value) {
  assert.ok(Number.isFinite(value) && value > 0, "timings must be finite and positive");
}

function median(values) {
  assert.ok(values.length > 0);
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

function near(actual, expected) {
  assert.ok(Number.isFinite(actual) && Number.isFinite(expected));
  assert.ok(Math.abs(actual - expected) <= Math.max(1, Math.abs(expected)) * 1e-10,
    `inconsistent derived statistic: ${actual} != ${expected}`);
}

function validatePairedCompile(summary, control, candidate) {
  keys(summary, ["control_median_s", "candidate_median_s", "median_paired_saved_s", "median_paired_saved_pct", "median_paired_ratio", "candidate_wins"]);
  assert.equal(control.length, 4);
  assert.equal(candidate.length, 4);
  [...control, ...candidate].forEach(positive);
  near(summary.control_median_s, median(control));
  near(summary.candidate_median_s, median(candidate));
  near(summary.median_paired_saved_s, median(control.map((a, i) => a - candidate[i])));
  near(summary.median_paired_saved_pct, median(control.map((a, i) => 100 * (1 - candidate[i] / a))));
  near(summary.median_paired_ratio, median(control.map((a, i) => a / candidate[i])));
  assert.equal(summary.candidate_wins, control.filter((a, i) => candidate[i] < a).length);
}

function validateResults(data) {
  keys(data, ["schema", "measured_date", "host", "protocol", "runs"]);
  assert.equal(data.schema, "grabby.build-times.v1");
  assert.equal(data.measured_date, "2026-09-09");
  assert.deepEqual(data.host, { name: "ci-bench", runner: "bench-solo", cpu: "AMD Ryzen 5 3600X", isolated_cpus: [4, 5], rust: "1.95.0" });
  assert.deepEqual(data.protocol, { cold_samples_per_arm: 1, edit_pairs: 4, edit_order: ["AB", "BA", "BA", "AB"], simulation_warmup_ticks: 200, simulation_measured_ticks: 2000, frame_warmup: 120, frame_samples: 300 });
  assert.equal(data.runs.length, 2, "two separate sequential experiments are required");
  data.runs.forEach((run, index) => {
    keys(run, ["label", "candidate", "adoption_approved", "source_report", "measurement", "delivery", "arms", "compile_comparison", "runtime_comparisons"]);
    keys(run.source_report, ["repository", "commit", "path", "sha256"]);
    keys(run.measurement, ["head", "run_id", "conclusion", "zip_sha256", "aggregate_log_sha256"]);
    keys(run.delivery, ["pr", "final_head", "final_ci_run", "final_ci_conclusion", "merged_at", "merge_commit", ...(index === 1 ? ["status"] : [])]);
    if (index === 1) assert.equal(run.delivery.status, "Merged after successful final-head CI and independent reviews");
    const [candidate, runId, head, reportHash, pr, ci, merge] = SOURCES[index];
    assert.equal(run.label, LABELS[index]);
    assert.equal(run.candidate, candidate);
    assert.equal(run.adoption_approved, true);
    assert.equal(run.measurement.conclusion, "success");
    assert.equal(run.measurement.run_id, runId);
    assert.equal(run.measurement.head, head);
    assert.match(run.measurement.zip_sha256, DIGEST);
    assert.match(run.measurement.aggregate_log_sha256, DIGEST);
    assert.equal(run.source_report.repository, "slowchop/grabby-next");
    assert.equal(run.source_report.commit, "f28f4428a6bb9f6b7a09551211840a777f8c686f");
    assert.equal(run.source_report.path, `design/build-speed-${index === 0 ? "adoption" : "quick"}-2026-09-09.json`);
    assert.equal(run.source_report.sha256, reportHash);
    assert.equal(run.delivery.pr, pr);
    assert.equal(run.delivery.final_ci_run, ci);
    assert.equal(run.delivery.final_ci_conclusion, "success");
    assert.equal(run.delivery.merge_commit, merge);
    assert.match(run.delivery.final_head, SHA);
    assert.match(run.delivery.merged_at, /^2026-09-09T\d{2}:\d{2}:\d{2}Z$/);
    assert.ok(Number.isFinite(Date.parse(run.delivery.merged_at)));
    assert.deepEqual(Object.keys(run.arms).sort(), ["control", candidate].sort());
    for (const arm of Object.values(run.arms)) {
      keys(arm, ["clean_elapsed_s", "clean_cpu_s", "clean_max_rss_kib", "clean_target_kib", "edit_elapsed_s", "edit_cpu_s"]);
      for (const key of ["clean_elapsed_s", "clean_cpu_s", "clean_max_rss_kib", "clean_target_kib"]) positive(arm[key]);
    }
    assert.equal(run.compile_comparison.candidate, candidate);
    keys(run.compile_comparison, ["candidate", "pairs", "paired_elapsed", "paired_cpu"]);
    assert.equal(run.compile_comparison.pairs, 4);
    validatePairedCompile(run.compile_comparison.paired_elapsed, run.arms.control.edit_elapsed_s, run.arms[candidate].edit_elapsed_s);
    validatePairedCompile(run.compile_comparison.paired_cpu, run.arms.control.edit_cpu_s, run.arms[candidate].edit_cpu_s);
    const expectedMetrics = ["crash-landing.luau", "earth-stations.luau"].flatMap(scene =>
      ["tick_median_ns", "tick_p95_ns", "extract_median_ns", "extract_p95_ns"].map(metric => `${scene}:${metric}`))
      .concat(["median_frame_ms", "p95_frame_ms", "mean_frame_ms"].map(metric => `software-rendered frames:${metric}`));
    assert.deepEqual(run.runtime_comparisons.map(row => `${row.scene}:${row.metric}`).sort(), expectedMetrics.sort());
    for (const row of run.runtime_comparisons) {
      keys(row, ["scene", "metric", "pairs", "control_median", "candidate_median", "median_paired_delta", "median_paired_change_pct"]);
      assert.deepEqual(row.pairs.map(pair => pair.sample), [1, 2, 3, 4]);
      for (const pair of row.pairs) {
        keys(pair, ["control", "candidate", "sample", "delta", "change_pct"]);
        positive(pair.control); positive(pair.candidate);
        near(pair.delta, pair.candidate - pair.control);
        near(pair.change_pct, 100 * (pair.candidate / pair.control - 1));
      }
      near(row.control_median, median(row.pairs.map(pair => pair.control)));
      near(row.candidate_median, median(row.pairs.map(pair => pair.candidate)));
      near(row.median_paired_delta, median(row.pairs.map(pair => pair.delta)));
      near(row.median_paired_change_pct, median(row.pairs.map(pair => pair.change_pct)));
    }
  });
  assert.ok(Date.parse(data.runs[0].delivery.merged_at) < Date.parse(data.runs[1].delivery.merged_at));
  return data;
}

function parseResults(text) {
  const data = JSON.parse(text);
  // The publisher copies these bytes. JSON.parse alone could silently drop a
  // duplicate key containing private text while the original bytes go public.
  assert.equal(text, `${JSON.stringify(data, null, 2)}\n`, "public results must be canonical JSON without duplicate keys");
  return validateResults(data);
}

function readResults() {
  return parseResults(fs.readFileSync(path.join(__dirname, "results.json"), "utf8"));
}

function escape(value) {
  return String(value).replace(/[&<>"']/g, char => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]);
}
function duration(seconds) {
  const rounded = Math.round(seconds);
  return `${Math.floor(rounded / 60)}m ${String(rounded % 60).padStart(2, "0")}s`;
}
function number(value) { return value.toFixed(2); }
function signed(value) { return `${value >= 0 ? "+" : ""}${number(value)}`; }
function runtime(run, scene, metric) {
  return run.runtime_comparisons.find(row => row.scene === scene && row.metric === metric);
}

function renderSection(data, publicSource = false) {
  validateResults(data);
  const datasetUrl = publicSource
    ? "https://github.com/slowchop/rustia/blob/main/bench/grabby-builds/results.json"
    : "grabby-builds.json";
  const [lto, quick] = data.runs;
  const quickArm = quick.arms[quick.candidate];
  const tail = runtime(quick, "crash-landing.luau", "tick_p95_ns");
  return `<section class="panel grabby-builds" id="grabby-builds" aria-labelledby="grabby-builds-title">
    <div class="grabby-intro">
      <p class="eyebrow">Grabby application builds · ${escape(data.measured_date)}</p>
      <h2 id="grabby-builds-title">Grabby build times</h2>
      <p><strong>${duration(quickArm.clean_elapsed_s)} full compilation · ${number(median(quickArm.edit_elapsed_s))}s median incremental</strong> with <code>just dev-quick</code>.</p>
      <p>Two changes shipped in order. These are application build measurements, separate from Rustia engine benchmarks.</p>
    </div>
    <div class="table-wrap"><table>
      <caption>Separate before/after runs on ci-bench; lower times are better.</caption>
      <thead><tr><th scope="col">Change</th><th scope="col">Full build<br>before → after</th><th scope="col">Incremental median<br>before → after</th><th scope="col">Paired edit saving</th></tr></thead>
      <tbody>${data.runs.map((run, i) => {
        const a = run.arms.control, b = run.arms[run.candidate], paired = run.compile_comparison.paired_elapsed;
        return `<tr><th scope="row">${i + 1}. ${escape(run.label)}</th><td>${duration(a.clean_elapsed_s)} → <strong>${duration(b.clean_elapsed_s)}</strong></td><td>${number(median(a.edit_elapsed_s))}s → <strong>${number(median(b.edit_elapsed_s))}s</strong></td><td>${number(paired.median_paired_saved_pct)}%<br><span>${number(paired.median_paired_saved_s)}s · ${paired.candidate_wins}/4 wins</span></td></tr>`;
      }).join("")}</tbody>
    </table></div>
    <div class="grabby-notes">
      <p><code>just dev</code> keeps full debugging. <code>just dev-quick</code> omits compiler source/type/line debugging detail and uses a separate build cache. Optimization and runtime checks stay enabled; release settings are unchanged.</p>
      <p><strong>Development runtime tradeoff:</strong> disabling LTO increased median simulation tick time by ${number(runtime(lto, "crash-landing.luau", "tick_median_ns").median_paired_change_pct)}% in Crash Landing and ${number(runtime(lto, "earth-stations.luau", "tick_median_ns").median_paired_change_pct)}% in Earth Stations. Reduced debugging then increased Crash Landing’s p95 tick time by <strong>${number(tail.median_paired_change_pct)}% (+${(tail.median_paired_delta / 1e6).toFixed(3)} ms)</strong>, in all four pairs. Runtime is not unchanged.</p>
      <details><summary>Runtime detail and measurement limits</summary>
        <div class="table-wrap"><table><caption>Median paired runtime changes; positive is slower.</caption><thead><tr><th scope="col">Experiment</th><th scope="col">Scene</th><th scope="col">Tick median</th><th scope="col">Tick p95</th><th scope="col">Extraction median</th></tr></thead>
        <tbody>${data.runs.flatMap(run => ["crash-landing.luau", "earth-stations.luau"].map(scene => `<tr><th scope="row">${escape(run.label)}</th><td>${scene === "crash-landing.luau" ? "Crash Landing" : "Earth Stations"}</td>${["tick_median_ns", "tick_p95_ns", "extract_median_ns"].map(metric => `<td>${signed(runtime(run, scene, metric).median_paired_change_pct)}%</td>`).join("")}</tr>`)).join("")}</tbody></table></div>
        <ul>
          <li>Same Ryzen 5 3600X host, two isolated cores (4–5), Rust 1.95. Full builds use empty target directories, control first, one sample per arm—not cold filesystem caches or forecasts for other machines.</li>
          <li>Incremental results use four retained-source edit pairs, ordered AB/BA/BA/AB. Marginal medians and medians of paired savings differ. These are two sequential experiments, not one three-arm comparison; their paired speedups must not be multiplied.</li>
          <li>Each simulation scene used four paired processes, 200 warmup and 2000 measured ticks. Snapshots agreed; the recorded independent review found no candidate-only defect in ten software-rendered captures per experiment.</li>
          <li>Software-Vulkan frame results were mixed at roughly 2.6–2.9 FPS, not physical-GPU capacity. Capture elapsed includes deliberate waits and is not time-to-menu. Stationary captures do not establish large-world or broad gameplay performance.</li>
          <li>Host-global swap activity and variable wall time remain limitations. The linked data labels GNU-time maximum process/child RSS, not aggregate concurrent memory; target footprints are per arm. Keeping both profiles consumes combined disk space.</li>
        </ul>
      </details>
      <p class="grabby-evidence"><a href="${datasetUrl}">Selected results and paired samples (JSON)</a> · Private Grabby evidence · ${data.runs.map((run, i) => `<a href="${REPO}/actions/runs/${run.measurement.run_id}">Experiment ${i + 1} run</a> · <a href="${REPO}/pull/${run.delivery.pr}">Merged PR ${run.delivery.pr}</a> · <a href="${REPO}/blob/${run.source_report.commit}/${run.source_report.path}">Reviewed report ${i + 1}</a>`).join(" · ")}</p>
      <p>Uploaded raw artifacts have seven-day retention. This page contains selected results, not private logs. Consistency checks do not repeat the recorded raw/compiler/screenshot audits.</p>
    </div>
  </section>`;
}

module.exports = { readResults, parseResults, validateResults, renderSection, median, duration, escape };
