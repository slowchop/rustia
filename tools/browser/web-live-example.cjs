#!/usr/bin/env node
"use strict";
// Execute the documented application using its Python HTTP server and a real browser.
const assert = require("node:assert/strict");
const { spawn } = require("node:child_process");
const path = require("node:path");
const { webkit } = require("playwright");

async function main() {
  const directory = path.resolve(__dirname, "../../demos/web-live");
  const server = spawn("python3", ["-u", "-m", "http.server", "0", "--bind", "127.0.0.1", "--directory", directory], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  let browser;
  let startup = "";
  let readyResolve, readyReject;
  const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
  server.stdout.on("data", chunk => {
    startup += chunk;
    const url = startup.match(/http:\/\/127\.0\.0\.1:\d+\//)?.[0];
    if (url) readyResolve(url);
  });
  server.stderr.on("data", chunk => { startup += chunk; });
  server.once("error", readyReject);
  const exited = new Promise(resolve => server.once("close", (code, signal) => {
    readyReject(new Error(`Example server stopped (${code ?? signal}): ${startup}`));
    resolve();
  }));
  const timeout = setTimeout(() => readyReject(new Error(`Example server startup timed out: ${startup}`)), 30000);
  try {
    const url = await ready;
    clearTimeout(timeout);
    browser = await webkit.launch({ headless: true, timeout: 30000 });
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", error => errors.push(String(error)));
    await page.goto(url, { timeout: 30000 });
    for (const reload of [false, false, true]) {
      if (reload) await page.reload({ timeout: 30000 });
      await page.waitForFunction(() => !document.querySelector("#run").disabled, undefined, { timeout: 30000 });
      const printed = page.waitForEvent("console", {
        predicate: message => message.text().trimEnd() === "Hello from Luau",
        timeout: 30000,
      });
      await page.locator("#run").click();
      await printed;
      assert.equal(await page.locator("#result").textContent(), "Result: 42");
    }
    assert.deepEqual(errors, [], "Browser module or application errors");
    if (process.env.RUSTIA_EXAMPLE_SCREENSHOT) {
      await page.screenshot({ path: process.env.RUSTIA_EXAMPLE_SCREENSHOT });
    }
    console.log(`Browser embedding recipe: WebKit ${browser.version()}; returned 42 and console output verified on two calls and reload.`);
  } finally {
    clearTimeout(timeout);
    try { if (browser) await browser.close(); }
    finally {
      server.kill("SIGTERM");
      const force = setTimeout(() => server.kill("SIGKILL"), 5000);
      await exited;
      clearTimeout(force);
    }
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
