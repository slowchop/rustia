#!/usr/bin/env node

"use strict";

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const KIND = /^[a-z][a-z0-9-]{0,31}$/;
const RUN_NUMBER = /^[1-9][0-9]*$/;
const BENCH_DATA_REMOTE =
  /^(?:git@github\.com:|https:\/\/github\.com\/)slowchop\/rustia-bench-data(?:\.git)?$/;
const MERGE_TIMEOUT_MS = 10 * 60 * 1000;
const POLL_INTERVAL_MS = 5 * 1000;

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

function publicationBranch(kind, runId, runAttempt) {
  if (!KIND.test(kind)) fail("publication kind must be a lowercase slug");
  if (!RUN_NUMBER.test(runId)) fail("GITHUB_RUN_ID must be a positive integer");
  if (!RUN_NUMBER.test(runAttempt)) fail("GITHUB_RUN_ATTEMPT must be a positive integer");
  return `publication/${kind}-${runId}-${runAttempt}`;
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

  const commit = runGit(resolved, ["rev-parse", "HEAD"]).stdout;
  if (!/^[0-9a-f]{40}$/.test(commit)) fail("could not resolve the publication commit");
  const branch = publicationBranch(
    kind,
    process.env.GITHUB_RUN_ID ?? "",
    process.env.GITHUB_RUN_ATTEMPT ?? "",
  );

  process.stdout.write(`pushing benchmark publication ${commit.slice(0, 12)} to ${branch}\n`);
  runGit(resolved, ["push", "origin", `${commit}:refs/heads/${branch}`]);

  const deadline = Date.now() + MERGE_TIMEOUT_MS;
  let lastFetchError = "";
  while (Date.now() < deadline) {
    const fetch = runGit(resolved, ["fetch", "--quiet", "origin", "main"], {
      allowFailure: true,
    });
    if (fetch.status === 0) {
      lastFetchError = "";
      const ancestor = runGit(
        resolved,
        ["merge-base", "--is-ancestor", commit, "origin/main"],
        { allowFailure: true },
      );
      if (ancestor.status === 0) {
        process.stdout.write(
          `benchmark publication ${commit.slice(0, 12)} merged into rustia-bench-data main\n`,
        );
        return;
      }
      if (ancestor.status !== 1) {
        fail(`could not test whether ${commit} reached origin/main: ${ancestor.stderr}`);
      }
    } else {
      lastFetchError = fetch.stderr;
    }
    await delay(POLL_INTERVAL_MS);
  }
  fail(
    `publication ${branch} did not merge into rustia-bench-data main within 10 minutes` +
      (lastFetchError ? `; last fetch error: ${lastFetchError}` : ""),
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
  assert.equal(publicationBranch("series", "123", "2"), "publication/series-123-2");
  assert.throws(() => publicationBranch("series", "0", "1"), /GITHUB_RUN_ID/);
  assert.throws(() => publicationBranch("bad/kind", "1", "1"), /lowercase slug/);
  assert.equal(BENCH_DATA_REMOTE.test("git@github.com:slowchop/rustia-bench-data.git"), true);
  assert.equal(BENCH_DATA_REMOTE.test("https://github.com/slowchop/rustia-bench-data"), true);
  assert.equal(BENCH_DATA_REMOTE.test("git@github.com:someone/rustia-bench-data.git"), false);
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
