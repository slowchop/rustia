#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");
const childProcess = require("child_process");

const root = path.resolve(__dirname, "..");
const changelogPath = path.join(root, "CHANGELOG.md");
const allowedSections = new Set(["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]);

function fail(message) {
  console.error(`check-release-notes: ${message}`);
  process.exit(1);
}

function cargoMetadata() {
  try {
    return JSON.parse(
      childProcess.execFileSync("cargo", ["metadata", "--no-deps", "--format-version", "1"], {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }),
    );
  } catch (error) {
    const stderr = error.stderr ? String(error.stderr).trim() : "";
    fail(`cargo metadata failed${stderr ? `\n${stderr}` : ""}`);
  }
}

function workspaceVersion(metadata) {
  const members = new Set(metadata.workspace_members);
  const versions = [...new Set(
    metadata.packages
      .filter((pkg) => members.has(pkg.id))
      .map((pkg) => pkg.version),
  )].sort();
  if (versions.length !== 1) {
    fail(`workspace packages must share one version, got ${versions.join(", ")}`);
  }
  return versions[0];
}

function sectionBody(text, heading) {
  const headingLine = `## [${heading}]`;
  const start = text.indexOf(`${headingLine}\n`);
  if (start === -1) {
    return null;
  }
  const bodyStart = start + headingLine.length + 1;
  const nextHeading = text.slice(bodyStart).search(/^## /m);
  return nextHeading === -1
    ? text.slice(bodyStart)
    : text.slice(bodyStart, bodyStart + nextHeading);
}

function validateChangelog(text, version) {
  if (!text.startsWith("# Changelog\n")) {
    fail("CHANGELOG.md must start with '# Changelog'");
  }
  if (!text.includes(`Current unpublished workspace version: \`${version}\`.`)) {
    fail(`CHANGELOG.md must name current unpublished workspace version ${version}`);
  }
  const unreleased = sectionBody(text, "Unreleased");
  if (unreleased == null) {
    fail("CHANGELOG.md must contain ## [Unreleased]");
  }
  const headings = [...unreleased.matchAll(/^### (.+)$/gm)].map((match) => match[1].trim());
  const unknown = headings.filter((heading) => !allowedSections.has(heading));
  if (unknown.length !== 0) {
    fail(`unknown Unreleased section heading(s): ${unknown.join(", ")}`);
  }
  if (headings.length === 0) {
    fail("Unreleased section must contain at least one change category");
  }
  if (!/^- .+/m.test(unreleased)) {
    fail("Unreleased section must contain at least one bullet");
  }
  if (!/^## Release Policy$/m.test(text)) {
    fail("CHANGELOG.md must contain ## Release Policy");
  }
}

let text;
try {
  text = fs.readFileSync(changelogPath, "utf8");
} catch (error) {
  fail(`could not read CHANGELOG.md: ${error.message}`);
}

const version = workspaceVersion(cargoMetadata());
validateChangelog(text, version);
console.log(`CHANGELOG.md ok for workspace version ${version}`);
