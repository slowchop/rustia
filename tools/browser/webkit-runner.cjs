#!/usr/bin/env node
"use strict";
// Cargo runner: serve the pinned wasm-bindgen page, then execute it in headless WebKit.
const { spawn } = require("node:child_process");
const { webkit } = require("playwright");
const { requirePassed } = require("./result.cjs");
async function main() {
  let browser;
  let readyResolve, readyReject;
  const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
  const child = spawn("wasm-bindgen-test-runner", process.argv.slice(2), {
    env: { ...process.env, NO_HEADLESS: "1", WASM_BINDGEN_USE_BROWSER: "1", WASM_BINDGEN_TEST_ADDRESS: "127.0.0.1:0" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let startup = "";
  child.stdout.on("data", chunk => {
    startup += chunk;
    const match = startup.match(/Interactive browsers tests are now available at (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) readyResolve(match[1]);
  });
  child.stderr.on("data", chunk => process.stderr.write(chunk));
  child.once("error", readyReject);
  const exited = new Promise(resolve => child.once("close", (code, signal) => {
    readyReject(new Error(`Wasm test server stopped (${code ?? signal}): ${startup}`));
    resolve();
  }));
  const timeout = setTimeout(() => readyReject(new Error(`Wasm server startup timed out: ${startup}`)), 30000);
  try {
    const url = await ready;
    clearTimeout(timeout);
    browser = await webkit.launch({ headless: true, timeout: 30000 });
    console.log(`Playwright WebKit ${browser.version()} (headless; not Safari)`);
    const page = await browser.newPage();
    const logs = [];
    page.on("console", message => logs.push(message.text()));
    page.on("pageerror", error => process.stderr.write(`${error}\n`));
    await page.goto(url, { timeout: 30000 });
    await page.waitForFunction(() => /test result:/.test(document.querySelector("#output")?.textContent ?? ""), undefined, { timeout: 120000 });
    const output = await page.locator("#output").textContent();
    process.stdout.write(output + "\n");
    // In the manual-server template, println! may go to the browser console.
    for (const line of logs.filter(line => line.startsWith("RUSTIA_PRIVATE_PLATFORM_GOLDEN "))) {
      if (!output.includes(line)) console.log(line);
    }
    requirePassed(output);
  } finally {
    clearTimeout(timeout);
    try { if (browser) await browser.close(); }
    finally {
      child.kill("SIGTERM");
      const force = setTimeout(() => child.kill("SIGKILL"), 5000);
      await exited;
      clearTimeout(force);
    }
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
