#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const policyPath = path.join(root, "bench", "operations-policy.json");
const resultsPath = path.join(root, "bench", "current-results.json");
const outputPath = path.join(root, "bench", "operations-status.json");
const checkOnly = process.argv.includes("--check");

function fail(message) {
  console.error(`render-benchmark-operations-status: ${message}`);
  process.exit(1);
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

function validatePolicy(policy) {
  if (policy.schema !== "rustia.bench.operations-policy.v1" || policy.schema_version !== 1) {
    fail(`${relative(policyPath)} must use rustia.bench.operations-policy.v1`);
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
  if (policy.runner?.timing_authority !== false) fail("shared runner timing_authority must remain false");
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
    runner: policy.runner,
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

function main() {
  const output = `${JSON.stringify(renderStatus(readJson(policyPath), readJson(resultsPath)), null, 2)}\n`;
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

if (require.main === module) main();

module.exports = { renderStatus, validatePolicy };
