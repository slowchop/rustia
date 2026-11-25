#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync, spawnSync } = require("node:child_process");

const sourceRoot = path.resolve(__dirname, "..");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-validated-push-"));
const repository = path.join(temporary, "repository");
const remote = path.join(temporary, "remote.git");
const redirectedRemote = path.join(temporary, "redirected.git");
const gitExecutable = execFileSync("sh", ["-c", "command -v git"], { encoding: "utf8" }).trim();
const environment = { ...process.env };
for (const name of Object.keys(environment)) if (name.startsWith("GIT_")) delete environment[name];

function run(command, args, cwd = repository, extraEnvironment = {}) {
  return spawnSync(command, args, {
    cwd,
    env: { ...environment, ...extraEnvironment },
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function git(args, cwd = repository) {
  return execFileSync("git", args, {
    cwd,
    env: environment,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

function commit(message, content = message) {
  fs.writeFileSync(path.join(repository, "content"), content);
  git(["add", "content"]);
  git(["commit", "-m", message]);
  return git(["rev-parse", "HEAD"]);
}

function validated(args = [], mode = "", extraEnvironment = {}) {
  return run(process.execPath, ["tools/validated-push.js", ...args], repository, {
    ...extraEnvironment,
    VALIDATED_PUSH_TEST_MODE: mode,
  });
}

function remoteHead(branch = "main") {
  const value = git(["--git-dir", remote, "rev-parse", `refs/heads/${branch}`], temporary);
  return value;
}

try {
  git(["init", "--bare", remote], temporary);
  git(["init", "--bare", redirectedRemote], temporary);
  git(["init", "-b", "main", repository], temporary);
  git(["config", "commit.gpgsign", "false"]);
  git(["config", "user.name", "Validated push test"]);
  git(["config", "user.email", "validated-push@example.invalid"]);
  git(["remote", "add", "origin", remote]);
  fs.mkdirSync(path.join(repository, "tools"));
  fs.copyFileSync(path.join(__dirname, "validated-push.js"), path.join(repository, "tools/validated-push.js"));
  fs.writeFileSync(path.join(repository, "tools/run-release-preflight.js"), `
    "use strict";
    const fs = require("node:fs");
    const cp = require("node:child_process");
    const mode = process.env.VALIDATED_PUSH_TEST_MODE;
    if (mode === "fail") process.exit(9);
    if (mode === "dirty") fs.writeFileSync("changed", "x");
    if (mode === "head") {
      fs.writeFileSync("content", "changed head");
      cp.execFileSync("git", ["add", "content"]);
      cp.execFileSync("git", ["commit", "-m", "changed during validation"]);
    }
    if (mode === "remote") {
      cp.execFileSync("git", ["--git-dir", process.env.VALIDATED_PUSH_TEST_REMOTE,
        "update-ref", "refs/heads/main", process.env.VALIDATED_PUSH_TEST_REMOTE_HEAD]);
    }
    if (mode === "remote-url") {
      cp.execFileSync("git", ["remote", "set-url", "--push", "origin",
        process.env.VALIDATED_PUSH_TEST_REDIRECT]);
    }
    if (mode === "inherited-git") {
      if (["GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_OBJECT_DIRECTORY"]
        .some(name => process.env[name] !== undefined)) process.exit(10);
      if (process.env.GIT_SSH_COMMAND !== "ssh transport fixture") process.exit(11);
    }
    if (mode === "preflight-root" && process.env.RUSTIA_PREFLIGHT_ROOT !== undefined) process.exit(12);
  `);
  fs.writeFileSync(path.join(repository, "tracked"), "fixture");
  git(["add", "."]);
  git(["commit", "-m", "fixture"]);
  const first = git(["rev-parse", "HEAD"]);

  // First push establishes the ordinary same-name upstream and bypasses the
  // local hook only after validation in the same process.
  const marker = path.join(temporary, "ordinary-hook-ran");
  fs.mkdirSync(path.join(repository, ".git", "hooks"), { recursive: true });
  fs.writeFileSync(path.join(repository, ".git", "hooks", "pre-push"), `#!/bin/sh\ntouch '${marker}'\n`);
  fs.chmodSync(path.join(repository, ".git", "hooks", "pre-push"), 0o755);
  const rejectHook = path.join(remote, "hooks", "pre-receive");
  fs.writeFileSync(rejectHook, "#!/bin/sh\nexit 1\n");
  fs.chmodSync(rejectHook, 0o755);
  let result = validated();
  assert.notEqual(result.status, 0, "rejected first push reported success");
  assert.notEqual(run("git", ["config", "--get", "branch.main.remote"]).status, 0,
    "failed first push established tracking");
  fs.rmSync(rejectHook);
  result = validated();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(remoteHead(), first);
  assert.equal(git(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"]), "origin/main");
  assert.equal(fs.existsSync(marker), false, "validated push unexpectedly invoked the duplicate hook");

  // An ordinary push still invokes the installed hook.
  const second = commit("second");
  git(["push"]);
  assert.equal(remoteHead(), second);
  assert.equal(fs.existsSync(marker), true, "ordinary push no longer invoked the hook");
  fs.rmSync(marker);

  // Existing upstream needs no remote argument and strips inherited Git
  // repository context before validation and push.
  const third = commit("third");
  result = validated([], "inherited-git", {
    GIT_DIR: path.join(repository, ".git"),
    GIT_WORK_TREE: repository,
    GIT_INDEX_FILE: path.join(repository, ".git", "index"),
    GIT_SSH_COMMAND: "ssh transport fixture",
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(remoteHead(), third);
  result = validated([], "preflight-root", {
    RUSTIA_PREFLIGHT_ROOT: path.join(temporary, "other-checkout"),
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(remoteHead(), third);

  const beforeFailures = remoteHead();
  const unpushedCandidate = commit("candidate after third");
  result = validated([], "fail");
  assert.notEqual(result.status, 0, "failed preflight pushed");
  assert.equal(remoteHead(), beforeFailures);
  result = validated([], "dirty");
  assert.notEqual(result.status, 0, "mutating preflight pushed");
  assert.equal(remoteHead(), beforeFailures);
  fs.rmSync(path.join(repository, "changed"), { force: true });
  result = validated([], "head");
  assert.notEqual(result.status, 0, "HEAD-changing preflight pushed");
  assert.equal(remoteHead(), beforeFailures);
  git(["reset", "--hard", unpushedCandidate]);

  // A remote config edit during validation aborts instead of redirecting the
  // push. Neither the captured destination nor the replacement receives it.
  result = validated([], "remote-url", {
    VALIDATED_PUSH_TEST_REDIRECT: redirectedRemote,
  });
  assert.notEqual(result.status, 0, "remote URL mutation redirected the push");
  assert.equal(remoteHead(), beforeFailures);
  assert.notEqual(
    run("git", ["--git-dir", redirectedRemote, "show-ref", "--verify", "--quiet", "refs/heads/main"], temporary).status,
    0,
  );
  git(["remote", "set-url", "--push", "origin", remote]);

  // Every dirty shape is rejected before validation.
  fs.writeFileSync(path.join(repository, "content"), "unstaged");
  assert.notEqual(validated().status, 0, "unstaged change accepted");
  git(["restore", "content"]);
  fs.writeFileSync(path.join(repository, "content"), "staged");
  git(["add", "content"]);
  assert.notEqual(validated().status, 0, "staged change accepted");
  git(["reset", "--hard", unpushedCandidate]);
  fs.writeFileSync(path.join(repository, "untracked"), "x");
  assert.notEqual(validated().status, 0, "untracked file accepted");
  fs.rmSync(path.join(repository, "untracked"));

  git(["checkout", "--detach"]);
  assert.notEqual(validated().status, 0, "detached HEAD accepted");
  git(["checkout", "main"]);

  for (const args of [
    ["--delete"], ["--force"], ["--force-with-lease", "--force-with-lease"],
    ["origin", "other"], ["HEAD:refs/heads/evil"], ["missing"], ["-remote"],
  ]) assert.notEqual(validated(args).status, 0, `unsafe arguments accepted: ${args.join(" ")}`);

  // A local branch may not inherit a differently named upstream. This avoids
  // accidentally sending a feature branch directly to main.
  git(["checkout", "-b", "feature"]);
  git(["config", "branch.feature.remote", "origin"]);
  git(["config", "branch.feature.merge", "refs/heads/main"]);
  assert.notEqual(validated().status, 0, "different-name upstream accepted");
  assert.equal(remoteHead(), beforeFailures);
  git(["checkout", "main"]);
  git(["branch", "-D", "feature"]);

  // Even if HEAD moves after the final clean check, the push sends only the
  // exact SHA captured before validation, never symbolic HEAD.
  const raceCandidate = git(["rev-parse", "HEAD"]);
  git(["checkout", "-b", "race-source"]);
  const movedHead = commit("post-check moved head", "post-check moved head");
  git(["checkout", "main"]);
  git(["branch", "-D", "race-source"]);
  const wrapperDirectory = path.join(temporary, "git-wrapper");
  fs.mkdirSync(wrapperDirectory);
  fs.writeFileSync(path.join(wrapperDirectory, "git"), `#!/bin/sh
if [ "$1" = "push" ]; then
  "${gitExecutable}" -C "$VALIDATED_PUSH_TEST_REPOSITORY" reset --hard "$VALIDATED_PUSH_TEST_MOVED_HEAD" >/dev/null
fi
exec "${gitExecutable}" "$@"
`);
  fs.chmodSync(path.join(wrapperDirectory, "git"), 0o755);
  result = validated([], "", {
    PATH: `${wrapperDirectory}:${environment.PATH}`,
    VALIDATED_PUSH_TEST_REPOSITORY: repository,
    VALIDATED_PUSH_TEST_MOVED_HEAD: movedHead,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(remoteHead(), raceCandidate, "post-check HEAD move changed the pushed commit");
  assert.equal(git(["rev-parse", "HEAD"]), movedHead);
  git(["reset", "--hard", raceCandidate]);

  // Exact force-with-lease succeeds, but only against the observed remote tip.
  git(["reset", "--hard", first]);
  const replacement = commit("replacement", "replacement");
  result = validated(["--force-with-lease"], "remote", {
    VALIDATED_PUSH_TEST_REMOTE: remote,
    VALIDATED_PUSH_TEST_REMOTE_HEAD: second,
  });
  assert.notEqual(result.status, 0, "changed remote tip defeated the exact lease");
  assert.equal(remoteHead(), second);
  result = validated(["--force-with-lease"]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(remoteHead(), replacement);
  assert.equal(git(["status", "--porcelain=v1", "--untracked-files=all"]), "");

  const unexpected = fs.readdirSync(repository).filter(name => name.includes("receipt") || name.includes("validated-push."));
  assert.deepEqual(unexpected, [], "validated push left a persistent authorization artifact");

  const hookSource = fs.readFileSync(path.join(sourceRoot, ".githooks/pre-push"), "utf8");
  const guardSource = fs.readFileSync(path.join(sourceRoot, "tools/pre-push.js"), "utf8");
  assert(hookSource.includes("tools/pre-push.js"));
  assert(guardSource.includes("tools/run-release-preflight.js"));
  assert(guardSource.includes("assertClean(head)"));

  console.log("Validated push gates and pushes exact HEAD once; ordinary push keeps the full hook.");
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
