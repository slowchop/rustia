#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const publishabilityPath = path.join(root, "design", "publishability.json");

function fail(message) {
  console.error(`check-package-dry-runs: ${message}`);
  process.exit(1);
}

function relative(filePath) {
  return path.relative(root, filePath).split(path.sep).join("/");
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(filePath)}: ${error.message}`);
  }
}

function runCargo(args) {
  try {
    childProcess.execFileSync("cargo", args, {
      cwd: root,
      stdio: ["ignore", "inherit", "inherit"],
    });
  } catch (_error) {
    fail(`cargo ${args.join(" ")} failed`);
  }
}

function checkPublishabilityCurrent() {
  try {
    childProcess.execFileSync(process.execPath, [
      path.join(root, "tools", "check-publishability.js"),
      "--check",
    ], {
      cwd: root,
      stdio: ["ignore", "inherit", "inherit"],
    });
  } catch (_error) {
    fail("design/publishability.json is stale or invalid");
  }
}

function packageByName(report) {
  if (!Array.isArray(report.packages)) {
    fail("design/publishability.json must include a packages array");
  }
  return new Map(report.packages.map((pkg) => [pkg.name, pkg]));
}

function publishCandidates(report) {
  if (report.schema_version !== 1) {
    fail("design/publishability.json must use schema_version 1");
  }
  const candidates = report.summary?.first_publish_candidates_after_policy_flip;
  if (!Array.isArray(candidates) || candidates.length === 0) {
    fail("design/publishability.json has no first publish candidates");
  }

  const packages = packageByName(report);
  const seen = new Set();
  for (const candidate of candidates) {
    if (typeof candidate !== "string" || candidate.length === 0) {
      fail("first publish candidates must be non-empty package names");
    }
    if (seen.has(candidate)) {
      fail(`duplicate first publish candidate ${candidate}`);
    }
    seen.add(candidate);

    const pkg = packages.get(candidate);
    if (!pkg) {
      fail(`first publish candidate ${candidate} is missing from package report`);
    }
    if (pkg.role !== "public-rustia-crate") {
      fail(`first publish candidate ${candidate} must be a public rustia crate`);
    }
    if (!Array.isArray(pkg.internal_dependencies) || pkg.internal_dependencies.length !== 0) {
      fail(`first publish candidate ${candidate} must not have internal dependencies`);
    }
  }
  return candidates;
}

checkPublishabilityCurrent();
const report = readJson(publishabilityPath);
const candidates = publishCandidates(report);
for (const name of candidates) {
  runCargo(["package", "--locked", "--no-verify", "--allow-dirty", "-p", name]);
}

console.log(`leaf package dry-runs ok for ${candidates.join(", ")}`);
