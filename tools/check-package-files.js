#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const outputPath = path.join(root, "design", "package-files.json");
const checkOnly = process.argv.includes("--check");

const forbiddenRules = [
  { name: "private", description: "private/ must not be packaged" },
  { name: "target", description: "build output must not be packaged" },
  { name: ".git", description: "git internals must not be packaged" },
  { name: ".claude", description: "agent-local state must not be packaged" },
  { name: ".codex", description: "agent-local state must not be packaged" },
  { name: ".fastembed_cache", description: "local caches must not be packaged" },
];

function fail(message) {
  console.error(`check-package-files: ${message}`);
  process.exit(1);
}

function runCargo(args) {
  try {
    return childProcess.execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    const detail = stderr ? `\n${stderr}` : "";
    fail(`cargo ${args.join(" ")} failed${detail}`);
  }
}

function slash(value) {
  return value.split(path.sep).join("/");
}

function workspacePackages(metadata) {
  const members = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => members.has(pkg.id))
    .sort((left, right) => left.manifest_path.localeCompare(right.manifest_path));
}

function packageFiles(packageName) {
  return runCargo(["package", "-p", packageName, "--allow-dirty", "--list"])
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

function validateFiles(pkg, files) {
  const violations = [];
  const required = new Set([".cargo_vcs_info.json", "Cargo.toml", "Cargo.toml.orig", "README.md"]);

  for (const file of files) {
    if (path.isAbsolute(file)) {
      violations.push(`${file}: absolute path`);
      continue;
    }
    if (file.includes("\\")) {
      violations.push(`${file}: contains backslash separators`);
    }
    const parts = file.split("/");
    if (parts.includes("..")) {
      violations.push(`${file}: contains parent path segment`);
    }
    for (const rule of forbiddenRules) {
      if (parts.includes(rule.name)) {
        violations.push(`${file}: ${rule.description}`);
      }
    }
    required.delete(file);
  }

  for (const file of required) {
    violations.push(`${pkg.name}: missing expected package file ${file}`);
  }
  if (!files.some((file) => file === "src/main.rs" || file === "src/lib.rs")) {
    violations.push(`${pkg.name}: missing src/main.rs or src/lib.rs`);
  }

  return violations;
}

function buildReport() {
  const metadata = JSON.parse(runCargo(["metadata", "--no-deps", "--format-version", "1"]));
  const packages = workspacePackages(metadata).map((pkg) => {
    const files = packageFiles(pkg.name);
    const violations = validateFiles(pkg, files);
    if (violations.length !== 0) {
      fail(`${pkg.name} package file audit failed:\n${violations.map((item) => `  - ${item}`).join("\n")}`);
    }
    return {
      name: pkg.name,
      version: pkg.version,
      manifest: slash(path.relative(root, pkg.manifest_path)),
      publish: publishValue(pkg.publish),
      file_count: files.length,
      files,
    };
  });

  return {
    schema_version: 1,
    source: "cargo package --list",
    command: "node tools/check-package-files.js",
    package_count: packages.length,
    forbidden_path_segments: forbiddenRules.map((rule) => rule.name),
    packages,
  };
}

function publishValue(value) {
  if (value === null) {
    return true;
  }
  if (Array.isArray(value) && value.length === 0) {
    return false;
  }
  return value;
}

const text = `${JSON.stringify(buildReport(), null, 2)}\n`;

if (checkOnly) {
  let current;
  try {
    current = fs.readFileSync(outputPath, "utf8");
  } catch (error) {
    fail(`could not read ${slash(path.relative(root, outputPath))}: ${error.message}`);
  }
  if (current !== text) {
    fail(`${slash(path.relative(root, outputPath))} is stale; run node tools/check-package-files.js`);
  }
  console.log(`${slash(path.relative(root, outputPath))} is current`);
} else {
  fs.writeFileSync(outputPath, text);
  console.log(`wrote ${slash(path.relative(root, outputPath))}`);
}
