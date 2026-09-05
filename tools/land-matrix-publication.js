#!/usr/bin/env node

"use strict";

// Lands an accepted cross-engine matrix cohort on main through a pull request.
//
// The organisation's branch rules let nothing reach main except through a pull
// request, with no bypass for anyone, so the workflow that measures the cohort
// cannot push it. It pushes the publication commit to a branch instead and
// this helper opens (or adopts) the pull request for that branch and merges it
// with a merge commit, the only merge shape the benchmark-operations
// artifacts accept. Nothing here is reviewed by hand: the cohort was accepted
// by tools/check-benchmark-results.js before the branch was pushed.
//
// Exit status 3 means main moved and the publication has to be rebuilt on top
// of it; the workflow's retry loop does that. Anything else that is not
// success is a failure the loop must not paper over.

const assert = require("node:assert/strict");

const TARGET_BRANCH = "main";
const BRANCH = /^matrix-publish\/[0-9a-f]{12}$/;
const EXIT_STALE = 3;
// GitHub computes mergeability after the pull request exists; the field reads
// null until it has.
const MERGEABILITY_POLLS = 20;
const MERGEABILITY_POLL_MS = 3 * 1000;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { selfTest: true };
  const parsed = { branch: undefined, title: undefined, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--branch") parsed.branch = argv[++index];
    else if (option === "--title") parsed.title = argv[++index];
    else return null;
  }
  if (!parsed.branch || !BRANCH.test(parsed.branch) || !parsed.title) return null;
  return parsed;
}

function environment(env) {
  const token = env.GITHUB_TOKEN;
  const repository = env.GITHUB_REPOSITORY;
  if (!token) fail("GITHUB_TOKEN is not set");
  if (!repository || !/^[^/\s]+\/[^/\s]+$/.test(repository)) fail("GITHUB_REPOSITORY is not owner/name");
  const server = env.GITHUB_SERVER_URL || "https://github.com";
  return {
    token,
    repository,
    apiUrl: (env.GITHUB_API_URL || "https://api.github.com").replace(/\/+$/, ""),
    runUrl: env.GITHUB_RUN_ID ? `${server}/${repository}/actions/runs/${env.GITHUB_RUN_ID}` : null,
  };
}

function pullRequestBody(runUrl) {
  return (
    "Automated landing of an accepted cross-engine matrix cohort measured on the owned host.\n\n" +
    `Produced and merged by the Cross-Engine Matrix (weekly) workflow${runUrl ? ` (${runUrl})` : ""}. ` +
    "`main` accepts changes only through pull requests, so the workflow lands its cohort this way " +
    "and merges with a merge commit as soon as GitHub reports the branch mergeable. Nothing here is " +
    "reviewed by hand: the cohort was accepted by `tools/check-benchmark-results.js` before the " +
    "branch was pushed."
  );
}

// The merge endpoint's answers that matter. 405 is "not mergeable" (main moved
// under the branch and the two now conflict) and 409 is "head branch was
// modified"; both mean the publication must be rebuilt on the current main.
// Anything else that is not 200 -- a permission the token lacks, a rule the
// pull request does not satisfy -- is not fixed by retrying.
function classifyMergeResponse(status) {
  if (status === 200) return "merged";
  if (status === 405 || status === 409) return "stale";
  return "failed";
}

