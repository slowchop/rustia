#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { readResults, parseResults, validateResults, renderSection, median, duration, escape } = require("../bench/grabby-builds/render.js");
const root = path.resolve(__dirname, "..");
const data = readResults();
const canonical = `${JSON.stringify(data, null, 2)}\n`;
assert.deepEqual(parseResults(canonical), data);
assert.throws(() => parseResults(canonical.replace("{", '{"schema":"private-sentinel",')));
assert.throws(() => parseResults(canonical.replace('"label":', '"label":"private-sentinel","label":')));
assert.throws(() => parseResults(`${canonical}\n`));
const clone = () => JSON.parse(JSON.stringify(data));
function rejects(change) {
  const invalid = clone();
  change(invalid);
  assert.throws(() => validateResults(invalid));
}

assert.equal(median([4, 1, 3, 2]), 2.5);
assert.equal(duration(1917.26), "31m 57s");
assert.equal(escape('<a href="x">&\''), "&lt;a href=&quot;x&quot;&gt;&amp;&#39;");
assert.deepEqual(data.runs.map(run => run.measurement.run_id), [34283789035, 34303222573]);
assert.deepEqual(data.runs.map(run => [run.arms.control.clean_elapsed_s, run.arms[run.candidate].clean_elapsed_s]), [[3422.68, 2526.43], [2521.36, 1917.26]]);
assert.deepEqual(data.runs.map(run => [run.compile_comparison.paired_elapsed.control_median_s, run.compile_comparison.paired_elapsed.candidate_median_s]), [[147.055, 80.97999999999999], [82.125, 24.755000000000003]]);
rejects(d => { d.runs[0].adoption_approved = false; });
rejects(d => { d.runs[1].source_report.sha256 = "0".repeat(64); });
rejects(d => { d.runs[1].measurement.head = "a".repeat(40); });
rejects(d => { d.runs[1].delivery.final_ci_conclusion = "failure"; });
rejects(d => { d.runs.reverse(); });
rejects(d => { d.runs[0].arms.control.edit_elapsed_s.pop(); });
rejects(d => { d.runs[0].arms.control.edit_elapsed_s[0] = Infinity; });
rejects(d => { d.runs[0].arms.control.clean_elapsed_s = 0; });
rejects(d => { d.runs[1].compile_comparison.paired_elapsed.median_paired_saved_s += 1; });
rejects(d => { d.runs[1].runtime_comparisons[0].pairs[0].sample = 2; });
rejects(d => { d.runs[1].runtime_comparisons[0].pairs[0].delta += 1; });
rejects(d => { d.runs[1].runtime_comparisons[0].median_paired_change_pct = NaN; });
rejects(d => { d.runs[1].runtime_comparisons[1] = d.runs[1].runtime_comparisons[0]; });
rejects(d => { d.runs[0].label = '<script>alert(1)</script>'; });
rejects(d => { d.runs[0].source_report.path = 'javascript:alert(1)'; });
// Extra private fields must fail validation before the JSON is publicly copied.
rejects(d => { d.private_log = "not public"; });
rejects(d => { d.runs[0].diagnostics = { load: "not public" }; });
rejects(d => { d.runs[0].arms.control.swap_in_pages = 1; });
rejects(d => { d.runs[0].delivery.user_quote = "not public"; });

const html = renderSection(data);
for (const text of ["57m 03s", "42m 06s", "42m 01s", "31m 57s", "147.06s", "80.98s", "82.13s", "24.76s", "45.39%", "67.44%", "6.84% (+0.301 ms)", "Runtime is not unchanged", "not one three-arm", "not be multiplied", "release settings are unchanged", "seven-day retention", "not aggregate concurrent memory", "not physical-GPU capacity"]) {
  assert.ok(html.includes(text), `missing result or limit: ${text}`);
}
assert.ok(html.includes('href="grabby-builds.json"'));
assert.equal((html.match(/id="grabby-builds"/g) || []).length, 1);
assert.equal(renderSection(clone()), html);

const publisher = fs.readFileSync(path.join(root, ".github/workflows/publish-benchmark-data.yml"), "utf8");
const memory = fs.readFileSync(path.join(root, ".github/workflows/rustia-memory.yml"), "utf8");
assert.ok(publisher.includes('install -m 644 bench/grabby-builds/results.json "$pages_dir/grabby-builds.json"'));
assert.ok(publisher.includes('git -C "$pages_dir" add index.html grabby-builds.json'));
assert.ok(publisher.indexOf("node tools/check-grabby-build-results.js") < publisher.indexOf("Publish data repository and Pages"));
assert.ok(publisher.includes("group: rustia-benchmark-data-publication"));
assert.ok(publisher.includes("ref: main"));
for (const value of ["bench/history/series/ci-bench-main.jsonl", "bench/memory/current-results.json", "bench/memory/history/runs.jsonl"]) {
  assert.ok(publisher.includes(`\"$data_dir/${value}\"`));
}
assert.ok(memory.includes("needs: benchmark\n    uses: ./.github/workflows/publish-benchmark-data.yml"));
assert.ok(memory.includes('git -C "$data_dir" add bench/memory\n'));
assert.ok(!memory.includes("gh-pages"));
assert.ok(!memory.includes("render-current-results"));
assert.ok(!memory.includes("bench/current-results.html"));
assert.ok(memory.includes('git checkout --detach "$BENCHMARK_COMMIT"'));
assert.ok(fs.readFileSync(path.join(root, "tools/run-release-preflight.js"), "utf8").includes('node("tools/check-grabby-build-results.js")'));

// Exercise the actual workflow-policy checker against in-memory mutations. No
// repository, workflow, dataset or historical memory result is modified.
const policySource = fs.readFileSync(path.join(__dirname, "check-workflows.js"), "utf8");
function policyWithMutation(file, transform) {
  const mockFs = { ...fs, readFileSync(filename, ...args) {
    const result = fs.readFileSync(filename, ...args);
    return String(filename).endsWith(file) ? transform(result) : result;
  } };
  vm.runInNewContext(policySource, {
    __dirname,
    require: name => name === "fs" ? mockFs : require(name),
    console: { log() {}, error() {} },
    process: { exit(code) { throw new Error(`policy rejected: ${code}`); } },
  }, { timeout: 5000 });
}
policyWithMutation("not-a-file", value => value);
for (const [file, transform] of [
  ["rustia-memory.yml", text => text.replace("needs: benchmark", "needs: missing")],
  ["rustia-memory.yml", text => text.replace("./.github/workflows/publish-benchmark-data.yml", "./.github/workflows/other.yml")],
  ["rustia-memory.yml", text => `${text}\n# gh-pages\n`],
  ["rustia-memory.yml", text => text.replace('add bench/memory\n', 'add bench/memory bench/current-results.html\n')],
  ["publish-benchmark-data.yml", text => text.replace("          ref: main", "          ref: historical")],
  ["publish-benchmark-data.yml", text => text.replace("group: rustia-benchmark-data-publication", "group: rustia-bench-host-serial")],
  ["publish-benchmark-data.yml", text => text.replace('node tools/check-grabby-build-results.js', 'echo skipped')],
  ["publish-benchmark-data.yml", text => text.replace('install -m 644 bench/grabby-builds/results.json "$pages_dir/grabby-builds.json"', 'echo skipped')],
]) assert.throws(() => policyWithMutation(file, transform), `policy must reject mutation of ${file}`);
console.log("Grabby selected results, paired statistics, privacy boundary and publication routing checks passed");
