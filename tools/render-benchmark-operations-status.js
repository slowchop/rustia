#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");
const { resolveHistoricalCommit } = require("./history-projection.js");

const root = path.resolve(__dirname, "..");
const policyPath = path.join(root, "bench", "operations-policy.json");
const resultsPath = path.join(root, "bench", "current-results.json");
const outputPath = path.join(root, "bench", "operations-status.json");
const publicProfilePath = path.join(root, "design", "public-source-profile.json");
const checkOnly = process.argv.includes("--check");
const PUBLIC_BRIDGE_STATES = Object.freeze([
  "pending-private-trusted-pipeline",
  "private-maintainer-cohorts-v1",
]);
const PUBLIC_MEASUREMENT_MODES = Object.freeze([
  "paused-pending-private-bridge",
  "public-automation-paused-private-manual-cohorts",
]);
// A runner class that names its machine names it by a provenance label. The
// timing authority's is the value every published table is pinned to, and this
// policy is rendered verbatim into bench/operations-status.json, which is
// mirrored into the public data repository. A label, never an address: no dots
// and no colons, so it can be neither a fully qualified name nor an IPv4 or IPv6
// address. Every emitter and validator carries this same literal; the self-test
// in tools/check-benchmark-results.js asserts it.
const HOST_LABEL = /^[A-Za-z0-9_-]{1,128}$/;

// Thrown, not exited: selfTest() hands validatePolicy policies it must refuse,
// and it needs to see the refusal to assert on it. The catch at the bottom of
// the file turns any refusal into the exit status.
function fail(message) {
  throw new Error(message);
}

function relative(filePath) {
  return path.relative(root, filePath).split(path.sep).join("/");
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`could not read ${relative(filePath)}: ${error.message}`);
  }
}

function git(args) {
  try {
    return childProcess.execFileSync("git", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    }).trim();
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`git ${args.join(" ")} failed${stderr ? `: ${stderr}` : ""}`);
  }
}

function requireString(value, label) {
  if (typeof value !== "string" || value.trim() === "") fail(`${label} must be a non-empty string`);
}

function requireHostLabel(value, label) {
  if (typeof value !== "string" || !HOST_LABEL.test(value)) {
    fail(
      `${label} names its machine ${JSON.stringify(value)}, which is not a label: ` +
      "a label may contain only [A-Za-z0-9_-], never an address",
    );
  }
}

