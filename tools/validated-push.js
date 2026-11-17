#!/usr/bin/env node
"use strict";

const childProcess = require("node:child_process");
const path = require("node:path");

const root = path.resolve(__dirname, "..");

function fail(message) {
  throw new Error(message);
}

function cleanEnvironment() {
  // Discover Git's repository-local variables from this checkout only. Clear
  // the known context variables for the discovery call first, so an inherited
  // linked-worktree GIT_DIR cannot redirect the query. Authentication and
  // transport settings such as GIT_SSH_COMMAND remain available to the push.
  const discoveryEnvironment = { ...process.env };
  const bootstrapLocalVariables = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES", "GIT_CONFIG", "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT", "GIT_OBJECT_DIRECTORY", "GIT_DIR", "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE", "GIT_GRAFT_FILE", "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS", "GIT_REPLACE_REF_BASE", "GIT_PREFIX",
    "GIT_SHALLOW_FILE", "GIT_COMMON_DIR",
  ];
  for (const name of bootstrapLocalVariables) delete discoveryEnvironment[name];
  const localVariables = childProcess.execFileSync(
    "git", ["-C", root, "rev-parse", "--local-env-vars"],
    { env: discoveryEnvironment, encoding: "utf8" },
  ).trim().split("\n").filter(Boolean);
  const environment = { ...process.env };
  for (const name of localVariables) delete environment[name];
  return environment;
}

const environment = cleanEnvironment();

function git(args, options = {}) {
  const output = childProcess.execFileSync("git", args, {
    cwd: root,
    env: environment,
    encoding: "utf8",
    stdio: options.inherit ? "inherit" : ["ignore", "pipe", "pipe"],
  });
  return options.inherit ? "" : output.trim();
}

function optionalGit(args) {
  const result = childProcess.spawnSync("git", args, {
    cwd: root,
    env: environment,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return result.status === 0 ? result.stdout.trim() : null;
}

function parseArguments(argv) {
  let requestedRemote = null;
  let forceWithLease = false;
  for (const argument of argv) {
    if (argument === "--force-with-lease") {
      if (forceWithLease) fail("--force-with-lease may be supplied only once");
      forceWithLease = true;
    } else if (argument.startsWith("-")) {
      fail(`unknown option ${argument}`);
    } else if (requestedRemote === null) {
      requestedRemote = argument;
    } else {
      fail("usage: validated-push.js [remote] [--force-with-lease]");
    }
  }
  return { requestedRemote, forceWithLease };
}

function assertClean(head) {
  const current = git(["rev-parse", "HEAD"]);
  const status = git(["status", "--porcelain=v1", "--untracked-files=all"]);
  if (current !== head || status) {
    fail("Commit all intended changes before pushing; validation requires an unchanged, clean checkout.");
  }
}

function currentBranch() {
  const branch = optionalGit(["symbolic-ref", "--quiet", "--short", "HEAD"]);
  if (!branch || !git(["check-ref-format", "--branch", branch])) {
    fail("validated push requires a checked-out branch, not detached HEAD");
  }
  return branch;
}

function remoteNames() {
  return git(["remote"]).split("\n").filter(Boolean);
}

function resolveDestination(branch, requestedRemote) {
  const configuredRemote = optionalGit(["config", "--get", `branch.${branch}.remote`]);
  const configuredMerge = optionalGit(["config", "--get", `branch.${branch}.merge`]);
  if ((configuredRemote === null) !== (configuredMerge === null)) {
    fail(`branch ${branch} has an incomplete upstream configuration`);
  }

  const remotes = remoteNames();
  if (!remotes.length) fail("validated push requires a configured remote");
  let remote;
  let remoteRef;
  let establishUpstream = false;

  if (configuredRemote !== null) {
    if (configuredRemote === ".") fail("validated push refuses a local-dot upstream");
    if (requestedRemote !== null && requestedRemote !== configuredRemote) {
      fail(`requested remote ${requestedRemote} does not match upstream remote ${configuredRemote}`);
    }
    remote = configuredRemote;
    remoteRef = configuredMerge;
  } else {
    remote = requestedRemote || (remotes.includes("origin") ? "origin" : remotes.length === 1 ? remotes[0] : null);
    if (!remote) fail("multiple remotes exist; name the remote for the first validated push");
    remoteRef = `refs/heads/${branch}`;
    establishUpstream = true;
  }

  if (remote.startsWith("-")) fail("validated push refuses an option-like remote name");
  if (!remotes.includes(remote)) fail(`unknown remote ${remote}`);
  const expectedRemoteRef = `refs/heads/${branch}`;
  if (remoteRef !== expectedRemoteRef) {
    fail(`validated push requires the same-name destination ${expectedRemoteRef}`);
  }
  git(["check-ref-format", remoteRef]);
  const pushUrl = git(["remote", "get-url", "--push", remote]);
  if (!pushUrl || pushUrl.startsWith("-")) fail("validated push refuses an option-like push URL");
  return { remote, remoteRef, pushUrl, establishUpstream };
}

function assertDestinationUnchanged(destination) {
  const configuredPushUrl = git(["remote", "get-url", "--push", destination.remote]);
  if (configuredPushUrl !== destination.pushUrl) {
    fail(`remote ${destination.remote} changed during validation`);
  }
}

function remoteTip(pushUrl, remoteRef) {
  const output = git(["ls-remote", "--heads", pushUrl, remoteRef]);
  if (!output) return null;
  const rows = output.split("\n").filter(Boolean).map(line => line.split(/\s+/));
  if (rows.length !== 1 || rows[0].length !== 2 || rows[0][1] !== remoteRef || !/^[0-9a-f]{40,64}$/.test(rows[0][0])) {
    fail(`remote returned an unexpected value for ${remoteRef}`);
  }
  return rows[0][0];
}

function main() {
  const { requestedRemote, forceWithLease } = parseArguments(process.argv.slice(2));
  const branch = currentBranch();
  const head = git(["rev-parse", "HEAD"]);
  const destination = resolveDestination(branch, requestedRemote);
  assertClean(head);

  let expectedRemoteTip = null;
  if (forceWithLease) {
    expectedRemoteTip = remoteTip(destination.pushUrl, destination.remoteRef);
    if (expectedRemoteTip === null) fail("--force-with-lease requires an existing remote branch");
  }

  childProcess.execFileSync(process.execPath, [path.join(root, "tools/run-release-preflight.js")], {
    cwd: root,
    env: environment,
    stdio: "inherit",
  });
  assertClean(head);
  assertDestinationUnchanged(destination);

  const pushArguments = ["push", "--no-verify"];
  if (expectedRemoteTip !== null) {
    pushArguments.push(`--force-with-lease=${destination.remoteRef}:${expectedRemoteTip}`);
  }
  pushArguments.push("--", destination.pushUrl, `${head}:${destination.remoteRef}`);
  git(pushArguments, { inherit: true });

  if (destination.establishUpstream) {
    // Pushing a captured URL prevents a mid-validation config edit from
    // redirecting the network operation. Establish local tracking only after
    // that exact object reached the captured destination successfully.
    assertDestinationUnchanged(destination);
    const trackingRef = `refs/remotes/${destination.remote}/${branch}`;
    git(["check-ref-format", trackingRef]);
    git(["update-ref", trackingRef, head]);
    git(["config", `branch.${branch}.remote`, destination.remote]);
    git(["config", `branch.${branch}.merge`, destination.remoteRef]);
  }
}

try {
  main();
} catch (error) {
  console.error(`validated-push: ${error.message}`);
  process.exitCode = 1;
}
