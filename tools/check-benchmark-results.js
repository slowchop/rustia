#!/usr/bin/env node
"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const expectedResults = path.join(root, "bench", "RESULTS.md");
const expectedJson = path.join(root, "bench", "current-results.json");
const expectedClaims = path.join(root, "bench", "CLAIMS.md");

function fail(message) {
  console.error(`check-benchmark-results: ${message}`);
  process.exit(1);
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function relative(filePath) {
  return slash(path.relative(root, filePath));
}

function runCargo(args) {
  try {
    childProcess.execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`cargo ${args.join(" ")} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function runNode(args) {
  try {
    childProcess.execFileSync(process.execPath, args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    const command = args
      .map((arg) => path.isAbsolute(arg) ? relative(arg) : arg)
      .join(" ");
    fail(`node ${command} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function read(filePath) {
  try {
    return fs.readFileSync(filePath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(filePath)}: ${error.message}`);
  }
}

function compare(expectedPath, actualPath, refreshCommand) {
  const expected = read(expectedPath);
  const actual = read(actualPath);
  if (expected !== actual) {
    fail(`${relative(expectedPath)} is stale; run ${refreshCommand}`);
  }
}

const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-bench-results-"));
try {
  const actualResults = path.join(tempDir, "RESULTS.md");
  const actualJson = path.join(tempDir, "current-results.json");
  const actualClaims = path.join(tempDir, "CLAIMS.md");
  runCargo([
    "run",
    "-p",
    "script-bench-harness",
    "--",
    "--refresh-results",
    "--results",
    actualResults,
    "--latest-json",
    actualJson,
  ]);
  compare(expectedResults, actualResults, "just gen-current-results");
  compare(expectedJson, actualJson, "just gen-current-results");
  runNode([
    path.join(root, "bench", "render-claims.js"),
    "--source",
    actualJson,
    "--output",
    actualClaims,
  ]);
  compare(expectedClaims, actualClaims, "just gen-current-results");
  console.log("benchmark markdown, JSON, and claim results are current");
} finally {
  fs.rmSync(tempDir, { recursive: true, force: true });
}