// The v1 schema had a single `runner`, which could only say "runners are not
// timing authorities". That was true of the shared pool and became false the
// moment an owned, qualified machine started producing every published number.
// v2 splits runners into classes so the guarantee can be stated where it holds
// rather than weakened everywhere, and the rules below make each half
// mechanical instead of prose in design/81.
function validateRunnerClasses(policy) {
  const classes = policy.runner_classes;
  if (classes === null || typeof classes !== "object" || Array.isArray(classes)) {
    fail("runner_classes must be an object keyed by class name");
  }
  const shared = classes["general-linux"];
  if (!shared || shared.environment !== "self-hosted-shared" || shared.timing_authority !== false) {
    // The original v1 assertion, narrowed to the class it was always about.
    fail("the general-linux class must remain shared and non-authoritative for timing");
  }
  const authoritative = [];
  for (const [name, runner] of Object.entries(classes)) {
    if (!Array.isArray(runner.allowed_roles) || runner.allowed_roles.length === 0) {
      fail(`runner_classes.${name}.allowed_roles must be a non-empty array`);
    }
    // The timing authority must name its machine, because every published
    // table is pinned to that name; any other class may. Whatever a class names
    // is published, so it is a label.
    if (runner.timing_authority === true) requireString(runner.host, `runner_classes.${name}.host`);
    if (runner.host != null) requireHostLabel(runner.host, `runner_classes.${name}`);
    if (runner.timing_authority !== true) {
      // A class that cannot be trusted with a timing may not be assigned one.
      // design/81 said this in prose; here it is a rule.
      const measuring = runner.allowed_roles.filter((role) => role.startsWith("measurement-"));
      if (measuring.length > 0) {
        fail(
          `runner_classes.${name} is not a timing authority but claims ${measuring.join(", ")}`,
        );
      }
      continue;
    }
    authoritative.push(name);
    const q = runner.qualification;
    if (q === null || typeof q !== "object") fail(`runner_classes.${name}.qualification must be an object`);
    if (!Array.isArray(q.isolated_cpus) || q.isolated_cpus.length === 0) {
      fail(`runner_classes.${name}.qualification.isolated_cpus must be a non-empty array`);
    }
    if (!Number.isSafeInteger(q.pinned_cpu) || !q.isolated_cpus.includes(q.pinned_cpu)) {
      // A pinned core the kernel still schedules on is the failure the whole
      // arrangement exists to prevent, and it is invisible in the timings.
      fail(`runner_classes.${name}.qualification.pinned_cpu must be one of its isolated CPUs`);
    }
    if (q.smt !== "disabled") fail(`runner_classes.${name}.qualification.smt must be "disabled"`);
    if (q.concurrent_jobs !== 1) fail(`runner_classes.${name}.qualification.concurrent_jobs must be 1`);
    // design/CURRENT.md lists the frequency ceiling among the properties this
    // host is qualified by, so the record enumerates it as a key rather than
    // leaving it to the prose in attested_by. A pinned core left to a dynamic
    // governor is the one qualification fault here that produced a wrong
    // number instead of a failed run. Only the two invariants are rules: the
    // attestation holds a core at its ceiling under the performance governor
    // or a range pinned to one frequency, so the governor a machine gets there
    // by is a reading it declares, not a value this may demand.
    const frequency = q.cpu_frequency;
    if (frequency === null || typeof frequency !== "object"
      || frequency.pinned_cores !== "held-at-ceiling"
      || frequency.boost !== "off") {
      fail(
        `runner_classes.${name}.qualification.cpu_frequency must hold every pinned core at its ` +
        "ceiling with boost off",
      );
    }
    const r = q.repeatability;
    if (r === null || typeof r !== "object"
      || typeof r.median_spread_percent !== "number"
      || typeof r.worst_spread_percent !== "number") {
      fail(`runner_classes.${name}.qualification.repeatability must record median and worst spread`);
    }
  }
  if (authoritative.length > 1) {
    // One authority, so the shared pool cannot be promoted later by quietly
    // adding a second.
    fail(`at most one runner class may be a timing authority; got ${authoritative.join(", ")}`);
  }
  const declared = policy.measurement?.authoritative_runner_classes;
  if (!Array.isArray(declared) || JSON.stringify(declared.slice().sort()) !== JSON.stringify(authoritative.slice().sort())) {
    fail("measurement.authoritative_runner_classes must list exactly the classes marked timing_authority");
  }
}

function validatePolicy(policy) {
  if (policy.schema !== "rustia.bench.operations-policy.v2" || policy.schema_version !== 2) {
    fail(`${relative(policyPath)} must use rustia.bench.operations-policy.v2`);
  }
  validateRunnerClasses(policy);
  if (!Array.isArray(policy.progress?.relevant_paths) || policy.progress.relevant_paths.length === 0) {
    fail("progress.relevant_paths must be a non-empty array");
  }
  for (const [index, relevantPath] of policy.progress.relevant_paths.entries()) {
    requireString(relevantPath, `progress.relevant_paths[${index}]`);
    if (path.isAbsolute(relevantPath) || relevantPath.split("/").includes("..")) {
      fail(`progress.relevant_paths[${index}] must remain repository-relative`);
    }
  }
  if (policy.measurement?.scheduled_billing_allowed !== false) {
    fail("scheduled benchmark billing must remain disabled");
  }
  if (policy.publication?.state !== "active") {
    fail("artifact-only publication must remain active");
  }
  if (policy.landscape?.state !== "paused") {
    fail("the unimplemented landscape lane must remain paused");
  }
  for (const [label, value] of [
    ["publication.reason", policy.publication.reason],
    ["publication.next_action", policy.publication.next_action],
    ["progress.next_action", policy.progress.next_action],
    ["landscape.reason", policy.landscape.reason],
    ["landscape.next_action", policy.landscape.next_action],
  ]) requireString(value, label);
}

