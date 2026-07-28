#!/usr/bin/env node
"use strict";

const childProcess = require("child_process");
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const packageName = "rustia-conformance";
const patchedCrates = [
  "rustia",
  "rustia-compiler",
  "rustia-lexer",
  "rustia-parser",
  "rustia-vm",
];

function fail(message) {
  console.error(`verify-conformance-package: ${message}`);
  process.exit(1);
}

function runCargo(args) {
  try {
    childProcess.execFileSync("cargo", args, {
      cwd: root,
      env: {
        ...process.env,
        CARGO_NET_OFFLINE: "true",
        RUSTC_WRAPPER: "",
      },
      stdio: ["ignore", "inherit", "inherit"],
    });
  } catch (_error) {
    fail(`cargo ${args.join(" ")} failed`);
  }
}

function cargoMetadata() {
  try {
    return JSON.parse(
      childProcess.execFileSync(
        "cargo",
        ["metadata", "--no-deps", "--format-version", "1"],
        { cwd: root, encoding: "utf8" },
      ),
    );
  } catch (error) {
    fail(`cargo metadata failed: ${error.message}`);
  }
}

function patchArguments() {
  return patchedCrates.flatMap((name) => [
    "--config",
    `patch.crates-io.${name}.path=${JSON.stringify(path.join(root, "crates", name))}`,
  ]);
}

function filesUnder(directory, prefix = "") {
  const files = [];
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...filesUnder(absolute, relative));
    } else if (entry.isFile()) {
      files.push(relative);
    } else {
      fail(`packaged corpus contains a non-regular entry at ${relative}`);
    }
  }
  return files.sort();
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

const metadata = cargoMetadata();
const packageMetadata = metadata.packages.find((entry) => entry.name === packageName);
if (!packageMetadata) fail(`${packageName} is missing from cargo metadata`);

const patches = patchArguments();
runCargo([
  "package",
  "-p",
  packageName,
  "--allow-dirty",
  "--offline",
  ...patches,
]);

const unpacked = path.join(
  root,
  "target",
  "package",
  `${packageName}-${packageMetadata.version}`,
);
const sourceCorpus = path.join(root, "conformance");
const packagedCorpus = path.join(unpacked, "corpus", "conformance");
for (const required of ["MANIFEST.tsv", "REFS.tsv"]) {
  if (!fs.statSync(path.join(packagedCorpus, required)).isFile()) {
    fail(`packaged corpus is missing ${required}`);
  }
}

const sourceFiles = filesUnder(sourceCorpus);
const packagedFiles = filesUnder(packagedCorpus);
if (JSON.stringify(packagedFiles) !== JSON.stringify(sourceFiles)) {
  fail("packaged corpus inventory differs from the canonical conformance corpus");
}
for (const file of sourceFiles) {
  if (
    sha256(path.join(sourceCorpus, ...file.split("/"))) !==
    sha256(path.join(packagedCorpus, ...file.split("/")))
  ) {
    fail(`packaged corpus bytes differ for ${file}`);
  }
}

runCargo([
  "test",
  "--offline",
  "--manifest-path",
  path.join(unpacked, "Cargo.toml"),
  ...patches,
]);

console.log(
  `${packageName} archive verified with ${packagedFiles.length} self-contained corpus files`,
);
