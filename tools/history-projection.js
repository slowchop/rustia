"use strict";

// A projection links historical source IDs to redacted commits. It does not
// restore original signatures, original artifact bytes, or binary identity.
// Original-side hashes are independently audited private provenance claims;
// this public verifier checks only the available projected objects.
const fs = require("node:fs");
const path = require("node:path");
const crypto = require("node:crypto");
const { execFileSync } = require("node:child_process");
const ROOT = path.resolve(__dirname, "..");
const OID = /^[0-9a-f]{40}$/;
const SHA256 = /^[0-9a-f]{64}$/;
const PROTECTED = [".cargo", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "crates", "conformance", "src", "bench/Cargo.toml", "bench/src", "bench/workloads"];
function git(args, root) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] });
}
function digest(bytes) { return crypto.createHash("sha256").update(bytes).digest("hex"); }
function loadProjection(root = ROOT) {
  const marker = path.join(root, "design/public-source-profile.json");
  if (!fs.existsSync(marker)) return null;
  const value = JSON.parse(fs.readFileSync(path.join(root, "design/history-projection.json"), "utf8"));
  if (value.schema !== "rustia.history-projection.v1" || JSON.stringify(value.protected_paths) !== JSON.stringify(PROTECTED)) throw new Error("invalid history projection schema or protected paths");
  if (!OID.test(value.original_head) || !OID.test(value.projected_head) || !Array.isArray(value.entries) || value.entries.length !== 1004) throw new Error("incomplete history projection");
  if (value.signature_counts?.original !== 879 || value.signature_counts?.projected !== 0 || !Array.isArray(value.metadata_projection) || value.metadata_projection.length !== 24) throw new Error("invalid projection audit inventory");
  const metadataPaths = new Set(value.metadata_projection.map(row => row.path));
  if (metadataPaths.size !== value.metadata_projection.length) throw new Error("duplicate metadata projection path");
  const original = new Set(), projected = new Set();
  for (const row of value.entries) {
    if (!OID.test(row.original) || !OID.test(row.projected) || !SHA256.test(row.protected_sha256) || original.has(row.original) || projected.has(row.projected)) throw new Error("invalid or duplicate projection entry");
    original.add(row.original); projected.add(row.projected);
  }
  if (!value.entries.some(row => row.original === value.original_head && row.projected === value.projected_head)) throw new Error("projection head missing from map");
  if (JSON.parse(fs.readFileSync(marker, "utf8")).historical_head !== value.projected_head) throw new Error("profile and projection heads disagree");
  return value;
}
function resolveHistoricalCommit(commit, root = ROOT) {
  const projection = loadProjection(root);
  return projection?.entries.find(row => row.original === commit)?.projected ?? commit;
}
function protectedFingerprint(commit, root) {
  const records = git(["ls-tree", "-r", "-z", commit, "--", ...PROTECTED], root).split("\0").filter(Boolean);
  records.sort((a, b) => Buffer.compare(Buffer.from(a.slice(a.indexOf("\t") + 1)), Buffer.from(b.slice(b.indexOf("\t") + 1))));
  return digest(records.length ? records.join("\0") + "\0" : "");
}
function verifyProjection(root = ROOT) {
  const value = loadProjection(root);
  if (!value) return null;
  const reachable = new Set(git(["rev-list", "--all"], root).trim().split("\n"));
  for (const row of value.entries) {
    if (!reachable.has(row.projected)) throw new Error(`projected commit unreachable: ${row.projected}`);
    if (protectedFingerprint(row.projected, root) !== row.protected_sha256) throw new Error(`protected source mismatch: ${row.projected}`);
  }
  git(["merge-base", "--is-ancestor", value.projected_head, "HEAD"], root);
  for (const row of value.metadata_projection) {
    if (typeof row.path !== "string" || path.isAbsolute(row.path) || row.path.split("/").includes("..") || !OID.test(row.original_blob) || !OID.test(row.projected_blob) || !SHA256.test(row.original_sha256) || !SHA256.test(row.projected_sha256)) throw new Error("invalid metadata projection row");
    const bytes = execFileSync("git", ["show", `${value.projected_head}:${row.path}`], { cwd: root, maxBuffer: 256 * 1024 * 1024 });
    if (digest(bytes) !== row.projected_sha256 || git(["rev-parse", `${value.projected_head}:${row.path}`], root).trim() !== row.projected_blob) throw new Error(`projected metadata mismatch: ${row.path}`);
  }
  return value;
}
module.exports = { loadProjection, resolveHistoricalCommit, verifyProjection, protectedFingerprint };