// The cleaned public-source tree is an explicit, separate operations profile.
// It carries historical measurements for transparency, but cannot authorize
// new measurements or publication while the private bridge is absent.
function validatePublicPolicy(policy) {
  if (policy.schema !== "rustia.bench.operations-policy.public.v1" || policy.schema_version !== 1) {
    fail(`${relative(policyPath)} must use rustia.bench.operations-policy.public.v1`);
  }
  if (policy.profile !== "public-source" || !PUBLIC_MEASUREMENT_MODES.includes(policy.measurement?.mode)
    || policy.measurement?.authoritative_runner_classes?.length !== 0
    || policy.measurement?.scheduled_billing_allowed !== false) {
    fail("public-source measurement must remain paused with no timing-authoritative runner or scheduled billing");
  }
  if (policy.publication?.state !== "paused" || policy.progress?.state !== "paused"
    || policy.landscape?.state !== "paused") {
    fail("public-source measurement, publication, progress, and landscape must remain paused");
  }
  for (const [label, value] of [
    ["measurement.reason", policy.measurement.reason],
    ["publication.reason", policy.publication.reason],
    ["publication.next_action", policy.publication.next_action],
    ["progress.next_action", policy.progress.next_action],
    ["landscape.reason", policy.landscape.reason],
    ["landscape.next_action", policy.landscape.next_action],
  ]) requireString(value, label);
  for (const phrase of [policy.measurement.reason, policy.publication.reason, policy.publication.next_action]) {
    if (!/private benchmark bridge/i.test(phrase)) fail("public-source paused state must identify the private benchmark bridge dependency");
  }
  const classes = policy.runner_classes;
  if (classes === null || typeof classes !== "object" || Array.isArray(classes) || Object.keys(classes).length === 0) {
    fail("public-source runner_classes must enumerate self-hosted correctness classes");
  }
  const expectedRoles = {
    "self-hosted-linux": ["build", "correctness-test", "artifact-validation", "fuzz-smoke"],
    "self-hosted-macos": ["build", "correctness-test", "artifact-validation"],
    "self-hosted-windows": ["build", "correctness-test", "artifact-validation"],
  };
  if (JSON.stringify(Object.keys(classes).sort()) !== JSON.stringify(Object.keys(expectedRoles).sort())) {
    fail("public-source runner_classes must enumerate exactly the reviewed self-hosted classes");
  }
  for (const [name, runner] of Object.entries(classes)) {
    if (runner.environment !== "self-hosted-shared" || runner.timing_authority !== false
      || Object.hasOwn(runner, "host") || Object.hasOwn(runner, "qualification")
      || !Array.isArray(runner.allowed_roles) || runner.allowed_roles.length === 0
      || JSON.stringify(runner.allowed_roles) !== JSON.stringify(expectedRoles[name])) {
      fail(`runner_classes.${name} must be a self-hosted correctness-only class with no host or measurement authority`);
    }
  }
  if (!Array.isArray(policy.progress?.relevant_paths) || policy.progress.relevant_paths.length === 0) {
    fail("progress.relevant_paths must be a non-empty array");
  }
  for (const [index, relevantPath] of policy.progress.relevant_paths.entries()) {
    requireString(relevantPath, `progress.relevant_paths[${index}]`);
    if (path.isAbsolute(relevantPath) || relevantPath.split("/").includes("..")) {
      fail(`progress.relevant_paths[${index}] must remain repository-relative`);
    }
  }
}

function measuredCohort(results) {
  if (results.schema !== "rustia.bench.current-results.v1" || results.schema_version !== 1) {
    fail(`${relative(resultsPath)} must use rustia.bench.current-results.v1`);
  }
  const commits = results.metadata?.commits;
  const window = results.metadata?.measurement_window_unix;
  if (!Array.isArray(commits) || commits.length !== 1 || !/^[0-9a-f]{40}$/.test(commits[0])) {
    fail("current results must contain one exact 40-character benchmark commit");
  }
  if (!Number.isSafeInteger(window?.start) || !Number.isSafeInteger(window?.end) || window.end < window.start) {
    fail("current results must contain a valid measurement window");
  }
  return { commit: commits[0], measuredAtUnix: window.end };
}

function commitRecord(commit) {
  const fields = git(["show", "-s", "--format=%H%x00%ct%x00%cI", commit]).split("\0");
  if (fields.length !== 3 || !/^[0-9a-f]{40}$/.test(fields[0])) fail(`could not identify commit ${commit}`);
  return { commit: fields[0], committed_at_unix: Number(fields[1]), committed_at: fields[2] };
}

function relevantCommits(measuredCommit, paths) {
  const output = git([
    "log",
    "--format=%H%x00%ct%x00%cI",
    `${measuredCommit}..HEAD`,
    "--",
    ...paths,
  ]);
  if (output === "") return [];
  return output.split("\n").map((line) => {
    const fields = line.split("\0");
    if (fields.length !== 3 || !/^[0-9a-f]{40}$/.test(fields[0])) fail("git emitted an invalid relevant commit");
    return { commit: fields[0], committed_at_unix: Number(fields[1]), committed_at: fields[2] };
  });
}

