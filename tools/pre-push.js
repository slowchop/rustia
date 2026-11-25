#!/usr/bin/env node
"use strict";
const fs = require("node:fs");
const { execFileSync } = require("node:child_process");
const path = require("node:path");
const root = path.resolve(__dirname, "..");
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
function assertClean(head) {
  if (git("rev-parse", "HEAD") !== head || git("status", "--porcelain", "--untracked-files=normal")) {
    throw new Error("Commit all intended changes before pushing; validation requires an unchanged, clean checkout.");
  }
}
try {
  const updates = fs.readFileSync(0, "utf8").trim().split("\n").filter(Boolean)
    .map(line => line.split(/\s+/));
  const writes = updates.filter(fields => !/^0+$/.test(fields[1] || ""));
  if (writes.length) {
    const head = git("rev-parse", "HEAD");
    for (const fields of writes) {
      if (fields.length !== 4 || git("rev-parse", `${fields[1]}^{commit}`) !== head) {
        throw new Error("Push only the checked-out commit so the pre-push tests validate the actual source.");
      }
    }
    assertClean(head);
    // Temporary consumer/review repos must not inherit this hook's Git directory.
    const environment = { ...process.env };
    for (const name of git("rev-parse", "--local-env-vars").split("\n")) delete environment[name];
    // The trusted Quick workflow may point the protected policy runner at its
    // verified candidate checkout. A local push must always validate this checkout.
    delete environment.RUSTIA_PREFLIGHT_ROOT;
    execFileSync(process.execPath, [path.join(root, "tools/run-release-preflight.js")], { cwd: root, stdio: "inherit", env: environment });
    assertClean(head);
  }
} catch (error) {
  console.error(`pre-push: ${error.message}`);
  process.exitCode = 1;
}