function client(config) {
  return async function api(method, route, body) {
    const response = await fetch(`${config.apiUrl}${route}`, {
      method,
      headers: {
        Authorization: `Bearer ${config.token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    let json = null;
    if (text) {
      try {
        json = JSON.parse(text);
      } catch {
        json = { message: text };
      }
    }
    return { status: response.status, json };
  };
}

function describe(status, json) {
  const message = json && typeof json.message === "string" ? json.message : "";
  return `${status}${message ? ` ${message}` : ""}`;
}

async function findOrCreatePullRequest(api, config, branch, title) {
  const owner = config.repository.split("/")[0];
  const query = `state=open&base=${TARGET_BRANCH}&head=${encodeURIComponent(`${owner}:${branch}`)}`;
  const listed = await api("GET", `/repos/${config.repository}/pulls?${query}`);
  if (listed.status !== 200) fail(`listing pull requests for ${branch} failed: ${describe(listed.status, listed.json)}`);
  if (Array.isArray(listed.json) && listed.json.length > 0) {
    const existing = listed.json[0];
    process.stdout.write(`adopting open pull request #${existing.number} for ${branch}\n`);
    return existing;
  }
  const created = await api("POST", `/repos/${config.repository}/pulls`, {
    title,
    head: branch,
    base: TARGET_BRANCH,
    body: pullRequestBody(config.runUrl),
  });
  if (created.status !== 201) fail(`creating the pull request for ${branch} failed: ${describe(created.status, created.json)}`);
  process.stdout.write(`opened pull request #${created.json.number} for ${branch}\n`);
  return created.json;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function awaitMergeability(api, config, number) {
  for (let poll = 0; poll < MERGEABILITY_POLLS; poll += 1) {
    const current = await api("GET", `/repos/${config.repository}/pulls/${number}`);
    if (current.status !== 200) fail(`reading pull request #${number} failed: ${describe(current.status, current.json)}`);
    if (current.json.mergeable === true) return { mergeable: true, state: current.json.mergeable_state };
    if (current.json.mergeable === false) return { mergeable: false, state: current.json.mergeable_state };
    await sleep(MERGEABILITY_POLL_MS);
  }
  fail(`GitHub did not report mergeability for pull request #${number} in time`);
}

async function deleteBranch(api, config, branch) {
  const removed = await api("DELETE", `/repos/${config.repository}/git/refs/heads/${branch}`);
  // 422 is "reference does not exist": the repository deletes merged head
  // branches itself, or a previous attempt already did.
  if (removed.status !== 204 && removed.status !== 422 && removed.status !== 404) {
    process.stderr.write(`warning: could not delete ${branch}: ${describe(removed.status, removed.json)}\n`);
  }
}

async function land(config, branch, title) {
  const api = client(config);
  const pullRequest = await findOrCreatePullRequest(api, config, branch, title);
  const readiness = await awaitMergeability(api, config, pullRequest.number);
  if (!readiness.mergeable) {
    process.stdout.write(
      `pull request #${pullRequest.number} is not mergeable (${readiness.state}); main moved under it\n`,
    );
    return EXIT_STALE;
  }
  const merged = await api("PUT", `/repos/${config.repository}/pulls/${pullRequest.number}/merge`, {
    merge_method: "merge",
  });
  const outcome = classifyMergeResponse(merged.status);
  if (outcome === "stale") {
    process.stdout.write(`merging pull request #${pullRequest.number} was refused: ${describe(merged.status, merged.json)}\n`);
    return EXIT_STALE;
  }
  if (outcome === "failed") {
    fail(`merging pull request #${pullRequest.number} failed: ${describe(merged.status, merged.json)}`);
  }
  process.stdout.write(`pull request #${pullRequest.number} merged as ${merged.json.sha} on ${TARGET_BRANCH}\n`);
  await deleteBranch(api, config, branch);
  return 0;
}

function selfTest() {
  assert.deepEqual(parseArgs(["self-test"]), { selfTest: true });
  assert.equal(parseArgs([]), null);
  assert.equal(parseArgs(["--branch", "matrix-publish/0123456789ab"]), null);
  assert.equal(parseArgs(["--branch", "main", "--title", "x"]), null);
  assert.equal(parseArgs(["--branch", "matrix-publish/0123456789ab", "--title", "x", "--extra"]), null);
  assert.deepEqual(parseArgs(["--branch", "matrix-publish/0123456789ab", "--title", "Publish cross-engine matrix 0123456789ab"]), {
    branch: "matrix-publish/0123456789ab",
    title: "Publish cross-engine matrix 0123456789ab",
    selfTest: false,
  });

  assert.throws(() => environment({}), /GITHUB_TOKEN/);
  assert.throws(() => environment({ GITHUB_TOKEN: "t" }), /GITHUB_REPOSITORY/);
  assert.throws(() => environment({ GITHUB_TOKEN: "t", GITHUB_REPOSITORY: "rustia" }), /GITHUB_REPOSITORY/);
  const config = environment({
    GITHUB_TOKEN: "t",
    GITHUB_REPOSITORY: "slowchop/rustia",
    GITHUB_API_URL: "https://api.github.com/",
    GITHUB_RUN_ID: "42",
  });
  assert.equal(config.apiUrl, "https://api.github.com");
  assert.equal(config.runUrl, "https://github.com/slowchop/rustia/actions/runs/42");
  assert.equal(environment({ GITHUB_TOKEN: "t", GITHUB_REPOSITORY: "slowchop/rustia" }).runUrl, null);
  assert.match(pullRequestBody(config.runUrl), /actions\/runs\/42/);
  assert.match(pullRequestBody(config.runUrl), /merge commit/);
  assert.match(pullRequestBody(null), /Cross-Engine Matrix \(weekly\) workflow\. /);

  // A refused merge because main moved is retried by rebuilding; a refused
  // merge for any other reason is a failure.
  assert.equal(classifyMergeResponse(200), "merged");
  assert.equal(classifyMergeResponse(405), "stale");
  assert.equal(classifyMergeResponse(409), "stale");
  assert.equal(classifyMergeResponse(403), "failed");
  assert.equal(classifyMergeResponse(404), "failed");
  assert.equal(classifyMergeResponse(422), "failed");
  assert.equal(EXIT_STALE, 3);
  process.stdout.write("land-matrix-publication self-test passed\n");
}

async function main() {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed === null) {
    process.stderr.write(
      "usage: node tools/land-matrix-publication.js --branch matrix-publish/<12-hex> --title <text>\n" +
        "       node tools/land-matrix-publication.js self-test\n",
    );
    process.exit(2);
  }
  if (parsed.selfTest) {
    selfTest();
    return;
  }
  process.exitCode = await land(environment(process.env), parsed.branch, parsed.title);
}

main().catch((error) => {
  process.stderr.write(`land-matrix-publication: ${error.message}\n`);
  process.exit(1);
});
