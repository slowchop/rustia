"use strict";

// Where per-VM memory accounting gets its cgroup, and why it cannot have one.
//
// Two entry points need this decision -- the cohort runner and the owned-host
// memory coordinator -- and they used to carry a copy each. The copies agreed
// until one was improved, which is the failure this module exists to prevent.
//
// The subtree is the one systemd delegates to a lingering user. That shape is
// deliberate: a systemd user manager keeps its own processes in init.scope, so
// the delegated directory holds none itself and can therefore enable controllers
// for children. A service unit's own cgroup cannot -- cgroup-v2 forbids a cgroup
// from holding processes and enabling controllers for its children at once -- so
// "just delegate the unit that runs the job" is not an alternative.

const fs = require("fs");
const path = require("path");

function subtreeFor(uid) {
  return `/sys/fs/cgroup/user.slice/user-${uid}.slice/user@${uid}.service`;
}

// Reports why the subtree is unusable, not merely that it is. Three causes need
// three different host changes and look identical from here: the subtree can be
// absent, or present and owned by this user yet unreachable because a hardened
// unit mounted /sys/fs/cgroup read-only into its namespace, or present and not
// owned. Reporting only the first sends whoever reads it to configure delegation
// that is already configured.
function delegatedCgroupRoot() {
  if (typeof process.getuid !== "function") return { root: null, reason: "unsupported" };
  const uid = process.getuid();
  if (uid === 0) return { root: null, reason: "root" };
  const candidate = subtreeFor(uid);
  if (!fs.existsSync(candidate)) return { root: null, reason: "absent" };
  try {
    fs.accessSync(path.join(candidate, "cgroup.subtree_control"), fs.constants.W_OK);
    return { root: candidate, reason: null };
  } catch (error) {
    // EROFS is a read-only mount rather than a permission problem, and it is the
    // one that inspecting the directory's ownership will not reveal.
    return { root: null, reason: error.code === "EROFS" ? "read-only" : "unwritable" };
  }
}

// Separate from the probe so a test can assert that each cause names its own
// remedy instead of the generic one.
function cgroupBlockerMessage(uid, reason) {
  const subtree = subtreeFor(uid);
  const causes = {
    absent:
      `${subtree} does not exist, so uid ${uid} has no subtree to create children in. ` +
      "Delegate one: loginctl enable-linger for that user, plus a systemd drop-in carrying " +
      "Delegate=memory.",
    "read-only":
      `${subtree} exists, but /sys/fs/cgroup is mounted read-only in this mount namespace, so ` +
      "nothing under it can be written whoever owns it. A systemd unit with " +
      "ProtectControlGroups=yes does exactly that. Keep the hardening and punch one hole: " +
      `ReadWritePaths=${subtree} on the unit that runs this job.`,
    unwritable:
      `${subtree} exists but uid ${uid} cannot write its cgroup.subtree_control. It has to be ` +
      "owned by that user, which Delegate=memory arranges.",
  };
  return (
    `per-VM memory accounting needs a writable cgroup-v2 subtree. ${causes[reason] ?? causes.absent} ` +
    "The cgroup is what the per-VM figure is measured in, so nothing about the measurement can be " +
    "salvaged without one."
  );
}

module.exports = { delegatedCgroupRoot, cgroupBlockerMessage, subtreeFor };
