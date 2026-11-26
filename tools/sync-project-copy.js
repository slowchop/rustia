#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const { blurb, origin, firstScriptIntro, features, upcoming } = JSON.parse(
  fs.readFileSync(path.join(root, "project-copy.json"), "utf8"),
);

const escapeHtml = (text) => text
  .replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;");

// Shared copy uses plain text with optional inline code and HTTPS links.
const inline = /`([^`]+)`|\[([^\]]+)\]\((https:\/\/[^\s)]+)\)/g;

function plainText(text) {
  return text.replace(inline, (_, code, label) => code ?? label);
}

function html(text, studioCredit = false) {
  const attributes = studioCredit ? ' class="studio-credit"' : "";
  const icon = studioCredit ? '<span class="studio-icon" aria-hidden="true"></span>' : "";
  return escapeHtml(text).replace(inline, (_, code, label, url) =>
    code !== undefined ? `<code>${code}</code>` : `<a${attributes} href="${url}">${icon}${label}</a>`);
}

function replaceBlock(source, name, start, end, replacement) {
  const begin = source.indexOf(start);
  const finish = source.indexOf(end);
  if (
    begin < 0 || finish < begin + start.length ||
    source.lastIndexOf(start) !== begin || source.lastIndexOf(end) !== finish
  ) {
    throw new Error(`${name} needs exactly one ordered pair of ${start.trim()} markers`);
  }
  return source.slice(0, begin + start.length) + replacement + source.slice(finish);
}

function readmeEntries(entries) {
  return entries.map(({ title, description, readmeLink }) => [
    "<details>",
    `<summary><strong>${escapeHtml(title)}</strong></summary>`,
    "",
    readmeLink ? `${description}\n\n${readmeLink}` : description,
    "",
    "</details>",
  ].join("\n")).join("\n\n");
}

function homepageEntries(entries) {
  return entries.map(({ title, description, icon, iconImage }) => {
    const mark = iconImage
      ? `<img src="${escapeHtml(iconImage)}" width="20" height="20" alt="">`
      : escapeHtml(icon);
    return [
      "      <div>",
      `        <dt><span class="feature-icon" aria-hidden="true">${mark}</span>${escapeHtml(title)}</dt>`,
      `        <dd>${html(description)}</dd>`,
      "      </div>",
    ].join("\n");
  }).join("\n");
}

function rustdocLines(markdown) {
  return markdown.split("\n").map((line) => line ? `//! ${line}` : "//!").join("\n");
}

function renderFirstScript() {
  const apiDocs = "https://rustia.dev/docs/api/rustia";
  const examplePath = "crates/rustia/examples/first_script.rs";
  const example = fs.readFileSync(path.join(root, examplePath), "utf8").trimEnd();
  return [
    firstScriptIntro,
    "",
    "In the Cargo project you [set up with Rustia](https://github.com/slowchop/rustia#try-it), replace `src/main.rs` with this code and run `cargo run`.",
    "",
    "```rust",
    example,
    "```",
    "",
    "You should see `The score is 42`.",
    "",
    `- [\`insert_global\`](${apiDocs}/struct.Rustia.html#method.insert_global) gives Luau the starting score, and [\`add_fn\`](${apiDocs}/struct.Rustia.html#method.add_fn) lets it call the Rust function.`,
    `- [\`source_name\`](${apiDocs}/struct.Chunk.html#method.source_name) gives the script a label that appears in error messages.`,
    `- [\`eval\`](${apiDocs}/struct.Chunk.html#method.eval) gives you a value back in Rust. Use [\`exec\`](${apiDocs}/struct.Chunk.html#method.exec) when you only want to run the script.`,
    "",
    "The example keeps the script in a string so it fits in one file. Read a `.luau` file at runtime if you want to edit it without rebuilding the Rust app.",
    "",
    `From a Rustia checkout, run [the example](https://github.com/slowchop/rustia/blob/main/${examplePath}) with \`cargo run --locked -p rustia --example first_script\`.`,
  ].join("\n");
}

function render() {
  const description = `${blurb}\n\n${origin}`;
  const firstScript = renderFirstScript();
  const sansIo = features.find(({ id }) => id === "sans-io");
  return [
    ["README.md", [
      ["<!-- project-copy: intro -->", "<!-- /project-copy: intro -->", description],
      ["<!-- project-copy: features -->", "<!-- /project-copy: features -->", readmeEntries(features)],
      ["<!-- project-copy: upcoming -->", "<!-- /project-copy: upcoming -->", readmeEntries(upcoming)],
      ["<!-- project-copy: first-script -->", "<!-- /project-copy: first-script -->", firstScript],
    ]],
    ["crates/rustia/Cargo.toml", [
      ["# project-copy: intro", "# /project-copy: intro", `description = ${JSON.stringify(plainText(description))}`],
    ]],
    ["crates/rustia/src/lib.rs", [
      ["//! <!-- project-copy: intro -->", "//! <!-- /project-copy: intro -->",
        rustdocLines(description)],
      ["//! <!-- project-copy: first-script -->", "//! <!-- /project-copy: first-script -->",
        rustdocLines(firstScript)],
      ["//! <!-- project-copy: sans-io -->", "//! <!-- /project-copy: sans-io -->",
        rustdocLines(`# ${sansIo.title}\n\n${sansIo.description}`)],
    ]],
    ["website/index.html", [
      ["    <!-- project-copy: intro -->", "    <!-- /project-copy: intro -->",
        `    <p>${html(blurb)}</p>\n    <p>${html(origin, true)}</p>`],
      ["      <!-- project-copy: features -->", "      <!-- /project-copy: features -->", homepageEntries(features)],
      ["      <!-- project-copy: upcoming -->", "      <!-- /project-copy: upcoming -->", homepageEntries(upcoming)],
    ]],
  ].map(([name, blocks]) => {
    const file = path.join(root, name);
    const current = fs.readFileSync(file, "utf8");
    const expected = blocks.reduce((source, [start, end, replacement]) =>
      replaceBlock(source, name, `${start}\n`, `\n${end}`, replacement), current);
    return { name, file, current, expected };
  });
}

const args = process.argv.slice(2);
if (args.length === 1 && args[0] === "--github-about") {
  console.log(plainText(blurb));
} else if (args.length === 0 || (args.length === 1 && args[0] === "--check")) {
  const stale = render().filter(({ current, expected }) => current !== expected);
  if (args.length) {
    if (stale.length) {
      console.error(`Shared project copy is stale in ${stale.map(({ name }) => name).join(", ")}. Run node tools/sync-project-copy.js.`);
      process.exitCode = 1;
    }
  } else {
    for (const { file, expected } of stale) fs.writeFileSync(file, expected);
  }
} else {
  console.error("Usage: node tools/sync-project-copy.js [--check | --github-about]");
  process.exitCode = 1;
}
