#!/usr/bin/env node

"use strict";

const fs = require("node:fs");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const ROOT = path.resolve(__dirname, "..");
const CHECKER = path.join(ROOT, "tools", "check-benchmark-results.js");
const FULL_COMMIT = /^[0-9a-f]{40}$/;
const MAP_HEADER = "const expectedEngineCohorts = new Map([\n";

function fail(message) {
  process.stderr.write(`register-matrix-cohort: ${message}\n`);
  process.exit(1);
}

function headCommit() {
  const commit = execFileSync("git", ["rev-parse", "HEAD"], {
    cwd: ROOT,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }).trim();
  if (!FULL_COMMIT.test(commit)) fail("HEAD did not resolve to a full lowercase commit");
  return commit;
}

// Registers HEAD as an expected frozen matrix cohort in
// tools/check-benchmark-results.js by cloning the engine/dialect map of the
// newest registered cohort. The freeze flow runs this beside
// create-linode-matrix-config.js, whose embedded engine list mirrors the same
// newest operator template, so the two stay consistent by construction.
function main() {
  const argv = process.argv.slice(2);
  let commit;
  if (argv.length === 0) {
    commit = headCommit();
  } else if (argv.length === 2 && argv[0] === "--commit" && FULL_COMMIT.test(argv[1])) {
    commit = argv[1];
  } else {
    fail("usage: register-matrix-cohort.js [--commit FULL_SHA] (default: HEAD)");
  }
  const text = fs.readFileSync(CHECKER, "utf8");
  if (text.includes(`"${commit}"`)) {
    process.stdout.write(`${commit} is already a registered matrix cohort\n`);
    return;
  }
  const headerIndex = text.indexOf(MAP_HEADER);
  if (headerIndex < 0) fail("expectedEngineCohorts map header not found");
  const entryStart = headerIndex + MAP_HEADER.length;
  if (text.slice(entryStart, entryStart + 4) !== "  [\n") {
    fail("expectedEngineCohorts must start with an entry block");
  }
  let depth = 0;
  let entryEnd = -1;
  for (let index = entryStart; index < text.length; index += 1) {
    const character = text[index];
    if (character === "[" || character === "(") depth += 1;
    else if (character === "]" || character === ")") {
      depth -= 1;
      if (depth === 0) {
        entryEnd = text.indexOf("\n", index) + 1;
        break;
      }
    }
  }
  if (entryEnd <= entryStart) fail("could not delimit the newest cohort entry");
  const template = text.slice(entryStart, entryEnd);
  const templateCommit = template.match(/"([0-9a-f]{40})"/);
  if (!templateCommit) fail("newest cohort entry does not name a full commit");
  const inserted = template.replace(`"${templateCommit[1]}"`, `"${commit}"`);
  const updated = text.slice(0, entryStart) + inserted + text.slice(entryStart);
  fs.writeFileSync(CHECKER, updated);
  process.stdout.write(`registered matrix cohort ${commit}\n`);
}

main();