function renderStatus(policy, results) {
  validatePolicy(policy);
  const measured = measuredCohort(results);
  git(["merge-base", "--is-ancestor", measured.commit, "HEAD"]);
  const pending = relevantCommits(measured.commit, policy.progress.relevant_paths);
  const newestRelevant = pending[0] ?? commitRecord(measured.commit);
  return {
    schema: "rustia.bench.operations-status.v1",
    schema_version: 1,
    runner_classes: policy.runner_classes,
    measurement: policy.measurement,
    publication: policy.publication,
    progress: {
      state: pending.length === 0 ? "current" : "pending",
      measured_commit: measured.commit,
      measured_at_unix: measured.measuredAtUnix,
      latest_relevant_commit: newestRelevant.commit,
      latest_relevant_committed_at_unix: newestRelevant.committed_at_unix,
      pending_relevant_commit_count: pending.length,
      next_action: pending.length === 0
        ? "No progress measurement is currently required."
        : policy.progress.next_action,
    },
    landscape: {
      state: policy.landscape.state,
      measured_commit: measured.commit,
      measured_at_unix: measured.measuredAtUnix,
      refresh_interval_days: policy.landscape.refresh_interval_days,
      reason: policy.landscape.reason,
      next_action: policy.landscape.next_action,
    },
  };
}

function renderPublicStatus(policy, results, bridgeState = "pending-private-trusted-pipeline", correctnessCi = "private-self-hosted") {
  validatePublicPolicy(policy);
  validatePublicBridgeState(bridgeState);
  if (!["private-self-hosted", "public-hosted"].includes(correctnessCi)) fail("unknown correctness CI mode");
  const measured = measuredCohort(results);
  const projectedMeasuredCommit = resolveHistoricalCommit(measured.commit, root);
  git(["merge-base", "--is-ancestor", projectedMeasuredCommit, "HEAD"]);
  const pending = relevantCommits(projectedMeasuredCommit, policy.progress.relevant_paths);
  const newestRelevant = pending[0] ?? commitRecord(projectedMeasuredCommit);
  const manualCohortsAvailable = bridgeState === "private-maintainer-cohorts-v1";
  const measurement = { ...policy.measurement };
  const publication = { ...policy.publication };
  if (manualCohortsAvailable) measurement.mode = "public-automation-paused-private-manual-cohorts";
  if (!manualCohortsAvailable) {
    measurement.mode = "paused-pending-private-bridge";
    measurement.reason = "Public measurement and history automation remain paused pending a reviewed trusted private benchmark bridge. Historical measurements do not authorize current timing or stability claims.";
    publication.reason = "Public benchmark-data publication remains paused pending a reviewed trusted private benchmark bridge.";
    publication.next_action = "Wait for a trusted private benchmark bridge review; keep publication disabled.";
  }
  return {
    schema: "rustia.bench.operations-status.v1",
    schema_version: 1,
    profile: "public-source",
    benchmark_bridge: bridgeState,
    runner_classes: correctnessCi === "public-hosted" ? Object.fromEntries(Object.entries(policy.runner_classes).map(([name, runner]) =>
      [name.replace("self-hosted-", "hosted-"), { ...runner, environment: "github-hosted" }])) : policy.runner_classes,
    measurement,
    publication,
    progress: {
      state: "paused",
      measured_commit: measured.commit,
      projected_measured_commit: projectedMeasuredCommit,
      measured_at_unix: measured.measuredAtUnix,
      latest_relevant_commit: newestRelevant.commit,
      latest_relevant_committed_at_unix: newestRelevant.committed_at_unix,
      pending_relevant_commit_count: pending.length,
      next_action: manualCohortsAvailable
        ? policy.progress.next_action
        : "Public benchmark progress remains paused pending a trusted private benchmark bridge.",
    },
    landscape: {
      state: "paused",
      measured_commit: measured.commit,
      projected_measured_commit: projectedMeasuredCommit,
      measured_at_unix: measured.measuredAtUnix,
      refresh_interval_days: policy.landscape.refresh_interval_days,
      reason: manualCohortsAvailable
        ? policy.landscape.reason
        : "Public engine landscape refresh remains paused pending a trusted private benchmark bridge.",
      next_action: manualCohortsAvailable
        ? policy.landscape.next_action
        : "Wait for a separate trusted private benchmark bridge review before resuming landscape refresh.",
    },
  };
}

