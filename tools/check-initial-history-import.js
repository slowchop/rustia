#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const crypto = require("node:crypto");
const {
  PREPARED_BASE, REPOSITORY, SCHEMA, ZERO_OID,
  parseManifest, validatePlan, validateRemoteReadback, validateRemoteUrl,
} = require("./initial-history-import.js");

function digest(bytes) { return crypto.createHash("sha256").update(bytes).digest("hex"); }
function fixture() {
  const refs = { "refs/heads/main": "HEAD" };
  for (let i = 0; i < 67; i += 1) refs[`refs/heads/archive/branch-${i}`] = (i + 1).toString(16).padStart(40, "0");
  for (let i = 0; i < 7; i += 1) refs[`refs/tags/tag-${i}`] = (i + 100).toString(16).padStart(40, "0");
  const head = "a".repeat(40);
  const localRefs = { ...refs, "refs/heads/main": head };
  const manifest = { schema: SCHEMA, repository: REPOSITORY, prepared_base: PREPARED_BASE, refs };
  const updates = Object.entries(refs).map(([localRef, oid]) => ({
    localRef,
    localOid: oid === "HEAD" ? head : oid,
    remoteRef: localRef,
    remoteOid: ZERO_OID,
  }));
  return { manifest, localRefs, head, updates };
}

function main() {
  const { manifest, localRefs, head, updates } = fixture();
  const bytes = Buffer.from(JSON.stringify(manifest));
  assert.deepEqual(parseManifest(bytes, digest(bytes)), manifest);
  assert.throws(() => parseManifest(bytes, "0".repeat(64)), /digest/);
  for (const changed of [
    { ...manifest, schema: "other" },
    { ...manifest, repository: "someone/else" },
    { ...manifest, prepared_base: "short" },
  ]) assert.throws(() => parseManifest(Buffer.from(JSON.stringify(changed)), digest(Buffer.from(JSON.stringify(changed)))), /schema, repository, or prepared_base/);
  const shortRefs = { ...manifest, refs: Object.fromEntries(Object.entries(manifest.refs).slice(1)) };
  assert.throws(() => parseManifest(Buffer.from(JSON.stringify(shortRefs)), digest(Buffer.from(JSON.stringify(shortRefs)))), /exactly 75 refs/);
  const badMain = { ...manifest, refs: { ...manifest.refs, "refs/heads/main": head } };
  assert.throws(() => parseManifest(Buffer.from(JSON.stringify(badMain)), digest(Buffer.from(JSON.stringify(badMain)))), /HEAD sentinel/);

  for (const url of ["https://github.com/slowchop/rustia-source", "https://github.com/slowchop/rustia-source.git", "git@github.com:slowchop/rustia-source.git", "ssh://git@github.com/slowchop/rustia-source.git"]) {
    assert.equal(validateRemoteUrl(url), undefined);
  }
  for (const url of ["https://github.com/other/rustia-source.git", "https://github.com/slowchop/rustia-source.evil", "https://github.com/slowchop/rustia-source/extra", "https://example.invalid/slowchop/rustia-source.git"]) {
    assert.throws(() => validateRemoteUrl(url), /exactly github\.com/);
  }
  assert.equal(validateRemoteReadback({
    repository: { full_name: REPOSITORY, private: true, fork: false }, actions: { enabled: false }, refs: "",
  }), undefined);
  for (const altered of [
    { repository: { full_name: REPOSITORY, private: false, fork: false }, actions: { enabled: false }, refs: "" },
    { repository: { full_name: "slowchop/other", private: true, fork: false }, actions: { enabled: false }, refs: "" },
    { repository: { full_name: REPOSITORY, private: true, fork: true }, actions: { enabled: false }, refs: "" },
    { repository: { full_name: REPOSITORY, private: true, fork: false }, actions: { enabled: true }, refs: "" },
    { repository: { full_name: REPOSITORY, private: true, fork: false }, actions: { enabled: false }, refs: "oid refs/heads/main\n" },
  ]) assert.throws(() => validateRemoteReadback(altered));

  const valid = (overrides = {}) => validatePlan({
    manifest, refs: localRefs, head, updates, isAncestor: () => true, ...overrides,
  });
  assert.equal(valid(), undefined);
  const changedUpdates = (change) => updates.map((entry, index) => index === 0 ? { ...entry, ...change } : entry);
  assert.throws(() => valid({ updates: updates.slice(1) }), /exactly 75 refs/);
  assert.throws(() => valid({ updates: [...updates.filter((_, index) => index !== 1), updates[0]] }), /duplicate push update/);
  assert.throws(() => valid({ updates: changedUpdates({ remoteOid: "b".repeat(40) }) }), /not empty/);
  assert.throws(() => valid({ updates: changedUpdates({ remoteRef: "refs/heads/unlisted" }) }), /outside the exact ref allowlist/);
  assert.throws(() => valid({ updates: changedUpdates({ localOid: "c".repeat(40) }) }), /differs from the reviewed local ref/);
  assert.throws(() => valid({ updates: changedUpdates({ remoteRef: "refs/tags/tag-0" }) }), /outside the exact ref allowlist/);
  assert.throws(() => valid({ refs: { ...localRefs, "refs/heads/extra": "d".repeat(40) } }), /exactly match/);
  assert.throws(() => valid({ refs: Object.fromEntries(Object.entries(localRefs).slice(1)) }), /exactly match/);
  assert.throws(() => valid({ refs: { ...localRefs, "refs/heads/main": "e".repeat(40) } }), /checked-out HEAD/);
  assert.throws(() => valid({ isAncestor: () => false }), /descend from the prepared f2 base/);
  assert.throws(() => valid({ updates: updates.map((entry) => ({ ...entry, remoteOid: entry.localOid })) }), /not empty/);

  console.log("Initial-history import manifest, remote-readback, and exact-ref fixtures pass.");
}

main();
