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

function readText(file) {
  return fs.readFileSync(file, "utf8").replaceAll("\r\n", "\n");
}

// Rustdoc renders the same Markdown as a crate guide and as a standalone web page.
function syncComparison(write = false) {
  const markdown = readText(path.join(root, source)).trimEnd();
  const readme = readText(path.join(root, "README.md"));
  const first = readme.indexOf(start);
  const last = readme.indexOf(end);
  if (first < 0 || last < first || first !== readme.lastIndexOf(start) || last !== readme.lastIndexOf(end)) {
    throw new Error("README needs one marked Luau comparison");
  }
  const section = markdown.replace(/^#/gm, "##");
  const expected = readme.slice(0, first + start.length) + "\n\n" + section + "\n\n" + readme.slice(last);
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "rustia-compare-"));
  try {
    const chart = readText(path.join(root, "crates/rustia/performance.svg"));
    const footer = path.join(temp, "chart.html");
    fs.writeFileSync(footer, '<section aria-labelledby="performance"><h2 id="performance">Measured performance</h2>\n' + chart + '</section>\n');
    execFileSync("rustdoc", ["--markdown-no-toc", "--markdown-css", "compare.css",
      "--html-after-content", footer, "--out-dir", temp, source], { cwd: root });
    const html = readText(path.join(temp, "compare.html"))
      .replace(/[ \t]+$/gm, "")
      .replaceAll("<th>", '<th scope="col">')
      .replace(/<tr><td>(.*?)<\/td>/g, '<tr><th scope="row">$1</th>')
      .replace("<div><table>", '<div class="table-scroll" role="region" aria-label="Luau project comparison" tabindex="0"><table>');
    for (const [file, contents] of [["README.md", expected], [page, html]]) {
      const target = path.join(root, file);
      if (write) fs.writeFileSync(target, contents);
      else if (readText(target) !== contents) {
        throw new Error(file + " differs from " + source + "; run node tools/sync-luau-comparison.js --write");
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