// The refusals in validatePolicy guard a file that is mirrored into the public
// data repository, so they are exercised before anything is rendered or
// checked: a validator that accepted everything would otherwise pass --check
// without anyone noticing. The committed policy is the one that matters, so
// each case is a one-edit variant of it rather than a lookalike fixture.
function selfTest(policy) {
  const variant = (edit) => {
    const copy = JSON.parse(JSON.stringify(policy));
    edit(copy);
    return copy;
  };
  const refuses = (pattern, edit) => {
    let message = null;
    try {
      validatePolicy(variant(edit));
    } catch (error) {
      message = error.message;
    }
    if (message == null || !pattern.test(message)) {
      fail(`self-test expected a refusal matching ${pattern}, got ${JSON.stringify(message)}`);
    }
  };
  const literal = (text) => new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));

  validatePolicy(variant(() => {}));
  const authority = Object.keys(policy.runner_classes).find((name) => policy.runner_classes[name].timing_authority === true);
  if (authority === undefined) fail(`self-test needs ${relative(policyPath)} to declare a timing authority`);
  refuses(literal(`runner_classes.${authority}.host must be a non-empty string`), (copy) => {
    delete copy.runner_classes[authority].host;
  });
  refuses(literal(`runner_classes.${authority}.host must be a non-empty string`), (copy) => {
    copy.runner_classes[authority].host = "";
  });
  const ceiling = literal(
    `runner_classes.${authority}.qualification.cpu_frequency must hold every pinned core at its ` +
    "ceiling with boost off",
  );
  refuses(ceiling, (copy) => {
    delete copy.runner_classes[authority].qualification.cpu_frequency;
  });
  refuses(ceiling, (copy) => {
    copy.runner_classes[authority].qualification.cpu_frequency.pinned_cores = "scaled";
  });
  refuses(ceiling, (copy) => {
    copy.runner_classes[authority].qualification.cpu_frequency.boost = "on";
  });
  // The attestation accepts a pinned range under any governor, so the governor
  // this machine gets there by is a declared reading rather than a rule.
  validatePolicy(variant((copy) => {
    copy.runner_classes[authority].qualification.cpu_frequency.governor = "userspace";
  }));
  // Only the authority must name a machine; another class may name none.
  validatePolicy(variant((copy) => {
    copy.runner_classes["general-linux"].host = null;
  }));
  // Whichever class names a machine, the name is published: a label, never an
  // address.
  for (const address of ["bench.example.com", "10.0.0.5", "fe80::1", "user@box"]) {
    for (const name of [authority, "general-linux"]) {
      refuses(literal(`runner_classes.${name} names its machine ${JSON.stringify(address)}, which is not a label`), (copy) => {
        copy.runner_classes[name].host = address;
      });
    }
  }
}

