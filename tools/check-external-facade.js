#!/usr/bin/env node
"use strict";

// Reuse the public integration contracts in a clean Cargo application whose
// only direct dependencies are rustia and serde. Compile extracted archives,
// not the workspace libraries or a published registry placeholder.
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");
const assert = require("node:assert/strict");

const root = path.resolve(__dirname, "..");
const crates = ["rustia", "rustia-compiler", "rustia-lexer", "rustia-parser", "rustia-vm"];
const contracts = [
  "callback_adapter_api", "facade_errors", "host_functions", "live_serde",
  "platform_golden", "settings_api", "transactional_install", "typed_invocation", "typed_objects",
];
const environment = { ...process.env, CARGO_NET_OFFLINE: "true", RUSTC_WRAPPER: "" };
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-facade-consumer-"));

function cargo(args, cwd = root, capture = false) {
  return execFileSync("cargo", args, {
    cwd, env: environment, encoding: "utf8", maxBuffer: 32 * 1024 * 1024,
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });
}

function lockedPackages(text) {
  return text.split("[[package]]").slice(1).map((block) => {
    const fields = Object.fromEntries([...block.matchAll(/^(name|version|source|checksum) = "([^"]*)"$/gm)]
      .map((match) => [match[1], match[2]]));
    return [fields.name, fields.version, fields.source ?? "", fields.checksum ?? ""].join("\t");
  });
}

try {
  const metadata = JSON.parse(cargo(["metadata", "--locked", "--no-deps", "--format-version", "1"], root, true));
  const sourceLock = fs.readFileSync(path.join(root, "Cargo.lock"), "utf8");
  const expectedPackages = new Set(lockedPackages(sourceLock));
  const serde = [...expectedPackages].find((row) => row.startsWith("serde\t")).split("\t")[1];
  const patches = crates.flatMap((name) => ["--config", `patch.crates-io.${name}.path=${JSON.stringify(path.join(root, "crates", name))}`]);
  const unpacked = path.join(temporary, "archives");
  fs.mkdirSync(unpacked);
  const packaged = new Map();
  for (const name of crates) {
    const entry = metadata.packages.find((pkg) => pkg.name === name);
    assert(entry, `missing runtime package ${name}`);
    cargo(["package", "--locked", "--offline", "--allow-dirty", "--no-verify", "-p", name, ...patches]);
    const archive = path.join(metadata.target_directory, "package", `${name}-${entry.version}.crate`);
    execFileSync("tar", ["-xf", archive, "-C", unpacked]);
    const directory = path.join(unpacked, `${name}-${entry.version}`);
    packaged.set(name, directory);
    // Archive libraries must be the exact source being reviewed. Tests and
    // package metadata receive their own canonical packaging checks.
    for (const file of fs.readdirSync(path.join(root, "crates", name, "src"), { recursive: true })) {
      const source = path.join(root, "crates", name, "src", file);
      if (fs.statSync(source).isFile()) {
        assert(fs.readFileSync(source).equals(fs.readFileSync(path.join(directory, "src", file))), `${name}/src/${file} differs in archive`);
      }
    }
  }
  const application = path.join(temporary, "application");
  fs.mkdirSync(path.join(application, "tests"), { recursive: true });
  fs.mkdirSync(path.join(application, "src"));
  fs.copyFileSync(path.join(root, "rust-toolchain.toml"), path.join(application, "rust-toolchain.toml"));
  fs.writeFileSync(path.join(application, "src", "lib.rs"), "// Clean external application; behavior comes from canonical contract tests.\n");
  const manifest = [
    "[package]", 'name = "rustia-private-consumer"', 'version = "0.0.0"', 'edition = "2021"', "publish = false", "",
    "[workspace]", "", "[features]", 'default = ["rustia/default"]', "", "[dependencies]",
    `rustia = { path = ${JSON.stringify(packaged.get("rustia"))}, default-features = false }`,
    `serde = { version = "=${serde}", features = ["derive"] }`, "", "[patch.crates-io]",
    ...crates.map((name) => `${name} = { path = ${JSON.stringify(packaged.get(name))} }`), "",
  ].join("\n");
  fs.writeFileSync(path.join(application, "Cargo.toml"), manifest);
  for (const contract of contracts) {
    const file = `${contract}.rs`;
    const source = fs.readFileSync(path.join(root, "crates/rustia/tests", file));
    const archived = fs.readFileSync(path.join(packaged.get("rustia"), "tests", file));
    assert(source.equals(archived), `packaged contract ${file} differs from source`);
    fs.writeFileSync(path.join(application, "tests", file), archived);
  }
  // Seed from the candidate's lock, then let Cargo add the application root
  // and prune unused packages offline. Any newly selected version/checksum
  // must already occur in that candidate lock. All execution is then locked.
  fs.writeFileSync(path.join(application, "Cargo.lock"), sourceLock);
  const resolved = JSON.parse(cargo(["metadata", "--offline", "--format-version", "1"], application, true));
  const app = resolved.packages.find((entry) => entry.name === "rustia-private-consumer");
  assert.deepEqual(app.dependencies.map((dependency) => dependency.name).sort(), ["rustia", "serde"]);
  assert.deepEqual(app.targets.filter((target) => target.kind.includes("test")).map((target) => target.name).sort(), contracts.slice().sort());
  for (const identity of lockedPackages(fs.readFileSync(path.join(application, "Cargo.lock"), "utf8"))) {
    assert(identity.startsWith("rustia-private-consumer\t") || expectedPackages.has(identity), `external consumer selected an unpinned package: ${identity}`);
  }
  for (const features of [[], ["--no-default-features"]]) {
    console.log(`External archived facade contracts: ${features.length ? "bare" : "default"} features`);
    cargo(["test", "--locked", "--offline", "--tests", ...features, "--", "--nocapture"], application);
  }
  console.log(`External facade acceptance passed: ${contracts.length} reused contract suites, archived runtime, default and bare features.`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
