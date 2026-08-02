#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const outputPath = path.join(root, "design", "crate-metadata.json");
const checkOnly = process.argv.includes("--check");

const expectedPackages = [
  "rustia-lexer",
  "rustia-parser",
  "rustia-vm",
  "rustia-compiler",
  "rustia",
  "rustia-cli",
  "rustia-conformance",
  "script-bench-harness",
];

const expected = {
  edition: "2021",
  rust_version: "1.95",
  license: "MIT OR Apache-2.0",
  repository: "https://github.com/slowchop/rustia",
  homepage: "https://github.com/slowchop/rustia",
  publish: false,
};

function fail(message) {
  console.error(`check-crate-metadata: ${message}`);
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
  const index = expectedPackages.indexOf(name);
  return index === -1 ? expectedPackages.length : index;
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

function readmePath(pkg) {
  if (!pkg.readme) {
    return null;
  }
  return path.resolve(path.dirname(pkg.manifest_path), pkg.readme);
}

function validateReadme(pkg) {
  const filePath = readmePath(pkg);
  if (!filePath) {
    return "missing readme";
  }
  if (!filePath.startsWith(root + path.sep)) {
    return `readme escapes workspace: ${pkg.readme}`;
  }
  if (!fs.existsSync(filePath)) {
    return `readme file does not exist: ${relative(filePath)}`;
  }
  return null;
}

function checkField(pkg, field, value, violations) {
  if (pkg[field] !== value) {
    violations.push(`${field} must be ${JSON.stringify(value)}, got ${JSON.stringify(pkg[field])}`);
  }
}

function validatePackage(pkg, workspaceNames, workspaceVersion) {
  const violations = [];
  for (const [field, value] of Object.entries(expected)) {
    if (field === "publish") {
      if (publishValue(pkg.publish) !== value) {
        violations.push(`publish must be ${value}, got ${JSON.stringify(pkg.publish)}`);
      }
    } else {
      checkField(pkg, field, value, violations);
    }
  }
  if (pkg.version !== workspaceVersion) {
    violations.push(`version must match workspace version ${workspaceVersion}, got ${pkg.version}`);
  }
  if (!pkg.description || pkg.description.trim().length < 10) {
    violations.push("description must be meaningful");
  }
  if (pkg.license_file !== null) {
    violations.push("license_file must not be set when license expression is set");
  }
  if (!Array.isArray(pkg.keywords) || pkg.keywords.length === 0 || pkg.keywords.length > 5) {
    violations.push("keywords must contain 1-5 entries");
  }
  if (!Array.isArray(pkg.categories) || pkg.categories.length === 0 || pkg.categories.length > 5) {
    violations.push("categories must contain 1-5 entries");
  }
  const readmeViolation = validateReadme(pkg);
  if (readmeViolation) {
    violations.push(readmeViolation);
  }
  for (const dep of pkg.dependencies) {
    if (dep.path && workspaceNames.has(dep.name) && dep.req !== `^${workspaceVersion}`) {
      violations.push(`internal dependency ${dep.name} must require ^${workspaceVersion}, got ${dep.req}`);
    }
  }
  return violations;
}

function buildReport() {
  const metadata = JSON.parse(
    runCargo(["metadata", "--locked", "--no-deps", "--format-version", "1"]),
  );
  const packages = workspacePackages(metadata);
  const names = packages.map((pkg) => pkg.name).sort(compareNames);
  if (names.join(",") !== expectedPackages.join(",")) {
    fail(`workspace package set changed; expected ${expectedPackages.join(", ")}, got ${names.join(", ")}`);
  }

  const versions = [...new Set(packages.map((pkg) => pkg.version))].sort();
  if (versions.length !== 1) {
    fail(`workspace packages must share one version, got ${versions.join(", ")}`);
  }
  const workspaceVersion = versions[0];
  const workspaceNames = new Set(names);
  const reports = packages.map((pkg) => {
    const violations = validatePackage(pkg, workspaceNames, workspaceVersion);
    if (violations.length !== 0) {
      fail(`${pkg.name} metadata audit failed:\n${violations.map((item) => `  - ${item}`).join("\n")}`);
    }
    return {
      name: pkg.name,
      version: pkg.version,
      manifest: relative(pkg.manifest_path),
      publish: publishValue(pkg.publish),
      edition: pkg.edition,
      rust_version: pkg.rust_version,
      license: pkg.license,
      description: pkg.description,
      readme: relative(readmePath(pkg)),
      repository: pkg.repository,
      homepage: pkg.homepage,
      keywords: pkg.keywords,
      categories: pkg.categories,
      internal_dependencies: pkg.dependencies
        .filter((dep) => dep.path && workspaceNames.has(dep.name))
        .map((dep) => ({
          name: dep.name,
          requirement: dep.req,
          optional: dep.optional,
        }))
        .sort((left, right) => compareNames(left.name, right.name)),
    };
  });

  return {
    schema_version: 1,
    source: "cargo metadata --no-deps",
    command: "node tools/check-crate-metadata.js",
    expected,
    summary: {
      package_count: reports.length,
      workspace_version: workspaceVersion,
      publish_false_count: reports.filter((pkg) => pkg.publish === false).length,
      all_readmes_exist: true,
      internal_dependency_versions_explicit: true,
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
    fail(`${relative(outputPath)} is stale; run node tools/check-crate-metadata.js`);
  }
  console.log(`${relative(outputPath)} is current`);
} else {
  fs.writeFileSync(outputPath, text, "utf8");
  console.log(`wrote ${relative(outputPath)}`);
}