function selfTestPublicPolicy(policy) {
  validatePublicPolicy(policy);
  validatePublicPolicy({ ...policy, measurement: { ...policy.measurement, mode: "paused-pending-private-bridge" } });
  for (const mode of ["enabled", "active", ""]) {
    let message = null;
    try { validatePublicPolicy({ ...policy, measurement: { ...policy.measurement, mode } }); }
    catch (error) { message = error.message; }
    if (message == null) fail(`public policy self-test accepted measurement mode ${JSON.stringify(mode)}`);
  }
  for (const state of PUBLIC_BRIDGE_STATES) validatePublicBridgeState(state);
  for (const state of ["enabled", "available", "", null]) {
    let message = null;
    try { validatePublicBridgeState(state); } catch (error) { message = error.message; }
    if (message == null) fail(`public bridge self-test accepted unknown state ${JSON.stringify(state)}`);
  }
  const results = readJson(resultsPath);
  const pendingStatus = renderPublicStatus(policy, results, "pending-private-trusted-pipeline");
  const manualStatus = renderPublicStatus(policy, results, "private-maintainer-cohorts-v1");
  const hostedStatus = renderPublicStatus(policy, results, "pending-private-trusted-pipeline", "public-hosted");
  if (Object.values(hostedStatus.runner_classes).some((runner) => runner.environment !== "github-hosted" ||
      runner.timing_authority !== false || Object.hasOwn(runner, "host")) ||
      hostedStatus.publication.state !== "paused" || hostedStatus.measurement.authoritative_runner_classes.length !== 0) {
    fail("hosted correctness selection must preserve the benchmark authority boundary");
  }
  if (pendingStatus.benchmark_bridge !== "pending-private-trusted-pipeline"
    || pendingStatus.measurement.mode !== "paused-pending-private-bridge"
    || !/pending a reviewed trusted private benchmark bridge/i.test(pendingStatus.measurement.reason)
    || manualStatus.benchmark_bridge !== "private-maintainer-cohorts-v1"
    || !/manual.*exact-cohort runtime comparisons/i.test(manualStatus.measurement.reason)
    || manualStatus.measurement.mode !== "public-automation-paused-private-manual-cohorts"
    || manualStatus.measurement.authoritative_runner_classes.length !== 0
    || manualStatus.measurement.scheduled_billing_allowed !== false
    || manualStatus.publication.state !== "paused"
    || manualStatus.progress.state !== "paused"
    || manualStatus.landscape.state !== "paused"
    || !/do not authorize public claims/i.test(manualStatus.measurement.reason)) {
    fail("public bridge self-test did not preserve paused public automation and publication states");
  }
  const refuses = (pattern, edit) => {
    const copy = JSON.parse(JSON.stringify(policy));
    edit(copy);
    let message = null;
    try { validatePublicPolicy(copy); } catch (error) { message = error.message; }
    if (message == null || !pattern.test(message)) {
      fail(`public self-test expected a refusal matching ${pattern}, got ${JSON.stringify(message)}`);
    }
  };
  refuses(/must remain paused/, (copy) => { copy.publication.state = "active"; });
  refuses(/no timing-authoritative runner/, (copy) => { copy.measurement.authoritative_runner_classes = ["hosted"]; });
  refuses(/correctness-only class/, (copy) => { copy.runner_classes[Object.keys(copy.runner_classes)[0]].allowed_roles.push("measurement-candidate"); });
  refuses(/private benchmark bridge dependency/, (copy) => { copy.publication.reason = "Publication is paused."; });
  refuses(/no host or measurement authority/, (copy) => { copy.runner_classes[Object.keys(copy.runner_classes)[0]].host = "runner-name"; });
  refuses(/exactly the reviewed self-hosted classes/, (copy) => { copy.runner_classes.extra = copy.runner_classes["self-hosted-linux"]; });
  refuses(/correctness-only class/, (copy) => { copy.runner_classes["self-hosted-linux"].allowed_roles.push("build"); });
  refuses(/correctness-only class/, (copy) => { copy.runner_classes["self-hosted-linux"].environment = "github-hosted"; });
}

function validatePublicBridgeState(state) {
  if (!PUBLIC_BRIDGE_STATES.includes(state)) {
    fail(`${relative(publicProfilePath)} must use a reviewed public benchmark bridge state`);
  }
  return state;
}

function publicBridgeState() {
  if (!fs.existsSync(publicProfilePath)) return false;
  const marker = readJson(publicProfilePath);
  if (marker.schema !== "rustia.public-source-profile.v1" || marker.state !== "private-rehearsal"
    || !/^[0-9a-f]{40}$/.test(marker.historical_head ?? "")
    || marker.historical_host_label !== "ci-bench"
    || marker.package_publication !== false || marker.visibility_authorized !== false) {
    fail(`${relative(publicProfilePath)} must declare the public-source private-rehearsal profile and exact historical head`);
  }
  return validatePublicBridgeState(marker.benchmark_bridge);
}

function main() {
  const policy = readJson(policyPath);
  const bridgeState = publicBridgeState();
  const publicProfile = bridgeState !== false;
  if (publicProfile) selfTestPublicPolicy(policy);
  else selfTest(policy);
  const results = readJson(resultsPath);
  const correctnessCi = publicProfile ? (readJson(publicProfilePath).correctness_ci ?? "private-self-hosted") : null;
  const status = publicProfile ? renderPublicStatus(policy, results, bridgeState, correctnessCi) : renderStatus(policy, results);
  const output = `${JSON.stringify(status, null, 2)}\n`;
  if (checkOnly) {
    let current;
    try {
      current = fs.readFileSync(outputPath, "utf8");
    } catch (error) {
      fail(`could not read ${relative(outputPath)}: ${error.message}`);
    }
    if (current !== output) {
      fail(`${relative(outputPath)} is stale; run node tools/render-benchmark-operations-status.js`);
    }
    console.log(`${relative(outputPath)} is current`);
    return;
  }
  fs.writeFileSync(outputPath, output);
  console.log(`wrote ${relative(outputPath)} from benchmark policy, results, and Git history`);
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`render-benchmark-operations-status: ${error.message}`);
    process.exit(1);
  }
}

module.exports = { renderPublicStatus, renderStatus, validatePolicy, validatePublicPolicy };
