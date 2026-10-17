#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const path = require("path");

const root = path.resolve(__dirname, "..");

function fail(message) {
  console.error(`check-readme-examples: ${message}`);
  process.exit(1);
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
