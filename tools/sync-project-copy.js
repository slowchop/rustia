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
    readmeLink ? `${description} · ${readmeLink}` : description,
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
  const examplePath = "crates/rustia/examples/first_script.rs";
  const example = fs.readFileSync(path.join(root, examplePath), "utf8").trimEnd();
  return [
    firstScriptIntro,
    "",
    "[Add Rustia to a Cargo project](https://github.com/slowchop/rustia#try-it), put this in `src/main.rs`, and run `cargo run`.",
    "",
    "```rust",
    example,
    "```",
    "",
    "The program prints `The score is 42`. Use [`eval`](https://rustia.dev/docs/api/rustia/struct.Chunk.html#method.eval) to get a value back from Luau,",
    "or [`exec`](https://rustia.dev/docs/api/rustia/struct.Chunk.html#method.exec) to discard its result.",
    "",
    `From a Rustia checkout, run [the example](https://github.com/slowchop/rustia/blob/main/${examplePath}) with \`cargo run --locked -p rustia --example first_script\`.`,
  ].join("\n");
}

function render() {
  const description = `${blurb}\n\n${origin}`;
  const firstScript = renderFirstScript();
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
