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
// The publication is one commit rendered on one main commit (--base): the
// operations status, the package and publishability inventories and the
// current-results report all describe that commit. GitHub would merge a clean
// branch onto a newer main just as readily, and the landed files would then
// describe the main before it. So the helper merges only while main is still
// --base, and names the pushed commit (--head) in the merge request so GitHub
// refuses if the branch moved. main is read immediately before the merge
// request; the merge endpoint takes no base precondition, so a commit landing
// between those two requests is the one race this cannot close.
//
// Exit status 3 means main moved and the publication has to be rebuilt on top
// of it; the workflow's retry loop does that. Anything else that is not
// success is a failure the loop must not paper over.

const assert = require("node:assert/strict");

const TARGET_BRANCH = "main";
const BRANCH = /^matrix-publish\/[0-9a-f]{12}$/;
const COMMIT = /^[0-9a-f]{40}$/;
const EXIT_STALE = 3;
// GitHub computes mergeability after the pull request exists, and again after
// every push to its branch; the field reads null until it has.
const MERGEABILITY_POLLS = 20;
const MERGEABILITY_POLL_MS = 3 * 1000;

function fail(message) {
  throw new Error(message);
}

function parseArgs(argv) {
  if (argv.length === 1 && argv[0] === "self-test") return { selfTest: true };
  const parsed = { branch: undefined, base: undefined, head: undefined, title: undefined, selfTest: false };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--branch") parsed.branch = argv[++index];
    else if (option === "--base") parsed.base = argv[++index];
    else if (option === "--head") parsed.head = argv[++index];
    else if (option === "--title") parsed.title = argv[++index];
    else return null;
  }
  if (!parsed.branch || !BRANCH.test(parsed.branch) || !parsed.title) return null;
  if (!COMMIT.test(parsed.base || "") || !COMMIT.test(parsed.head || "") || parsed.base === parsed.head) return null;
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
    "and merges with a merge commit as soon as GitHub reports the branch mergeable, provided `main` " +
    "is still the commit the publication was rendered on. Nothing here is reviewed by hand: the " +
    "cohort was accepted by `tools/check-benchmark-results.js` before the branch was pushed."
  );
}

// The merge endpoint's answers that matter. 409 is "head branch was modified":
// the branch no longer holds the commit this run pushed. 405 covers two kinds
// of refusal. "Base branch was modified" and "Pull Request is not mergeable"
// mean main moved under the branch, which rebuilding on the current main
// fixes. Every other 405 is a rule the pull request does not satisfy -- a
// repository ruleset, a required check or review, a disabled merge method --
// and, like anything else that is not 200, is not fixed by retrying.
const STALE_MERGE_REFUSAL = /^(?:Base branch was modified|Pull Request is not mergeable)\b/i;

