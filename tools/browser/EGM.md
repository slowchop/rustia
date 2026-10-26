# Browser Correctness Runner

## Overview

Pinned Playwright WebKit support for the shared WebAssembly correctness suite. Headless WebKit execution is distinct from the manual real-Safari release check.

## Interface

- [package.json](./package.json) and [package-lock.json](./package-lock.json) pin the private runner's Playwright dependency.
- [webkit-runner.cjs](./webkit-runner.cjs) serves as Cargo's Wasm test runner: it starts `wasm-bindgen-test-runner`, opens its local test page in headless WebKit, emits results and shared golden transcripts, and checks the result.
- [result.cjs](./result.cjs) exports `requirePassed`; [result.test.cjs](./result.test.cjs) covers its acceptance and rejection cases.
- [The parent coordinator](../run-browser-correctness.sh) selects browser/mode suites and validates aggregate execution evidence.

## Invariants

- A successful result requires exactly one successful test summary with at least one passing test and zero failures. A launched browser alone is not execution evidence.
- The test server binds to loopback on an ephemeral port. Server startup, browser launch, navigation, and test completion have bounded waits.
- Browser and server cleanup runs even when execution or result validation fails.

## Side Effects

The runner starts a local test server and headless browser, executes guest correctness tests, and writes test output and diagnostics. It terminates its server during cleanup, escalating to forced termination after five seconds if needed. It does not collect performance measurements.

## Verification

From the repository root: `node --test tools/browser/result.test.cjs` validates result parsing without launching a browser; `python3 tools/check-browser-correctness.py` validates workflow/runner policy. Full browser execution follows [the platform contract](../../design/96-browser-platform-acceptance.md).
