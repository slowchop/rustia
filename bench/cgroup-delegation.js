"use strict";

// Where per-VM memory accounting gets its cgroup, and why it cannot have one.
//
// Two entry points need this decision -- the cohort runner and the owned-host
// memory coordinator -- and they used to carry a copy each. The copies agreed
// until one was improved, which is the failure this module exists to prevent.
//
// The subtree is whichever cgroup-v2 delegation this process already sits
// inside: the topmost ancestor of its own cgroup whose cgroup.subtree_control
// it can write. It has to be an ancestor. Moving the measured child into a
// fresh measurement cgroup needs write access to cgroup.procs of the common
// ancestor of source and destination, and outside the delegated subtree that
// file belongs to root. A subtree delegated to the same user elsewhere in the
// tree -- a lingering user@<uid>.service beside the unit that runs the job --
// accepts the mkdir and refuses the move, which is how a fixed path here once
// sent a host towards delegation it could never use.
//
// systemd produces the shape in two ways. An interactive session is under
// user@<uid>.service, which delegates by default and keeps its own processes
// in init.scope. A service unit needs Delegate=memory together with
// DelegateSubgroup=: cgroup-v2 forbids a cgroup from holding processes and
// enabling controllers for its children at once, so the unit's main process
// has to live in a subgroup for the delegated root to stay empty and able to
// enable the memory controller.

const fs = require("fs");
const path = require("path");

const CGROUP_MOUNT = "/sys/fs/cgroup";
const PROC_SELF_CGROUP = "/proc/self/cgroup";

// The filesystem questions the probe asks, gathered so a test can answer them
// with a tree that does not exist and a uid that is not its own.
const defaultIo = Object.freeze({
  uid: () => (typeof process.getuid === "function" ? process.getuid() : null),
  readFile(file) {
    try {
      return fs.readFileSync(file, "utf8");
    } catch {
      return null;
    }
  },
  // null when this process may write the file, otherwise the errno name
  // access(2) gave. EROFS is the one worth telling apart: the file is ours and
  // the mount is read-only, which no amount of ownership fixes.
  writeBlocker(file) {
    try {
      fs.accessSync(file, fs.constants.W_OK);
      return null;
    } catch (error) {
      return error.code ?? "EACCES";
    }
  },
});

// The unified-hierarchy entry of /proc/self/cgroup, relative to the mount.
function ownCgroupPath(text) {
  if (text === null) return null;
  for (const line of text.split("\n")) {
    if (line.startsWith("0::")) return line.slice(3).trim();
  }
  return null;
}

function words(text) {
  return (text ?? "").trim().split(/\s+/).filter(Boolean);
}

// A candidate this user can write still has to be able to hold a measurement
// cgroup with the memory controller enabled.
function inspectRoot(root, probe, io) {
  const controllers = words(io.readFile(path.posix.join(root, "cgroup.controllers")));
  if (!controllers.includes("memory")) return { ...probe, reason: "no-memory", at: root };
  const procs = io.readFile(path.posix.join(root, "cgroup.procs"));
  if (procs !== null && words(procs).length > 0) return { ...probe, reason: "occupied", at: root };
  return { ...probe, root, reason: null };
}

// Reports why there is no usable subtree, not merely that there is none. The
// causes need different host changes and look identical from the outside:
// nothing delegated to this user on the path to its cgroup; a delegated root
// present and owned by this user yet unreachable because a hardened unit
// mounted /sys/fs/cgroup read-only into its namespace; a delegated root that
// still holds the unit's own processes; or one delegated without the memory
// controller. Reporting only the first sends whoever reads it to configure
// delegation that is already configured.
function delegatedCgroupRoot({ mount = CGROUP_MOUNT, cgroupFile = PROC_SELF_CGROUP, io = defaultIo } = {}) {
  const uid = io.uid();
  const probe = { root: null, reason: null, uid, own: null, at: null };
  if (uid === null) return { ...probe, reason: "unsupported" };
  if (uid === 0) return { ...probe, reason: "root" };
  const relative = ownCgroupPath(io.readFile(cgroupFile));
  if (relative === null) return { ...probe, reason: "no-cgroup2" };
  probe.own = path.posix.join(mount, relative);
  const segments = relative.split("/").filter(Boolean);
  let readOnlyAt = null;
  // Walk from the top so the first writable directory is the delegation
  // point itself, not a subgroup of it.
  for (let depth = 1; depth <= segments.length; depth += 1) {
    const candidate = path.posix.join(mount, ...segments.slice(0, depth));
    const blocker = io.writeBlocker(path.posix.join(candidate, "cgroup.subtree_control"));
    if (blocker === null) return inspectRoot(candidate, probe, io);
    if (blocker === "EROFS" && readOnlyAt === null) readOnlyAt = candidate;
  }
  if (readOnlyAt !== null) return { ...probe, reason: "read-only", at: readOnlyAt };
  return { ...probe, reason: "undelegated" };
}

// Separate from the probe so a test can assert that each cause names its own
// remedy instead of the generic one.
function cgroupBlockerMessage(probe) {
  const { uid, own, at, reason } = probe;
  const causes = {
    unsupported: "this platform reports no user id, so there is no cgroup-v2 hierarchy to measure in.",
    "no-cgroup2":
      `${PROC_SELF_CGROUP} has no unified-hierarchy (0::) entry, so this process is not on ` +
      "cgroup v2 and cannot be given a measurement cgroup.",
    "read-only":
      `${at} is delegated to uid ${uid}, but /sys/fs/cgroup is mounted read-only in this mount ` +
      "namespace, so nothing under it can be written whoever owns it. A systemd unit with " +
      "ProtectControlGroups=yes does exactly that; set ProtectControlGroups=no on the unit that " +
      "runs this job.",
    undelegated:
      `no ancestor of ${own} lets uid ${uid} write cgroup.subtree_control, so there is no ` +
      "delegated subtree to create the measurement cgroup in. Delegate one that contains this " +
      "process: on a systemd service that is Delegate=memory plus DelegateSubgroup= on the unit " +
      `that runs the job. A subtree delegated elsewhere, such as a lingering user@${uid}.service ` +
      "beside that unit, is no use: cgroup v2 refuses to move a process across a common ancestor " +
      "the mover cannot write.",
    occupied:
      `${at} is delegated to uid ${uid} but still holds processes, and cgroup v2 will not enable ` +
      "the memory controller for the children of a cgroup that has processes of its own. On " +
      "systemd, DelegateSubgroup= moves the unit's processes into a subgroup and leaves the " +
      "delegated root empty.",
    "no-memory":
      `${at} is delegated to uid ${uid} without the memory controller. Delegate=memory on the ` +
      "unit is what puts it there.",
  };
  return (
    "per-VM memory accounting needs a writable cgroup-v2 subtree with the memory controller: " +
    `${causes[reason] ?? causes.undelegated} The cgroup is what the per-VM figure is measured ` +
    "in, so nothing about the measurement can be salvaged without one."
  );
}

module.exports = { delegatedCgroupRoot, cgroupBlockerMessage, ownCgroupPath, CGROUP_MOUNT };
