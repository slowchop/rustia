#!/usr/bin/env node
"use strict";

const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const root = path.resolve(__dirname, "..");
const source = "crates/rustia/compare.md";
const page = "demos/web-live/compare.html";
const start = "<!-- rustia-luau-comparison -->";
const end = "<!-- /rustia-luau-comparison -->";

const iconMarker = "<!-- Generated platform image references -->";
const iconRevision = "6a212ab9742cb1b1c03b26bdd27c720eadcba492";

function readText(file) {
  return fs.readFileSync(file, "utf8").replaceAll("\r\n", "\n");
}

// Rustdoc renders the standalone browser page from the shared comparison.
function syncComparison(write = false) {
  const original = readText(path.join(root, source));
  const guide = original.split(iconMarker)[0].trimEnd();
  const embedded = [];
  const hosted = [];
  for (const platform of ["linux", "macos", "windows"]) {
    const file = `demos/web-live/icons/${platform}.svg`;
    const svg = readText(path.join(root, file));
    const pinned = execFileSync("git", ["show", `${iconRevision}:${file}`], { cwd: root, encoding: "utf8" });
    if (pinned !== svg) throw new Error(file + " differs from the README icon revision");
    const ref = `[rustia-${platform}]: `;
    embedded.push(ref + "data:image/svg+xml;base64," + Buffer.from(svg).toString("base64"));
    hosted.push(ref + `https://github.com/slowchop/rustia/raw/${iconRevision}/${file}`);
  }
  // The standalone browser comparison also works offline.
  const markdown = guide + "\n\n" + iconMarker + "\n\n" + embedded.join("\n") + "\n";
  const readme = readText(path.join(root, "README.md"));
  const first = readme.indexOf(start);
  const last = readme.indexOf(end);
  if (first < 0 || last < first || first !== readme.lastIndexOf(start) || last !== readme.lastIndexOf(end)) {
    throw new Error("README needs one marked Luau comparison");
  }
  const section = guide.replace(/^#/gm, "##") + "\n\n" + hosted.join("\n");
  const expected = readme.slice(0, first + start.length) + "\n\n" + section + "\n\n" + readme.slice(last);
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-compare-"));
  try {
    const chart = readText(path.join(root, "crates/rustia/performance.svg"));
    const footer = path.join(temp, "chart.html");
    fs.writeFileSync(footer, '<section aria-labelledby="performance"><h2 id="performance">Measured performance</h2>\n' + chart + '</section>\n');
    const input = path.join(temp, "compare.md");
    fs.writeFileSync(input, markdown);
    execFileSync("rustdoc", ["--markdown-no-toc", "--markdown-css", "compare.css",
      "--html-after-content", footer, "--out-dir", temp, input], { cwd: root });
    const html = readText(path.join(temp, "compare.html"))
      .replace(/[ \t]+$/gm, "")
      .replaceAll("<th>", '<th scope="col">')
      .replace(/<tr><td>(.*?)<\/td>/g, '<tr><th scope="row">$1</th>')
      .replace("<div><table>", '<div class="table-scroll" role="region" aria-label="Luau project comparison" tabindex="0"><table>');
    for (const [file, contents] of [[source, markdown], ["README.md", expected], [page, html]]) {
      const target = path.join(root, file);
      if (write) fs.writeFileSync(target, contents);
      else if (readText(target) !== contents) {
        throw new Error(file + " comparison is stale; run node tools/sync-luau-comparison.js --write");
      }
    }
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
}

if (require.main === module) {
  const args = process.argv.slice(2);
  if (args.length && !(args.length === 1 && args[0] === "--write")) {
    throw new Error("usage: node tools/sync-luau-comparison.js [--write]");
  }
  syncComparison(args.length === 1);
}
module.exports = { syncComparison };
