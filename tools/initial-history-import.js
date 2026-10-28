"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const { execFileSync } = require("node:child_process");

const REPOSITORY = "slowchop/rustia-source";
const SCHEMA = "rustia.initial-history-import.v1";
const MANIFEST_SHA256 = "0b986d9ec810f56754e90e73a0059c638637bf93f4f0ba46271f76fff44eca7f";
const PREPARED_BASE = "f2ed8568dc7c10a258d555f7e617e0c30a41787f";
const REF_COUNT = 75;
const FULL_OID = /^[0-9a-f]{40}$/;
const ZERO_OID = "0".repeat(40);

function reject(message) { throw new Error(message); }
function sha256(bytes) { return crypto.createHash("sha256").update(bytes).digest("hex"); }

function parseManifest(bytes, expectedDigest = MANIFEST_SHA256) {
  if (sha256(bytes) !== expectedDigest) reject("initial-history manifest digest does not match the reviewed allowlist");
  let manifest;
  try { manifest = JSON.parse(bytes.toString("utf8")); }
  catch (error) { reject(`initial-history manifest is not valid JSON: ${error.message}`); }
  if (manifest.schema !== SCHEMA || manifest.repository !== REPOSITORY
    || manifest.prepared_base !== PREPARED_BASE) {
    reject("initial-history manifest schema, repository, or prepared_base is invalid");
  }
  if (JSON.stringify(Object.keys(manifest).sort()) !== JSON.stringify(["prepared_base", "refs", "repository", "schema"])) {
    reject("initial-history manifest must contain only the reviewed schema fields");
  }
  const refs = manifest.refs;
  if (refs == null || typeof refs !== "object" || Array.isArray(refs) || Object.keys(refs).length !== REF_COUNT) {
    reject(`initial-history manifest must contain exactly ${REF_COUNT} refs`);
  }
  if (refs["refs/heads/main"] !== "HEAD") reject("initial-history manifest must use the literal HEAD sentinel for refs/heads/main");
  for (const [name, oid] of Object.entries(refs)) {
    if (!/^refs\/(?:heads|tags)\/[A-Za-z0-9._/-]+$/.test(name) || name.includes("..") || name.endsWith("/")) {
      reject(`initial-history manifest has invalid target ref ${JSON.stringify(name)}`);
    }
    if (name !== "refs/heads/main" && !FULL_OID.test(oid)) reject(`initial-history manifest has invalid object ID for ${name}`);
  }
  return manifest;
}

function validateRemoteUrl(url) {
  const accepted = [
    "https://github.com/slowchop/rustia-source",
    "https://github.com/slowchop/rustia-source.git",
    "git@github.com:slowchop/rustia-source.git",
    "ssh://git@github.com/slowchop/rustia-source.git",
  ];
  if (!accepted.includes(url)) reject("initial-history import remote must be exactly github.com/slowchop/rustia-source");
}

function validateRemoteReadback({ repository, actions, refs }) {
  if (repository?.full_name !== REPOSITORY || repository?.private !== true || repository?.fork !== false) {
    reject("GitHub readback must confirm the exact target repository is private and not a fork");
  }
  if (actions?.enabled !== false) reject("GitHub readback must confirm Actions are disabled for the initial ref import");
  if (typeof refs !== "string" || refs.trim() !== "") reject("initial-history import target must have no advertised refs");
}

function validatePlan({ manifest, refs, head, updates, isAncestor }) {
  if (Object.keys(refs).length !== REF_COUNT || Object.keys(refs).length !== Object.keys(manifest.refs).length
    || Object.keys(refs).some((name) => !(name in manifest.refs))
    || Object.keys(manifest.refs).some((name) => !(name in refs))) {
    reject("local heads and tags must exactly match the complete initial-history manifest");
  }
  if (refs["refs/heads/main"] !== head) reject("refs/heads/main must point at the checked-out HEAD");
  for (const [name, oid] of Object.entries(manifest.refs)) {
    const expected = name === "refs/heads/main" ? head : oid;
    if (refs[name] !== expected) reject(`local object ID for ${name} does not match the reviewed manifest`);
  }
  if (!isAncestor(manifest.prepared_base, head)) reject("main must descend from the prepared f2 base");
  if (!Array.isArray(updates) || updates.length !== REF_COUNT) reject(`initial import must update exactly ${REF_COUNT} refs`);
  const seen = new Set();
  for (const update of updates) {
    const { localRef, localOid, remoteRef, remoteOid } = update;
    if (seen.has(localRef)) reject(`duplicate push update for ${localRef}`);
    seen.add(localRef);
    if (localRef !== remoteRef || !(localRef in manifest.refs)) reject(`push update is outside the exact ref allowlist: ${localRef}`);
    if (remoteOid !== ZERO_OID) reject(`remote ref ${remoteRef} is not empty`);
    if (localOid !== refs[localRef]) reject(`push object ID for ${localRef} differs from the reviewed local ref`);
  }
  if (seen.size !== REF_COUNT || Object.keys(manifest.refs).some((name) => !seen.has(name))) {
    reject("initial import is missing one or more allowlisted refs");
  }
}

function git(args, cwd) {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
}

function localSnapshot(root) {
  const head = git(["rev-parse", "HEAD"], root).trim();
  if (!FULL_OID.test(head)) reject("HEAD is not a full commit ID");
  const refs = {};
  const text = git(["for-each-ref", "--format=%(refname) %(objectname)", "refs/heads", "refs/tags"], root).trim();
  if (text) for (const line of text.split("\n")) {
    const match = line.match(/^(refs\/(?:heads|tags)\/[A-Za-z0-9._/-]+) ([0-9a-f]{40})$/);
    if (!match || refs[match[1]]) reject("local ref inventory is malformed or duplicated");
    refs[match[1]] = match[2];
  }
  return { head, refs };
}

function verifyProjection(root) {
  const projection = require("./history-projection.js").verifyProjection(root);
  if (!projection || projection.entries.length !== 1004) reject("full history projection verification did not complete");
}

function verifyRemote(url) {
  const repository = JSON.parse(execFileSync("gh", ["api", `repos/${REPOSITORY}`], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
  const actions = JSON.parse(execFileSync("gh", ["api", `repos/${REPOSITORY}/actions/permissions`], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));
  const refs = execFileSync("git", ["ls-remote", "--refs", url], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  validateRemoteReadback({ repository, actions, refs });
}

module.exports = {
  MANIFEST_SHA256, PREPARED_BASE, REPOSITORY, SCHEMA, ZERO_OID,
  localSnapshot, parseManifest, validatePlan, validateRemoteReadback, validateRemoteUrl,
  verifyProjection, verifyRemote,
};