function classifyMergeResponse(status, message) {
  if (status === 200) return "merged";
  if (status === 409) return "stale";
  if (status === 405 && STALE_MERGE_REFUSAL.test(message)) return "stale";
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

// Ruleset refusals arrive as several paragraphs; the log wants one line.
function messageOf(json) {
  return json && typeof json.message === "string" ? json.message.replace(/\s+/g, " ").trim() : "";
}

function describe(status, json) {
  const message = messageOf(json);
  return `${status}${message ? ` ${message}` : ""}`;
}

async function findOrCreatePullRequest(io, config, branch, title) {
  const owner = config.repository.split("/")[0];
  const query = `state=open&base=${TARGET_BRANCH}&head=${encodeURIComponent(`${owner}:${branch}`)}`;
  const listed = await io.api("GET", `/repos/${config.repository}/pulls?${query}`);
  if (listed.status !== 200) fail(`listing pull requests for ${branch} failed: ${describe(listed.status, listed.json)}`);
  if (Array.isArray(listed.json) && listed.json.length > 0) {
    const existing = listed.json[0];
    io.log(`adopting open pull request #${existing.number} for ${branch}`);
    return existing;
  }
  const created = await io.api("POST", `/repos/${config.repository}/pulls`, {
    title,
    head: branch,
    base: TARGET_BRANCH,
    body: pullRequestBody(config.runUrl),
  });
  if (created.status !== 201) fail(`creating the pull request for ${branch} failed: ${describe(created.status, created.json)}`);
  io.log(`opened pull request #${created.json.number} for ${branch}`);
  return created.json;
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// An adopted pull request can still show the previous attempt's head for a
// moment after the force-push, with mergeability computed for that commit, so
// the answer counts only once the head is the commit this run pushed.
async function awaitMergeability(io, config, number, head) {
  let seen = null;
  for (let poll = 0; poll < MERGEABILITY_POLLS; poll += 1) {
    const current = await io.api("GET", `/repos/${config.repository}/pulls/${number}`);
    if (current.status !== 200) fail(`reading pull request #${number} failed: ${describe(current.status, current.json)}`);
    seen = current.json.head ? current.json.head.sha : null;
    if (seen === head && typeof current.json.mergeable === "boolean") {
      return { mergeable: current.json.mergeable, state: current.json.mergeable_state };
    }
    await io.wait(MERGEABILITY_POLL_MS);
  }
  fail(`GitHub did not report mergeability of ${head} for pull request #${number} in time (head ${seen})`);
}

async function readTargetBranch(io, config) {
  const ref = await io.api("GET", `/repos/${config.repository}/git/ref/heads/${TARGET_BRANCH}`);
  if (ref.status !== 200) fail(`reading ${TARGET_BRANCH} failed: ${describe(ref.status, ref.json)}`);
  const sha = ref.json && ref.json.object ? ref.json.object.sha : null;
  if (typeof sha !== "string" || !COMMIT.test(sha)) fail(`GitHub named no commit for ${TARGET_BRANCH}`);
  return sha;
}

async function deleteBranch(io, config, branch) {
  const removed = await io.api("DELETE", `/repos/${config.repository}/git/refs/heads/${branch}`);
  // 422 is "reference does not exist": the repository deletes merged head
  // branches itself, or a previous attempt already did.
  if (removed.status !== 204 && removed.status !== 422 && removed.status !== 404) {
    io.log(`warning: could not delete ${branch}: ${describe(removed.status, removed.json)}`);
  }
}

// io carries every effect -- the GitHub API, the poll delay, the log -- so the
// self-test can drive each branch against a scripted GitHub.
async function land(io, config, { branch, base, head, title }) {
  const pullRequest = await findOrCreatePullRequest(io, config, branch, title);
  const number = pullRequest.number;
  const readiness = await awaitMergeability(io, config, number, head);
  if (!readiness.mergeable) {
    io.log(`pull request #${number} is not mergeable (${readiness.state}); main moved under it`);
    return EXIT_STALE;
  }
  const current = await readTargetBranch(io, config);
  if (current !== base) {
    io.log(
      `${TARGET_BRANCH} is now ${current}, not ${base}, the commit the publication was rendered on; ` +
        `leaving pull request #${number} unmerged`,
    );
    return EXIT_STALE;
  }
  const merged = await io.api("PUT", `/repos/${config.repository}/pulls/${number}/merge`, {
    merge_method: "merge",
    sha: head,
  });
  const outcome = classifyMergeResponse(merged.status, messageOf(merged.json));
  if (outcome === "stale") {
    io.log(`merging pull request #${number} was refused: ${describe(merged.status, merged.json)}`);
    return EXIT_STALE;
  }
  if (outcome === "failed") {
    fail(`merging pull request #${number} failed: ${describe(merged.status, merged.json)}`);
  }
  io.log(`pull request #${number} merged as ${merged.json.sha} on ${TARGET_BRANCH}`);
  await deleteBranch(io, config, branch);
  return 0;
}

// A scripted GitHub for the self-test: each route answers from its own list,
// repeating the last answer, and every request is recorded so a test can also
// say what was never sent.
function scriptedGitHub(routes) {
  const requests = [];
  const served = new Map();
  async function api(method, route, body) {
    const key = `${method} ${route}`;
    requests.push({ key, body });
    const answers = routes[key];
    if (!answers) fail(`self-test: unexpected request ${key}`);
    const index = served.get(key) || 0;
    served.set(key, index + 1);
    return answers[Math.min(index, answers.length - 1)];
  }
  return { api, requests };
}

async function landingSelfTest() {
  const config = { repository: "slowchop/rustia", runUrl: null };
  const branch = "matrix-publish/0123456789ab";
  const base = "a".repeat(40);
  const head = "b".repeat(40);
  const other = "c".repeat(40);
  const target = { branch, base, head, title: "Publish cross-engine matrix 0123456789ab" };
  const repo = "/repos/slowchop/rustia";
  const LIST = `GET ${repo}/pulls?state=open&base=main&head=${encodeURIComponent(`slowchop:${branch}`)}`;
  const CREATE = `POST ${repo}/pulls`;
  const READ = `GET ${repo}/pulls/7`;
  const MAIN = `GET ${repo}/git/ref/heads/main`;
  const MERGE = `PUT ${repo}/pulls/7/merge`;
  const DELETE = `DELETE ${repo}/git/refs/heads/${branch}`;
  const pull = (sha, mergeable, state) => ({
    status: 200,
    json: { number: 7, head: { sha }, mergeable, mergeable_state: state },
  });
  const mainAt = (sha) => ({ status: 200, json: { ref: "refs/heads/main", object: { type: "commit", sha } } });

  async function attempt(overrides) {
    const github = scriptedGitHub({
      [LIST]: [{ status: 200, json: [] }],
      [CREATE]: [{ status: 201, json: { number: 7 } }],
      [READ]: [pull(head, true, "clean")],
      [MAIN]: [mainAt(base)],
      [MERGE]: [{ status: 200, json: { sha: "d".repeat(40), merged: true } }],
      [DELETE]: [{ status: 204, json: null }],
      ...overrides,
    });
    const lines = [];
    let waits = 0;
    const io = {
      api: github.api,
      wait: async () => {
        waits += 1;
      },
      log: (line) => lines.push(line),
    };
    let status;
    let error;
    try {
      status = await land(io, config, target);
    } catch (caught) {
      error = caught.message;
    }
    const keys = github.requests.map((request) => request.key);
    return { status, error, keys, requests: github.requests, log: lines.join("\n"), waits };
  }

  // main is still the publication's parent: merge exactly the pushed commit,
  // with main read after mergeability and immediately before the merge.
  const landed = await attempt({});
  assert.equal(landed.status, 0);
  assert.deepEqual(landed.keys, [LIST, CREATE, READ, MAIN, MERGE, DELETE]);
  assert.deepEqual(landed.requests.find((request) => request.key === MERGE).body, { merge_method: "merge", sha: head });
  assert.equal(landed.waits, 0);

  // main moved, even though the branch still merges cleanly: rebuild, and
  // never send the merge.
  const moved = await attempt({ [MAIN]: [mainAt(other)] });
  assert.equal(moved.status, EXIT_STALE);
  assert.deepEqual(moved.keys, [LIST, CREATE, READ, MAIN]);
  assert.match(moved.log, new RegExp(`main is now ${other}, not ${base}, the commit the publication was rendered on`));

  // main cannot be read: a failure, not a rebuild, and no merge.
  const unreadable = await attempt({ [MAIN]: [{ status: 404, json: { message: "Not Found" } }] });
  assert.match(unreadable.error, /^reading main failed: 404 Not Found$/);
  assert.ok(!unreadable.keys.includes(MERGE));
  const nameless = await attempt({ [MAIN]: [{ status: 200, json: { object: { sha: "not-a-commit" } } }] });
  assert.match(nameless.error, /^GitHub named no commit for main$/);
  assert.ok(!nameless.keys.includes(MERGE));

  // An adopted pull request that still shows the previous attempt's head: its
  // mergeability does not count until the head is the pushed commit.
  const adopted = await attempt({
    [LIST]: [{ status: 200, json: [{ number: 7 }] }],
    [READ]: [pull(other, true, "clean"), pull(head, null, "unknown"), pull(head, true, "clean")],
  });
  assert.equal(adopted.status, 0);
  assert.deepEqual(adopted.keys, [LIST, READ, READ, READ, MAIN, MERGE, DELETE]);
  assert.equal(adopted.waits, 2);
  assert.match(adopted.log, /adopting open pull request #7/);
  const neverSynced = await attempt({ [READ]: [pull(other, true, "clean")] });
  assert.equal(
    neverSynced.error,
    `GitHub did not report mergeability of ${head} for pull request #7 in time (head ${other})`,
  );
  assert.equal(neverSynced.waits, MERGEABILITY_POLLS);
  assert.ok(!neverSynced.keys.includes(MAIN));

  // A conflict with the moved main: rebuild without reading main or merging.
  const conflicted = await attempt({ [READ]: [pull(head, false, "dirty")] });
  assert.equal(conflicted.status, EXIT_STALE);
  assert.deepEqual(conflicted.keys, [LIST, CREATE, READ]);

  // Refusals that rebuilding fixes: main moved between the read and the merge,
  // or the branch no longer holds the pushed commit.
  for (const [status, message] of [
    [405, "Base branch was modified. Review and try the merge again."],
    [405, "Pull Request is not mergeable"],
    [409, "Head branch was modified. Review and try the merge again."],
  ]) {
    const refused = await attempt({ [MERGE]: [{ status, json: { message } }] });
    assert.equal(refused.status, EXIT_STALE, message);
    assert.match(refused.log, new RegExp(`merging pull request #7 was refused: ${status} `));
    assert.ok(!refused.keys.includes(DELETE));
  }
  // Refusals it does not: they fail at once, carrying GitHub's reason on one line.
  for (const [status, message, reason] of [
    [
      405,
      "Repository rule violations found\n\nChanges must be made through a pull request.\n\n",
      "405 Repository rule violations found Changes must be made through a pull request.",
    ],
    [405, "Merge commits are not allowed on this repository.", "405 Merge commits are not allowed on this repository."],
    [405, "", "405"],
    [403, "Resource not accessible by integration", "403 Resource not accessible by integration"],
  ]) {
    const refused = await attempt({ [MERGE]: [{ status, json: message ? { message } : null }] });
    assert.equal(refused.status, undefined, reason);
    assert.equal(refused.error, `merging pull request #7 failed: ${reason}`);
    assert.ok(!refused.keys.includes(DELETE));
  }
}

async function selfTest() {
  const branch = "matrix-publish/0123456789ab";
  const base = "a".repeat(40);
  const head = "b".repeat(40);
  const title = "Publish cross-engine matrix 0123456789ab";
  const valid = ["--branch", branch, "--base", base, "--head", head, "--title", title];
  assert.deepEqual(parseArgs(["self-test"]), { selfTest: true });
  assert.equal(parseArgs([]), null);
  assert.deepEqual(parseArgs(valid), { branch, base, head, title, selfTest: false });
  assert.equal(parseArgs(["--branch", branch, "--title", title]), null);
  assert.equal(parseArgs(["--branch", branch, "--base", base, "--title", title]), null);
  assert.equal(parseArgs(["--branch", branch, "--head", head, "--title", title]), null);
  assert.equal(parseArgs(["--branch", branch, "--base", base, "--head", head]), null);
  assert.equal(parseArgs(["--branch", "main", "--base", base, "--head", head, "--title", title]), null);
  assert.equal(parseArgs(["--branch", branch, "--base", base.slice(0, 12), "--head", head, "--title", title]), null);
  assert.equal(parseArgs(["--branch", branch, "--base", base, "--head", head.toUpperCase(), "--title", title]), null);
  assert.equal(parseArgs(["--branch", branch, "--base", base, "--head", base, "--title", title]), null);
  assert.equal(parseArgs([...valid, "--extra"]), null);

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
  assert.equal(classifyMergeResponse(200, ""), "merged");
  assert.equal(classifyMergeResponse(405, "Base branch was modified. Review and try the merge again."), "stale");
  assert.equal(classifyMergeResponse(405, "Pull Request is not mergeable"), "stale");
  assert.equal(
    classifyMergeResponse(405, "Repository rule violations found Changes must be made through a pull request."),
    "failed",
  );
  assert.equal(classifyMergeResponse(405, "Required status check \"ci\" is expected."), "failed");
  assert.equal(classifyMergeResponse(405, ""), "failed");
  // A refusal is classified by what it is, not by what it quotes.
  assert.equal(classifyMergeResponse(405, "Repository rule violations found Pull Request is not mergeable"), "failed");
  assert.equal(classifyMergeResponse(409, "Head branch was modified. Review and try the merge again."), "stale");
  assert.equal(classifyMergeResponse(403, ""), "failed");
  assert.equal(classifyMergeResponse(404, ""), "failed");
  assert.equal(classifyMergeResponse(422, ""), "failed");
  assert.equal(messageOf({ message: "a\n\nb\n" }), "a b");
  assert.equal(messageOf(null), "");
  assert.equal(EXIT_STALE, 3);

  await landingSelfTest();
  process.stdout.write("land-matrix-publication self-test passed\n");
}

async function main() {
  const parsed = parseArgs(process.argv.slice(2));
  if (parsed === null) {
    process.stderr.write(
      "usage: node tools/land-matrix-publication.js --branch matrix-publish/<12-hex>\n" +
        "         --base <commit> --head <commit> --title <text>\n" +
        "       node tools/land-matrix-publication.js self-test\n",
    );
    process.exit(2);
  }
  if (parsed.selfTest) {
    await selfTest();
    return;
  }
  const config = environment(process.env);
  const io = {
    api: client(config),
    wait: sleep,
    log: (line) => process.stdout.write(`${line}\n`),
  };
  process.exitCode = await land(io, config, parsed);
}

main().catch((error) => {
  process.stderr.write(`land-matrix-publication: ${error.message}\n`);
  process.exit(1);
});
