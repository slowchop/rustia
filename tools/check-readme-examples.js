#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const path = require("path");
const fs = require("fs");
const assert = require("assert/strict");

const root = path.resolve(__dirname, "..");

function fail(message) {
  console.error(`check-readme-examples: ${message}`);
  process.exit(1);
}

// Code-fence tests use the current library, so also check the actual Cargo
// pin and repository source links readers copy. History-map aliases are not
// fetchable Git revisions and must not make a stale installation pin pass.
function verifySourceReferences(readme) {
  const pins = [...readme.matchAll(/^rustia = \{ git = "https:\/\/github\.com\/slowchop\/rustia", rev = "([a-f0-9]{40})" \}$/gm)];
  assert.equal(pins.length, 1, "README must pin one exact Rustia Git dependency");
  const git = (args) => childProcess.execFileSync("git", args, {
    cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"],
  }).trim();
  assert.equal(git(["cat-file", "-t", pins[0][1]]), "commit", "README dependency must name a real commit");
  git(["merge-base", "--is-ancestor", pins[0][1], "HEAD"]);
  const links = [...readme.matchAll(/https:\/\/github\.com\/slowchop\/rustia\/(?:blob|tree)\/(main|[a-f0-9]{40})\/([^\s)#]+)/g)];
  assert(links.length > 0, "README must link to its source documentation");
  for (const [, ref, file] of links) {
    const commit = ref === "main" ? "HEAD" : ref;
    assert.equal(git(["cat-file", "-t", commit]), "commit", `README link uses an absent revision: ${ref}`);
    git(["cat-file", "-e", `${commit}:${decodeURIComponent(file)}`]);
  }
}

try {
  const readme = fs.readFileSync(path.join(root, "README.md"), "utf8");
  verifySourceReferences(readme);
  // Catch the regression that fence tests missed: valid Rust with an invalid
  // dependency pin or immutable source path still gives unusable instructions.
  assert.throws(() => verifySourceReferences(readme.replace(/rev = "[a-f0-9]{40}"/, `rev = "${"0".repeat(40)}"`)));
  assert.throws(() => verifySourceReferences(`${readme}\nhttps://github.com/slowchop/rustia/blob/main/README-NOT-PRESENT.md`));
} catch (error) {
  fail(`README dependency or source link is invalid: ${error.message}`);
}

// Ask Cargo for the actual library path rather than assuming target/debug:
// CARGO_TARGET_DIR and platform-specific artifact names remain Cargo's concern.
let messages;
try {
  process.stdout.write("Building rustia for README examples\n");
  const output = childProcess.execFileSync(
    "cargo",
    ["build", "--locked", "-p", "rustia", "--lib", "--message-format=json"],
    {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
      maxBuffer: 16 * 1024 * 1024,
    },
  );
  messages = output.trim().split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line));
} catch (error) {
  fail(`could not build rustia or read Cargo artifact messages: ${error.message}`);
}

const library = messages.find((message) => (
  message.reason === "compiler-artifact"
  && message.target.name === "rustia"
  && message.target.kind.includes("lib")
));
const rlib = library?.filenames.find((file) => file.endsWith(".rlib"));
if (!rlib) {
  fail("Cargo did not report the rustia library artifact");
}
const dependencyDirectories = new Set(messages
  .filter((message) => message.reason === "compiler-artifact")
  .flatMap((message) => message.filenames)
  .filter((file) => /\.(?:rlib|rmeta|dylib|so|dll)$/.test(file))
  .map((file) => path.dirname(file)));

// Rustdoc reads and executes the Markdown fences itself. There is no copied
// snippet to drift from the README and no include_str! path that breaks when
// Cargo copies the workspace README into a published crate's root directory.
try {
  childProcess.execFileSync("rustdoc", [
    "--test",
    "README.md",
    "--edition=2021",
    "--extern", `rustia=${rlib}`,
    ...[...dependencyDirectories].flatMap((directory) => ["-L", `dependency=${directory}`]),
  ], {
    cwd: root,
    stdio: "inherit",
  });
} catch (_error) {
  fail("README Rust examples failed to compile or run");
}
