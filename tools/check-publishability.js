#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const outputPath = path.join(root, "design", "publishability.json");
const packageFilesPath = path.join(root, "design", "package-files.json");
const checkOnly = process.argv.includes("--check");

const publicOrderHint = [
  "rustia-lexer",
  "rustia-parser",
  "rustia-vm",
  "rustia-compiler",
  "rustia",
  "rustia-cli",
  "rustia-conformance",
  "script-bench-harness",
];

function fail(message) {
  console.error(`check-publishability: ${message}`);
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
    return childProcess.execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`cargo ${args.join(" ")} failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function readJson(filePath) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (error) {
    fail(`could not parse ${relative(filePath)}: ${error.message}`);
  }
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

function orderIndex(name) {
  const index = publicOrderHint.indexOf(name);
  return index === -1 ? publicOrderHint.length : index;
}

function compareNames(left, right) {
  return orderIndex(left) - orderIndex(right) || left.localeCompare(right);
}

function workspacePackages(metadata) {
  const members = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => members.has(pkg.id))
    .sort((left, right) => compareNames(left.name, right.name));
}

function internalDependencies(pkg, workspaceNames) {
  return pkg.dependencies
    .filter((dep) => dep.path && workspaceNames.has(dep.name))
    .map((dep) => ({
      name: dep.name,
      requirement: dep.req,
      optional: dep.optional,
    }))
    .sort((left, right) => compareNames(left.name, right.name));
}

function packageFileEntry(packageFiles, name) {
  return packageFiles.packages.find((pkg) => pkg.name === name) ?? null;
}

function blockersFor(pkg, internalDeps) {
  const blockers = [];
  if (publishValue(pkg.publish) === false) {
    blockers.push({
      code: "publish_false",
      detail: "The workspace intentionally keeps this crate private with publish = false.",
    });
  }
  if (internalDeps.length !== 0) {
    blockers.push({
      code: "internal_dependencies_unpublished",
      detail: `Requires published or removed internal crates first: ${internalDeps.map((dep) => dep.name).join(", ")}.`,
    });
  }
  if (pkg.name === "script-bench-harness") {
    blockers.push({
      code: "benchmark_adapter_strategy",
      detail: "Standalone split or rustia-adapter packaging strategy must be chosen before publishing the benchmark crate.",
    });
  }
  return blockers;
}

function topologicalOrder(packagesByName) {
  const remaining = new Set(packagesByName.keys());
  const order = [];
  while (remaining.size !== 0) {
    const ready = [...remaining]
      .filter((name) => {
        const pkg = packagesByName.get(name);
        return pkg.internal_dependencies.every((dep) => !remaining.has(dep.name));
      })
      .sort(compareNames);
    if (ready.length === 0) {
      fail(`internal dependency cycle among ${[...remaining].sort(compareNames).join(", ")}`);
    }
    const name = ready[0];
    remaining.delete(name);
    order.push(name);
  }
  return order;
}

function buildReport() {
  const metadata = JSON.parse(
    runCargo(["metadata", "--locked", "--no-deps", "--format-version", "1"]),
  );
  const packageFiles = readJson(packageFilesPath);
  if (packageFiles.schema_version !== 1 || !Array.isArray(packageFiles.packages)) {
    fail("design/package-files.json must be a schema_version 1 package-file report");
  }

  const packages = workspacePackages(metadata);
  const workspaceNames = new Set(packages.map((pkg) => pkg.name));
  const reports = packages.map((pkg) => {
    const internalDeps = internalDependencies(pkg, workspaceNames);
    const packageFile = packageFileEntry(packageFiles, pkg.name);
    if (!packageFile) {
      fail(`design/package-files.json has no entry for ${pkg.name}`);
    }
    const blockers = blockersFor(pkg, internalDeps);
    return {
      name: pkg.name,
      version: pkg.version,
      manifest: relative(pkg.manifest_path),
      publish: publishValue(pkg.publish),
      role: pkg.name === "script-bench-harness" ? "standalone-benchmark" : "public-rustia-crate",
      package_file_audit: {
        file_count: packageFile.file_count,
        forbidden_path_violations: 0,
      },
      internal_dependencies: internalDeps,
      status: blockers.length === 0 ? "publishable_by_current_audit" : "blocked",
      blockers,
    };
  });

  const packagesByName = new Map(reports.map((pkg) => [pkg.name, pkg]));
  const fullOrder = topologicalOrder(packagesByName);
  const publicOrder = fullOrder.filter((name) => packagesByName.get(name).role === "public-rustia-crate");
  const firstCandidates = reports
    .filter((pkg) => pkg.role === "public-rustia-crate" && pkg.internal_dependencies.length === 0)
    .map((pkg) => pkg.name)
    .sort(compareNames);

  return {
    schema_version: 1,
    source: "cargo metadata --no-deps plus design/package-files.json",
    command: "node tools/check-publishability.js",
    summary: {
      package_count: reports.length,
      public_rustia_crate_count: reports.filter((pkg) => pkg.role === "public-rustia-crate").length,
      publish_false_count: reports.filter((pkg) => pkg.publish === false).length,
      blocked_count: reports.filter((pkg) => pkg.status === "blocked").length,
      first_publish_candidates_after_policy_flip: firstCandidates,
      public_crate_publish_order: publicOrder,
      benchmark_crate_status: "blocked until standalone split or rustia-adapter strategy is chosen",
    },
    packages: reports,
  };
}

const text = `${JSON.stringify(buildReport(), null, 2)}\n`;

if (checkOnly) {
  let current;
  try {
    current = fs.readFileSync(outputPath, "utf8");
  } catch (error) {
    fail(`could not read ${relative(outputPath)}: ${error.message}`);
  }
  if (current !== text) {
    fail(`${relative(outputPath)} is stale; run node tools/check-publishability.js`);
  }
  console.log(`${relative(outputPath)} is current`);
} else {
  fs.writeFileSync(outputPath, text, "utf8");
  console.log(`wrote ${relative(outputPath)}`);
}
