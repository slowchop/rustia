#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const KIND = /^[a-z][a-z0-9-]{0,31}$/;
const BENCH_DATA_REMOTE =
  /^(?:git@github\.com:|https:\/\/github\.com\/)slowchop\/rustia-bench-data(?:\.git)?$/;
const TARGET_BRANCH = "main";
// A push loses only when another publication landed between this job's clone
// and its push. Each retry rebases the single publication commit onto the
// freshly fetched main and pushes again; the loop is bounded so a branch that
// keeps moving fails the job instead of spinning.
const PUSH_ATTEMPTS = 5;
const RETRY_DELAY_MS = 5 * 1000;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { selfTest: true };
  const parsed = { repoDir: undefined, kind: undefined, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--repo-dir") parsed.repoDir = argv[++index];
    else if (option === "--kind") parsed.kind = argv[++index];
    else return null;
  }
  if (!parsed.repoDir || !parsed.kind || !KIND.test(parsed.kind)) return null;
  return parsed;
}

// Git reports a stale push as "! [rejected] ... (fetch first)" or
// "(non-fast-forward)". Anything else -- authentication, a branch rule the
// deploy key may not bypass, a network failure -- is not fixed by rebasing
// and must surface as-is.
function isStalePushRejection(stderr) {
  return /\[rejected\]/.test(stderr) && /fetch first|non-fast-forward/.test(stderr);
}

function runGit(repoDir, args, { allowFailure = false } = {}) {
  const result = spawnSync("git", ["-C", repoDir, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0 && !allowFailure) {
    const stderr = String(result.stderr || "").trim();
    fail(`git ${args.join(" ")} failed${stderr ? `: ${stderr}` : ""}`);
  }
  return {
    status: result.status,
    stdout: String(result.stdout || "").trim(),
    stderr: String(result.stderr || "").trim(),
  };
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

function headCommit(repoDir) {
  const commit = runGit(repoDir, ["rev-parse", "HEAD"]).stdout;
  if (!/^[0-9a-f]{40}$/.test(commit)) fail("could not resolve the publication commit");
  return commit;
}

async function publish({ repoDir, kind }) {
  const resolved = path.resolve(repoDir);
  if (!fs.existsSync(resolved)) fail(`repository directory does not exist: ${resolved}`);
  const repositoryRoot = path.resolve(runGit(resolved, ["rev-parse", "--show-toplevel"]).stdout);
  if (fs.realpathSync(repositoryRoot) !== fs.realpathSync(resolved)) {
    fail("--repo-dir must name the root of the benchmark-data checkout");
  }
  const remote = runGit(resolved, ["remote", "get-url", "origin"]).stdout;
  if (!BENCH_DATA_REMOTE.test(remote)) {
    fail(`origin is not slowchop/rustia-bench-data: ${remote}`);
  }

  for (let attempt = 1; attempt <= PUSH_ATTEMPTS; attempt += 1) {
    const commit = headCommit(resolved);
    process.stdout.write(
      `pushing ${kind} publication ${commit.slice(0, 12)} to rustia-bench-data ${TARGET_BRANCH}` +
        (attempt > 1 ? ` (attempt ${attempt})` : "") +
        "\n",
    );
    // A plain fast-forward push: never forced, so a stale checkout can only be
    // rejected, never overwrite what another publication landed.
    const push = runGit(resolved, ["push", "origin", `${commit}:refs/heads/${TARGET_BRANCH}`], {
      allowFailure: true,
    });
    if (push.status === 0) {
      process.stdout.write(
        `${kind} publication ${commit.slice(0, 12)} is on rustia-bench-data ${TARGET_BRANCH}\n`,
      );
      return;
    }
    if (!isStalePushRejection(push.stderr)) {
      fail(`push to rustia-bench-data ${TARGET_BRANCH} failed: ${push.stderr}`);
    }
    process.stdout.write(
      `${TARGET_BRANCH} moved under the ${kind} publication; rebasing and retrying\n`,
    );
    runGit(resolved, ["fetch", "--quiet", "origin", TARGET_BRANCH]);
    const rebase = runGit(resolved, ["rebase", "--quiet", `origin/${TARGET_BRANCH}`], {
      allowFailure: true,
    });
    if (rebase.status !== 0) {
      runGit(resolved, ["rebase", "--abort"], { allowFailure: true });
      const conflicts = rebase.stderr
        .split("\n")
        .filter((line) => /^(?:CONFLICT|error:)/.test(line))
        .join("; ");
      fail(
        `${kind} publication ${commit.slice(0, 12)} conflicts with newer commits on ` +
          `rustia-bench-data ${TARGET_BRANCH}; re-run the workflow to rebuild it from the ` +
          `current data${conflicts ? `: ${conflicts}` : ""}`,
      );
    }
    await delay(RETRY_DELAY_MS);
  }
  fail(
    `could not push the ${kind} publication after ${PUSH_ATTEMPTS} attempts; ` +
      `rustia-bench-data ${TARGET_BRANCH} kept moving`,
  );
}

function selfTest() {
  assert.deepEqual(
    parseArgs(["--repo-dir", "/tmp/data", "--kind", "series"]),
    { repoDir: "/tmp/data", kind: "series", selfTest: false },
  );
  assert.equal(parseArgs(["--repo-dir", "/tmp/data"]), null);
  assert.equal(parseArgs(["--repo-dir", "/tmp/data", "--kind", "Series"]), null);
  assert.equal(parseArgs(["--repo-dir", "/tmp/data", "--kind", "series", "extra"]), null);
  assert.deepEqual(parseArgs(["self-test"]), { selfTest: true });
  assert.equal(BENCH_DATA_REMOTE.test("git@github.com:slowchop/rustia-bench-data.git"), true);
  assert.equal(BENCH_DATA_REMOTE.test("https://github.com/slowchop/rustia-bench-data"), true);
  assert.equal(BENCH_DATA_REMOTE.test("git@github.com:someone/rustia-bench-data.git"), false);
  assert.equal(
    isStalePushRejection(
      "To github.com:slowchop/rustia-bench-data.git\n" +
        " ! [rejected]        abc123 -> main (fetch first)\n" +
        "error: failed to push some refs to 'github.com:slowchop/rustia-bench-data.git'",
    ),
    true,
  );
  assert.equal(
    isStalePushRejection(" ! [rejected]        abc123 -> main (non-fast-forward)"),
    true,
  );
  assert.equal(
    isStalePushRejection(
      "remote: error: GH013: Repository rule violations found for refs/heads/main.\n" +
        "remote: - Changes must be made through a pull request.\n" +
        " ! [remote rejected] abc123 -> main (push declined due to repository rule violations)",
    ),
    false,
  );
  assert.equal(isStalePushRejection("git@github.com: Permission denied (publickey)."), false);
  process.stdout.write("benchmark publication push self-test passed\n");
}

async function main() {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed === null) {
    process.stderr.write(
      "usage: push-benchmark-data-publication.js --repo-dir DIR --kind KIND\n" +
        "       push-benchmark-data-publication.js self-test\n",
    );
    process.exitCode = 2;
  } else if (parsed.selfTest) {
    selfTest();
  } else {
    await publish(parsed);
  }
}

main().catch((error) => {
  process.stderr.write(`push-benchmark-data-publication: ${error.message}\n`);
  process.exitCode = 1;
});
