#!/usr/bin/env node
"use strict";
const fs = require("node:fs");
const { execFileSync } = require("node:child_process");
const path = require("node:path");
const root = path.resolve(__dirname, "..");
const initialImport = require("./initial-history-import.js");
const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
function assertClean(head) {
  if (git("rev-parse", "HEAD") !== head || git("status", "--porcelain", "--untracked-files=normal")) {
    throw new Error("Commit all intended changes before pushing; validation requires an unchanged, clean checkout.");
  }
}
try {
  const updates = fs.readFileSync(0, "utf8").trim().split("\n").filter(Boolean)
    .map(line => line.split(/\s+/));
  const manifestPath = process.env.RUSTIA_INITIAL_HISTORY_IMPORT;
  if (manifestPath !== undefined) {
    if (!manifestPath.trim()) throw new Error("RUSTIA_INITIAL_HISTORY_IMPORT must name the reviewed private manifest");
    if (process.argv.length !== 4) throw new Error("initial-history import requires Git's exact remote name and URL arguments");
    const remoteUrl = process.argv[3];
    initialImport.validateRemoteUrl(remoteUrl);
    const resolvedManifestPath = path.resolve(root, manifestPath);
    const manifestBytes = fs.readFileSync(resolvedManifestPath);
    const manifest = initialImport.parseManifest(manifestBytes);
    const parseUpdates = () => updates.map(fields => {
      if (fields.length !== 4) throw new Error("malformed pre-push update during initial-history import");
      return { localRef: fields[0], localOid: fields[1], remoteRef: fields[2], remoteOid: fields[3] };
    });
    const checkLocal = () => {
      const { head, refs } = initialImport.localSnapshot(root);
      initialImport.validatePlan({
        manifest, refs, head, updates: parseUpdates(),
        isAncestor: (base, commit) => {
          try { git("merge-base", "--is-ancestor", base, commit); return true; }
          catch { return false; }
        },
      });
      return head;
    };
    const head = checkLocal();
    initialImport.verifyRemote(remoteUrl);
    assertClean(head);
    initialImport.verifyProjection(root);

    // This is the ordinary full inventory and hook path. The opt-in applies
    // only to validating the initial ref update, not to tests run by preflight.
    const environment = { ...process.env };
    delete environment.RUSTIA_INITIAL_HISTORY_IMPORT;
    for (const name of git("rev-parse", "--local-env-vars").split("\n")) delete environment[name];
    execFileSync(process.execPath, [path.join(root, "tools/run-release-preflight.js")], {
      cwd: root, stdio: "inherit", env: environment,
    });

    assertClean(head);
    if (!fs.readFileSync(resolvedManifestPath).equals(manifestBytes)) throw new Error("initial-history manifest changed during validation");
    initialImport.parseManifest(fs.readFileSync(resolvedManifestPath));
    if (checkLocal() !== head) throw new Error("HEAD changed during initial-history validation");
    initialImport.verifyProjection(root);
    initialImport.verifyRemote(remoteUrl);
  } else {
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
    execFileSync(process.execPath, [path.join(root, "tools/run-release-preflight.js")], { cwd: root, stdio: "inherit", env: environment });
    assertClean(head);
  }
  }
} catch (error) {
  console.error(`pre-push: ${error.message}`);
  process.exitCode = 1;
}
